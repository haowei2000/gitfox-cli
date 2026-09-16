//! `--json FIELDS` for pull requests: gh's field names and value shapes,
//! computed from GitFox's pull request and whatever extra resources the
//! requested fields need.

use gitfox_client::{
    Commit, FileDiff, GitFoxClient, Principal, PullRequest, PullRequestActivity, PullRequestChecks,
    PullRequestReviewer, PullRequestState, RepoRef,
};
use serde_json::{Value, json};

use crate::error::Result;
use crate::export::{ExportSpec, time_value};

/// Every field `--json` accepts on a pull request: gh's names first, then the
/// snake_case keys of gf's own envelope that gh has no name for.
pub const PR_FIELDS: &[&str] = &[
    "additions",
    "assignees",
    "author",
    "autoMergeRequest",
    "baseRefName",
    "baseRefOid",
    "body",
    "changedFiles",
    "closed",
    "closedAt",
    "closingIssuesReferences",
    "comments",
    "commits",
    "createdAt",
    "deletions",
    "files",
    "fullDatabaseId",
    "headRefName",
    "headRefOid",
    "headRepository",
    "headRepositoryOwner",
    "id",
    "isCrossRepository",
    "isDraft",
    "labels",
    "latestReviews",
    "maintainerCanModify",
    "mergeCommit",
    "mergeStateStatus",
    "mergeable",
    "mergedAt",
    "mergedBy",
    "milestone",
    "number",
    "potentialMergeCommit",
    "projectCards",
    "projectItems",
    "reactionGroups",
    "reviewDecision",
    "reviewRequests",
    "reviews",
    "state",
    "statusCheckRollup",
    "title",
    "updatedAt",
    "url",
    // gf's own names
    "description",
    "is_draft",
    "source_branch",
    "target_branch",
    "created",
    "updated",
    "merged",
    "web_url",
    "stats",
    "merge_check_status",
    "merge_conflicts",
    "merge_method",
    "source_sha",
    "merger",
    "check_summary",
];

/// Resources beyond the pull request itself, fetched only when a requested
/// field reads them.
#[derive(Debug, Default)]
pub struct PrExtras {
    pub comments: Option<Vec<PullRequestActivity>>,
    pub reviewers: Option<Vec<PullRequestReviewer>>,
    pub commits: Option<Vec<Commit>>,
    pub files: Option<Vec<FileDiff>>,
    pub checks: Option<PullRequestChecks>,
}

impl PrExtras {
    /// Fetch what `spec`'s fields need for one pull request — nothing, for the
    /// common case of plain fields.
    pub async fn fetch(
        client: &GitFoxClient,
        repo: &RepoRef,
        number: u64,
        spec: Option<&ExportSpec>,
        with_comments: bool,
    ) -> Result<Self> {
        let mut extras = Self::default();
        let wants = |fields: &[&str]| spec.is_some_and(|s| s.wants_any(fields));

        if with_comments || wants(&["comments"]) {
            let activities = client
                .pull_requests()
                .activities(repo, number, &["comment", "change-comment"])
                .await?;
            extras.comments = Some(activities.into_iter().filter(|a| a.is_comment()).collect());
        }
        if wants(&[
            "reviews",
            "latestReviews",
            "reviewRequests",
            "reviewDecision",
        ]) {
            extras.reviewers = Some(client.pull_requests().reviewers(repo, number).await?);
        }
        if wants(&["commits"]) {
            extras.commits = Some(client.pull_requests().commits(repo, number, 1, 100).await?);
        }
        if wants(&["files"]) {
            extras.files = Some(client.pull_requests().diff_files(repo, number).await?);
        }
        if wants(&["statusCheckRollup"]) {
            extras.checks = Some(client.pull_requests().checks(repo, number).await?);
        }
        Ok(extras)
    }
}

/// gh's `{"login": …}` actor object.
pub fn actor(principal: Option<&Principal>) -> Value {
    match principal {
        None => Value::Null,
        Some(p) => json!({
            "id": p.id.map(|id| id.to_string()).unwrap_or_default(),
            "login": p.uid.clone().unwrap_or_default(),
            "name": p.display_name.clone().unwrap_or_default(),
            "is_bot": false,
        }),
    }
}

/// `OPEN`, `CLOSED`, `MERGED` — gh's uppercase states.
pub fn gh_state(state: PullRequestState) -> &'static str {
    match state {
        PullRequestState::Open => "OPEN",
        PullRequestState::Closed => "CLOSED",
        PullRequestState::Merged => "MERGED",
    }
}

/// A GitFox check status in gh's vocabulary.
pub fn gh_check_state(status: &str) -> &'static str {
    match status {
        "success" => "SUCCESS",
        "failure" => "FAILURE",
        "error" => "ERROR",
        "running" => "IN_PROGRESS",
        "pending" | "blocked" | "waiting_on_dependencies" => "PENDING",
        "skipped" | "declined" => "SKIPPED",
        "killed" => "CANCELLED",
        _ => "PENDING",
    }
}

/// gh's `bucket` for a check: `pass`, `fail`, `pending`, `skipping`, `cancel`.
pub fn gh_check_bucket(status: &str) -> &'static str {
    match status {
        "success" => "pass",
        "failure" | "error" => "fail",
        "skipped" | "declined" => "skipping",
        "killed" => "cancel",
        _ => "pending",
    }
}

fn review_state(decision: &str) -> &'static str {
    match decision {
        "approved" => "APPROVED",
        "changereq" => "CHANGES_REQUESTED",
        "reviewed" => "COMMENTED",
        _ => "PENDING",
    }
}

/// One field of a pull request, as gh would print it.
pub fn pr_field(pr: &PullRequest, extras: &PrExtras, repo: &RepoRef, name: &str) -> Value {
    let stats = pr.stats.clone().unwrap_or_default();
    match name {
        "number" => json!(pr.number),
        "title" => json!(pr.title),
        "body" | "description" => json!(pr.description),
        "state" => json!(gh_state(pr.state)),
        "isDraft" | "is_draft" => json!(pr.is_draft),
        "url" | "web_url" => json!(pr.web_url),
        "id" => json!(format!("{}#{}", repo.full(), pr.number)),
        "fullDatabaseId" => json!(pr.number.to_string()),
        "author" => actor(pr.author.as_ref()),
        "headRefName" | "source_branch" => json!(pr.source_branch),
        "baseRefName" | "target_branch" => json!(pr.target_branch),
        "headRefOid" | "source_sha" => json!(pr.source_sha.clone().unwrap_or_default()),
        "baseRefOid" => json!(pr.merge_target_sha.clone().unwrap_or_default()),
        "createdAt" => time_value(pr.created),
        "updatedAt" => time_value(pr.updated),
        "closedAt" => time_value(pr.closed.or(pr.merged)),
        "mergedAt" => time_value(pr.merged),
        "created" => json!(pr.created),
        "updated" => json!(pr.updated),
        "merged" => json!(pr.merged),
        "closed" => json!(pr.state != PullRequestState::Open),
        "mergedBy" | "merger" => actor(pr.merger.as_ref()),
        "additions" => json!(stats.additions.unwrap_or(0)),
        "deletions" => json!(stats.deletions.unwrap_or(0)),
        "changedFiles" => json!(stats.files_changed.unwrap_or(0)),
        "stats" => serde_json::to_value(&pr.stats).unwrap_or(Value::Null),
        "isCrossRepository" => json!(matches!(
            (pr.source_repo_id, pr.target_repo_id),
            (Some(s), Some(t)) if s != t
        )),
        "headRepository" => json!({ "id": pr.source_repo_id.map(|id| id.to_string()).unwrap_or_default(), "name": repo.name() }),
        "headRepositoryOwner" => json!({ "id": "", "login": repo.space() }),
        "labels" => json!(
            pr.labels
                .iter()
                .map(|label| json!({
                    "id": label.id.map(|id| id.to_string()).unwrap_or_default(),
                    "name": label.name(),
                    "description": "",
                    "color": label
                        .value_color
                        .as_deref()
                        .or(label.color.as_deref())
                        .and_then(gitfox_client::label_color_hex)
                        .unwrap_or_default(),
                }))
                .collect::<Vec<_>>()
        ),
        "mergeable" => json!(match pr.merge_check_status.as_deref() {
            Some("mergeable") => "MERGEABLE",
            Some("conflict") => "CONFLICTING",
            _ => "UNKNOWN",
        }),
        "mergeStateStatus" => json!(match pr.merge_check_status.as_deref() {
            Some("mergeable") => "CLEAN",
            Some("conflict") => "DIRTY",
            _ => "UNKNOWN",
        }),
        "merge_check_status" => json!(pr.merge_check_status),
        "merge_conflicts" => json!(pr.merge_conflicts),
        "merge_method" => json!(pr.merge_method),
        "check_summary" => serde_json::to_value(&pr.check_summary).unwrap_or(Value::Null),
        "comments" => json!(
            extras
                .comments
                .iter()
                .flatten()
                .map(|c| json!({
                    "id": c.id.to_string(),
                    "author": actor(c.author.as_ref()),
                    "authorAssociation": "",
                    "body": c.text,
                    "createdAt": time_value(c.created),
                    "includesCreatedEdit": c.edited.is_some_and(|e| Some(e) != c.created),
                    "isMinimized": false,
                    "minimizedReason": "",
                    "reactionGroups": [],
                    "url": pr.web_url,
                    "viewerDidAuthor": false,
                }))
                .collect::<Vec<_>>()
        ),
        "reviews" | "latestReviews" => json!(
            extras
                .reviewers
                .iter()
                .flatten()
                .filter(|r| r.decision() != "pending")
                .map(|r| json!({
                    "id": r.reviewer.as_ref().and_then(|p| p.id).map(|id| id.to_string()).unwrap_or_default(),
                    "author": actor(r.reviewer.as_ref()),
                    "authorAssociation": "",
                    "body": "",
                    "submittedAt": time_value(r.updated),
                    "includesCreatedEdit": false,
                    "reactionGroups": [],
                    "state": review_state(r.decision()),
                    "commit": { "oid": r.sha.clone().unwrap_or_default() },
                }))
                .collect::<Vec<_>>()
        ),
        "reviewRequests" => json!(
            extras
                .reviewers
                .iter()
                .flatten()
                .filter(|r| r.decision() == "pending")
                .map(|r| {
                    let p = r.reviewer.clone().unwrap_or_default();
                    json!({
                        "__typename": "User",
                        "login": p.uid.unwrap_or_default(),
                        "name": p.display_name.unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>()
        ),
        "reviewDecision" => {
            let reviewers = extras.reviewers.as_deref().unwrap_or_default();
            json!(if reviewers.iter().any(|r| r.decision() == "changereq") {
                "CHANGES_REQUESTED"
            } else if reviewers.iter().any(|r| r.decision() == "approved") {
                "APPROVED"
            } else if reviewers.iter().any(|r| r.decision() == "pending") {
                "REVIEW_REQUIRED"
            } else {
                ""
            })
        }
        "commits" => json!(
            extras
                .commits
                .iter()
                .flatten()
                .map(|c| {
                    let author = c.author.clone().unwrap_or_default();
                    json!({
                        "oid": c.sha,
                        "messageHeadline": c.title,
                        "messageBody": c.body(),
                        "authoredDate": author.when.clone().unwrap_or_default(),
                        "committedDate": c.committer.as_ref().and_then(|s| s.when.clone()).unwrap_or_default(),
                        "authors": [{
                            "email": author.identity.email,
                            "id": "",
                            "login": "",
                            "name": author.identity.name,
                        }],
                    })
                })
                .collect::<Vec<_>>()
        ),
        "files" => json!(
            extras
                .files
                .iter()
                .flatten()
                .map(|f| json!({
                    "path": f.path,
                    "additions": f.additions.unwrap_or(0),
                    "deletions": f.deletions.unwrap_or(0),
                    "changeType": match f.status.as_deref() {
                        Some("added") => "ADDED",
                        Some("deleted") => "DELETED",
                        Some("renamed") => "RENAMED",
                        Some("copied") => "COPIED",
                        _ => "MODIFIED",
                    },
                }))
                .collect::<Vec<_>>()
        ),
        "statusCheckRollup" => json!(
            extras
                .checks
                .iter()
                .flat_map(|c| c.checks.iter())
                .map(|c| {
                    let status = c.check.status.as_str();
                    let finished = !c.check.status.is_pending();
                    json!({
                        "__typename": "CheckRun",
                        "name": c.check.identifier,
                        "status": if finished { "COMPLETED" } else if status == "running" { "IN_PROGRESS" } else { "QUEUED" },
                        "conclusion": if finished { gh_check_state(status) } else { "" },
                        "detailsUrl": c.check.link.clone().unwrap_or_default(),
                        "startedAt": time_value(c.check.started),
                        "completedAt": time_value(c.check.ended),
                        "workflowName": "",
                    })
                })
                .collect::<Vec<_>>()
        ),
        "assignees" | "closingIssuesReferences" | "projectCards" | "projectItems"
        | "reactionGroups" => json!([]),
        "maintainerCanModify" => json!(false),
        "autoMergeRequest" | "milestone" | "mergeCommit" | "potentialMergeCommit" => Value::Null,
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr() -> PullRequest {
        serde_json::from_value(json!({
            "number": 124,
            "title": "feat(assets): add icons",
            "description": "## Summary",
            "state": "open",
            "is_draft": false,
            "author": { "id": 7, "uid": "li.lei", "display_name": "李雷" },
            "source_branch": "feat/icons",
            "target_branch": "develop",
            "source_sha": "04e53c1c",
            "merge_target_sha": "0853993c",
            "source_repo_id": 54,
            "target_repo_id": 54,
            "created": 1_789_346_930_642i64,
            "updated": 1_789_346_930_801i64,
            "merge_check_status": "mergeable",
            "stats": { "additions": 95, "deletions": 0, "files_changed": 19, "commits": 1 },
            "labels": [{ "id": 3, "key": "priority", "value": "high", "color": "red" }],
            "web_url": "https://git.example.com/ai/backend/pulls/124"
        }))
        .unwrap()
    }

    fn field(name: &str) -> Value {
        pr_field(
            &pr(),
            &PrExtras::default(),
            &RepoRef::parse("ai/backend").unwrap(),
            name,
        )
    }

    #[test]
    fn gh_fields_have_ghs_shapes() {
        assert_eq!(field("state"), "OPEN");
        assert_eq!(field("headRefName"), "feat/icons");
        assert_eq!(field("baseRefName"), "develop");
        assert_eq!(field("createdAt"), "2026-09-14T00:48:50Z");
        assert_eq!(field("author")["login"], "li.lei");
        assert_eq!(field("author")["name"], "李雷");
        assert_eq!(field("mergeable"), "MERGEABLE");
        assert_eq!(field("changedFiles"), 19);
        assert_eq!(field("closed"), false);
        assert!(field("mergedAt").is_null());
        assert_eq!(field("isCrossRepository"), false);
        assert_eq!(field("labels")[0]["name"], "priority:high");
        assert_eq!(field("labels")[0]["color"], "ef4444");
        assert_eq!(field("url"), "https://git.example.com/ai/backend/pulls/124");
        assert_eq!(field("assignees"), json!([]));
    }

    #[test]
    fn gf_names_keep_the_envelopes_shapes() {
        assert_eq!(field("source_branch"), "feat/icons");
        assert_eq!(field("created"), 1_789_346_930_642i64);
        assert_eq!(field("description"), "## Summary");
    }

    #[test]
    fn every_listed_field_has_a_value_arm() {
        // A name in PR_FIELDS that fell through to `_` would silently export
        // null; the fields that legitimately are null say so explicitly.
        let null_by_design = [
            "autoMergeRequest",
            "milestone",
            "mergeCommit",
            "potentialMergeCommit",
            "mergedAt",
            "mergedBy",
            "merger",
            "closedAt",
            "merged",
            "merge_method",
            "check_summary",
        ];
        for name in PR_FIELDS {
            if null_by_design.contains(name) {
                continue;
            }
            assert!(!field(name).is_null(), "`{name}` exported null");
        }
    }

    #[test]
    fn review_fields_are_derived_from_the_reviewers() {
        let extras = PrExtras {
            reviewers: Some(
                serde_json::from_value(json!([
                    { "reviewer": { "uid": "a" }, "review_decision": "approved", "sha": "abc" },
                    { "reviewer": { "uid": "b" }, "review_decision": "pending" }
                ]))
                .unwrap(),
            ),
            ..Default::default()
        };
        let repo = RepoRef::parse("ai/backend").unwrap();
        let reviews = pr_field(&pr(), &extras, &repo, "reviews");
        assert_eq!(reviews.as_array().unwrap().len(), 1);
        assert_eq!(reviews[0]["state"], "APPROVED");
        assert_eq!(reviews[0]["commit"]["oid"], "abc");
        assert_eq!(
            pr_field(&pr(), &extras, &repo, "reviewRequests")[0]["login"],
            "b"
        );
        assert_eq!(
            pr_field(&pr(), &extras, &repo, "reviewDecision"),
            "APPROVED"
        );
    }
}
