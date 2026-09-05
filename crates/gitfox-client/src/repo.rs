//! Repository endpoints.
//!
//! Verified against the GitFox API v1.3.0 OpenAPI document (`GET /openapi.yaml`
//! on any instance, no authentication required):
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list repositories (all spaces) | `GET /api/v1/repos` |
//! | list repositories in a space   | `GET /api/v1/spaces/{space_ref}/repos` |
//! | get a repository               | `GET /api/v1/repos/{repo_ref}` |
//! | list branches                  | `GET /api/v1/repos/{repo_ref}/branches` |
//!
//! `{repo_ref}` is [`crate::RepoRef::encoded`] (`ai%2Fbackend`).
//!
//! `fx repo list|view|clone` land in v0.2; [`ReposApi::get`] exists now because
//! `fx pr create` needs the default branch to pick a base.

use crate::client::{GitFoxClient, Query};
use crate::error::Result;
use crate::models::{RepoRef, Repository};

pub struct ReposApi<'a> {
    client: &'a GitFoxClient,
}

impl<'a> ReposApi<'a> {
    pub(crate) fn new(client: &'a GitFoxClient) -> Self {
        Self { client }
    }

    /// `GET /api/v1/repos/{repo_ref}`
    pub async fn get(&self, repo: &RepoRef) -> Result<Repository> {
        self.client
            .get_json(&format!("/api/v1/repos/{}", repo.encoded()))
            .await
    }

    /// `GET /api/v1/repos` — spans every space the caller can see.
    pub async fn list(
        &self,
        query: Option<&str>,
        page: u32,
        limit: u32,
    ) -> Result<Vec<Repository>> {
        let mut q = Query::new();
        q.push_opt("query", query)
            .push("page", page)
            .push("limit", limit);
        self.client.get_json(&q.apply("/api/v1/repos")).await
    }

    /// `GET /api/v1/spaces/{space_ref}/repos`
    pub async fn list_in_space(
        &self,
        space: &str,
        page: u32,
        limit: u32,
    ) -> Result<Vec<Repository>> {
        let mut q = Query::new();
        q.push("page", page).push("limit", limit);
        let space = space.replace('/', "%2F");
        self.client
            .get_json(&q.apply(&format!("/api/v1/spaces/{space}/repos")))
            .await
    }
}
