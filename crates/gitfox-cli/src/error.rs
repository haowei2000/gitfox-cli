//! The CLI's stable error contract.
//!
//! Two things here are public API and must not churn between releases:
//! the string in [`ErrorCode::as_str`] and the number in
//! [`ErrorCode::exit_code`]. Agents and CI branch on both.

use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, CliError>;

// Every variant is part of the published contract (`docs/exit-codes.md`).
// `NotImplemented` currently has no caller — every command in the surface is
// wired up — but it stays: removing a documented code would break a consumer
// that switches on the full set.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    AuthRequired,
    AuthFailed,
    NotFound,
    RepoNotFound,
    PrNotFound,
    PipelineNotFound,
    InvalidArgument,
    ApiError,
    NetworkError,
    Timeout,
    RateLimited,
    ConfigError,
    GitContextError,
    NotImplemented,
    /// A gh flag or command whose feature GitFox does not have.
    Unsupported,
    /// `fx pr checks`: a check failed. Exit 1, as `gh pr checks` does.
    ChecksFailed,
    /// `fx pr checks`: checks are still running. Exit 8, as `gh pr checks`
    /// does.
    ChecksPending,
    /// `fx run view|watch --exit-status`: the run did not succeed. Exit 1, as
    /// gh does.
    RunFailed,
    /// A confirmation was declined.
    Cancelled,
    Unexpected,
}

impl ErrorCode {
    /// The `error.code` value emitted in JSON output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthRequired => "AUTH_REQUIRED",
            Self::AuthFailed => "AUTH_FAILED",
            Self::NotFound => "NOT_FOUND",
            Self::RepoNotFound => "REPO_NOT_FOUND",
            Self::PrNotFound => "PR_NOT_FOUND",
            Self::PipelineNotFound => "PIPELINE_NOT_FOUND",
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::ApiError => "API_ERROR",
            Self::NetworkError => "NETWORK_ERROR",
            Self::Timeout => "TIMEOUT",
            Self::RateLimited => "RATE_LIMITED",
            Self::ConfigError => "CONFIG_ERROR",
            Self::GitContextError => "GIT_CONTEXT_ERROR",
            Self::NotImplemented => "NOT_IMPLEMENTED",
            Self::Unsupported => "UNSUPPORTED",
            Self::ChecksFailed => "CHECKS_FAILED",
            Self::ChecksPending => "CHECKS_PENDING",
            Self::RunFailed => "RUN_FAILED",
            Self::Cancelled => "CANCELLED",
            Self::Unexpected => "UNEXPECTED",
        }
    }

    /// The process exit code. See `docs/exit-codes.md`.
    ///
    /// `CHECKS_FAILED`, `CHECKS_PENDING` and `RUN_FAILED` reuse the numbers gh
    /// gives the same outcomes, so a script ported from gh keeps working; the
    /// `error.code` string is what tells them apart from `UNEXPECTED` and
    /// `GIT_CONTEXT_ERROR`.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Unexpected | Self::ChecksFailed | Self::RunFailed => 1,
            Self::InvalidArgument | Self::Cancelled => 2,
            Self::AuthRequired | Self::AuthFailed => 3,
            Self::NotFound | Self::RepoNotFound | Self::PrNotFound | Self::PipelineNotFound => 4,
            Self::ApiError => 5,
            Self::NetworkError | Self::Timeout | Self::RateLimited => 6,
            Self::ConfigError => 7,
            Self::GitContextError | Self::ChecksPending => 8,
            Self::NotImplemented | Self::Unsupported => 9,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CliError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<Value>,
    /// Shown to humans only; never part of the JSON contract.
    pub hint: Option<String>,
    /// The command already printed everything a person needs, and only the
    /// exit code is left to report — `gh pr checks` with a red check prints its
    /// table and exits 1 without an extra line. Machine output is unaffected.
    pub silent: bool,
}

impl CliError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            hint: None,
            silent: false,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Report only through the exit code when a person is reading.
    pub fn silenced(mut self) -> Self {
        self.silent = true;
        self
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ConfigError, message)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    /// A gh flag or command that parses but that GitFox cannot honour.
    ///
    /// Better than clap's "unexpected argument": the caller learns the flag is
    /// known and why it does nothing here, instead of hunting for a typo.
    pub fn unsupported(what: impl std::fmt::Display, why: impl std::fmt::Display) -> Self {
        Self::new(
            ErrorCode::Unsupported,
            format!("{what} is not supported: {why}"),
        )
    }

    pub fn exit_code(&self) -> i32 {
        self.code.exit_code()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "code": self.code.as_str(),
            "message": self.message,
            "details": self.details.clone().unwrap_or(Value::Null),
        })
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

impl From<gitfox_client::Error> for CliError {
    fn from(err: gitfox_client::Error) -> Self {
        use gitfox_client::Error as E;
        match err {
            E::AuthRequired => CliError::new(ErrorCode::AuthRequired, err.to_string())
                .with_hint("set GITFOX_TOKEN, pass --token, or run `fx auth login`"),
            E::AuthFailed => CliError::new(ErrorCode::AuthFailed, err.to_string())
                .with_hint("the token may be expired or lack the required scope"),
            E::NotFound {
                kind,
                ref reference,
            } => CliError::new(
                ErrorCode::NotFound,
                format!("{kind} not found: {reference}"),
            ),
            E::Timeout(secs) => CliError::new(ErrorCode::Timeout, err.to_string())
                .with_details(json!({ "timeout_secs": secs }))
                .with_hint("raise the limit with --timeout or GITFOX_TIMEOUT"),
            E::Network(ref detail) => CliError::new(ErrorCode::NetworkError, err.to_string())
                .with_details(json!({ "detail": detail })),
            E::RateLimited {
                retry_after,
                ref message,
            } => CliError::new(ErrorCode::RateLimited, format!("rate limited: {message}"))
                .with_details(json!({ "retry_after_secs": retry_after }))
                .with_hint("fx already retried; raise --retries or wait before trying again"),
            E::Api {
                status,
                ref message,
                ref body,
            } => CliError::new(ErrorCode::ApiError, format!("HTTP {status}: {message}"))
                .with_details(json!({ "status": status, "body": body })),
            E::InvalidUrl(_) | E::Builder(_) => CliError::config(err.to_string()),
            E::Decode(ref detail) => CliError::new(ErrorCode::ApiError, err.to_string())
                .with_details(json!({ "detail": detail })),
        }
    }
}

/// The error for an `io::Error` while writing output — which, after the
/// broken-pipe case is forgiven, means something is really wrong.
pub fn unexpected(err: std::io::Error) -> CliError {
    CliError::new(ErrorCode::Unexpected, err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_the_documented_table() {
        let expected = [
            (ErrorCode::Unexpected, 1),
            (ErrorCode::ChecksFailed, 1),
            (ErrorCode::RunFailed, 1),
            (ErrorCode::InvalidArgument, 2),
            (ErrorCode::Cancelled, 2),
            (ErrorCode::AuthRequired, 3),
            (ErrorCode::AuthFailed, 3),
            (ErrorCode::NotFound, 4),
            (ErrorCode::RepoNotFound, 4),
            (ErrorCode::PrNotFound, 4),
            (ErrorCode::PipelineNotFound, 4),
            (ErrorCode::ApiError, 5),
            (ErrorCode::NetworkError, 6),
            (ErrorCode::Timeout, 6),
            (ErrorCode::RateLimited, 6),
            (ErrorCode::ConfigError, 7),
            (ErrorCode::GitContextError, 8),
            (ErrorCode::ChecksPending, 8),
            (ErrorCode::NotImplemented, 9),
            (ErrorCode::Unsupported, 9),
        ];
        for (code, exit) in expected {
            assert_eq!(code.exit_code(), exit, "{}", code.as_str());
        }
    }

    #[test]
    fn client_errors_map_onto_stable_codes() {
        let cases: Vec<(gitfox_client::Error, ErrorCode)> = vec![
            (gitfox_client::Error::AuthRequired, ErrorCode::AuthRequired),
            (gitfox_client::Error::AuthFailed, ErrorCode::AuthFailed),
            (
                gitfox_client::Error::NotFound {
                    kind: "resource",
                    reference: "x".into(),
                },
                ErrorCode::NotFound,
            ),
            (gitfox_client::Error::Timeout(30), ErrorCode::Timeout),
            (
                gitfox_client::Error::Network("dns".into()),
                ErrorCode::NetworkError,
            ),
            (
                gitfox_client::Error::Api {
                    status: 500,
                    message: "boom".into(),
                    body: None,
                },
                ErrorCode::ApiError,
            ),
            (
                gitfox_client::Error::InvalidUrl("x".into()),
                ErrorCode::ConfigError,
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(CliError::from(input).code, expected);
        }
    }

    #[test]
    fn error_json_always_has_code_message_and_details() {
        let value = CliError::config("nope").to_json();
        assert_eq!(value["code"], "CONFIG_ERROR");
        assert_eq!(value["message"], "nope");
        assert!(value.get("details").is_some());
    }

    #[test]
    fn an_unsupported_flag_names_itself_and_the_reason() {
        let err = CliError::unsupported("`--auto`", "GitFox has no auto-merge");
        assert_eq!(err.code, ErrorCode::Unsupported);
        assert_eq!(err.exit_code(), 9);
        assert_eq!(
            err.message,
            "`--auto` is not supported: GitFox has no auto-merge"
        );
        // Silence is opt-in; it never changes the code or the JSON.
        let silent = CliError::new(ErrorCode::ChecksFailed, "1 failing check").silenced();
        assert!(silent.silent);
        assert_eq!(silent.to_json()["code"], "CHECKS_FAILED");
    }
}
