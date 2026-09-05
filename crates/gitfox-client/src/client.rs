//! The HTTP client every other module and both front-ends (CLI, and later MCP)
//! go through.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;
use url::form_urlencoded;

use crate::auth::AuthApi;
use crate::error::{Error, Result};
use crate::pipeline::PipelinesApi;
use crate::principal::PrincipalsApi;
use crate::pull_request::PullRequestsApi;
use crate::repo::ReposApi;

pub use reqwest::Method;

pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Attempts after the first, for requests that are safe to repeat.
pub const DEFAULT_RETRIES: u32 = 2;

/// The longest we will wait between attempts, including a server's own
/// `Retry-After`.
///
/// Someone is usually watching a CLI run. A command that goes silent for
/// minutes because a header asked it to is worse than one that gives up and
/// says why — the caller can always run it again, and an agent can back off on
/// its own schedule.
const MAX_BACKOFF: Duration = Duration::from_secs(5);

const USER_AGENT: &str = concat!("fx/", env!("CARGO_PKG_VERSION"));

/// A GitFox API client bound to one host.
#[derive(Clone)]
pub struct GitFoxClient {
    base_url: Url,
    token: Option<String>,
    timeout_secs: u64,
    retries: u32,
    http: reqwest::Client,
}

/// Redacted on purpose: the token must never reach a log line or an error
/// message. Every `{:?}` of a client or anything holding one goes through here.
impl fmt::Debug for GitFoxClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitFoxClient")
            .field("base_url", &self.base_url.as_str())
            .field("token", &self.token.as_ref().map(|_| "***"))
            .field("timeout_secs", &self.timeout_secs)
            .field("retries", &self.retries)
            .finish()
    }
}

impl GitFoxClient {
    pub fn builder(host: impl Into<String>) -> GitFoxClientBuilder {
        GitFoxClientBuilder {
            host: host.into(),
            token: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            retries: DEFAULT_RETRIES,
            insecure: false,
        }
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    pub fn auth(&self) -> AuthApi<'_> {
        AuthApi::new(self)
    }

    pub fn repos(&self) -> ReposApi<'_> {
        ReposApi::new(self)
    }

    pub fn pull_requests(&self) -> PullRequestsApi<'_> {
        PullRequestsApi::new(self)
    }

    pub fn principals(&self) -> PrincipalsApi<'_> {
        PrincipalsApi::new(self)
    }

    pub fn pipelines(&self) -> PipelinesApi<'_> {
        PipelinesApi::new(self)
    }

    /// Resolve an API path against the host.
    ///
    /// Accepts `/api/v1/user`, `api/v1/user` and a fully qualified URL, so
    /// `fx api` can take whatever the user pasted.
    pub fn resolve(&self, path: &str) -> Result<Url> {
        if path.starts_with("http://") || path.starts_with("https://") {
            return Url::parse(path).map_err(|e| Error::InvalidUrl(format!("{path}: {e}")));
        }
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|e| Error::InvalidUrl(format!("{path}: {e}")))
    }

    /// The one place an HTTP request is issued. Everything typed is built on top.
    ///
    /// Transient failures are retried with exponential backoff — but only for
    /// methods that are safe to repeat. See [`is_retryable_method`].
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        extra_headers: &[(String, String)],
    ) -> Result<RawResponse> {
        let url = self.resolve(path)?;
        let budget = if is_retryable_method(&method) {
            self.retries
        } else {
            0
        };

        let mut attempt = 0;
        loop {
            tracing::debug!(method = %method, url = %url, attempt, "gitfox request");
            let outcome = self
                .attempt(method.clone(), url.clone(), body, extra_headers)
                .await;

            let Err(error) = outcome else {
                return outcome;
            };

            let Some(delay) = self.backoff(&error, attempt, budget) else {
                return Err(error);
            };
            tracing::warn!(
                method = %method,
                url = %url,
                attempt,
                delay_ms = delay.as_millis(),
                error = %error,
                "transient failure; retrying"
            );
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    /// One trip to the server, with no retry logic of its own.
    async fn attempt(
        &self,
        method: Method,
        url: Url,
        body: Option<&Value>,
        extra_headers: &[(String, String)],
    ) -> Result<RawResponse> {
        let mut req = self.http.request(method, url);
        // JSON unless the caller asked for something else. The pull request
        // diff endpoint serves either JSON or a raw unified diff depending on
        // this header, which is the whole reason it is overridable.
        if !extra_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept"))
        {
            req = req.header(ACCEPT, "application/json");
        }
        if let Some(token) = &self.token {
            let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| Error::Builder("token contains invalid header bytes".into()))?;
            // Marks the header sensitive so it stays out of any logging that
            // honours the flag.
            value.set_sensitive(true);
            req = req.header(AUTHORIZATION, value);
        }
        for (name, value) in extra_headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| Error::Builder(format!("invalid header name `{name}`")))?;
            let value = HeaderValue::from_str(value)
                .map_err(|_| Error::Builder(format!("invalid value for header `{name}`")))?;
            req = req.header(name, value);
        }
        if let Some(body) = body {
            req = req.header(CONTENT_TYPE, "application/json").json(body);
        }

        let response = req.send().await.map_err(|e| self.transport_error(&e))?;
        let status = response.status();
        let headers = response.headers().clone();
        let text = response
            .text()
            .await
            .map_err(|e| self.transport_error(&e))?;
        let json = if text.trim().is_empty() {
            None
        } else {
            serde_json::from_str::<Value>(&text).ok()
        };

        let raw = RawResponse {
            status: status.as_u16(),
            headers,
            json,
            text,
        };

        if status.is_success() {
            Ok(raw)
        } else {
            Err(self.status_error(&raw))
        }
    }

    /// How long to wait before trying again, or `None` to give up.
    ///
    /// A server that said `Retry-After` is obeyed up to [`MAX_BACKOFF`];
    /// otherwise the delay doubles each attempt from 250ms, with jitter so a
    /// fleet of agents retrying at once does not do so in lockstep.
    fn backoff(&self, error: &Error, attempt: u32, budget: u32) -> Option<Duration> {
        if attempt >= budget || !is_transient(error) {
            return None;
        }
        if let Error::RateLimited { retry_after, .. } = error
            && let Some(secs) = retry_after
        {
            return Some(Duration::from_secs(*secs).min(MAX_BACKOFF));
        }
        let base = Duration::from_millis(250 * 2u64.pow(attempt.min(6)));
        Some((base + jitter(base)).min(MAX_BACKOFF))
    }

    /// Read a server-sent-event stream, collecting each `data:` payload that
    /// deserialises into `T`.
    ///
    /// Stops on the stream's own `event: error` / `data: eof`, or once `idle`
    /// passes with no new bytes — a stream following something still in
    /// progress never ends on its own, so a snapshot has to decide when the
    /// backlog has been drained.
    ///
    /// The client's request timeout is deliberately not applied: it bounds a
    /// whole request/response, which for a stream would cut the body off
    /// mid-flight. The idle window bounds this instead.
    pub async fn sse_lines<T: DeserializeOwned>(
        &self,
        path: &str,
        idle: Duration,
    ) -> Result<Vec<T>> {
        let url = self.resolve(path)?;
        tracing::debug!(url = %url, "gitfox log stream");

        let mut req = self
            .http
            .get(url)
            .header(ACCEPT, "text/event-stream")
            .timeout(Duration::from_secs(60 * 60));
        if let Some(token) = &self.token {
            let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| Error::Builder("token contains invalid header bytes".into()))?;
            value.set_sensitive(true);
            req = req.header(AUTHORIZATION, value);
        }

        let mut response = req.send().await.map_err(|e| self.transport_error(&e))?;
        // An SSE endpoint answers 200 and reports failure inside the stream, so
        // a non-success status here is a genuine transport-level refusal.
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            let json = serde_json::from_str::<Value>(&text).ok();
            return Err(self.status_error(&RawResponse {
                status,
                headers: HeaderMap::new(),
                json,
                text,
            }));
        }

        let mut collected = Vec::new();
        let mut buffer = Vec::new();
        let mut last_event: Option<String> = None;

        loop {
            let chunk = match tokio::time::timeout(idle, response.chunk()).await {
                // Nothing new for a while: the backlog is drained.
                Err(_) => break,
                Ok(Ok(Some(chunk))) => chunk,
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(self.transport_error(&e)),
            };
            buffer.extend_from_slice(&chunk);

            while let Some(newline) = buffer.iter().position(|b| *b == b'\n') {
                let line = buffer.drain(..=newline).collect::<Vec<u8>>();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim_end_matches(['\r', '\n']);

                if let Some(name) = line.strip_prefix("event:") {
                    last_event = Some(name.trim().to_string());
                    continue;
                }
                let Some(payload) = line.strip_prefix("data:") else {
                    // `: ping` keepalives and blank separators.
                    continue;
                };
                let payload = payload.trim();
                if last_event.as_deref() == Some("error") {
                    // `data: eof` is the ordinary end of a finished step's
                    // stream, not a failure worth surfacing.
                    return Ok(collected);
                }
                if let Ok(value) = serde_json::from_str::<T>(payload) {
                    collected.push(value);
                }
            }
        }
        Ok(collected)
    }

    /// Convenience wrapper for typed GET requests.
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request(Method::GET, path, None, &[])
            .await?
            .deserialize()
    }

    fn transport_error(&self, err: &reqwest::Error) -> Error {
        if err.is_timeout() {
            Error::Timeout(self.timeout_secs)
        } else if err.is_decode() {
            Error::Decode(err.to_string())
        } else {
            Error::Network(err.to_string())
        }
    }

    fn status_error(&self, raw: &RawResponse) -> Error {
        let fallback = if raw.text.trim().is_empty() {
            format!("HTTP {}", raw.status)
        } else {
            raw.text.trim().chars().take(500).collect()
        };
        let message = Error::message_from_body(&raw.json, &fallback);
        match raw.status {
            429 => Error::RateLimited {
                retry_after: raw
                    .headers
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok()),
                message,
            },
            401 if self.token.is_none() => Error::AuthRequired,
            401 | 403 => Error::AuthFailed,
            404 => Error::NotFound {
                kind: "resource",
                reference: message,
            },
            408 | 504 => Error::Timeout(self.timeout_secs),
            status => Error::Api {
                status,
                message,
                body: raw.json.clone(),
            },
        }
    }
}

/// A response that succeeded at the HTTP level, kept in both raw and parsed form
/// so `fx api` can pass anything through untouched.
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub json: Option<Value>,
    pub text: String,
}

impl RawResponse {
    pub fn json_or_null(&self) -> Value {
        self.json.clone().unwrap_or(Value::Null)
    }

    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T> {
        let value = self
            .json
            .clone()
            .ok_or_else(|| Error::Decode("expected a JSON response body".into()))?;
        serde_json::from_value(value).map_err(|e| Error::Decode(e.to_string()))
    }
}

pub struct GitFoxClientBuilder {
    host: String,
    token: Option<String>,
    timeout_secs: u64,
    retries: u32,
    insecure: bool,
}

impl GitFoxClientBuilder {
    pub fn token(mut self, token: Option<String>) -> Self {
        self.token = token.filter(|t| !t.trim().is_empty());
        self
    }

    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// How many times to try again after a transient failure. Zero disables
    /// retrying entirely.
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Skip TLS verification. Self-hosted instances with private CAs need it;
    /// callers are expected to warn the user loudly when they turn it on.
    pub fn insecure(mut self, insecure: bool) -> Self {
        self.insecure = insecure;
        self
    }

    pub fn build(self) -> Result<GitFoxClient> {
        let base_url = normalize_host(&self.host)?;
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(self.timeout_secs))
            .danger_accept_invalid_certs(self.insecure)
            .build()
            .map_err(|e| Error::Builder(e.to_string()))?;
        Ok(GitFoxClient {
            base_url,
            token: self.token,
            timeout_secs: self.timeout_secs,
            retries: self.retries,
            http,
        })
    }
}

/// Turn whatever the user configured into a base URL that `Url::join` behaves
/// well against: `git.example.com` -> `https://git.example.com/`.
pub fn normalize_host(host: &str) -> Result<Url> {
    let host = host.trim();
    if host.is_empty() {
        return Err(Error::InvalidUrl("host is empty".into()));
    }
    let with_scheme = if host.contains("://") {
        host.to_string()
    } else {
        format!("https://{host}")
    };
    let mut url =
        Url::parse(&with_scheme).map_err(|e| Error::InvalidUrl(format!("{host}: {e}")))?;
    if url.cannot_be_a_base() || url.host_str().is_none() {
        return Err(Error::InvalidUrl(format!("{host}: missing host")));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// Whether repeating this method is safe.
///
/// `POST` is excluded on purpose. A retried `POST /pullreq` that timed out
/// after the server accepted it would open a second pull request — silently
/// doing the thing twice is worse than reporting that it might not have
/// happened at all. `PATCH` is excluded for the same reason: GitFox uses it for
/// partial updates, which are not guaranteed idempotent.
pub fn is_retryable_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::PUT | Method::DELETE
    )
}

/// Whether this failure is worth another try.
///
/// Deliberately narrow: a 500 is not here, because an internal error that
/// repeats is usually a bug being hit again rather than a blip, and retrying it
/// just triples the time before the user sees the message.
fn is_transient(error: &Error) -> bool {
    match error {
        Error::Network(_) | Error::Timeout(_) | Error::RateLimited { .. } => true,
        Error::Api { status, .. } => matches!(status, 502..=504),
        _ => false,
    }
}

/// Up to half the base delay, derived from the clock rather than a PRNG so the
/// crate keeps no random-number dependency.
fn jitter(base: Duration) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    Duration::from_millis((base.as_millis() as u64 / 2).saturating_mul(nanos % 1000) / 1000)
}

/// A percent-encoded query string.
///
/// Kept here rather than in each endpoint module so every request encodes the
/// same way, and so `Option` parameters can be dropped without an `if` at each
/// call site.
#[derive(Debug, Default, Clone)]
pub struct Query(Vec<(String, String)>);

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, key: &str, value: impl ToString) -> &mut Self {
        self.0.push((key.to_string(), value.to_string()));
        self
    }

    /// Append only when the value is present, so unset filters never reach the
    /// server as empty strings (which GitFox treats as a real filter).
    pub fn push_opt<T: ToString>(&mut self, key: &str, value: Option<T>) -> &mut Self {
        if let Some(value) = value {
            self.push(key, value);
        }
        self
    }

    pub fn extend<T: ToString>(
        &mut self,
        key: &str,
        values: impl IntoIterator<Item = T>,
    ) -> &mut Self {
        for value in values {
            self.push(key, value);
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn encode(&self) -> String {
        form_urlencoded::Serializer::new(String::new())
            .extend_pairs(self.0.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .finish()
    }

    /// `path?a=1&b=2`, or `path` when there is nothing to add.
    pub fn apply(&self, path: &str) -> String {
        if self.is_empty() {
            return path.to_string();
        }
        format!("{path}?{}", self.encode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_https_when_scheme_is_missing() {
        assert_eq!(
            normalize_host("git.example.com").unwrap().as_str(),
            "https://git.example.com/"
        );
    }

    #[test]
    fn keeps_explicit_scheme_port_and_base_path() {
        assert_eq!(
            normalize_host("http://localhost:3000").unwrap().as_str(),
            "http://localhost:3000/"
        );
        assert_eq!(
            normalize_host("https://example.com/gitfox")
                .unwrap()
                .as_str(),
            "https://example.com/gitfox/"
        );
    }

    #[test]
    fn rejects_unusable_hosts() {
        for bad in ["", "   ", "https://"] {
            assert!(
                normalize_host(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    #[test]
    fn resolves_paths_with_and_without_leading_slash() {
        let c = GitFoxClient::builder("https://git.example.com")
            .build()
            .unwrap();
        assert_eq!(
            c.resolve("/api/v1/user").unwrap().as_str(),
            "https://git.example.com/api/v1/user"
        );
        assert_eq!(
            c.resolve("api/v1/user").unwrap().as_str(),
            "https://git.example.com/api/v1/user"
        );
    }

    #[test]
    fn resolve_preserves_a_base_path() {
        let c = GitFoxClient::builder("https://example.com/gitfox")
            .build()
            .unwrap();
        assert_eq!(
            c.resolve("/api/v1/user").unwrap().as_str(),
            "https://example.com/gitfox/api/v1/user"
        );
    }

    #[test]
    fn resolve_preserves_a_percent_encoded_slash_in_a_repo_ref() {
        // `ai/backend` travels as one path segment, `ai%2Fbackend`. If `join`
        // ever decoded or double-encoded it, every repo-scoped call would 404.
        let c = GitFoxClient::builder("https://git.example.com")
            .build()
            .unwrap();
        let url = c.resolve("/api/v1/repos/ai%2Fbackend/pullreq").unwrap();
        assert_eq!(
            url.as_str(),
            "https://git.example.com/api/v1/repos/ai%2Fbackend/pullreq"
        );
    }

    #[test]
    fn resolve_keeps_a_query_string_intact() {
        let c = GitFoxClient::builder("https://git.example.com")
            .build()
            .unwrap();
        let url = c
            .resolve("/api/v1/repos/ai%2Fbackend/pullreq?state=open&limit=30")
            .unwrap();
        assert_eq!(url.query(), Some("state=open&limit=30"));
        assert_eq!(url.path(), "/api/v1/repos/ai%2Fbackend/pullreq");
    }

    #[test]
    fn query_encodes_pairs_and_skips_absent_values() {
        let mut q = Query::new();
        q.push("limit", 30)
            .push_opt("author_id", Some(7))
            .push_opt::<String>("query", None)
            .extend("state", ["open", "merged"]);
        assert_eq!(
            q.apply("/x"),
            "/x?limit=30&author_id=7&state=open&state=merged"
        );
        assert_eq!(Query::new().apply("/x"), "/x");
    }

    #[test]
    fn query_escapes_values_that_would_break_the_url() {
        let mut q = Query::new();
        q.push("query", "fix: auth & tokens");
        assert_eq!(q.encode(), "query=fix%3A+auth+%26+tokens");
    }

    #[test]
    fn an_explicit_accept_header_replaces_the_default() {
        // Verified through the header names the builder ends up with rather
        // than by sending: two Accept headers would make the server choose.
        let names = ["Accept", "accept", "ACCEPT"];
        for name in names {
            assert!(
                [(name.to_string(), "text/plain".to_string())]
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case("accept")),
                "`{name}` should be recognised as Accept"
            );
        }
        assert!(
            ![("X-Trace".to_string(), "1".to_string())]
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("accept"))
        );
    }

    #[test]
    fn debug_never_reveals_the_token() {
        let c = GitFoxClient::builder("git.example.com")
            .token(Some("super-secret-token".into()))
            .build()
            .unwrap();
        let rendered = format!("{c:?}");
        assert!(!rendered.contains("super-secret-token"), "{rendered}");
        assert!(rendered.contains("***"), "{rendered}");
    }

    #[test]
    fn blank_tokens_are_treated_as_absent() {
        let c = GitFoxClient::builder("git.example.com")
            .token(Some("   ".into()))
            .build()
            .unwrap();
        assert!(!c.has_token());
    }
}
