//! `POST /api/pair` (spec §6.7). Filled in by Phase 4 (`P4-T3`).

use axum::http::StatusCode;
use axum::response::Response;
use protocol::api::ErrorKind;

use crate::gatekeeper::api_error;

pub async fn pair() -> Response {
    api_error(StatusCode::SERVICE_UNAVAILABLE, ErrorKind::InvalidRequest, "Pairing isn't available yet.")
}
