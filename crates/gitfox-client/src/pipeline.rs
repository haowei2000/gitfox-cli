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
//! | retry           | `POST …/executions/{execution_number}/retry` |
//! | cancel          | `POST …/executions/{execution_number}/cancel` |
//! | trigger a run   | `POST …/pipelines/{pipeline_identifier}/executions?branch=…` |
//!
//! Two shapes of this API drive the CLI's design:
//!
//! * `?latest=true` on the pipeline list embeds each pipeline's most recent
//!   execution in full, so "what is CI doing in this repository" is one
//!   request rather than one per pipeline.
//! * Logs are addressed per *step*, by stage number and step number, and only
//!   the single-execution endpoint returns the stage tree. So
//!   `fx pipeline logs --failed` reads the execution first, walks it for steps
//!   that failed, and fetches only those.

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

/// Pipeline identifiers are a single path segment, so anything that would end
/// the segment or start a query has to be escaped.
fn urlencode(segment: &str) -> String {
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
    fn urlencode_leaves_ordinary_identifiers_alone() {
        assert_eq!(urlencode("build-and-test_v2.1~x"), "build-and-test_v2.1~x");
    }
}
