//! `fx pr` — list, view, create and merge pull requests.
//!
//! Every command here works without arguments inside a checkout: the repository
//! comes from the git remote and, where a number is optional, the pull request
//! is the one for the current branch. That is the difference between a tool you
//! reach for and one you look up the syntax for each time.

use gitfox_client::{
    CreatePullRequest, GitFoxClient, MergePullRequest, MergeResult, PullRequest, PullRequestFilter,
    RepoRef,
};
use serde_json::{Value, json};

use crate::cli::{PrCommand, PrCreateArgs, PrListArgs, PrMergeArgs, PrNumberArgs, PrSubcommand};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::git;
use crate::output::{Render, key_values, plain_table, relative_time};

pub async fn run(cmd: PrCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        PrSubcommand::List(args) => list(args, ctx).await,
        PrSubcommand::View(args) => view(args, ctx).await,
        PrSubcommand::Create(args) => create(args, ctx).await,
        PrSubcommand::Merge(args) => merge(args, ctx).await,
        PrSubcommand::Checkout(_) => Err(CliError::not_implemented("fx pr checkout", "v0.5")),
        PrSubcommand::Diff(_) => Err(CliError::not_implemented("fx pr diff", "v0.5")),
        PrSubcommand::Checks(_) => Err(CliError::not_implemented("fx pr checks", "v0.5")),
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(args: PrListArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;

    let author_id = match args.author.as_deref() {
        None => None,
        // A bare number is already a principal id; anything else is a login and
        // costs one lookup, because the API filter is numeric.
        Some(author) => match author.parse::<i64>() {
            Ok(id) => Some(id),
            Err(_) => {
                let principal = client.principals().find_by_login(author).await?;
                let principal = principal.ok_or_else(|| {
                    CliError::new(ErrorCode::NotFound, format!("no user matches `{author}`"))
                })?;
                Some(principal.id.ok_or_else(|| {
                    CliError::new(
                        ErrorCode::ApiError,
                        format!("`{author}` resolved to a user with no id"),
                    )
                })?)
            }
        },
    };

    let filter = PullRequestFilter {
        state: args.state.expand(),
        author_id,
        limit: args.limit,
        ..Default::default()
    };
    // A 404 on the repo-scoped path means the repository, not a pull request —
    // GitFox also answers 404 rather than 401 for repositories the caller
    // cannot see, so this is the common case for a typo or a missing grant.
    let items = client
        .pull_requests()
        .list(&repo, &filter)
        .await
        .map_err(|e| not_found_as_repo(e, &repo))?;

    ctx.renderer
        .emit(&PullRequestList {
            repo: repo.full(),
            items,
        })
        .map_err(unexpected)
}

// ---------------------------------------------------------------------------
// view
// ---------------------------------------------------------------------------

async fn view(args: PrNumberArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pr = resolve_pull_request(args.number, &repo, &client, ctx).await?;
    ctx.renderer.emit(&PullRequestView(pr)).map_err(unexpected)
}

/// A number if given, otherwise the open pull request for the current branch.
async fn resolve_pull_request(
    number: Option<u64>,
    repo: &RepoRef,
    client: &GitFoxClient,
    ctx: &Context,
) -> Result<PullRequest> {
    if let Some(number) = number {
        return client
            .pull_requests()
            .get(repo, number)
            .await
            .map_err(|e| not_found_as_pr(e, &number.to_string()));
    }

    let branch = ctx.branch()?;
    client
        .pull_requests()
        .find_for_branch(repo, branch)
        .await
        .map_err(|e| not_found_as_repo(e, repo))?
        .ok_or_else(|| {
            CliError::new(
                ErrorCode::PrNotFound,
                format!("no open pull request for branch `{branch}` in {repo}"),
            )
            .with_hint("pass a number, or open one with `fx pr create`")
        })
}

/// The client reports a generic 404; at this point we know it was the repo.
fn not_found_as_repo(err: gitfox_client::Error, repo: &RepoRef) -> CliError {
    match err {
        gitfox_client::Error::NotFound { .. } => CliError::new(
            ErrorCode::RepoNotFound,
            format!("no repository {repo}, or you cannot see it"),
        )
        .with_hint("check -R / GITFOX_REPO, and that the token grants access"),
        other => CliError::from(other),
    }
}

/// The client reports a generic 404; at this point we know what was missing.
fn not_found_as_pr(err: gitfox_client::Error, reference: &str) -> CliError {
    match err {
        gitfox_client::Error::NotFound { .. } => CliError::new(
            ErrorCode::PrNotFound,
            format!("no pull request #{reference}"),
        ),
        other => CliError::from(other),
    }
}

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

async fn create(args: PrCreateArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;

    let head = match args.head.clone() {
        Some(head) => head,
        None => ctx.branch()?.to_string(),
    };
    let base = match args.base.clone() {
        Some(base) => base,
        None => default_branch(&client, &repo).await?,
    };
    if base == head {
        return Err(CliError::invalid_argument(format!(
            "base and head are both `{base}`; a pull request needs two different branches"
        )));
    }

    let (title, description) = title_and_body(&args, &base, &head, ctx)?;

    let created = client
        .pull_requests()
        .create(
            &repo,
            &CreatePullRequest {
                title,
                description,
                source_branch: head.clone(),
                target_branch: base.clone(),
                source_repo_ref: None,
                is_draft: args.draft,
            },
        )
        .await?;

    ctx.renderer
        .emit(&PullRequestCreated(created))
        .map_err(unexpected)
}

async fn default_branch(client: &GitFoxClient, repo: &RepoRef) -> Result<String> {
    let repository = client.repos().get(repo).await.map_err(|e| match e {
        gitfox_client::Error::NotFound { .. } => {
            CliError::new(ErrorCode::RepoNotFound, format!("no repository {repo}"))
        }
        other => CliError::from(other),
    })?;
    repository.default_branch.ok_or_else(|| {
        CliError::config(format!("{repo} has no default branch"))
            .with_hint("pass --base explicitly")
    })
}

/// `--title`/`--body`, else `--fill` from the branch's commits, else a prompt.
fn title_and_body(
    args: &PrCreateArgs,
    base: &str,
    head: &str,
    ctx: &Context,
) -> Result<(String, String)> {
    if let Some(title) = args.title.clone() {
        return Ok((title, args.body.clone().unwrap_or_default()));
    }

    if args.fill {
        let commits = git::commits_between(base, head);
        let (title, body) = git::fill_from_commits(&commits).ok_or_else(|| {
            CliError::invalid_argument(format!("no commits on `{head}` that are not on `{base}`"))
                .with_hint("push the branch first, or pass --title")
        })?;
        return Ok((title, args.body.clone().unwrap_or(body)));
    }

    ctx.require_interactive("a pull request title")?;
    let title: String = dialoguer::Input::new()
        .with_prompt("Title")
        .interact_text()
        .map_err(|e| CliError::invalid_argument(format!("could not read the title: {e}")))?;
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(CliError::invalid_argument("a title is required"));
    }
    let body = match args.body.clone() {
        Some(body) => body,
        None => dialoguer::Input::<String>::new()
            .with_prompt("Description")
            .allow_empty(true)
            .interact_text()
            .map_err(|e| CliError::invalid_argument(format!("could not read the body: {e}")))?,
    };
    Ok((title, body))
}

// ---------------------------------------------------------------------------
// merge
// ---------------------------------------------------------------------------

async fn merge(args: PrMergeArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pr = resolve_pull_request(args.number, &repo, &client, ctx).await?;

    let result = client
        .pull_requests()
        .merge(
            &repo,
            pr.number,
            &MergePullRequest {
                method: Some(args.method.into()),
                dry_run: args.dry_run,
                ..Default::default()
            },
        )
        .await?;

    // GitFox has no delete-branch flag on merge, so this is a second request —
    // and only worth making when the merge actually happened.
    let mut branch_deleted = result.branch_deleted.unwrap_or(false);
    if args.delete_branch && !args.dry_run && !branch_deleted {
        client
            .pull_requests()
            .delete_source_branch(&repo, pr.number)
            .await?;
        branch_deleted = true;
    }

    ctx.renderer
        .emit(&PullRequestMerged {
            number: pr.number,
            title: pr.title.clone(),
            source_branch: pr.source_branch.clone(),
            target_branch: pr.target_branch.clone(),
            dry_run: args.dry_run,
            branch_deleted,
            result,
        })
        .map_err(unexpected)
}

fn unexpected(err: std::io::Error) -> CliError {
    CliError::new(ErrorCode::Unexpected, err.to_string())
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

fn branch_arrow(pr: &PullRequest) -> String {
    format!("{} → {}", pr.source_branch, pr.target_branch)
}

fn state_colour(state: &str) -> &'static str {
    match state {
        "open" => "\x1b[32m",
        "merged" => "\x1b[35m",
        "draft" => "\x1b[2m",
        _ => "\x1b[31m",
    }
}

struct PullRequestList {
    repo: String,
    items: Vec<PullRequest>,
}

impl Render for PullRequestList {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.repo,
            "count": self.items.len(),
            "items": self.items,
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.items
            .iter()
            .map(|pr| serde_json::to_value(pr).unwrap_or(Value::Null))
            .collect()
    }

    fn to_human(&self, color: bool) -> String {
        if self.items.is_empty() {
            return format!("No pull requests found in {}", self.repo);
        }
        let rows: Vec<Vec<String>> = self
            .items
            .iter()
            .map(|pr| {
                let state = pr.display_state();
                let state = if color {
                    format!("{}{state}\x1b[0m", state_colour(state))
                } else {
                    state.to_string()
                };
                vec![
                    format!("#{}", pr.number),
                    pr.title.clone(),
                    state,
                    branch_arrow(pr),
                    pr.author_label(),
                    pr.updated.map(relative_time).unwrap_or_default(),
                ]
            })
            .collect();
        plain_table(
            &["number", "title", "state", "branches", "author", "updated"],
            &rows,
        )
    }
}

struct PullRequestView(PullRequest);

impl Render for PullRequestView {
    fn to_json(&self) -> Value {
        serde_json::to_value(&self.0).unwrap_or(Value::Null)
    }

    fn to_human(&self, color: bool) -> String {
        let pr = &self.0;
        let (bold, reset) = if color {
            ("\x1b[1m", "\x1b[0m")
        } else {
            ("", "")
        };
        let state = pr.display_state();
        let state = if color {
            format!("{}{state}{reset}", state_colour(state))
        } else {
            state.to_string()
        };

        let mut pairs = vec![
            ("State", state),
            ("Branches", branch_arrow(pr)),
            ("Author", pr.author_label()),
        ];
        if let Some(updated) = pr.updated {
            pairs.push(("Updated", relative_time(updated)));
        }
        if let Some(stats) = &pr.stats {
            pairs.push((
                "Changes",
                format!(
                    "{} commits, {} files, +{} -{}",
                    stats.commits.unwrap_or(0),
                    stats.files_changed.unwrap_or(0),
                    stats.additions.unwrap_or(0),
                    stats.deletions.unwrap_or(0)
                ),
            ));
        }
        if !pr.merge_conflicts.is_empty() {
            pairs.push(("Conflicts", pr.merge_conflicts.join(", ")));
        }
        if let Some(url) = &pr.web_url {
            pairs.push(("Web", url.clone()));
        }

        let mut out = format!(
            "{bold}#{} {}{reset}\n{}",
            pr.number,
            pr.title,
            key_values(&pairs)
        );
        if !pr.description.trim().is_empty() {
            out.push_str("\n\n");
            out.push_str(pr.description.trim());
        }
        out
    }
}

struct PullRequestCreated(PullRequest);

impl Render for PullRequestCreated {
    fn to_json(&self) -> Value {
        serde_json::to_value(&self.0).unwrap_or(Value::Null)
    }

    fn to_human(&self, color: bool) -> String {
        let pr = &self.0;
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut out = format!(
            "{green}✓{reset} Created pull request #{} ({})\n  {}",
            pr.number,
            branch_arrow(pr),
            pr.title
        );
        if let Some(url) = &pr.web_url {
            out.push_str(&format!("\n  {url}"));
        }
        out
    }
}

struct PullRequestMerged {
    number: u64,
    title: String,
    source_branch: String,
    target_branch: String,
    dry_run: bool,
    branch_deleted: bool,
    result: MergeResult,
}

impl Render for PullRequestMerged {
    fn to_json(&self) -> Value {
        json!({
            "number": self.number,
            "title": self.title,
            "source_branch": self.source_branch,
            "target_branch": self.target_branch,
            "dry_run": self.dry_run,
            "merged": !self.dry_run,
            "mergeable": self.result.mergeable,
            "sha": self.result.sha,
            "branch_deleted": self.branch_deleted,
            "conflict_files": self.result.conflict_files,
            "allowed_methods": self.result.allowed_methods,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, red, reset) = if color {
            ("\x1b[32m", "\x1b[31m", "\x1b[0m")
        } else {
            ("", "", "")
        };

        if self.dry_run {
            let mergeable = self.result.mergeable.unwrap_or(false);
            let mut out = if mergeable {
                format!("{green}✓{reset} #{} can be merged", self.number)
            } else {
                format!("{red}✗{reset} #{} cannot be merged", self.number)
            };
            if !self.result.conflict_files.is_empty() {
                out.push_str(&format!(
                    "\n  conflicts: {}",
                    self.result.conflict_files.join(", ")
                ));
            }
            return out;
        }

        let mut out = format!(
            "{green}✓{reset} Merged #{} into {}\n  {}",
            self.number, self.target_branch, self.title
        );
        if let Some(sha) = &self.result.sha {
            out.push_str(&format!("\n  {}", &sha[..sha.len().min(12)]));
        }
        if self.branch_deleted {
            out.push_str(&format!("\n  deleted branch {}", self.source_branch));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u64, state: &str, draft: bool) -> PullRequest {
        serde_json::from_value(json!({
            "number": number,
            "title": "feat: add OAuth",
            "state": state,
            "is_draft": draft,
            "source_branch": "feat/oauth",
            "target_branch": "main",
            "author": { "uid": "whw" },
            "web_url": "https://git.example.com/pr/12"
        }))
        .unwrap()
    }

    #[test]
    fn list_json_carries_the_repository_and_a_count() {
        let list = PullRequestList {
            repo: "ai/backend".into(),
            items: vec![pr(12, "open", false), pr(13, "merged", false)],
        };
        let value = list.to_json();
        assert_eq!(value["repository"], "ai/backend");
        assert_eq!(value["count"], 2);
        assert_eq!(value["items"][1]["number"], 13);
    }

    #[test]
    fn list_jsonl_emits_one_pull_request_per_line() {
        let list = PullRequestList {
            repo: "ai/backend".into(),
            items: vec![pr(12, "open", false), pr(13, "merged", false)],
        };
        let rows = list.to_jsonl();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["number"], 12);
    }

    #[test]
    fn an_empty_list_says_so_instead_of_printing_a_bare_header() {
        let list = PullRequestList {
            repo: "ai/backend".into(),
            items: vec![],
        };
        assert!(list.to_human(false).contains("No pull requests"));
        assert_eq!(list.to_json()["count"], 0);
    }

    #[test]
    fn a_draft_is_shown_as_draft_in_the_table() {
        let list = PullRequestList {
            repo: "ai/backend".into(),
            items: vec![pr(12, "open", true)],
        };
        let text = list.to_human(false);
        assert!(text.contains("draft"), "{text}");
        assert!(text.contains("feat/oauth → main"), "{text}");
    }

    #[test]
    fn the_human_view_never_emits_colour_when_colour_is_off() {
        let view = PullRequestView(pr(12, "open", false));
        assert!(!view.to_human(false).contains('\x1b'));
        assert!(view.to_human(true).contains('\x1b'));
    }

    #[test]
    fn a_dry_run_merge_reports_mergeability_and_merges_nothing() {
        let merged = PullRequestMerged {
            number: 12,
            title: "feat: add OAuth".into(),
            source_branch: "feat/oauth".into(),
            target_branch: "main".into(),
            dry_run: true,
            branch_deleted: false,
            result: MergeResult {
                mergeable: Some(false),
                conflict_files: vec!["src/main.rs".into()],
                ..Default::default()
            },
        };
        let value = merged.to_json();
        assert_eq!(value["merged"], false);
        assert_eq!(value["mergeable"], false);
        assert_eq!(value["conflict_files"][0], "src/main.rs");
        let text = merged.to_human(false);
        assert!(text.contains("cannot be merged"), "{text}");
        assert!(text.contains("src/main.rs"), "{text}");
    }

    #[test]
    fn a_real_merge_reports_the_sha_and_the_deleted_branch() {
        let merged = PullRequestMerged {
            number: 12,
            title: "feat: add OAuth".into(),
            source_branch: "feat/oauth".into(),
            target_branch: "main".into(),
            dry_run: false,
            branch_deleted: true,
            result: MergeResult {
                sha: Some("0123456789abcdef0123".into()),
                ..Default::default()
            },
        };
        let value = merged.to_json();
        assert_eq!(value["merged"], true);
        assert_eq!(value["branch_deleted"], true);
        let text = merged.to_human(false);
        assert!(
            text.contains("0123456789ab") || text.contains("0123456789ab"),
            "{text}"
        );
        assert!(text.contains("deleted branch feat/oauth"), "{text}");
    }
}
