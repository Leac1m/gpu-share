//! The gatekeeper's exposed surface (spec §6.8), defined here — not inside
//! the gatekeeper crate itself — so it is a *contract* both sides can check
//! against, not an implementation detail one crate could quietly widen.
//!
//! Spec §7 promises "there is no file, shell, or remote-desktop endpoint to
//! misuse." [`ENDPOINTS`] is the executable form of that promise: when the
//! real gatekeeper is built (roadmap `P2-T5`), its router setup should be
//! built *from* this table, and its test suite should assert that every
//! path not in this table 404s. See roadmap.md, "What to automate vs.
//! verify by hand", item 3.

/// An HTTP method, kept tiny on purpose — the gatekeeper never needs PUT,
/// PATCH or DELETE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// Who is allowed to call an endpoint, from least to most trusted. See spec
/// §6.7 (pairing) and §6.8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    /// Anyone who can reach the host on the LAN, paired or not.
    Anyone,
    /// Only a device whose certificate the host has paired with and not
    /// removed.
    Paired,
}

#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    pub method: Method,
    pub path: &'static str,
    pub access: Access,
}

/// The complete, exact list of endpoints the gatekeeper serves. Nothing
/// else. If a task needs a new endpoint, it starts by adding a row here,
/// which is also where the reviewer's attention should go — a new row
/// widens the network's reach into the host and should be justified against
/// spec §7 before the handler is written.
pub const ENDPOINTS: &[Endpoint] = &[
    Endpoint { method: Method::Get, path: "/api/info", access: Access::Anyone },
    Endpoint { method: Method::Post, path: "/api/register", access: Access::Anyone },
    Endpoint { method: Method::Post, path: "/api/pair", access: Access::Anyone },
    Endpoint { method: Method::Get, path: "/v1/models", access: Access::Paired },
    Endpoint { method: Method::Post, path: "/v1/chat/completions", access: Access::Paired },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_spec_section_6_8_exactly() {
        let paths: Vec<&str> = ENDPOINTS.iter().map(|e| e.path).collect();
        assert_eq!(
            paths,
            vec![
                "/api/info",
                "/api/register",
                "/api/pair",
                "/v1/models",
                "/v1/chat/completions",
            ],
            "the gatekeeper's endpoint table has drifted from spec \u{a7}6.8 \u{2014} \
             update the spec first if this is intentional"
        );
    }

    #[test]
    fn only_the_chat_and_model_endpoints_require_pairing() {
        let paired: Vec<&str> = ENDPOINTS
            .iter()
            .filter(|e| e.access == Access::Paired)
            .map(|e| e.path)
            .collect();
        assert_eq!(paired, vec!["/v1/models", "/v1/chat/completions"]);
    }

    // NB: there is no "unknown paths 404" test here — that belongs to the
    // future `gatekeeper` crate, exercised against the real HTTP server
    // (roadmap P2-T5). This crate only owns the contract, not the server.
}
