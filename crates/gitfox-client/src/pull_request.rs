//! Pull request endpoints.
//!
//! Verified against the GitFox API v1.3.0 OpenAPI document:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list          | `GET /api/v1/repos/{repo_ref}/pullreq` |
//! | list in space | `GET /api/v1/spaces/{space_ref}/pullreq` |
//! | view          | `GET /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}` |
//! | create        | `POST /api/v1/repos/{repo_ref}/pullreq` |
//! | merge         | `POST …/pullreq/{pullreq_number}/merge` |
//! | delete branch | `DELETE …/pullreq/{pullreq_number}/branch` |
//! | close/reopen  | `POST …/pullreq/{pullreq_number}/state` |
//! | diff          | `GET …/pullreq/{pullreq_number}/diff` (v0.5) |
//! | checks        | `GET …/pullreq/{pullreq_number}/checks` (v0.5) |
//! | commits       | `GET …/pullreq/{pullreq_number}/commits` (v0.5) |

use serde_json::json;

use crate::client::{GitFoxClient, Method, Query};
use crate::error::Result;
use crate::models::{
    CreatePullRequest, MergePullRequest, MergeResult, PullRequest, PullRequestState, RepoRef,
};

/// Filters for [`PullRequestsApi::list`].
///
/// An empty `state` means "whatever the server defaults to"; the CLI always
/// sets one so the default never surprises anybody.
#[derive(Debug, Clone)]
pub struct PullRequestFilter {
    pub state: Vec<PullRequestState>,
    pub author_id: Option<i64>,
    pub source_branch: Option<String>,
    pub target_branch: Option<String>,
    pub query: Option<String>,
    pub page: u32,
    pub limit: u32,
}

impl Default for PullRequestFilter {
    fn default() -> Self {
        Self {
            state: vec![PullRequestState::Open],
            author_id: None,
            source_branch: None,
            target_branch: None,
            query: None,
            page: 1,
            limit: 30,
        }
    }
}

impl PullRequestFilter {
    fn to_query(&self) -> Query {
        let mut q = Query::new();
        q.extend("state", self.state.iter().map(|s| s.as_str()))
            .push_opt("author_id", self.author_id)
            .push_opt("source_branch", self.source_branch.as_deref())
            .push_opt("target_branch", self.target_branch.as_deref())
            .push_opt("query", self.query.as_deref())
            .push("page", self.page)
            .push("limit", self.limit);
        q
    }
}

pub struct PullRequestsApi<'a> {
    client: &'a GitFoxClient,
}

impl<'a> PullRequestsApi<'a> {
    pub(crate) fn new(client: &'a GitFoxClient) -> Self {
        Self { client }
    }

    /// `GET /api/v1/repos/{repo_ref}/pullreq`
    pub async fn list(
        &self,
        repo: &RepoRef,
        filter: &PullRequestFilter,
    ) -> Result<Vec<PullRequest>> {
        let path = format!("/api/v1/repos/{}/pullreq", repo.encoded());
        self.client.get_json(&filter.to_query().apply(&path)).await
    }

    /// `GET /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}`
    pub async fn get(&self, repo: &RepoRef, number: u64) -> Result<PullRequest> {
        self.client
            .get_json(&format!(
                "/api/v1/repos/{}/pullreq/{number}",
                repo.encoded()
            ))
            .await
    }

    /// The open pull request whose source branch is `branch`, if there is one.
    ///
    /// This is how `fx pr view` works with no number: the branch you are
    /// standing on identifies the pull request.
    pub async fn find_for_branch(
        &self,
        repo: &RepoRef,
        branch: &str,
    ) -> Result<Option<PullRequest>> {
        let filter = PullRequestFilter {
            source_branch: Some(branch.to_string()),
            limit: 2,
            ..Default::default()
        };
        Ok(self.list(repo, &filter).await?.into_iter().next())
    }

    /// `POST /api/v1/repos/{repo_ref}/pullreq`
    pub async fn create(&self, repo: &RepoRef, input: &CreatePullRequest) -> Result<PullRequest> {
        let body = serde_json::to_value(input).map_err(|e| crate::Error::Decode(e.to_string()))?;
        self.client
            .request(
                Method::POST,
                &format!("/api/v1/repos/{}/pullreq", repo.encoded()),
                Some(&body),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `POST /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}/merge`
    pub async fn merge(
        &self,
        repo: &RepoRef,
        number: u64,
        input: &MergePullRequest,
    ) -> Result<MergeResult> {
        let body = serde_json::to_value(input).map_err(|e| crate::Error::Decode(e.to_string()))?;
        self.client
            .request(
                Method::POST,
                &format!("/api/v1/repos/{}/pullreq/{number}/merge", repo.encoded()),
                Some(&body),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `DELETE /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}/branch`
    ///
    /// Separate from the merge call: GitFox has no `delete_branch` flag on
    /// merge, so `fx pr merge --delete-branch` is two requests.
    pub async fn delete_source_branch(&self, repo: &RepoRef, number: u64) -> Result<()> {
        self.client
            .request(
                Method::DELETE,
                &format!("/api/v1/repos/{}/pullreq/{number}/branch", repo.encoded()),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `POST /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}/state`
    pub async fn set_state(
        &self,
        repo: &RepoRef,
        number: u64,
        state: PullRequestState,
    ) -> Result<PullRequest> {
        let body = json!({ "state": state.as_str() });
        self.client
            .request(
                Method::POST,
                &format!("/api/v1/repos/{}/pullreq/{number}/state", repo.encoded()),
                Some(&body),
                &[],
            )
            .await?
            .deserialize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_filter_asks_for_open_pull_requests_only() {
        let q = PullRequestFilter::default().to_query();
        assert_eq!(q.apply("/p"), "/p?state=open&page=1&limit=30");
    }

    #[test]
    fn several_states_become_repeated_parameters() {
        let filter = PullRequestFilter {
            state: vec![PullRequestState::Open, PullRequestState::Merged],
            ..Default::default()
        };
        assert!(
            filter
                .to_query()
                .encode()
                .contains("state=open&state=merged")
        );
    }

    #[test]
    fn unset_filters_are_left_out_entirely() {
        let encoded = PullRequestFilter::default().to_query().encode();
        for absent in ["author_id", "source_branch", "target_branch", "query"] {
            assert!(
                !encoded.contains(absent),
                "{absent} should be absent: {encoded}"
            );
        }
    }
}
