//! Pull request endpoints.
//!
//! Verified against the GitFox API v1.3.0 OpenAPI document:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list            | `GET /api/v1/repos/{repo_ref}/pullreq` |
//! | view            | `GET /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}` |
//! | create          | `POST /api/v1/repos/{repo_ref}/pullreq` |
//! | edit            | `PATCH …/pullreq/{pullreq_number}` (title and description only) |
//! | merge           | `POST …/pullreq/{pullreq_number}/merge` |
//! | delete branch   | `DELETE …/pullreq/{pullreq_number}/branch` |
//! | close/reopen    | `POST …/pullreq/{pullreq_number}/state` |
//! | draft/ready     | `POST …/pullreq/{pullreq_number}/state` with `is_draft` |
//! | diff            | `GET …/pullreq/{pullreq_number}/diff` |
//! | checks          | `GET …/pullreq/{pullreq_number}/checks` |
//! | commits         | `GET …/pullreq/{pullreq_number}/commits` |
//! | timeline        | `GET …/pullreq/{pullreq_number}/activities` |
//! | comment         | `POST …/pullreq/{pullreq_number}/comments` |
//! | edit comment    | `PATCH …/comments/{pullreq_comment_id}` |
//! | delete comment  | `DELETE …/comments/{pullreq_comment_id}` |
//! | review          | `POST …/pullreq/{pullreq_number}/reviews` |
//! | reviewers       | `GET` / `PUT …/reviewers`, `DELETE …/reviewers/{id}` |
//! | labels          | `GET` / `PUT …/labels`, `DELETE …/labels/{label_id}` |
//!
//! The space-wide listing (`GET /spaces/{space_ref}/pullreq`) is in the
//! document too, but answers 500 on the instances this was checked against, so
//! nothing here depends on it.
//!
//! The diff endpoint content-negotiates: `application/json` yields per-file
//! entries, `text/plain` a raw unified diff. `fx pr diff` picks by output mode.

use serde_json::json;

use crate::client::{GitFoxClient, Method, Query};
use crate::error::Result;
use crate::models::{
    Commit, CreatePullRequest, FileDiff, MergePullRequest, MergeResult, PullRequest,
    PullRequestActivity, PullRequestChecks, PullRequestLabels, PullRequestReviewer,
    PullRequestState, RepoRef, ReviewDecision, UpdatePullRequest,
};

/// Filters for [`PullRequestsApi::list`].
///
/// An empty `state` means "whatever the server defaults to"; the CLI always
/// sets one so the default never surprises anybody.
#[derive(Debug, Clone)]
pub struct PullRequestFilter {
    pub state: Vec<PullRequestState>,
    pub author_id: Option<i64>,
    pub reviewer_id: Option<i64>,
    pub source_branch: Option<String>,
    pub target_branch: Option<String>,
    pub query: Option<String>,
    pub label_id: Vec<i64>,
    pub value_id: Vec<i64>,
    /// `approved`, `changereq`, `pending` or `reviewed`.
    pub review_decision: Vec<String>,
    /// Ask GitFox to embed each pull request's check summary.
    pub include_checks: bool,
    pub page: u32,
    pub limit: u32,
}

impl Default for PullRequestFilter {
    fn default() -> Self {
        Self {
            state: vec![PullRequestState::Open],
            author_id: None,
            reviewer_id: None,
            source_branch: None,
            target_branch: None,
            query: None,
            label_id: Vec::new(),
            value_id: Vec::new(),
            review_decision: Vec::new(),
            include_checks: false,
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
            .push_opt("reviewer_id", self.reviewer_id)
            .push_opt("source_branch", self.source_branch.as_deref())
            .push_opt("target_branch", self.target_branch.as_deref())
            .push_opt("query", self.query.as_deref())
            .extend("label_id", &self.label_id)
            .extend("value_id", &self.value_id)
            .extend("review_decision", &self.review_decision);
        if self.include_checks {
            q.push("include_checks", "true");
        }
        q.push("page", self.page).push("limit", self.limit);
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

    fn path(repo: &RepoRef, number: u64) -> String {
        format!("/api/v1/repos/{}/pullreq/{number}", repo.encoded())
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
        self.client.get_json(&Self::path(repo, number)).await
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

    /// The most recent pull request from `branch` in any state — what a branch
    /// names once its pull request has been merged or closed.
    pub async fn find_any_for_branch(
        &self,
        repo: &RepoRef,
        branch: &str,
    ) -> Result<Option<PullRequest>> {
        if let Some(open) = self.find_for_branch(repo, branch).await? {
            return Ok(Some(open));
        }
        let filter = PullRequestFilter {
            state: vec![PullRequestState::Merged, PullRequestState::Closed],
            source_branch: Some(branch.to_string()),
            limit: 20,
            ..Default::default()
        };
        let mut found = self.list(repo, &filter).await?;
        found.sort_by_key(|pr| std::cmp::Reverse(pr.number));
        Ok(found.into_iter().next())
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

    /// `PATCH /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}`
    pub async fn update(
        &self,
        repo: &RepoRef,
        number: u64,
        input: &UpdatePullRequest,
    ) -> Result<PullRequest> {
        let body = serde_json::to_value(input).map_err(|e| crate::Error::Decode(e.to_string()))?;
        self.client
            .request(Method::PATCH, &Self::path(repo, number), Some(&body), &[])
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
                &format!("{}/merge", Self::path(repo, number)),
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
                &format!("{}/branch", Self::path(repo, number)),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `GET …/pullreq/{pullreq_number}/diff` as structured per-file entries.
    ///
    /// The endpoint content-negotiates: JSON here, raw text in
    /// [`Self::diff_text`]. Both describe the same change.
    pub async fn diff_files(&self, repo: &RepoRef, number: u64) -> Result<Vec<FileDiff>> {
        self.client
            .get_json(&format!("{}/diff", Self::path(repo, number)))
            .await
    }

    /// `GET …/pullreq/{pullreq_number}/diff` as a raw unified diff.
    pub async fn diff_text(&self, repo: &RepoRef, number: u64) -> Result<String> {
        let response = self
            .client
            .request(
                Method::GET,
                &format!("{}/diff", Self::path(repo, number)),
                None,
                &[("Accept".to_string(), "text/plain".to_string())],
            )
            .await?;
        Ok(response.text)
    }

    /// `GET …/pullreq/{pullreq_number}/checks`
    pub async fn checks(&self, repo: &RepoRef, number: u64) -> Result<PullRequestChecks> {
        self.client
            .get_json(&format!("{}/checks", Self::path(repo, number)))
            .await
    }

    /// `GET …/pullreq/{pullreq_number}/commits`, oldest first.
    pub async fn commits(
        &self,
        repo: &RepoRef,
        number: u64,
        page: u32,
        limit: u32,
    ) -> Result<Vec<Commit>> {
        let mut q = Query::new();
        q.push("page", page).push("limit", limit);
        self.client
            .get_json(&q.apply(&format!("{}/commits", Self::path(repo, number))))
            .await
    }

    /// `POST /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}/state`
    pub async fn set_state(
        &self,
        repo: &RepoRef,
        number: u64,
        state: PullRequestState,
    ) -> Result<PullRequest> {
        self.post_state(repo, number, json!({ "state": state.as_str() }))
            .await
    }

    /// Mark an open pull request as a draft, or as ready for review.
    ///
    /// The same endpoint as [`Self::set_state`]: GitFox treats "draft" as a
    /// facet of the open state rather than a state of its own.
    pub async fn set_draft(&self, repo: &RepoRef, number: u64, draft: bool) -> Result<PullRequest> {
        self.post_state(repo, number, json!({ "state": "open", "is_draft": draft }))
            .await
    }

    async fn post_state(
        &self,
        repo: &RepoRef,
        number: u64,
        body: serde_json::Value,
    ) -> Result<PullRequest> {
        self.client
            .request(
                Method::POST,
                &format!("{}/state", Self::path(repo, number)),
                Some(&body),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `GET …/pullreq/{pullreq_number}/activities`, optionally only some kinds
    /// (`comment`, `change-comment`, `system`).
    pub async fn activities(
        &self,
        repo: &RepoRef,
        number: u64,
        kinds: &[&str],
    ) -> Result<Vec<PullRequestActivity>> {
        let mut q = Query::new();
        q.extend("kind", kinds.iter().copied());
        self.client
            .get_json(&q.apply(&format!("{}/activities", Self::path(repo, number))))
            .await
    }

    /// `POST …/pullreq/{pullreq_number}/comments`
    pub async fn comment(
        &self,
        repo: &RepoRef,
        number: u64,
        text: &str,
    ) -> Result<PullRequestActivity> {
        self.client
            .request(
                Method::POST,
                &format!("{}/comments", Self::path(repo, number)),
                Some(&json!({ "text": text })),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `PATCH …/comments/{pullreq_comment_id}`
    pub async fn update_comment(
        &self,
        repo: &RepoRef,
        number: u64,
        comment_id: i64,
        text: &str,
    ) -> Result<PullRequestActivity> {
        self.client
            .request(
                Method::PATCH,
                &format!("{}/comments/{comment_id}", Self::path(repo, number)),
                Some(&json!({ "text": text })),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `DELETE …/comments/{pullreq_comment_id}`
    pub async fn delete_comment(&self, repo: &RepoRef, number: u64, comment_id: i64) -> Result<()> {
        self.client
            .request(
                Method::DELETE,
                &format!("{}/comments/{comment_id}", Self::path(repo, number)),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `POST …/pullreq/{pullreq_number}/reviews`
    ///
    /// A review is a decision about one commit; GitFox has no review body, so
    /// anything the reviewer wants to say is a separate comment.
    pub async fn review(
        &self,
        repo: &RepoRef,
        number: u64,
        commit_sha: &str,
        decision: ReviewDecision,
    ) -> Result<()> {
        self.client
            .request(
                Method::POST,
                &format!("{}/reviews", Self::path(repo, number)),
                Some(&json!({ "commit_sha": commit_sha, "decision": decision.as_str() })),
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `GET …/pullreq/{pullreq_number}/reviewers`
    pub async fn reviewers(&self, repo: &RepoRef, number: u64) -> Result<Vec<PullRequestReviewer>> {
        self.client
            .get_json(&format!("{}/reviewers", Self::path(repo, number)))
            .await
    }

    /// `PUT …/pullreq/{pullreq_number}/reviewers`
    pub async fn add_reviewer(
        &self,
        repo: &RepoRef,
        number: u64,
        reviewer_id: i64,
    ) -> Result<PullRequestReviewer> {
        self.client
            .request(
                Method::PUT,
                &format!("{}/reviewers", Self::path(repo, number)),
                Some(&json!({ "reviewer_id": reviewer_id })),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `DELETE …/pullreq/{pullreq_number}/reviewers/{pullreq_reviewer_id}`
    pub async fn remove_reviewer(
        &self,
        repo: &RepoRef,
        number: u64,
        reviewer_id: i64,
    ) -> Result<()> {
        self.client
            .request(
                Method::DELETE,
                &format!("{}/reviewers/{reviewer_id}", Self::path(repo, number)),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `GET …/pullreq/{pullreq_number}/labels` — every label that could be
    /// attached, each saying whether it is.
    pub async fn labels(&self, repo: &RepoRef, number: u64) -> Result<PullRequestLabels> {
        self.client
            .get_json(&format!("{}/labels?limit=100", Self::path(repo, number)))
            .await
    }

    /// `PUT …/pullreq/{pullreq_number}/labels`
    ///
    /// `value` names a value by text; GitFox creates it when the label allows
    /// new values. `value_id` picks an existing one.
    pub async fn assign_label(
        &self,
        repo: &RepoRef,
        number: u64,
        label_id: i64,
        value_id: Option<i64>,
        value: Option<&str>,
    ) -> Result<()> {
        let mut body = json!({ "label_id": label_id });
        if let Some(value_id) = value_id {
            body["value_id"] = json!(value_id);
        }
        if let Some(value) = value {
            body["value"] = json!(value);
        }
        self.client
            .request(
                Method::PUT,
                &format!("{}/labels", Self::path(repo, number)),
                Some(&body),
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `DELETE …/pullreq/{pullreq_number}/labels/{label_id}`
    pub async fn unassign_label(&self, repo: &RepoRef, number: u64, label_id: i64) -> Result<()> {
        self.client
            .request(
                Method::DELETE,
                &format!("{}/labels/{label_id}", Self::path(repo, number)),
                None,
                &[],
            )
            .await
            .map(|_| ())
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
        for absent in [
            "author_id",
            "reviewer_id",
            "source_branch",
            "target_branch",
            "query",
            "label_id",
            "value_id",
            "review_decision",
            "include_checks",
        ] {
            assert!(
                !encoded.contains(absent),
                "{absent} should be absent: {encoded}"
            );
        }
    }

    #[test]
    fn label_and_review_filters_repeat_like_the_state_filter() {
        let filter = PullRequestFilter {
            label_id: vec![3, 4],
            value_id: vec![9],
            reviewer_id: Some(7),
            review_decision: vec!["approved".into()],
            include_checks: true,
            ..Default::default()
        };
        let encoded = filter.to_query().encode();
        for expected in [
            "label_id=3&label_id=4",
            "value_id=9",
            "reviewer_id=7",
            "review_decision=approved",
            "include_checks=true",
        ] {
            assert!(encoded.contains(expected), "{expected}: {encoded}");
        }
    }
}
