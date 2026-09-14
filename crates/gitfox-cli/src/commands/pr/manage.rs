//! Changing a pull request: close, reopen, ready, edit, comment, review,
//! update-branch.

use gitfox_client::{
    GitFoxClient, PullRequest, PullRequestActivity, PullRequestState, RepoRef, ReviewDecision,
    UpdatePullRequest,
};
use serde_json::{Value, json};

use super::{Target, branch_arrow, delete_local_branch, pr_web_url, resolve, resolve_label};
use crate::cli::{
    PrCloseArgs, PrCommentArgs, PrEditArgs, PrReadyArgs, PrReopenArgs, PrReviewArgs,
    PrUpdateBranchArgs,
};
use crate::context::{Context, principal_id};
use crate::error::{CliError, ErrorCode, Result};
use crate::interact;
use crate::output::Render;

// ---------------------------------------------------------------------------
// close / reopen / ready
// ---------------------------------------------------------------------------

pub async fn close(args: PrCloseArgs, ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let Target { repo, pr } = resolve(Some(&args.selector), ctx, &client).await?;

    match pr.state {
        PullRequestState::Merged => {
            return Err(CliError::invalid_argument(format!(
                "pull request #{} can't be closed because it was already merged",
                pr.number
            )));
        }
        PullRequestState::Closed => {
            ctx.warn(&format!("pull request #{} is already closed", pr.number));
            return ctx.renderer.emit(&StateChanged::unchanged(pr));
        }
        PullRequestState::Open => {}
    }

    if let Some(comment) = args.comment.as_deref() {
        client
            .pull_requests()
            .comment(&repo, pr.number, comment)
            .await?;
    }
    let closed = client
        .pull_requests()
        .set_state(&repo, pr.number, PullRequestState::Closed)
        .await?;

    let mut branch_deleted = false;
    if args.delete_branch {
        client
            .pull_requests()
            .delete_source_branch(&repo, pr.number)
            .await?;
        branch_deleted = true;
        delete_local_branch(ctx, &repo, &pr.source_branch, &pr.target_branch);
    }

    ctx.renderer.emit(&StateChanged {
        verb: "Closed",
        pr: closed,
        branch_deleted,
    })
}

pub async fn reopen(args: PrReopenArgs, ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let Target { repo, pr } = resolve(Some(&args.selector), ctx, &client).await?;

    match pr.state {
        PullRequestState::Merged => {
            return Err(CliError::invalid_argument(format!(
                "pull request #{} can't be reopened because it was already merged",
                pr.number
            )));
        }
        PullRequestState::Open => {
            ctx.warn(&format!("pull request #{} is already open", pr.number));
            return ctx.renderer.emit(&StateChanged::unchanged(pr));
        }
        PullRequestState::Closed => {}
    }

    let reopened = client
        .pull_requests()
        .set_state(&repo, pr.number, PullRequestState::Open)
        .await?;
    if let Some(comment) = args.comment.as_deref() {
        client
            .pull_requests()
            .comment(&repo, pr.number, comment)
            .await?;
    }

    ctx.renderer.emit(&StateChanged {
        verb: "Reopened",
        pr: reopened,
        branch_deleted: false,
    })
}

pub async fn ready(args: PrReadyArgs, ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let Target { repo, pr } = resolve(args.target.selector.as_deref(), ctx, &client).await?;

    if pr.state != PullRequestState::Open {
        return Err(CliError::invalid_argument(format!(
            "pull request #{} is {}",
            pr.number,
            pr.state.as_str()
        )));
    }
    if pr.is_draft == args.undo {
        ctx.warn(&format!(
            "pull request #{} is already {}",
            pr.number,
            if args.undo {
                "a draft"
            } else {
                "\"ready for review\""
            }
        ));
        return ctx.renderer.emit(&StateChanged::unchanged(pr));
    }

    let changed = client
        .pull_requests()
        .set_draft(&repo, pr.number, args.undo)
        .await?;
    ctx.renderer.emit(&StateChanged {
        verb: if args.undo {
            "Converted to draft"
        } else {
            "Marked as ready for review"
        },
        pr: changed,
        branch_deleted: false,
    })
}

struct StateChanged {
    /// `None` when nothing needed doing.
    verb: &'static str,
    pr: PullRequest,
    branch_deleted: bool,
}

impl StateChanged {
    fn unchanged(pr: PullRequest) -> Self {
        Self {
            verb: "",
            pr,
            branch_deleted: false,
        }
    }
}

impl Render for StateChanged {
    fn to_json(&self) -> Value {
        json!({
            "number": self.pr.number,
            "title": self.pr.title,
            "state": self.pr.state.as_str(),
            "is_draft": self.pr.is_draft,
            "changed": !self.verb.is_empty(),
            "branch_deleted": self.branch_deleted,
            "web_url": self.pr.web_url,
        })
    }

    fn to_human(&self, color: bool) -> String {
        if self.verb.is_empty() {
            return String::new();
        }
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut out = format!(
            "{green}✓{reset} {} pull request #{} ({})",
            self.verb, self.pr.number, self.pr.title
        );
        if self.branch_deleted {
            out.push_str(&format!("\n  deleted branch {}", self.pr.source_branch));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// edit
// ---------------------------------------------------------------------------

pub async fn edit(args: PrEditArgs, ctx: &Context) -> Result<()> {
    if args.base.is_some() {
        return Err(CliError::unsupported(
            "`--base`",
            "GitFox cannot change a pull request's target branch",
        )
        .with_hint("close it and open a new one against the other branch"));
    }
    if !args.add_assignee.is_empty() || !args.remove_assignee.is_empty() {
        return Err(CliError::unsupported(
            "assignees",
            "GitFox pull requests have reviewers, not assignees",
        )
        .with_hint("use --add-reviewer / --remove-reviewer"));
    }
    if args.milestone.is_some() || args.remove_milestone {
        return Err(CliError::unsupported(
            "milestones",
            "GitFox has no milestones",
        ));
    }
    if !args.add_project.is_empty() || !args.remove_project.is_empty() {
        return Err(CliError::unsupported("projects", "GitFox has no projects"));
    }
    if !args.attach.is_empty() {
        return Err(CliError::unsupported(
            "`--attach`",
            "fx cannot upload attachments to a GitFox pull request",
        ));
    }

    let body = interact::body_from(args.body.as_deref(), args.body_file.as_deref())?;
    let nothing = args.title.is_none()
        && body.is_none()
        && args.add_reviewer.is_empty()
        && args.remove_reviewer.is_empty()
        && args.add_label.is_empty()
        && args.remove_label.is_empty();
    if nothing {
        return Err(CliError::invalid_argument("nothing to edit").with_hint(
            "pass --title, --body, --add-reviewer, --add-label or their --remove-* forms",
        ));
    }

    let client = ctx.client()?;
    let Target { repo, pr } = resolve(args.target.selector.as_deref(), ctx, &client).await?;
    let pulls = client.pull_requests();

    if args.title.is_some() || body.is_some() {
        pulls
            .update(
                &repo,
                pr.number,
                &UpdatePullRequest {
                    title: args.title.clone(),
                    description: body,
                },
            )
            .await?;
    }
    for login in &args.add_reviewer {
        let id = principal_id(&client, login).await?;
        pulls.add_reviewer(&repo, pr.number, id).await?;
    }
    for login in &args.remove_reviewer {
        let id = principal_id(&client, login).await?;
        pulls.remove_reviewer(&repo, pr.number, id).await?;
    }
    for name in &args.add_label {
        let (label, value_id, value) = resolve_label(&client, &repo, name).await?;
        pulls
            .assign_label(&repo, pr.number, label, value_id, value.as_deref())
            .await?;
    }
    for name in &args.remove_label {
        let key = name.split(':').next().unwrap_or(name).trim();
        let attached = pr.labels.iter().find(|l| l.key.eq_ignore_ascii_case(key));
        let id = match attached.and_then(|l| l.id) {
            Some(id) => id,
            None => resolve_label(&client, &repo, key).await?.0,
        };
        pulls.unassign_label(&repo, pr.number, id).await?;
    }

    let edited = pulls.get(&repo, pr.number).await?;
    ctx.renderer.emit(&Edited(edited))
}

struct Edited(PullRequest);

impl Render for Edited {
    fn to_json(&self) -> Value {
        serde_json::to_value(&self.0).unwrap_or(Value::Null)
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut out = format!(
            "{green}✓{reset} Edited pull request #{} ({})\n  {}",
            self.0.number,
            branch_arrow(&self.0),
            self.0.title
        );
        if let Some(url) = &self.0.web_url {
            out.push_str(&format!("\n  {url}"));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// comment
// ---------------------------------------------------------------------------

pub async fn comment(args: PrCommentArgs, ctx: &Context) -> Result<()> {
    if !args.attach.is_empty() {
        return Err(CliError::unsupported(
            "`--attach`",
            "fx cannot upload attachments to a GitFox pull request",
        ));
    }
    let client = ctx.client()?;
    let Target { repo, pr } = resolve(args.target.selector.as_deref(), ctx, &client).await?;

    if args.web {
        return interact::open_in_browser(ctx, &pr_web_url(ctx, &repo, &pr, None)?);
    }

    if args.delete_last {
        let last = last_own_comment(&client, &repo, pr.number)
            .await?
            .ok_or_else(|| {
                CliError::new(ErrorCode::NotFound, "no comments found for current user")
            })?;
        if !args.yes {
            interact::confirm(
                ctx,
                &format!("Delete your last comment on #{}?", pr.number),
                "--yes",
            )?;
        }
        client
            .pull_requests()
            .delete_comment(&repo, pr.number, last.id)
            .await?;
        return ctx.renderer.emit(&Commented {
            action: "Deleted",
            number: pr.number,
            comment: last,
            url: pr.web_url.clone(),
        });
    }

    let body = match interact::body_from(args.body.as_deref(), args.body_file.as_deref())? {
        Some(body) => body,
        None if args.editor => interact::edit_text(ctx, "", "COMMENT.md")?,
        None => {
            ctx.require_interactive("a comment body")
                .map_err(|e| e.with_hint("pass --body or --body-file"))?;
            dialoguer::Input::<String>::new()
                .with_prompt("Comment")
                .interact_text()
                .map_err(|e| {
                    CliError::invalid_argument(format!("could not read the comment: {e}"))
                })?
        }
    };
    if body.trim().is_empty() {
        return Err(CliError::invalid_argument("comment body cannot be blank"));
    }

    if args.edit_last {
        match last_own_comment(&client, &repo, pr.number).await? {
            Some(last) => {
                let updated = client
                    .pull_requests()
                    .update_comment(&repo, pr.number, last.id, &body)
                    .await?;
                return ctx.renderer.emit(&Commented {
                    action: "Edited",
                    number: pr.number,
                    comment: updated,
                    url: pr.web_url.clone(),
                });
            }
            None if !args.create_if_none => {
                return Err(CliError::new(
                    ErrorCode::NotFound,
                    "no comments found for current user",
                )
                .with_hint("pass --create-if-none to add one instead"));
            }
            None => {}
        }
    }

    let created = client
        .pull_requests()
        .comment(&repo, pr.number, &body)
        .await?;
    ctx.renderer.emit(&Commented {
        action: "Added",
        number: pr.number,
        comment: created,
        url: pr.web_url.clone(),
    })
}

/// The caller's most recent comment on the pull request.
async fn last_own_comment(
    client: &GitFoxClient,
    repo: &RepoRef,
    number: u64,
) -> Result<Option<PullRequestActivity>> {
    let me = client.auth().current_user().await?;
    let activities = client
        .pull_requests()
        .activities(repo, number, &["comment", "change-comment"])
        .await?;
    Ok(activities
        .into_iter()
        .filter(|a| a.is_comment())
        .filter(|a| {
            let author = a.author.as_ref();
            match (author.and_then(|p| p.id), me.id) {
                (Some(author), Some(me)) => author == me,
                _ => author.and_then(|p| p.uid.as_deref()) == me.uid.as_deref(),
            }
        })
        .max_by_key(|a| (a.created.unwrap_or(0), a.id)))
}

struct Commented {
    action: &'static str,
    number: u64,
    comment: PullRequestActivity,
    url: Option<String>,
}

impl Render for Commented {
    fn to_json(&self) -> Value {
        json!({
            "action": self.action.to_ascii_lowercase(),
            "number": self.number,
            "comment_id": self.comment.id,
            "text": self.comment.text,
            "web_url": self.url,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut out = format!(
            "{green}✓{reset} {} a comment on #{}",
            self.action, self.number
        );
        if let Some(url) = &self.url {
            out.push_str(&format!("\n  {url}"));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// review
// ---------------------------------------------------------------------------

pub async fn review(args: PrReviewArgs, ctx: &Context) -> Result<()> {
    let decision = if args.approve {
        ReviewDecision::Approved
    } else if args.request_changes {
        ReviewDecision::Changereq
    } else if args.comment {
        ReviewDecision::Reviewed
    } else {
        if ctx.config.non_interactive {
            return Err(CliError::invalid_argument(
                "--approve, --request-changes, or --comment required when not running interactively",
            ));
        }
        let choice = dialoguer::Select::new()
            .with_prompt("What kind of review do you want to give?")
            .items(["Comment", "Approve", "Request changes"])
            .default(0)
            .interact()
            .map_err(|e| CliError::invalid_argument(format!("could not read the choice: {e}")))?;
        match choice {
            1 => ReviewDecision::Approved,
            2 => ReviewDecision::Changereq,
            _ => ReviewDecision::Reviewed,
        }
    };
    let body = interact::body_from(args.body.as_deref(), args.body_file.as_deref())?
        .filter(|b| !b.trim().is_empty());
    if body.is_none() && decision != ReviewDecision::Approved {
        return Err(CliError::invalid_argument(format!(
            "body cannot be blank for {} review",
            if decision == ReviewDecision::Changereq {
                "request-changes"
            } else {
                "comment"
            }
        )));
    }

    let client = ctx.client()?;
    let Target { repo, pr } = resolve(args.target.selector.as_deref(), ctx, &client).await?;
    let sha = match pr.source_sha.clone().filter(|s| !s.is_empty()) {
        Some(sha) => sha,
        None => client
            .pull_requests()
            .commits(&repo, pr.number, 1, 100)
            .await?
            .last()
            .map(|c| c.sha.clone())
            .ok_or_else(|| {
                CliError::new(
                    ErrorCode::ApiError,
                    format!("#{} reports no head commit to review", pr.number),
                )
            })?,
    };

    // A GitFox review is a decision about a commit with no text of its own,
    // so the text travels as a comment beside it.
    if let Some(body) = &body {
        client
            .pull_requests()
            .comment(&repo, pr.number, body)
            .await?;
    }
    client
        .pull_requests()
        .review(&repo, pr.number, &sha, decision)
        .await?;

    ctx.renderer.emit(&Reviewed {
        number: pr.number,
        decision,
        commit: sha,
        commented: body.is_some(),
    })
}

struct Reviewed {
    number: u64,
    decision: ReviewDecision,
    commit: String,
    commented: bool,
}

impl Render for Reviewed {
    fn to_json(&self) -> Value {
        json!({
            "number": self.number,
            "decision": self.decision.as_str(),
            "commit_sha": self.commit,
            "commented": self.commented,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, yellow, reset) = if color {
            ("\x1b[32m", "\x1b[33m", "\x1b[0m")
        } else {
            ("", "", "")
        };
        match self.decision {
            ReviewDecision::Approved => {
                format!("{green}✓{reset} Approved pull request #{}", self.number)
            }
            ReviewDecision::Changereq => format!(
                "{yellow}+{reset} Requested changes to pull request #{}",
                self.number
            ),
            ReviewDecision::Reviewed => {
                format!("{green}✓{reset} Reviewed pull request #{}", self.number)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// update-branch
// ---------------------------------------------------------------------------

pub async fn update_branch(args: PrUpdateBranchArgs, ctx: &Context) -> Result<()> {
    if !args.rebase {
        return Err(CliError::unsupported(
            "updating a branch by merging",
            "GitFox can only update a pull request branch by rebasing it",
        )
        .with_hint("pass --rebase"));
    }
    let client = ctx.client()?;
    let Target { repo, pr } = resolve(args.target.selector.as_deref(), ctx, &client).await?;
    if pr.state != PullRequestState::Open {
        return Err(CliError::invalid_argument(format!(
            "pull request #{} is {}",
            pr.number,
            pr.state.as_str()
        )));
    }

    let result = client
        .repos()
        .rebase(
            &repo,
            &pr.target_branch,
            &pr.source_branch,
            pr.source_sha.as_deref(),
        )
        .await?;

    let conflicts: Vec<String> = result
        .get("conflict_files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if !conflicts.is_empty() {
        return Err(CliError::new(
            ErrorCode::ApiError,
            format!(
                "#{} cannot be rebased onto {} without conflicts",
                pr.number, pr.target_branch
            ),
        )
        .with_details(json!({ "conflict_files": conflicts })));
    }

    ctx.renderer.emit(&BranchUpdated {
        number: pr.number,
        source_branch: pr.source_branch.clone(),
        target_branch: pr.target_branch.clone(),
        already_up_to_date: result
            .get("already_ancestor")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        new_sha: result
            .get("new_head_branch_sha")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

struct BranchUpdated {
    number: u64,
    source_branch: String,
    target_branch: String,
    already_up_to_date: bool,
    new_sha: Option<String>,
}

impl Render for BranchUpdated {
    fn to_json(&self) -> Value {
        json!({
            "number": self.number,
            "source_branch": self.source_branch,
            "target_branch": self.target_branch,
            "already_up_to_date": self.already_up_to_date,
            "sha": self.new_sha,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        if self.already_up_to_date {
            return format!(
                "{green}✓{reset} #{}'s branch is already up to date with {}",
                self.number, self.target_branch
            );
        }
        format!(
            "{green}✓{reset} Rebased {} onto {} for #{}",
            self.source_branch, self.target_branch, self.number
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(state: &str) -> PullRequest {
        serde_json::from_value(json!({
            "number": 12, "title": "feat: add OAuth", "state": state,
            "source_branch": "feat/oauth", "target_branch": "main"
        }))
        .unwrap()
    }

    #[test]
    fn a_state_change_says_what_happened_and_an_unchanged_one_says_nothing() {
        let closed = StateChanged {
            verb: "Closed",
            pr: pr("closed"),
            branch_deleted: true,
        };
        assert_eq!(closed.to_json()["changed"], true);
        assert_eq!(closed.to_json()["state"], "closed");
        let text = closed.to_human(false);
        assert!(text.contains("Closed pull request #12"), "{text}");
        assert!(text.contains("deleted branch feat/oauth"), "{text}");

        let unchanged = StateChanged::unchanged(pr("open"));
        assert_eq!(unchanged.to_json()["changed"], false);
        assert_eq!(unchanged.to_human(false), "");
    }

    #[test]
    fn a_review_reports_its_decision_in_gitfoxs_words() {
        let r = Reviewed {
            number: 12,
            decision: ReviewDecision::Changereq,
            commit: "abc".into(),
            commented: true,
        };
        assert_eq!(r.to_json()["decision"], "changereq");
        assert!(r.to_human(false).contains("Requested changes"));
    }
}
