//! LAN discovery messages. See spec §6.6 and roadmap Phase 3.

use serde::{Deserialize, Serialize};

/// The modes an install can offer. A machine with a supported NVIDIA GPU can
/// offer both; every install offers at least `Use`. See spec §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Share,
    Use,
}

/// What a Share-mode host is doing right now. `Off` is never announced: an
/// off host stops announcing entirely (spec §6.3, §6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareStatus {
    Available,
    Busy,
    Paused,
}

/// The operating system an install is running on, sent so the client can
/// show a sensible icon and so future OS-specific quirks can be reasoned
/// about without a separate lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Windows,
    Linux,
}

/// The announcement message sent by both hosts and clients over UDP
/// multicast, and used as the `POST /api/register` reply body. Matches the
/// JSON shape in spec §6.6 exactly — the golden fixture in
/// `tests/fixtures/announcement.json` is the contract; do not change either
/// without updating the other and bumping `PROTOCOL_VERSION` if the change
/// isn't backward compatible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Announcement {
    pub protocol: String,
    /// SHA-256 fingerprint of this device's certificate (spec §6.7). This is
    /// the device's permanent identity, not a random session id.
    pub id: String,
    pub name: String,
    pub os: Os,
    pub app_version: String,
    pub modes: Vec<Mode>,
    /// Only present for devices currently in Share mode (spec §6.6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ShareStatus>,
    pub port: u16,
    pub announce: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example announcement from spec §6.6, checked in as a golden
    /// fixture so a change to this struct's `serde` shape is caught here
    /// rather than as a silent handshake failure between two installs.
    fn fixture() -> &'static str {
        include_str!("../tests/fixtures/announcement.json")
    }

    #[test]
    fn deserializes_the_spec_example() {
        let a: Announcement = serde_json::from_str(fixture()).expect("valid announcement JSON");
        assert_eq!(a.protocol, "1.0");
        assert_eq!(a.name, "Friend's Laptop");
        assert_eq!(a.os, Os::Windows);
        assert_eq!(a.modes, vec![Mode::Share, Mode::Use]);
        assert_eq!(a.status, Some(ShareStatus::Available));
        assert!(a.announce);
    }

    #[test]
    fn round_trips_through_json() {
        let a: Announcement = serde_json::from_str(fixture()).unwrap();
        let re_serialized = serde_json::to_string(&a).unwrap();
        let re_parsed: Announcement = serde_json::from_str(&re_serialized).unwrap();
        assert_eq!(a, re_parsed);
    }

    #[test]
    fn status_is_omitted_for_use_only_devices() {
        let mut a: Announcement = serde_json::from_str(fixture()).unwrap();
        a.status = None;
        a.modes = vec![Mode::Use];
        let json = serde_json::to_value(&a).unwrap();
        assert!(
            json.get("status").is_none(),
            "a Use-only device must not send a status field (spec \u{a7}6.6)"
        );
    }
}
