//! Pipeline / CI endpoints.
//!
//! Verified against the GitFox API v1.3.0 OpenAPI document:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list pipelines  | `GET /api/v1/repos/{repo_ref}/pipelines` |
//! | list executions | `GET …/pipelines/{pipeline_identifier}/executions` |
//! | view execution  | `GET …/executions/{execution_number}` |
//! | step logs       | `GET …/executions/{execution_number}/logs/{stage_number}/{step_number}` |
//! | live step logs  | `GET …/logs/{stage_number}/{step_number}/stream` (SSE, undocumented) |
//! | retry           | `POST …/executions/{execution_number}/retry` |
//! | cancel          | `POST …/executions/{execution_number}/cancel` |
//! | trigger a run   | `POST …/pipelines/{pipeline_identifier}/executions?branch=…` |
//! | delete a run    | `DELETE …/executions/{execution_number}` |
//! | view pipeline   | `GET …/pipelines/{pipeline_identifier}` |
//! | enable/disable  | `PATCH …/pipelines/{pipeline_identifier}` with `disabled` |
//! | definition      | `GET …/pipelines/{pipeline_identifier}/content` |
//!
//! Two shapes of this API drive the CLI's design:
//!
//! * `?latest=true` on the pipeline list embeds each pipeline's most recent
//!   execution in full, so "what is CI doing in this repository" is one
//!   request rather than one per pipeline.
//! * Logs are addressed per *step*, by stage number and step number, and only
//!   the single-execution endpoint returns the stage tree. So
//!   `gf pipeline logs --failed` reads the execution first, walks it for steps
//!   that failed, and fetches only those.
//! * A step's log is not persisted until it finishes: the static endpoint
//!   answers 404 for a running step, and its output is only reachable over the
//!   SSE stream. The two endpoints are complementary, not alternatives.

use std::time::Duration;

use crate::client::{GitFoxClient, Method, Query};
use crate::error::Result;
use crate::models::{Execution, LogLine, Pipeline, RepoRef};

pub struct PipelinesApi<'a> {
    client: &'a GitFoxClient,
}

impl<'a> PipelinesApi<'a> {
    pub(crate) fn new(client: &'a GitFoxClient) -> Self {
        Self { client }
    }

    /// `GET /api/v1/repos/{repo_ref}/pipelines`
    ///
    /// With `latest`, every pipeline carries its most recent run.
    pub async fn list(
        &self,
        repo: &RepoRef,
        latest: bool,
        page: u32,
        limit: u32,
    ) -> Result<Vec<Pipeline>> {
        let mut q = Query::new();
        q.push("page", page).push("limit", limit);
        if latest {
            q.push("latest", "true");
        }
        let path = format!("/api/v1/repos/{}/pipelines", repo.encoded());
        self.client.get_json(&q.apply(&path)).await
    }

    /// `GET /api/v1/repos/{repo_ref}/pipelines/{pipeline}/executions`
    ///
    /// The executions here carry no stage tree; use [`Self::get_execution`] for
    /// one that does.
    pub async fn list_executions(
        &self,
        repo: &RepoRef,
        pipeline: &str,
        page: u32,
        limit: u32,
    ) -> Result<Vec<Execution>> {
        let mut q = Query::new();
        q.push("page", page).push("limit", limit);
        let path = format!(
            "/api/v1/repos/{}/pipelines/{}/executions",
            repo.encoded(),
            urlencode(pipeline)
        );
        self.client.get_json(&q.apply(&path)).await
    }

    /// `GET …/executions/{execution_number}` — includes stages and steps.
    pub async fn get_execution(
        &self,
        repo: &RepoRef,
        pipeline: &str,
        number: u64,
    ) -> Result<Execution> {
        self.client
            .get_json(&self.execution_path(repo, pipeline, number))
            .await
    }

    /// `GET …/executions/{n}/logs/{stage_number}/{step_number}`
    pub async fn step_logs(
        &self,
        repo: &RepoRef,
        pipeline: &str,
        number: u64,
        stage: i64,
        step: i64,
    ) -> Result<Vec<LogLine>> {
        let path = format!(
            "{}/logs/{stage}/{step}",
            self.execution_path(repo, pipeline, number)
        );
        self.client.get_json(&path).await
    }

    /// Live log lines for a step that is still running.
    ///
    /// The static log endpoint answers 404 until a step finishes — GitFox does
    /// not persist the log before then — so a running step's output is only
    /// reachable over this server-sent-event stream. It is not in the
    /// instance's OpenAPI document; the payload is the same [`LogLine`] the
    /// static endpoint returns.
    ///
    /// Returns once the stream says `eof`, or once `idle` passes with nothing
    /// new arriving — which is how a snapshot of a still-running step ends,
    /// since that stream stays open indefinitely.
    pub async fn step_logs_live(
        &self,
        repo: &RepoRef,
        pipeline: &str,
        number: u64,
        stage: i64,
        step: i64,
        idle: Duration,
    ) -> Result<Vec<LogLine>> {
        let path = format!(
            "{}/logs/{stage}/{step}/stream",
            self.execution_path(repo, pipeline, number)
        );
        self.client.sse_lines(&path, idle).await
    }

    /// `POST …/executions/{execution_number}/retry`
    pub async fn retry(&self, repo: &RepoRef, pipeline: &str, number: u64) -> Result<Execution> {
        self.post(&format!(
            "{}/retry",
            self.execution_path(repo, pipeline, number)
        ))
        .await
    }

    /// `POST …/executions/{execution_number}/cancel`
    pub async fn cancel(&self, repo: &RepoRef, pipeline: &str, number: u64) -> Result<Execution> {
        self.post(&format!(
            "{}/cancel",
            self.execution_path(repo, pipeline, number)
        ))
        .await
    }

    /// `DELETE …/executions/{execution_number}`
    pub async fn delete_execution(
        &self,
        repo: &RepoRef,
        pipeline: &str,
        number: u64,
    ) -> Result<()> {
        self.client
            .request(
                Method::DELETE,
                &self.execution_path(repo, pipeline, number),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `GET /api/v1/repos/{repo_ref}/pipelines/{pipeline_identifier}`
    pub async fn get(&self, repo: &RepoRef, pipeline: &str) -> Result<Pipeline> {
        self.client
            .get_json(&self.pipeline_path(repo, pipeline))
            .await
    }

    /// `PATCH …/pipelines/{pipeline_identifier}` with `disabled`.
    pub async fn set_disabled(
        &self,
        repo: &RepoRef,
        pipeline: &str,
        disabled: bool,
    ) -> Result<Pipeline> {
        self.client
            .request(
                Method::PATCH,
                &self.pipeline_path(repo, pipeline),
                Some(&serde_json::json!({ "disabled": disabled })),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `GET …/pipelines/{pipeline_identifier}/content` — the YAML definition
    /// on the pipeline's default branch.
    ///
    /// GitFox sends it base64-encoded, without saying so; the document names
    /// no encoding field. It is decoded here, and passed through unchanged if
    /// it turns out not to be base64 of UTF-8 text.
    pub async fn content(&self, repo: &RepoRef, pipeline: &str) -> Result<String> {
        let value: serde_json::Value = self
            .client
            .get_json(&format!("{}/content", self.pipeline_path(repo, pipeline)))
            .await?;
        let raw = value
            .get("content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Ok(decode_definition(raw))
    }

    fn pipeline_path(&self, repo: &RepoRef, pipeline: &str) -> String {
        format!(
            "/api/v1/repos/{}/pipelines/{}",
            repo.encoded(),
            urlencode(pipeline)
        )
    }

    /// `POST …/pipelines/{pipeline}/executions?branch=…`
    pub async fn trigger(
        &self,
        repo: &RepoRef,
        pipeline: &str,
        branch: Option<&str>,
    ) -> Result<Execution> {
        let mut q = Query::new();
        q.push_opt("branch", branch);
        let path = format!(
            "/api/v1/repos/{}/pipelines/{}/executions",
            repo.encoded(),
            urlencode(pipeline)
        );
        self.post(&q.apply(&path)).await
    }

    async fn post(&self, path: &str) -> Result<Execution> {
        self.client
            .request(Method::POST, path, None, &[])
            .await?
            .deserialize()
    }

    fn execution_path(&self, repo: &RepoRef, pipeline: &str, number: u64) -> String {
        format!(
            "/api/v1/repos/{}/pipelines/{}/executions/{number}",
            repo.encoded(),
            urlencode(pipeline)
        )
    }
}

/// A pipeline definition as GitFox sends it: base64, or — should an instance
/// ever send it plain — the text itself.
fn decode_definition(raw: &str) -> String {
    crate::models::decode_base64(raw)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| raw.to_string())
}

/// Pipeline identifiers are a single path segment, so anything that would end
/// the segment or start a query has to be escaped.
pub(crate) fn urlencode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_paths(pipeline: &str) -> (String, String) {
        let client = GitFoxClient::builder("https://git.example.com")
            .build()
            .unwrap();
        let api = PipelinesApi::new(&client);
        let repo = RepoRef::parse("ai/backend").unwrap();
        let execution = api.execution_path(&repo, pipeline, 182);
        (execution.clone(), format!("{execution}/logs/1/2"))
    }

    #[test]
    fn execution_and_log_paths_are_built_from_the_documented_shape() {
        let (execution, logs) = api_paths("default");
        assert_eq!(
            execution,
            "/api/v1/repos/ai%2Fbackend/pipelines/default/executions/182"
        );
        assert_eq!(
            logs,
            "/api/v1/repos/ai%2Fbackend/pipelines/default/executions/182/logs/1/2"
        );
    }

    #[test]
    fn a_pipeline_name_with_a_slash_or_space_cannot_break_out_of_its_segment() {
        let (execution, _) = api_paths("team/build pipeline");
        assert_eq!(
            execution,
            "/api/v1/repos/ai%2Fbackend/pipelines/team%2Fbuild%20pipeline/executions/182"
        );
        // And the escaped form survives resolution against the base URL.
        let client = GitFoxClient::builder("https://git.example.com")
            .build()
            .unwrap();
        assert!(
            client
                .resolve(&execution)
                .unwrap()
                .as_str()
                .ends_with("/pipelines/team%2Fbuild%20pipeline/executions/182")
        );
    }

    #[test]
    fn a_definition_is_decoded_from_base64_and_plain_text_passes_through() {
        assert_eq!(
            decode_definition("dmVyc2lvbjogMQpraW5kOiBwaXBlbGluZQo="),
            "version: 1\nkind: pipeline\n"
        );
        // Plain YAML is not valid base64 (the colon and spaces), so it stays.
        assert_eq!(decode_definition("version: 1\n"), "version: 1\n");
    }

    #[test]
    fn urlencode_leaves_ordinary_identifiers_alone() {
        assert_eq!(urlencode("build-and-test_v2.1~x"), "build-and-test_v2.1~x");
    }
}
