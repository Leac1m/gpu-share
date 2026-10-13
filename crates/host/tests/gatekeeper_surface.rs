//! Roadmap suite 3: the gatekeeper serves exactly the five endpoints in spec §6.8, and 404s
//! everything else. Suite 4 (first part): unpaired devices can read name and status only.

mod common;
use common::*;

use reqwest::{Method, StatusCode};

#[tokio::test]
async fn only_the_five_endpoints_exist() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    let http = raw(&host, &me.id);
    let base = format!("https://{}", host.addr);

    let plausible = [
        (Method::GET, "/"),
        (Method::GET, "/index.html"),
        (Method::GET, "/health"),
        (Method::GET, "/metrics"),
        (Method::GET, "/props"),
        (Method::POST, "/props"),
        (Method::GET, "/slots"),
        (Method::POST, "/slots/0?action=save"),
        (Method::POST, "/completion"),
        (Method::POST, "/completions"),
        (Method::POST, "/v1/completions"),
        (Method::POST, "/v1/embeddings"),
        (Method::POST, "/embedding"),
        (Method::POST, "/tokenize"),
        (Method::POST, "/detokenize"),
        (Method::POST, "/apply-template"),
        (Method::POST, "/infill"),
        (Method::POST, "/rerank"),
        (Method::GET, "/lora-adapters"),
        (Method::GET, "/v1/models/model-a"),
        (Method::GET, "/api/files"),
        (Method::GET, "/api/info/../../etc/passwd"),
        (Method::GET, "/api/shell"),
        (Method::POST, "/api/exec"),
        (Method::GET, "/api/screen"),
        (Method::GET, "/models"),
        (Method::POST, "/upload"),
        (Method::GET, "/v1/chat/completions"),
        (Method::DELETE, "/v1/chat/completions"),
        (Method::PUT, "/api/pair"),
        (Method::POST, "/api/info"),
        (Method::GET, "/api/register"),
        (Method::OPTIONS, "/v1/models"),
        (Method::GET, "/API/INFO"),
        (Method::GET, "/api/info/"),
    ];
    for (m, p) in plausible {
        let r = http.request(m.clone(), format!("{base}{p}")).bearer_auth("token-x").send().await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{m} {p} must not exist");
    }

    assert_eq!(http.get(format!("{base}/api/info")).send().await.unwrap().status(), StatusCode::OK);
    assert_ne!(http.post(format!("{base}/api/register")).send().await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_ne!(http.post(format!("{base}/api/pair")).send().await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_ne!(http.get(format!("{base}/v1/models")).send().await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_ne!(http.post(format!("{base}/v1/chat/completions")).send().await.unwrap().status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unpaired_devices_see_name_and_status_and_nothing_else() {
    let host = start_host(&[]).await;
    let stranger = host.client(false);
    let info = stranger.api.info().await.unwrap();
    assert_eq!(info.name, "Test host");
    assert_eq!(info.protocol, protocol::PROTOCOL_VERSION);
    assert_eq!(info.id, host.core.identity.fingerprint());
    assert!(info.status.is_some());

    let kind = |e: client::api::ClientError| e.api_kind();
    assert_eq!(kind(stranger.api.models().await.unwrap_err()), Some(protocol::api::ErrorKind::Unauthorized));
    assert_eq!(
        kind(stranger.api.chat(&chat_req("model-a", "hi")).await.err().unwrap()),
        Some(protocol::api::ErrorKind::Unauthorized)
    );
}

#[tokio::test]
async fn a_stolen_token_from_another_certificate_is_useless() {
    let host = start_host(&[]).await;
    let _victim = host.client(true);
    let victim_token = format!("token-{}", &_victim.id.fingerprint()[..8]);
    let thief = host.client_with_token(&victim_token);
    assert!(thief.api.models().await.is_err(), "the token only works with the certificate it was issued to");
    assert!(_victim.api.models().await.is_ok());
}

#[tokio::test]
async fn a_paired_device_with_the_wrong_token_is_refused() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    let wrong = client::api::HostClient::new(
        &me.id,
        host.addr,
        netcore::tls::ServerPin::Fingerprint(host.core.identity.fingerprint().into()),
        Some("not-my-token".into()),
    )
    .unwrap();
    assert!(wrong.models().await.is_err());
}

#[tokio::test]
async fn paired_devices_get_the_offered_models() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    let ids: Vec<String> = me.api.models().await.unwrap().data.into_iter().map(|m| m.id).collect();
    assert_eq!(ids, vec!["model-a", "model-b"]);
    host.core.update_settings(|s| s.disabled.push("model-b".into()));
    let ids: Vec<String> = me.api.models().await.unwrap().data.into_iter().map(|m| m.id).collect();
    assert_eq!(ids, vec!["model-a"], "switched-off models are hidden from friends");
}

#[tokio::test]
async fn when_sharing_stops_the_port_closes() {
    let mut host = start_host(&[]).await;
    let me = host.client(true);
    assert!(me.api.info().await.is_ok());
    host.gk.take().unwrap().stop().await;
    assert!(matches!(me.api.info().await, Err(client::api::ClientError::Network(_))));
}

#[tokio::test]
async fn an_oversized_body_is_refused() {
    let host = start_host(&[]).await;
    let me = host.client(true);
    let big = "x".repeat(2 * 1024 * 1024);
    let r = raw(&host, &me.id)
        .post(format!("https://{}/v1/chat/completions", host.addr))
        .bearer_auth(format!("token-{}", &me.id.fingerprint()[..8]))
        .header("content-type", "application/json")
        .body(format!("{{\"model\":\"model-a\",\"messages\":[{{\"role\":\"user\",\"content\":\"{big}\"}}]}}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
