//! Pairing (`P4-T3`, `P4-T4`, spec §6.7): `POST /api/pair` and the owner's decision.
//!
//! A request needs a client certificate (its fingerprint becomes the device id). The host
//! computes the 6-digit code from its own fingerprint, the fingerprint it sees on the
//! connection, and the client's nonce, and shows it in the owner's popup. The request then
//! waits. Nothing is granted until the owner clicks Allow: only then is a device token issued
//! and the device stored. One request is shown at a time; a device the owner just turned away
//! has to wait a minute before asking again, so nobody can fill the owner's screen with popups.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use protocol::api::ErrorKind;
use protocol::pairing::{pairing_code, valid_nonce, PairRequest, PairResponse};
use rand::RngCore;
use tokio::sync::oneshot;

use crate::core::{now_unix, HostCore};
use crate::gatekeeper::{api_error, Peer};

/// How long a request waits for the owner before giving up.
pub const OWNER_TIMEOUT: Duration = Duration::from_secs(120);
/// After "Not now", the same device can't ask again for this long.
pub const DENY_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_NAME_CHARS: usize = 48;

/// The request on the owner's screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub device: String,
    pub name: String,
    pub code: String,
    seq: u64,
}

#[derive(Default)]
pub struct Pairing {
    inner: Mutex<PairingInner>,
}

#[derive(Default)]
struct PairingInner {
    pending: Option<(Pending, oneshot::Sender<bool>)>,
    denied: HashMap<String, Instant>,
    seq: u64,
}

impl Pairing {
    pub fn pending(&self) -> Option<Pending> {
        self.inner.lock().unwrap().pending.as_ref().map(|(p, _)| p.clone())
    }

    /// The owner's answer to the request on screen.
    pub fn decide(&self, allow: bool) {
        let mut inner = self.inner.lock().unwrap();
        if let Some((p, tx)) = inner.pending.take() {
            if !allow {
                inner.denied.insert(p.device.clone(), Instant::now());
            }
            let _ = tx.send(allow);
        }
    }

    fn open(&self, device: &str, name: String, code: String) -> Result<(u64, oneshot::Receiver<bool>), Refusal> {
        let mut inner = self.inner.lock().unwrap();
        inner.denied.retain(|_, at| at.elapsed() < DENY_COOLDOWN);
        if inner.denied.contains_key(device) {
            return Err(Refusal::CoolingDown);
        }
        if let Some((p, _)) = &inner.pending {
            if p.device != device {
                return Err(Refusal::Busy);
            }
        }
        inner.seq += 1;
        let seq = inner.seq;
        let (tx, rx) = oneshot::channel();
        // the same device asking again replaces its own earlier request
        inner.pending = Some((Pending { device: device.into(), name, code, seq }, tx));
        Ok((seq, rx))
    }

    /// Takes the request off the owner's screen if it is still this one (client gave up).
    fn close(&self, seq: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.pending.as_ref().is_some_and(|(p, _)| p.seq == seq) {
            inner.pending = None;
        }
    }
}

enum Refusal {
    Busy,
    CoolingDown,
}

/// Clears the popup if the handler ends for any reason (timeout, client disconnected).
struct CloseOnDrop<'a>(&'a Pairing, u64);

impl Drop for CloseOnDrop<'_> {
    fn drop(&mut self) {
        self.0.close(self.1);
    }
}

/// A device name as the owner will see it: printable, trimmed, bounded.
pub fn clean_name(raw: &str) -> String {
    let s: String = raw.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(MAX_NAME_CHARS).collect();
    if s.is_empty() {
        "A computer".into()
    } else {
        s
    }
}

fn new_token() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub async fn pair(State(core): State<Arc<HostCore>>, Extension(peer): Extension<Peer>, body: Bytes) -> Response {
    let Some(device) = peer.fingerprint else {
        return api_error(StatusCode::BAD_REQUEST, ErrorKind::InvalidRequest, "Pairing needs this computer's certificate.");
    };
    let req: PairRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return api_error(StatusCode::BAD_REQUEST, ErrorKind::InvalidRequest, "Bad pairing request."),
    };
    if !valid_nonce(&req.nonce) || !protocol::compatible(&req.protocol) {
        return api_error(StatusCode::BAD_REQUEST, ErrorKind::InvalidRequest, "Bad pairing request.");
    }
    let name = clean_name(&req.name);
    let code = pairing_code(core.identity.fingerprint(), &device, &req.nonce);
    let (seq, decision) = match core.pairing.open(&device, name.clone(), code) {
        Ok(x) => x,
        Err(Refusal::Busy) => {
            return api_error(StatusCode::CONFLICT, ErrorKind::Busy, "The owner is answering another request. Try again in a minute.")
        }
        Err(Refusal::CoolingDown) => {
            return api_error(StatusCode::FORBIDDEN, ErrorKind::Denied, "The owner said not now. Wait a minute before asking again.")
        }
    };
    let _close = CloseOnDrop(&core.pairing, seq);
    match tokio::time::timeout(OWNER_TIMEOUT, decision).await {
        Ok(Ok(true)) => {
            let token = new_token();
            if let Err(e) = core.store.pair(&device, &name, &token, now_unix()) {
                log::error!("could not save a paired device: {e}");
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, ErrorKind::InvalidRequest, "The host couldn't save the pairing.");
            }
            core.toast("Connected", format!("{name} can now ask questions."));
            Json(PairResponse { token }).into_response()
        }
        Ok(Ok(false)) | Ok(Err(_)) => api_error(StatusCode::FORBIDDEN, ErrorKind::Denied, "The owner said not now."),
        Err(_) => api_error(StatusCode::REQUEST_TIMEOUT, ErrorKind::Denied, "The owner didn't answer in time."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_for_the_popup() {
        assert_eq!(clean_name("  Tunde's PC\n"), "Tunde's PC");
        assert_eq!(clean_name("\u{7}\u{1b}[31m"), "[31m");
        assert_eq!(clean_name(""), "A computer");
        assert_eq!(clean_name(&"x".repeat(500)).len(), MAX_NAME_CHARS);
    }

    #[test]
    fn one_request_on_screen_at_a_time() {
        let p = Pairing::default();
        let (_, _rx) = p.open("a", "A".into(), "111111".into()).ok().unwrap();
        assert!(matches!(p.open("b", "B".into(), "222222".into()), Err(Refusal::Busy)));
        assert!(p.open("a", "A".into(), "333333".into()).is_ok(), "the same device may ask again");
        assert_eq!(p.pending().unwrap().code, "333333");
    }

    #[test]
    fn a_turned_away_device_waits_before_asking_again() {
        let p = Pairing::default();
        let (_, _rx) = p.open("a", "A".into(), "1".into()).ok().unwrap();
        p.decide(false);
        assert!(p.pending().is_none());
        assert!(matches!(p.open("a", "A".into(), "1".into()), Err(Refusal::CoolingDown)));
        assert!(p.open("b", "B".into(), "2".into()).is_ok(), "others aren't affected");
    }

    #[test]
    fn closing_only_removes_its_own_request() {
        let p = Pairing::default();
        let (old, _rx1) = p.open("a", "A".into(), "1".into()).ok().unwrap();
        let (_new, _rx2) = p.open("a", "A".into(), "2".into()).ok().unwrap();
        p.close(old);
        assert_eq!(p.pending().unwrap().code, "2");
    }
}
