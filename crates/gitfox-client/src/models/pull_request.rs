use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::Principal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

impl PullRequestState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Merged => "merged",
        }
    }
}

impl fmt::Display for PullRequestState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a pull request is merged. The wire values are GitFox's, including the
/// hyphen in `fast-forward`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
    FastForward,
}

impl MergeMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
            Self::FastForward => "fast-forward",
        }
    }
}

impl fmt::Display for MergeMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MergeMethod {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "merge" => Ok(Self::Merge),
            "squash" => Ok(Self::Squash),
            "rebase" => Ok(Self::Rebase),
            "fast-forward" | "fastforward" | "ff" => Ok(Self::FastForward),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PullRequestStats {
    #[serde(default)]
    pub commits: Option<i64>,
    #[serde(default)]
    pub files_changed: Option<i64>,
    #[serde(default)]
    pub additions: Option<i64>,
    #[serde(default)]
    pub deletions: Option<i64>,
    #[serde(default)]
    pub conversations: Option<i64>,
    #[serde(default)]
    pub unresolved_count: Option<i64>,
}

/// A pull request.
///
/// Timestamps stay as the raw epoch integers GitFox sends. Interpreting them is
/// a presentation concern, and keeping them raw means the JSON a machine reads
/// never depends on this crate's idea of a date format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub state: PullRequestState,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub author: Option<Principal>,
    #[serde(default)]
    pub merger: Option<Principal>,
    #[serde(default)]
    pub source_branch: String,
    #[serde(default)]
    pub target_branch: String,
    /// Differs from `target_repo_id` when the pull request comes from a fork.
    #[serde(default)]
    pub source_repo_id: Option<i64>,
    #[serde(default)]
    pub target_repo_id: Option<i64>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
    #[serde(default)]
    pub merged: Option<i64>,
    #[serde(default)]
    pub closed: Option<i64>,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default)]
    pub stats: Option<PullRequestStats>,
    #[serde(default)]
    pub merge_check_status: Option<String>,
    #[serde(default)]
    pub merge_conflicts: Vec<String>,
    #[serde(default)]
    pub merge_method: Option<MergeMethod>,
}

impl PullRequest {
    /// `open`, `draft`, `merged` or `closed` — draft is a display state, not an
    /// API one, but it is the first thing a reviewer wants to know.
    pub fn display_state(&self) -> &'static str {
        if self.state == PullRequestState::Open && self.is_draft {
            "draft"
        } else {
            self.state.as_str()
        }
    }

    pub fn author_label(&self) -> String {
        self.author
            .as_ref()
            .map(Principal::label)
            .unwrap_or_default()
    }
}

/// The body of `POST /repos/{repo_ref}/pullreq`.
#[derive(Debug, Clone, Serialize)]
pub struct CreatePullRequest {
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_repo_ref: Option<String>,
    pub is_draft: bool,
}

/// The body of `POST /repos/{repo_ref}/pullreq/{n}/merge`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MergePullRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<MergeMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sha: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bypass_rules: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
}

/// The answer to a merge attempt.
///
/// A `dry_run` merge returns the same shape with `mergeable` telling you whether
/// the real thing would work — which is what `fx pr merge --dry-run` reports.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergeResult {
    #[serde(default)]
    pub sha: Option<String>,
    #[serde(default)]
    pub branch_deleted: Option<bool>,
    #[serde(default)]
    pub mergeable: Option<bool>,
    #[serde(default)]
    pub dry_run: Option<bool>,
    #[serde(default)]
    pub conflict_files: Vec<String>,
    #[serde(default)]
    pub allowed_methods: Vec<MergeMethod>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_methods_use_the_wire_spelling() {
        assert_eq!(MergeMethod::FastForward.as_str(), "fast-forward");
        assert_eq!(
            serde_json::to_string(&MergeMethod::FastForward).unwrap(),
            "\"fast-forward\""
        );
        assert_eq!("fast-forward".parse(), Ok(MergeMethod::FastForward));
        assert_eq!("FF".parse(), Ok(MergeMethod::FastForward));
        assert_eq!("nonsense".parse::<MergeMethod>(), Err(()));
    }

    #[test]
    fn a_draft_reads_as_draft_rather_than_open() {
        let mut pr: PullRequest = serde_json::from_value(serde_json::json!({
            "number": 12, "title": "Add OAuth", "state": "open", "is_draft": true
        }))
        .unwrap();
        assert_eq!(pr.display_state(), "draft");
        pr.is_draft = false;
        assert_eq!(pr.display_state(), "open");
        pr.state = PullRequestState::Merged;
        pr.is_draft = true;
        assert_eq!(pr.display_state(), "merged");
    }

    #[test]
    fn a_minimal_payload_still_decodes() {
        let pr: PullRequest =
            serde_json::from_value(serde_json::json!({ "number": 1, "state": "closed" })).unwrap();
        assert_eq!(pr.number, 1);
        assert!(pr.title.is_empty());
        assert!(pr.author.is_none());
        assert_eq!(pr.author_label(), "");
    }

    #[test]
    fn merge_body_omits_everything_unset() {
        let body = serde_json::to_value(MergePullRequest {
            method: Some(MergeMethod::Squash),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(body, serde_json::json!({ "method": "squash" }));
    }
}

/// One file's worth of a pull request diff.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileDiff {
    #[serde(default)]
    pub path: String,
    /// Set when the file was renamed or copied.
    #[serde(default)]
    pub old_path: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub additions: Option<i64>,
    #[serde(default)]
    pub deletions: Option<i64>,
    #[serde(default)]
    pub changes: Option<i64>,
    #[serde(default)]
    pub is_binary: Option<bool>,
    #[serde(default)]
    pub is_submodule: Option<bool>,
    /// The unified diff for this file. Absent for binaries and submodules.
    #[serde(default)]
    pub patch: Option<String>,
    #[serde(default)]
    pub sha: Option<String>,
    #[serde(default)]
    pub old_sha: Option<String>,
}

/// A status check reported against a pull request's head commit.
///
/// Reuses [`crate::CiStatus`] — a check status is a CI status, and the same
/// open-set reasoning applies: GitFox lists `error`, `failure`, `pending`,
/// `running` and `success` today, and this keeps whichever word arrives.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Check {
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub status: crate::CiStatus,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub started: Option<i64>,
    #[serde(default)]
    pub ended: Option<i64>,
    #[serde(default)]
    pub reported_by: Option<super::Principal>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PullRequestCheck {
    /// Whether a merge is blocked while this check is not green.
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default)]
    pub bypassable: Option<bool>,
    #[serde(default)]
    pub check: Check,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PullRequestChecks {
    /// The commit the checks were reported against.
    #[serde(default)]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub checks: Vec<PullRequestCheck>,
}

impl PullRequestChecks {
    /// Whether anything required is not green.
    ///
    /// A required check that is still running counts as blocking: the answer to
    /// "can this merge" is no, not yet.
    pub fn required_blocking(&self) -> Vec<&PullRequestCheck> {
        self.checks
            .iter()
            .filter(|c| c.required.unwrap_or(false) && !c.check.status.is_success())
            .collect()
    }

    pub fn any_failed(&self) -> bool {
        self.checks.iter().any(|c| c.check.status.is_failed())
    }
}
