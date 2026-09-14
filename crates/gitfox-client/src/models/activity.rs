use serde::{Deserialize, Serialize};

use super::Principal;

/// One entry in a pull request's timeline: a comment, a code comment, or a
/// system event such as a merge or a title change.
///
/// `kind` and `kind_type` stay strings for the same reason [`crate::CiStatus`]
/// does — GitFox grows event types, and an unknown one must still decode.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PullRequestActivity {
    #[serde(default)]
    pub id: i64,
    /// `comment`, `change-comment` or `system`.
    #[serde(default)]
    pub kind: String,
    /// `comment`, `code-comment`, `merge`, `state-change`, `review-submit`, …
    #[serde(default, rename = "type")]
    pub kind_type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub author: Option<Principal>,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
    #[serde(default)]
    pub edited: Option<i64>,
    /// Set once the comment was deleted; GitFox keeps the entry.
    #[serde(default)]
    pub deleted: Option<i64>,
    #[serde(default)]
    pub resolved: Option<i64>,
    #[serde(default)]
    pub code_comment: Option<CodeComment>,
}

impl PullRequestActivity {
    /// A comment a person wrote, as opposed to an event GitFox recorded.
    pub fn is_comment(&self) -> bool {
        matches!(self.kind.as_str(), "comment" | "change-comment")
            && self.deleted.is_none_or(|d| d == 0)
    }
}

/// Where a code comment is anchored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodeComment {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub line_new: Option<i64>,
    #[serde(default)]
    pub line_old: Option<i64>,
    #[serde(default)]
    pub outdated: Option<bool>,
    #[serde(default)]
    pub source_sha: Option<String>,
}

/// A reviewer on a pull request, and where their review stands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PullRequestReviewer {
    #[serde(default)]
    pub reviewer: Option<Principal>,
    #[serde(default)]
    pub added_by: Option<Principal>,
    /// `approved`, `changereq`, `pending` or `reviewed`.
    #[serde(default)]
    pub review_decision: Option<String>,
    /// The commit the latest review was given against.
    #[serde(default)]
    pub sha: Option<String>,
    /// `assigned`, `requested` or `self_assigned`.
    #[serde(default, rename = "type")]
    pub kind_type: Option<String>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
}

impl PullRequestReviewer {
    pub fn decision(&self) -> &str {
        self.review_decision.as_deref().unwrap_or("pending")
    }
}

/// What a review says. The wire values are GitFox's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewDecision {
    Approved,
    Changereq,
    Reviewed,
}

impl ReviewDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Changereq => "changereq",
            Self::Reviewed => "reviewed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_system_event_is_not_a_comment_and_a_deleted_comment_is_gone() {
        let event: PullRequestActivity =
            serde_json::from_value(json!({ "id": 1, "kind": "system", "type": "merge" })).unwrap();
        assert!(!event.is_comment());
        assert_eq!(event.kind_type, "merge");

        let comment: PullRequestActivity = serde_json::from_value(
            json!({ "id": 2, "kind": "comment", "type": "comment", "text": "LGTM", "deleted": 0 }),
        )
        .unwrap();
        assert!(comment.is_comment());

        let deleted: PullRequestActivity = serde_json::from_value(
            json!({ "id": 3, "kind": "comment", "type": "comment", "deleted": 1_756_000_000_000i64 }),
        )
        .unwrap();
        assert!(!deleted.is_comment());
    }

    #[test]
    fn a_reviewer_without_a_decision_is_pending() {
        let r: PullRequestReviewer =
            serde_json::from_value(json!({ "reviewer": { "uid": "whw" } })).unwrap();
        assert_eq!(r.decision(), "pending");
    }
}
