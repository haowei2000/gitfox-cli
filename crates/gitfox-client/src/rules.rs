//! Protection rules and gitspaces.
//!
//! Verified against the GitFox API v1.3.0 OpenAPI document:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list rules       | `GET /api/v1/repos/{repo_ref}/rules` |
//! | view rule        | `GET /api/v1/repos/{repo_ref}/rules/{rule_identifier}` |
//! | list gitspaces   | `GET /api/v1/gitspaces` |
//! | view gitspace    | `GET /api/v1/gitspaces/{gitspace_identifier}` |
//! | create gitspace  | `POST /api/v1/gitspaces` |
//! | delete gitspace  | `DELETE /api/v1/gitspaces/{gitspace_identifier}` |
//! | start / stop     | `POST …/gitspaces/{gitspace_identifier}/action` |
//! | gitspace logs    | `GET …/gitspaces/{gitspace_identifier}/logs/stream` (SSE) |
//!
//! Gitspaces are an instance feature that can be switched off; an instance
//! with them disabled answers the gitspace endpoints with a bare 500, so
//! callers should check [`crate::SystemConfig::gitspace_enabled`] first.

use std::time::Duration;

use serde_json::json;

use crate::client::{GitFoxClient, Method, Query};
use crate::error::Result;
use crate::models::{CreateGitspace, Gitspace, LogLine, RepoRef, Rule};
use crate::pipeline::urlencode;

pub struct RulesApi<'a> {
    client: &'a GitFoxClient,
}

impl<'a> RulesApi<'a> {
    pub(crate) fn new(client: &'a GitFoxClient) -> Self {
        Self { client }
    }

    /// `GET /api/v1/repos/{repo_ref}/rules`
    pub async fn list(&self, repo: &RepoRef, page: u32, limit: u32) -> Result<Vec<Rule>> {
        let mut q = Query::new();
        q.push("page", page).push("limit", limit);
        self.client
            .get_json(&q.apply(&format!("/api/v1/repos/{}/rules", repo.encoded())))
            .await
    }

    /// `GET /api/v1/repos/{repo_ref}/rules/{rule_identifier}`
    pub async fn get(&self, repo: &RepoRef, identifier: &str) -> Result<Rule> {
        self.client
            .get_json(&format!(
                "/api/v1/repos/{}/rules/{}",
                repo.encoded(),
                urlencode(identifier)
            ))
            .await
    }
}

pub struct GitspacesApi<'a> {
    client: &'a GitFoxClient,
}

impl<'a> GitspacesApi<'a> {
    pub(crate) fn new(client: &'a GitFoxClient) -> Self {
        Self { client }
    }

    fn path(identifier: &str) -> String {
        format!("/api/v1/gitspaces/{}", urlencode(identifier))
    }

    /// `GET /api/v1/gitspaces`
    pub async fn list(&self, page: u32, limit: u32) -> Result<Vec<Gitspace>> {
        let mut q = Query::new();
        q.push("sort", "updated")
            .push("order", "desc")
            .push("page", page)
            .push("limit", limit);
        self.client.get_json(&q.apply("/api/v1/gitspaces")).await
    }

    /// `GET /api/v1/gitspaces/{gitspace_identifier}`
    pub async fn get(&self, identifier: &str) -> Result<Gitspace> {
        self.client.get_json(&Self::path(identifier)).await
    }

    /// `POST /api/v1/gitspaces`
    pub async fn create(&self, input: &CreateGitspace) -> Result<Gitspace> {
        let body = serde_json::to_value(input).map_err(|e| crate::Error::Decode(e.to_string()))?;
        self.client
            .request(Method::POST, "/api/v1/gitspaces", Some(&body), &[])
            .await?
            .deserialize()
    }

    /// `DELETE /api/v1/gitspaces/{gitspace_identifier}`
    pub async fn delete(&self, identifier: &str) -> Result<()> {
        self.client
            .request(Method::DELETE, &Self::path(identifier), None, &[])
            .await
            .map(|_| ())
    }

    /// `POST …/action` with `start` or `stop`.
    pub async fn action(&self, identifier: &str, action: &str) -> Result<Gitspace> {
        self.client
            .request(
                Method::POST,
                &format!("{}/action", Self::path(identifier)),
                Some(&json!({ "action": action })),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `GET …/logs/stream`, collected until the stream goes quiet for `idle`.
    pub async fn logs(&self, identifier: &str, idle: Duration) -> Result<Vec<LogLine>> {
        self.client
            .sse_lines(&format!("{}/logs/stream", Self::path(identifier)), idle)
            .await
    }
}
