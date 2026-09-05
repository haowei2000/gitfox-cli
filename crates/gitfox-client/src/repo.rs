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
//! Note the two response shapes: `GET /repos/{repo_ref}` and the space-scoped
//! listing carry `is_public`, the instance-wide `GET /repos` does not. See
//! [`Repository`] for how that difference is represented.

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
}
