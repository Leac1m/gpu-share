//! Roadmap suite 4 (`P4-T6`): who can call what. Five kinds of caller × the five endpoints.
//! Unpaired devices get name, status and protocol version, and nothing else.

mod common;
use common::*;

use netcore::tls::ServerPin;
use reqwest::StatusCode;

#[derive(Debug, Clone, Copy)]
enum Caller {
    NoCertificate,
    Unpaired,
    PairedRightToken,
    PairedWrongToken,
    Removed,
}

async fn call(host: &TestHost, who: Caller, method: &str, path: &str) -> StatusCode {
    let id = netcore::Identity::generate().unwrap();
    let pin = ServerPin::Fingerprint(host.core.identity.fingerprint().into());
    let tls = match who {
        Caller::NoCertificate => netcore::tls::anonymous_client_config(pin).unwrap(),
        _ => netcore::tls::client_config(&id, pin).unwrap(),
    };
    let token = match who {
        Caller::PairedRightToken | Caller::PairedWrongToken | Caller::Removed => {
            host.core.store.pair(id.fingerprint(), "Friend", "right", 1).unwrap();
            if matches!(who, Caller::Removed) {
                host.core.unpair(id.fingerprint());
            }
            if matches!(who, Caller::PairedWrongToken) { "wrong" } else { "right" }
        }
        _ => "right",
    };
    let http = reqwest::Client::builder().use_preconfigured_tls(tls).no_proxy().build().unwrap();
    let url = format!("https://{}{path}", host.addr);
    let body = match path {
        "/api/register" => serde_json::to_value(host.core.announcement(true)).unwrap(),
        "/api/pair" => serde_json::json!({"protocol": "1.0", "name": "x", "nonce": "zz"}), // invalid nonce: no popup
        _ => serde_json::json!({"model": "model-a", "messages": [{"role": "user", "content": "hi"}]}),
    };
    let req = if method == "GET" { http.get(url) } else { http.post(url).json(&body) };
    req.bearer_auth(token).send().await.unwrap().status()
}

#[tokio::test]
async fn the_access_matrix() {
    let host = start_host(&[]).await;
    use Caller::*;
    let ok = StatusCode::OK;
    let no = StatusCode::UNAUTHORIZED;
    let bad = StatusCode::BAD_REQUEST;
    // (endpoint, [no cert, unpaired, paired, wrong token, removed])
    let table: [(&str, &str, [StatusCode; 5]); 5] = [
        ("GET", "/api/info", [ok, ok, ok, ok, ok]),
        ("POST", "/api/register", [ok, ok, ok, ok, ok]),
        ("POST", "/api/pair", [bad, bad, bad, bad, bad]),
        ("GET", "/v1/models", [no, no, ok, no, no]),
        ("POST", "/v1/chat/completions", [no, no, ok, no, no]),
    ];
    for (m, path, expect) in table {
        for (who, want) in [NoCertificate, Unpaired, PairedRightToken, PairedWrongToken, Removed].into_iter().zip(expect) {
            assert_eq!(call(&host, who, m, path).await, want, "{who:?} {m} {path}");
        }
    }
}

#[tokio::test]
async fn pairing_needs_a_certificate() {
    let host = start_host(&[]).await;
    let pin = ServerPin::Fingerprint(host.core.identity.fingerprint().into());
    let http = reqwest::Client::builder()
        .use_preconfigured_tls(netcore::tls::anonymous_client_config(pin).unwrap())
        .no_proxy()
        .build()
        .unwrap();
    let r = http
        .post(format!("https://{}/api/pair", host.addr))
        .json(&serde_json::json!({"protocol": "1.0", "name": "x", "nonce": "0123456789abcdef0123456789abcdef"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(host.core.pairing.pending().is_none(), "no popup for an anonymous caller");
}
