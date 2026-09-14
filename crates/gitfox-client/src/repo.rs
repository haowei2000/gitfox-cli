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
//! | create                         | `POST /api/v1/repos` (`fork_id` makes a fork) |
//! | delete (soft)                  | `DELETE /api/v1/repos/{repo_ref}` |
//! | edit description               | `PATCH /api/v1/repos/{repo_ref}` |
//! | change visibility              | `POST …/public-access` |
//! | change default branch          | `POST …/default-branch` |
//! | rename                         | `POST …/move` |
//! | sync a mirror or fork          | `POST …/sync` |
//! | read a path                    | `GET …/content/{path}` |
//! | rebase a branch                | `POST …/rebase` |
//! | commit diff                    | `GET …/commits/{commit_sha}/diff` |
//! | gitignore / license templates  | `GET /api/v1/resources/gitignore`, `…/license` |
//!
//! `{repo_ref}` is [`crate::RepoRef::encoded`] (`ai%2Fbackend`), or a numeric
//! repository id — which is how a fork's parent is looked up.
//!
//! Note the two response shapes: `GET /repos/{repo_ref}` and the space-scoped
//! listing carry `is_public`, the instance-wide `GET /repos` does not. See
//! [`Repository`] for how that difference is represented.

use serde_json::json;

use crate::client::{GitFoxClient, Method, Query};
use crate::error::Result;
use crate::models::{Content, CreateRepository, LicenseTemplate, RepoRef, Repository};

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

    /// `GET /api/v1/repos/{id}` — the same endpoint, addressed by id.
    pub async fn get_by_id(&self, id: i64) -> Result<Repository> {
        self.client.get_json(&format!("/api/v1/repos/{id}")).await
    }

    /// `GET /api/v1/repos` — spans every space the caller can see.
    ///
    /// Answers with the narrower shape: [`Repository::is_public`] is `None`
    /// here. Use [`Self::list_in_space`] when visibility matters.
    pub async fn list(
        &self,
        query: Option<&str>,
        sort: RepoSort,
        page: u32,
        limit: u32,
    ) -> Result<Vec<Repository>> {
        self.client
            .get_json(&list_query(query, sort, page, limit).apply("/api/v1/repos"))
            .await
    }

    /// `GET /api/v1/spaces/{space_ref}/repos` — includes visibility.
    pub async fn list_in_space(
        &self,
        space: &str,
        query: Option<&str>,
        sort: RepoSort,
        page: u32,
        limit: u32,
    ) -> Result<Vec<Repository>> {
        let space = space.replace('/', "%2F");
        self.client
            .get_json(
                &list_query(query, sort, page, limit)
                    .apply(&format!("/api/v1/spaces/{space}/repos")),
            )
            .await
    }

    /// `POST /api/v1/repos`
    pub async fn create(&self, input: &CreateRepository) -> Result<Repository> {
        let body = serde_json::to_value(input).map_err(|e| crate::Error::Decode(e.to_string()))?;
        self.client
            .request(Method::POST, "/api/v1/repos", Some(&body), &[])
            .await?
            .deserialize()
    }

    /// `DELETE /api/v1/repos/{repo_ref}` — a soft delete GitFox can restore.
    /// Returns when it was deleted, which is what a restore needs.
    pub async fn delete(&self, repo: &RepoRef) -> Result<Option<i64>> {
        let response = self
            .client
            .request(
                Method::DELETE,
                &format!("/api/v1/repos/{}", repo.encoded()),
                None,
                &[],
            )
            .await?;
        Ok(response
            .json
            .as_ref()
            .and_then(|v| v.get("deleted_at"))
            .and_then(serde_json::Value::as_i64))
    }

    /// `PATCH /api/v1/repos/{repo_ref}` — the description is all it edits.
    pub async fn update_description(
        &self,
        repo: &RepoRef,
        description: &str,
    ) -> Result<Repository> {
        self.post_like(
            Method::PATCH,
            &format!("/api/v1/repos/{}", repo.encoded()),
            json!({ "description": description }),
        )
        .await
    }

    /// `POST …/public-access`
    pub async fn set_public(&self, repo: &RepoRef, is_public: bool) -> Result<Repository> {
        self.post_like(
            Method::POST,
            &format!("/api/v1/repos/{}/public-access", repo.encoded()),
            json!({ "is_public": is_public }),
        )
        .await
    }

    /// `POST …/default-branch`
    pub async fn set_default_branch(&self, repo: &RepoRef, branch: &str) -> Result<Repository> {
        self.post_like(
            Method::POST,
            &format!("/api/v1/repos/{}/default-branch", repo.encoded()),
            json!({ "name": branch }),
        )
        .await
    }

    /// `POST …/move` with a new identifier: renames within the same space.
    pub async fn rename(&self, repo: &RepoRef, identifier: &str) -> Result<Repository> {
        self.post_like(
            Method::POST,
            &format!("/api/v1/repos/{}/move", repo.encoded()),
            json!({ "identifier": identifier }),
        )
        .await
    }

    /// `POST …/sync` — pull the upstream of a mirror or a fork.
    pub async fn sync(&self, repo: &RepoRef) -> Result<()> {
        self.client
            .request(
                Method::POST,
                &format!("/api/v1/repos/{}/sync", repo.encoded()),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `GET …/content/{path}` at `git_ref`, or the default branch.
    pub async fn content(
        &self,
        repo: &RepoRef,
        path: &str,
        git_ref: Option<&str>,
    ) -> Result<Content> {
        let mut q = Query::new();
        q.push_opt("git_ref", git_ref)
            .push("include_commit", "false");
        let path = path.trim_matches('/');
        self.client
            .get_json(&q.apply(&format!(
                "/api/v1/repos/{}/content/{}",
                repo.encoded(),
                encode_path(path)
            )))
            .await
    }

    /// `POST …/rebase` — rebase `head_branch` onto `base_branch`.
    pub async fn rebase(
        &self,
        repo: &RepoRef,
        base_branch: &str,
        head_branch: &str,
        head_commit_sha: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut body = json!({ "base_branch": base_branch, "head_branch": head_branch });
        if let Some(sha) = head_commit_sha {
            body["head_commit_sha"] = json!(sha);
        }
        Ok(self
            .client
            .request(
                Method::POST,
                &format!("/api/v1/repos/{}/rebase", repo.encoded()),
                Some(&body),
                &[],
            )
            .await?
            .json_or_null())
    }

    /// `GET …/commits/{commit_sha}/diff` — a raw unified diff.
    pub async fn commit_diff(&self, repo: &RepoRef, sha: &str) -> Result<String> {
        Ok(self
            .client
            .request(
                Method::GET,
                &format!("/api/v1/repos/{}/commits/{sha}/diff", repo.encoded()),
                None,
                &[("Accept".to_string(), "text/plain".to_string())],
            )
            .await?
            .text)
    }

    /// `GET /api/v1/resources/gitignore` — template names only.
    pub async fn gitignore_templates(&self) -> Result<Vec<String>> {
        self.client.get_json("/api/v1/resources/gitignore").await
    }

    /// `GET /api/v1/resources/license`
    pub async fn license_templates(&self) -> Result<Vec<LicenseTemplate>> {
        self.client.get_json("/api/v1/resources/license").await
    }

    async fn post_like(
        &self,
        method: Method,
        path: &str,
        body: serde_json::Value,
    ) -> Result<Repository> {
        self.client
            .request(method, path, Some(&body), &[])
            .await?
            .deserialize()
    }
}

fn list_query(query: Option<&str>, sort: RepoSort, page: u32, limit: u32) -> Query {
    let mut q = Query::new();
    q.push_opt("query", query)
        .push("sort", sort.as_str())
        .push("order", sort.order())
        .push("page", page)
        .push("limit", limit);
    q
}

/// Percent-encode each segment of a repository path, keeping the slashes that
/// separate them.
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
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
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// How a repository listing is ordered. GitFox sorts by `identifier`, `created`
/// or `updated`; name goes ascending because an alphabetical list read backwards
/// helps nobody, while the other two mean "most recent first".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepoSort {
    #[default]
    Name,
    Created,
    Updated,
}

impl RepoSort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "identifier",
            Self::Created => "created",
            Self::Updated => "updated",
        }
    }

    pub fn order(self) -> &'static str {
        match self {
            Self::Name => "asc",
            Self::Created | Self::Updated => "desc",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listings_are_ordered_the_way_each_sort_key_is_read() {
        assert_eq!(RepoSort::Name.as_str(), "identifier");
        assert_eq!(RepoSort::Name.order(), "asc");
        assert_eq!(RepoSort::Updated.as_str(), "updated");
        assert_eq!(RepoSort::Updated.order(), "desc");
    }

    #[test]
    fn the_space_listing_encodes_a_nested_space_as_one_segment() {
        let q = list_query(None, RepoSort::Name, 1, 30);
        assert_eq!(
            q.apply("/api/v1/spaces/org%2Fteam/repos"),
            "/api/v1/spaces/org%2Fteam/repos?sort=identifier&order=asc&page=1&limit=30"
        );
    }

    #[test]
    fn a_search_term_is_escaped_not_concatenated() {
        let q = list_query(Some("back end"), RepoSort::Name, 1, 30);
        assert!(q.encode().contains("query=back+end"), "{}", q.encode());
    }

    #[test]
    fn a_content_path_keeps_its_slashes_and_escapes_the_rest() {
        assert_eq!(encode_path("src/main.rs"), "src/main.rs");
        assert_eq!(encode_path("docs/a b#c.md"), "docs/a%20b%23c.md");
        assert_eq!(encode_path(""), "");
    }
}
