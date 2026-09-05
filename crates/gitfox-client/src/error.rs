//! Typed errors for the GitFox API client.
//!
//! The CLI maps these onto its own stable, machine readable error codes, so
//! variants here describe *what went wrong at the API layer* and nothing about
//! presentation or process exit codes.

use serde_json::Value;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No credentials were supplied for a request that needs them.
    #[error("GitFox authentication is required")]
    AuthRequired,

    /// Credentials were supplied but rejected (401/403).
    #[error("GitFox authentication failed")]
    AuthFailed,

    /// The server answered 404 for the requested resource.
    #[error("{kind} not found: {reference}")]
    NotFound {
        kind: &'static str,
        reference: String,
    },

    /// The request did not complete within the configured timeout.
    #[error("request timed out after {0}s")]
    Timeout(u64),

    /// DNS, TLS or connection level failure.
    #[error("network error: {0}")]
    Network(String),

    /// The server asked us to slow down (429), optionally saying for how long.
    #[error("rate limited by the server: {message}")]
    RateLimited {
        retry_after: Option<u64>,
        message: String,
    },

    /// Any other non-success HTTP status.
    #[error("API error (HTTP {status}): {message}")]
    Api {
        status: u16,
        message: String,
        body: Option<Value>,
    },

    /// The configured host could not be turned into a usable base URL.
    #[error("invalid GitFox host URL: {0}")]
    InvalidUrl(String),

    /// The response body was not the JSON we expected.
    #[error("could not decode response: {0}")]
    Decode(String),

    /// The client itself could not be constructed.
    #[error("could not build HTTP client: {0}")]
    Builder(String),
}

impl Error {
    /// Best-effort extraction of a human message from a GitFox error body.
    pub(crate) fn message_from_body(body: &Option<Value>, fallback: &str) -> String {
        let Some(value) = body else {
            return fallback.to_string();
        };
        for key in ["message", "error", "detail"] {
            if let Some(s) = value.get(key).and_then(Value::as_str)
                && !s.is_empty()
            {
                return s.to_string();
            }
        }
        fallback.to_string()
    }
}
