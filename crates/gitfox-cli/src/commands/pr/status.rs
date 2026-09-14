//! `fx pr status` — the pull requests that concern you in this repository.
//!
//! Three sections, as in gh: the current branch's pull request, the ones you
//! opened, and the ones waiting on your review. GitFox's space-wide listing is
//! unreliable across instances, so this reads the repository's own list with
//! the author filter, and [`super::awaiting_review`] for reviews.

use gitfox_client::{GitFoxClient, PullRequest, PullRequestFilter, RepoRef};
use serde_json::{Value, json};

use super::fields::{PR_FIELDS, PrExtras, pr_field};
use super::{branch_arrow, not_found_as_repo, painted_state};
use crate::cli::PrStatusArgs;
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::export;
use crate::output::Render;

pub async fn status(args: PrStatusArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let me = client.auth().current_user().await?;
    let my_id = me.id.ok_or_else(|| {
        CliError::new(
            ErrorCode::ApiError,
            "GitFox did not report an id for the current user",
        )
    })?;

    let current = match ctx.git.branch.as_deref() {
        Some(branch) => client
            .pull_requests()
            .find_any_for_branch(&repo, branch)
            .await
            .map_err(|e| not_found_as_repo(e, &repo))?,
        None => None,
    };

    let created_by = open_prs(
        &client,
        &repo,
        PullRequestFilter {
            author_id: Some(my_id),
            include_checks: true,
            ..Default::default()
        },
    )
    .await?;
    let needs_review = super::awaiting_review(&client, &repo, my_id).await?;

    ctx.renderer.emit(&Status {
        repo,
        branch: ctx.git.branch.clone(),
        current,
        created_by,
        needs_review,
        conflict_status: args.conflict_status,
    })
}

async fn open_prs(
    client: &GitFoxClient,
    repo: &RepoRef,
    filter: PullRequestFilter,
) -> Result<Vec<PullRequest>> {
    client
        .pull_requests()
        .list(
            repo,
            &PullRequestFilter {
                limit: 30,
                ..filter
            },
        )
        .await
        .map_err(|e| not_found_as_repo(e, repo))
}

struct Status {
    repo: RepoRef,
    branch: Option<String>,
    current: Option<PullRequest>,
    created_by: Vec<PullRequest>,
    needs_review: Vec<PullRequest>,
    conflict_status: bool,
}

impl Status {
    fn section(&self, title: &str, prs: &[PullRequest], empty: &str, color: bool) -> String {
        let (bold, dim, reset) = if color {
            ("\x1b[1m", "\x1b[2m", "\x1b[0m")
        } else {
            ("", "", "")
        };
        let mut out = format!("{bold}{title}{reset}\n");
        if prs.is_empty() {
            out.push_str(&format!("  {dim}{empty}{reset}\n"));
        }
        for pr in prs {
            out.push_str(&format!(
                "  #{}  {} [{}]\n",
                pr.number,
                pr.title,
                branch_arrow(pr)
            ));
            let mut facts = vec![painted_state(pr, color)];
            if let Some(summary) = &pr.check_summary {
                let failing = summary.failure + summary.error;
                let pending = summary.pending + summary.running;
                facts.push(if failing > 0 {
                    format!("{failing} failing checks")
                } else if pending > 0 {
                    format!("{pending} pending checks")
                } else if summary.success > 0 {
                    "checks passing".to_string()
                } else {
                    "no checks".to_string()
                });
            }
            if self.conflict_status {
                facts.push(match pr.merge_check_status.as_deref() {
                    Some("mergeable") => "no merge conflicts".to_string(),
                    Some("conflict") => "merge conflicts".to_string(),
                    _ => "merge status unknown".to_string(),
                });
            }
            out.push_str(&format!("    - {}\n", facts.join(" - ")));
        }
        out
    }
}

impl Render for Status {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.repo.full(),
            "current_branch": self.current,
            "created_by": self.created_by,
            "needs_review": self.needs_review,
        })
    }

    fn export_fields(&self) -> &'static [&'static str] {
        PR_FIELDS
    }

    /// gh's shape: `{"currentBranch": …, "createdBy": […], "needsReview": […]}`.
    fn export(&self, fields: &[String]) -> Value {
        let extras = PrExtras::default();
        let one =
            |pr: &PullRequest| export::select(fields, |f| pr_field(pr, &extras, &self.repo, f));
        json!({
            "currentBranch": self.current.as_ref().map(one),
            "createdBy": self.created_by.iter().map(one).collect::<Vec<_>>(),
            "needsReview": self.needs_review.iter().map(one).collect::<Vec<_>>(),
        })
    }

    fn to_human(&self, color: bool) -> String {
        let mut out = format!("\nRelevant pull requests in {}\n\n", self.repo);
        match (&self.branch, &self.current) {
            (Some(_), Some(pr)) => {
                out.push_str(&self.section("Current branch", std::slice::from_ref(pr), "", color))
            }
            (Some(branch), None) => out.push_str(&self.section(
                "Current branch",
                &[],
                &format!("There is no pull request associated with [{branch}]"),
                color,
            )),
            (None, _) => {
                out.push_str(&self.section("Current branch", &[], "Not on a branch", color))
            }
        }
        out.push('\n');
        out.push_str(&self.section(
            "Created by you",
            &self.created_by,
            "You have no open pull requests",
            color,
        ));
        out.push('\n');
        out.push_str(&self.section(
            "Requesting a code review from you",
            &self.needs_review,
            "You have no pull requests to review",
            color,
        ));
        out.trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u64) -> PullRequest {
        serde_json::from_value(json!({
            "number": number, "title": "feat", "state": "open",
            "source_branch": "feat", "target_branch": "main",
            "check_summary": { "success": 2, "failure": 1 }
        }))
        .unwrap()
    }

    #[test]
    fn status_exports_ghs_three_keys() {
        let status = Status {
            repo: RepoRef::parse("ai/backend").unwrap(),
            branch: Some("feat".into()),
            current: Some(pr(12)),
            created_by: vec![pr(12), pr(13)],
            needs_review: vec![],
            conflict_status: false,
        };
        let value = status.export(&["number".into()]);
        assert_eq!(value["currentBranch"], json!({ "number": 12 }));
        assert_eq!(value["createdBy"][1], json!({ "number": 13 }));
        assert_eq!(value["needsReview"], json!([]));

        let text = status.to_human(false);
        assert!(text.contains("Current branch"), "{text}");
        assert!(text.contains("1 failing checks"), "{text}");
        assert!(
            text.contains("You have no pull requests to review"),
            "{text}"
        );
    }
}
