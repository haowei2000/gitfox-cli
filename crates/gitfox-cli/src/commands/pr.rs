//! `fx pr` — list, view, create and merge pull requests.
//!
//! Every command here works without arguments inside a checkout: the repository
//! comes from the git remote and, where a number is optional, the pull request
//! is the one for the current branch. That is the difference between a tool you
//! reach for and one you look up the syntax for each time.

use gitfox_client::{
    CreatePullRequest, FileDiff, GitFoxClient, MergePullRequest, MergeResult, PullRequest,
    PullRequestChecks, PullRequestFilter, RepoRef,
};
use serde_json::{Value, json};

use crate::cli::{
    PrCommand, PrCreateArgs, PrDiffArgs, PrListArgs, PrMergeArgs, PrNumberArgs, PrSubcommand,
};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::git;
use crate::output::{Render, key_values, plain_table, relative_time};
use crate::paginate;

pub async fn run(cmd: PrCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        PrSubcommand::List(args) => list(args, ctx).await,
        PrSubcommand::View(args) => view(args, ctx).await,
        PrSubcommand::Create(args) => create(args, ctx).await,
        PrSubcommand::Merge(args) => merge(args, ctx).await,
        PrSubcommand::Checkout(args) => checkout(args, ctx).await,
        PrSubcommand::Diff(args) => diff(args, ctx).await,
        PrSubcommand::Checks(args) => checks(args, ctx).await,
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

    let base = PullRequestFilter {
        state: args.state.expand(),
        author_id,
        ..Default::default()
    };

    // A 404 on the repo-scoped path means the repository, not a pull request —
    // GitFox also answers 404 rather than 401 for repositories the caller
    // cannot see, so this is the common case for a typo or a missing grant.
    let (client, repo_ref, base) = (&client, &repo, &base);
    let paged = paginate::collect(args.limit, move |page, limit| async move {
        let filter = PullRequestFilter {
            page,
            limit,
            ..base.clone()
        };
        client.pull_requests().list(repo_ref, &filter).await
    })
    .await
    .map_err(|e| not_found_as_repo(e, &repo))?;

    ctx.renderer
        .emit(&PullRequestList {
            repo: repo.full(),
            items: paged.items,
            truncated: paged.truncated,
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

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

async fn diff(args: PrDiffArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pr = resolve_pull_request(args.number, &repo, &client, ctx).await?;

    // The endpoint serves either form. A person wants the patch their pager and
    // syntax highlighter understand; a machine wants it split by file. Asking
    // for the one that suits the output mode beats reassembling either.
    let raw = if ctx.config.output.is_machine() || args.name_only {
        None
    } else {
        Some(client.pull_requests().diff_text(&repo, pr.number).await?)
    };
    let files = match &raw {
        Some(_) => Vec::new(),
        None => client.pull_requests().diff_files(&repo, pr.number).await?,
    };

    ctx.renderer
        .emit(&PullRequestDiff {
            number: pr.number,
            source_branch: pr.source_branch.clone(),
            target_branch: pr.target_branch.clone(),
            name_only: args.name_only,
            raw,
            files,
        })
        .map_err(unexpected)
}

// ---------------------------------------------------------------------------
// checks
// ---------------------------------------------------------------------------

async fn checks(args: PrNumberArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pr = resolve_pull_request(args.number, &repo, &client, ctx).await?;
    let checks = client.pull_requests().checks(&repo, pr.number).await?;

    ctx.renderer
        .emit(&PullRequestChecksView {
            number: pr.number,
            checks,
        })
        .map_err(unexpected)
}

// ---------------------------------------------------------------------------
// checkout
// ---------------------------------------------------------------------------

async fn checkout(args: PrNumberArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pr = resolve_pull_request(args.number, &repo, &client, ctx).await?;

    // A fork's branch does not live on this remote, and guessing at where it
    // does would be worse than saying so.
    if let (Some(source), Some(target)) = (pr.source_repo_id, pr.target_repo_id)
        && source != target
    {
        return Err(CliError::new(
            ErrorCode::GitContextError,
            format!(
                "#{} comes from a fork, which fx cannot check out yet",
                pr.number
            ),
        )
        .with_hint("add the fork as a remote and check the branch out with git"));
    }

    let remote = git::remote_name().ok_or_else(|| {
        CliError::new(
            ErrorCode::GitContextError,
            "no git remote to fetch from, or not inside a git repository",
        )
    })?;
    let branch = pr.source_branch.clone();
    if branch.is_empty() {
        return Err(CliError::new(
            ErrorCode::ApiError,
            format!("#{} reports no source branch", pr.number),
        ));
    }

    git::fetch(&remote, &branch).map_err(git_failed)?;

    let existed = git::local_branch_exists(&branch);
    if existed {
        git::checkout(&branch).map_err(git_failed)?;
        // Fast-forward only: a diverged local branch is the user's to resolve.
        git::merge_ff_only("FETCH_HEAD").map_err(|message| {
            git_failed(message).with_hint(format!(
                "`{branch}` has diverged from {remote}; reconcile it yourself"
            ))
        })?;
    } else {
        git::checkout_new(&branch, "FETCH_HEAD").map_err(git_failed)?;
        // Best effort: some remotes have no tracking ref for a fresh branch.
        let _ = git::set_upstream(&branch, &remote);
    }

    ctx.renderer
        .emit(&PullRequestCheckedOut {
            number: pr.number,
            title: pr.title.clone(),
            branch,
            remote,
            existed,
        })
        .map_err(unexpected)
}

fn git_failed(message: String) -> CliError {
    CliError::new(ErrorCode::GitContextError, message)
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
    /// Whether the server had more than `--limit` allowed through.
    truncated: bool,
}

impl Render for PullRequestList {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.repo,
            "count": self.items.len(),
            "truncated": self.truncated,
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
        let mut out = plain_table(
            &["number", "title", "state", "branches", "author", "updated"],
            &rows,
        );
        if self.truncated {
            out.push_str(&format!(
                "\n\nShowing {} of more; raise --limit to see the rest.",
                self.items.len()
            ));
        }
        out
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
            truncated: false,
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
            truncated: true,
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
            truncated: false,
        };
        assert!(list.to_human(false).contains("No pull requests"));
        assert_eq!(list.to_json()["count"], 0);
    }

    #[test]
    fn a_draft_is_shown_as_draft_in_the_table() {
        let list = PullRequestList {
            repo: "ai/backend".into(),
            items: vec![pr(12, "open", true)],
            truncated: false,
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

// ---------------------------------------------------------------------------
// v0.5 rendering
// ---------------------------------------------------------------------------

struct PullRequestDiff {
    number: u64,
    source_branch: String,
    target_branch: String,
    name_only: bool,
    /// The raw unified diff, when that is what was fetched.
    raw: Option<String>,
    files: Vec<FileDiff>,
}

impl Render for PullRequestDiff {
    fn to_json(&self) -> Value {
        json!({
            "number": self.number,
            "source_branch": self.source_branch,
            "target_branch": self.target_branch,
            "count": self.files.len(),
            "files": self.files.iter().map(|f| json!({
                "path": f.path,
                "old_path": f.old_path,
                "status": f.status,
                "additions": f.additions,
                "deletions": f.deletions,
                "changes": f.changes,
                "is_binary": f.is_binary,
                "is_submodule": f.is_submodule,
                // Omitted wholesale under --name-only, which is the point of it.
                "patch": if self.name_only { None } else { f.patch.clone() },
            })).collect::<Vec<_>>(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.to_json()["files"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn to_human(&self, _color: bool) -> String {
        // Handed over verbatim so a pager or a highlighter sees a real diff.
        if let Some(raw) = &self.raw {
            return raw.trim_end().to_string();
        }
        if self.files.is_empty() {
            return format!("#{} changes nothing", self.number);
        }
        let rows: Vec<Vec<String>> = self
            .files
            .iter()
            .map(|f| {
                vec![
                    f.status.clone().unwrap_or_default(),
                    format!("+{}", f.additions.unwrap_or(0)),
                    format!("-{}", f.deletions.unwrap_or(0)),
                    f.path.clone(),
                ]
            })
            .collect();
        plain_table(&["status", "added", "removed", "path"], &rows)
    }
}

struct PullRequestChecksView {
    number: u64,
    checks: PullRequestChecks,
}

impl Render for PullRequestChecksView {
    fn to_json(&self) -> Value {
        json!({
            "number": self.number,
            "commit_sha": self.checks.commit_sha,
            "count": self.checks.checks.len(),
            "failed": self.checks.any_failed(),
            "blocking": self.checks.required_blocking().len(),
            "checks": self.checks.checks.iter().map(|c| json!({
                "name": c.check.identifier,
                "status": c.check.status.as_str(),
                "required": c.required.unwrap_or(false),
                "bypassable": c.bypassable.unwrap_or(false),
                "summary": c.check.summary,
                "link": c.check.link,
                "started": c.check.started,
                "ended": c.check.ended,
            })).collect::<Vec<_>>(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.to_json()["checks"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn to_human(&self, color: bool) -> String {
        if self.checks.checks.is_empty() {
            return format!("No checks reported for #{}", self.number);
        }
        let rows: Vec<Vec<String>> = self
            .checks
            .checks
            .iter()
            .map(|c| {
                let status = c.check.status.as_str();
                let painted = if color {
                    let colour = match status {
                        "success" => "\x1b[32m",
                        "failure" | "error" => "\x1b[31m",
                        _ => "\x1b[33m",
                    };
                    format!("{colour}{status}\x1b[0m")
                } else {
                    status.to_string()
                };
                vec![
                    c.check.identifier.clone(),
                    painted,
                    if c.required.unwrap_or(false) {
                        "required"
                    } else {
                        ""
                    }
                    .to_string(),
                    c.check.summary.clone().unwrap_or_default(),
                ]
            })
            .collect();

        let mut out = plain_table(&["check", "status", "", "summary"], &rows);
        let blocking = self.checks.required_blocking();
        if !blocking.is_empty() {
            out.push_str(&format!(
                "\n\n{} required check{} not passing.",
                blocking.len(),
                if blocking.len() == 1 { " is" } else { "s are" }
            ));
        }
        out
    }
}

struct PullRequestCheckedOut {
    number: u64,
    title: String,
    branch: String,
    remote: String,
    existed: bool,
}

impl Render for PullRequestCheckedOut {
    fn to_json(&self) -> Value {
        json!({
            "number": self.number,
            "title": self.title,
            "branch": self.branch,
            "remote": self.remote,
            "created_branch": !self.existed,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let verb = if self.existed { "Updated" } else { "Created" };
        format!(
            "{green}✓{reset} {verb} branch {} for #{}\n  {}",
            self.branch, self.number, self.title
        )
    }
}

#[cfg(test)]
mod v05_tests {
    use super::*;

    #[test]
    fn name_only_drops_the_patches_from_the_json() {
        let files = vec![FileDiff {
            path: "src/main.rs".into(),
            additions: Some(10),
            deletions: Some(2),
            patch: Some("@@ -1 +1 @@".into()),
            ..Default::default()
        }];
        let with_patch = PullRequestDiff {
            number: 12,
            source_branch: "feat/x".into(),
            target_branch: "main".into(),
            name_only: false,
            raw: None,
            files: files.clone(),
        };
        assert_eq!(with_patch.to_json()["files"][0]["patch"], "@@ -1 +1 @@");

        let without = PullRequestDiff {
            name_only: true,
            files,
            ..with_patch
        };
        assert!(without.to_json()["files"][0]["patch"].is_null());
    }

    #[test]
    fn a_raw_diff_is_handed_over_untouched() {
        let diff = PullRequestDiff {
            number: 12,
            source_branch: "feat/x".into(),
            target_branch: "main".into(),
            name_only: false,
            raw: Some("diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n".into()),
            files: vec![],
        };
        let text = diff.to_human(false);
        assert!(text.starts_with("diff --git"), "{text}");
        assert!(
            text.ends_with("+b"),
            "trailing blank lines trimmed: {text:?}"
        );
    }

    fn check(name: &str, status: &str, required: bool) -> gitfox_client::PullRequestCheck {
        serde_json::from_value(json!({
            "required": required,
            "check": { "identifier": name, "status": status, "summary": "…" }
        }))
        .unwrap()
    }

    #[test]
    fn checks_report_what_blocks_a_merge() {
        let view = PullRequestChecksView {
            number: 12,
            checks: PullRequestChecks {
                commit_sha: Some("abc".into()),
                checks: vec![
                    check("build", "success", true),
                    check("lint", "failure", true),
                    check("optional-audit", "failure", false),
                ],
            },
        };
        let value = view.to_json();
        assert_eq!(value["count"], 3);
        assert_eq!(value["failed"], true);
        // Only the required, non-green ones block.
        assert_eq!(value["blocking"], 1);
        assert_eq!(value["checks"][1]["required"], true);
        assert!(
            view.to_human(false)
                .contains("1 required check is not passing")
        );
    }

    #[test]
    fn a_required_check_still_running_counts_as_blocking() {
        let checks = PullRequestChecks {
            commit_sha: None,
            checks: vec![check("build", "running", true)],
        };
        // "Can this merge?" is no — not yet.
        assert_eq!(checks.required_blocking().len(), 1);
        assert!(!checks.any_failed());
    }

    #[test]
    fn no_checks_is_a_sentence_not_an_empty_table() {
        let view = PullRequestChecksView {
            number: 12,
            checks: PullRequestChecks::default(),
        };
        assert!(view.to_human(false).contains("No checks"));
        assert_eq!(view.to_json()["count"], 0);
    }

    #[test]
    fn checkout_says_whether_the_branch_was_new() {
        let created = PullRequestCheckedOut {
            number: 12,
            title: "feat: add OAuth".into(),
            branch: "feat/oauth".into(),
            remote: "origin".into(),
            existed: false,
        };
        assert_eq!(created.to_json()["created_branch"], true);
        assert!(created.to_human(false).starts_with("✓ Created branch"));

        let updated = PullRequestCheckedOut {
            existed: true,
            ..created
        };
        assert_eq!(updated.to_json()["created_branch"], false);
        assert!(updated.to_human(false).starts_with("✓ Updated branch"));
    }
}
