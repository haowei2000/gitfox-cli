//! `fx pr` — pull requests, with gh's commands and flags.
//!
//! Every command here works without arguments inside a checkout: the repository
//! comes from the git remote and, where a pull request is optional, it is the
//! one for the current branch. A pull request can also be named the ways gh
//! accepts — `12`, `#12`, its URL, or its head branch.

mod fields;
mod manage;
mod status;

use std::time::Duration;

use gitfox_client::{
    CreatePullRequest, FileDiff, GitFoxClient, MergePullRequest, MergeResult, PullRequest,
    PullRequestChecks, PullRequestFilter, RepoRef,
};
use serde_json::{Value, json};

use crate::cli::{
    ColorWhen, PrCheckoutArgs, PrChecksArgs, PrCommand, PrCreateArgs, PrDiffArgs, PrListArgs,
    PrMergeArgs, PrSubcommand, PrViewArgs,
};
use crate::context::{Context, principal_id};
use crate::error::{CliError, ErrorCode, Result};
use crate::export;
use crate::git;
use crate::interact;
use crate::output::{Render, key_values, plain_table, relative_time};
use crate::paginate;

pub use fields::{PR_FIELDS, PrExtras, gh_check_bucket, gh_check_state, pr_field};

pub async fn run(cmd: PrCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        PrSubcommand::List(args) => list(args, ctx).await,
        PrSubcommand::View(args) => view(args, ctx).await,
        PrSubcommand::Create(args) => create(args, ctx).await,
        PrSubcommand::Merge(args) => merge(args, ctx).await,
        PrSubcommand::Checkout(args) => checkout(args, ctx).await,
        PrSubcommand::Diff(args) => diff(args, ctx).await,
        PrSubcommand::Checks(args) => checks(args, ctx).await,
        PrSubcommand::Close(args) => manage::close(args, ctx).await,
        PrSubcommand::Reopen(args) => manage::reopen(args, ctx).await,
        PrSubcommand::Ready(args) => manage::ready(args, ctx).await,
        PrSubcommand::Edit(args) => manage::edit(args, ctx).await,
        PrSubcommand::Comment(args) => manage::comment(args, ctx).await,
        PrSubcommand::Review(args) => manage::review(args, ctx).await,
        PrSubcommand::Status(args) => status::status(args, ctx).await,
        PrSubcommand::UpdateBranch(args) => manage::update_branch(args, ctx).await,
        PrSubcommand::Lock(_) | PrSubcommand::Unlock(_) => {
            super::gh_only::refuse("pr lock", "GitFox pull requests cannot be locked")
        }
        PrSubcommand::Revert(_) => super::gh_only::refuse(
            "pr revert",
            "GitFox cannot revert a merged pull request; revert the merge commit with git and open a new one",
        ),
    }
}

// ---------------------------------------------------------------------------
// naming a pull request
// ---------------------------------------------------------------------------

/// What a `NUMBER | URL | BRANCH` argument names.
#[derive(Debug, Clone, PartialEq)]
pub enum Selector {
    Number(u64),
    Branch(String),
}

/// Read a selector. A URL can name the repository too, which then wins over
/// `-R` and the checkout, as it does in gh.
pub fn parse_selector(raw: &str) -> Result<(Option<RepoRef>, Selector)> {
    let raw = raw.trim();
    let digits = raw.strip_prefix('#').unwrap_or(raw);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        let number = digits
            .parse()
            .map_err(|_| CliError::invalid_argument(format!("`{raw}` is not a number")))?;
        return Ok((None, Selector::Number(number)));
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        let not_a_pr = || {
            CliError::invalid_argument(format!("`{raw}` is not a pull request URL"))
                .with_hint("expected …/space/repo/pulls/NUMBER")
        };
        let url = url::Url::parse(raw).map_err(|_| not_a_pr())?;
        let segments: Vec<&str> = url
            .path_segments()
            .map(|s| s.filter(|seg| !seg.is_empty()).collect())
            .unwrap_or_default();
        let at = segments
            .iter()
            .position(|s| *s == "pulls" || *s == "pull")
            .ok_or_else(not_a_pr)?;
        let number = segments
            .get(at + 1)
            .and_then(|n| n.parse::<u64>().ok())
            .ok_or_else(not_a_pr)?;
        let repo = (at >= 2)
            .then(|| RepoRef::parse(&segments[..at].join("/")).ok())
            .flatten();
        return Ok((repo, Selector::Number(number)));
    }
    if raw.is_empty() {
        return Err(CliError::invalid_argument("an empty pull request selector"));
    }
    Ok((None, Selector::Branch(raw.to_string())))
}

/// A pull request and the repository it lives in.
pub struct Target {
    pub repo: RepoRef,
    pub pr: PullRequest,
}

/// The pull request a command acts on: the selector, or the current branch's.
pub async fn resolve(
    selector: Option<&str>,
    ctx: &Context,
    client: &GitFoxClient,
) -> Result<Target> {
    let (url_repo, selector) = match selector {
        Some(raw) => {
            let (repo, selector) = parse_selector(raw)?;
            (repo, Some(selector))
        }
        None => (None, None),
    };
    let repo = match url_repo {
        Some(repo) => repo,
        None => ctx.repo()?,
    };

    let pr = match selector {
        Some(Selector::Number(number)) => client
            .pull_requests()
            .get(&repo, number)
            .await
            .map_err(|e| not_found_as_pr(e, &number.to_string()))?,
        Some(Selector::Branch(branch)) => find_for_branch(client, &repo, &branch).await?,
        None => {
            let branch = ctx.branch()?;
            find_for_branch(client, &repo, branch)
                .await
                .map_err(|e| e.with_hint("pass a number, or open one with `fx pr create`"))?
        }
    };
    Ok(Target { repo, pr })
}

async fn find_for_branch(
    client: &GitFoxClient,
    repo: &RepoRef,
    branch: &str,
) -> Result<PullRequest> {
    client
        .pull_requests()
        .find_any_for_branch(repo, branch)
        .await
        .map_err(|e| not_found_as_repo(e, repo))?
        .ok_or_else(|| {
            CliError::new(
                ErrorCode::PrNotFound,
                format!("no pull request for branch `{branch}` in {repo}"),
            )
        })
}

/// The client reports a generic 404; at this point we know it was the repo.
pub fn not_found_as_repo(err: gitfox_client::Error, repo: &RepoRef) -> CliError {
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

/// `key` or `key:value` → the label's id and, for a value, the value's id.
pub async fn resolve_label(
    client: &GitFoxClient,
    repo: &RepoRef,
    name: &str,
) -> Result<(i64, Option<i64>, Option<String>)> {
    let (key, value) = match name.split_once(':') {
        Some((key, value)) => (key.trim(), Some(value.trim())),
        None => (name.trim(), None),
    };
    let labels = client
        .labels()
        .list(repo, Some(key), true, 1, 100)
        .await
        .map_err(|e| not_found_as_repo(e, repo))?;
    let label = labels
        .iter()
        .find(|l| l.key.eq_ignore_ascii_case(key))
        .ok_or_else(|| {
            CliError::new(ErrorCode::NotFound, format!("no label `{key}` in {repo}"))
                .with_hint("see `fx label list`, or create it with `fx label create`")
        })?;
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Ok((label.id, None, None));
    };
    let values = client.labels().values(repo, &label.key).await?;
    match values.iter().find(|v| v.value.eq_ignore_ascii_case(value)) {
        Some(existing) => Ok((label.id, Some(existing.id), None)),
        // A dynamic label takes new values; GitFox creates the value on assign.
        None => Ok((label.id, None, Some(value.to_string()))),
    }
}

/// Open pull requests in `repo` waiting on a review from `reviewer_id`,
/// other than the reviewer's own.
///
/// GitFox's `reviewer_id` filter is the direct way to ask, but some instances
/// answer it with a bare HTTP 500. Then each open pull request's reviewers are
/// read instead: slower, and still the right answer.
pub async fn awaiting_review(
    client: &GitFoxClient,
    repo: &RepoRef,
    reviewer_id: i64,
) -> Result<Vec<PullRequest>> {
    let not_theirs = |pr: &PullRequest| pr.author.as_ref().and_then(|a| a.id) != Some(reviewer_id);
    let filtered = PullRequestFilter {
        reviewer_id: Some(reviewer_id),
        review_decision: vec!["pending".into()],
        include_checks: true,
        limit: 50,
        ..Default::default()
    };
    match client.pull_requests().list(repo, &filtered).await {
        Ok(prs) => Ok(prs.into_iter().filter(not_theirs).collect()),
        Err(gitfox_client::Error::Api { status: 500, .. }) => {
            tracing::debug!(%repo, "reviewer filter failed; reading reviewers per pull request");
            let open = client
                .pull_requests()
                .list(
                    repo,
                    &PullRequestFilter {
                        include_checks: true,
                        limit: 50,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| not_found_as_repo(e, repo))?;
            let mut waiting = Vec::new();
            for pr in open.into_iter().filter(not_theirs) {
                let reviewers = client.pull_requests().reviewers(repo, pr.number).await?;
                if reviewers.iter().any(|r| {
                    r.reviewer.as_ref().and_then(|p| p.id) == Some(reviewer_id)
                        && r.decision() == "pending"
                }) {
                    waiting.push(pr);
                }
            }
            Ok(waiting)
        }
        Err(other) => Err(not_found_as_repo(other, repo)),
    }
}

/// Delete the local copy of a pull request's branch after the remote one
/// went, the way gh's `--delete-branch` does — but only in a checkout of the
/// same repository, and never out from under uncommitted work.
fn delete_local_branch(ctx: &Context, repo: &RepoRef, branch: &str, base: &str) {
    let same_repo = ctx
        .git
        .to_context()
        .repo_for(ctx.config.host_key.as_deref())
        .is_some_and(|r| r == repo.full());
    if !same_repo || !git::local_branch_exists(branch) {
        return;
    }
    if git::current_branch().as_deref() == Some(branch)
        && let Err(message) = git::checkout(base)
    {
        ctx.warn(&format!(
            "kept the local branch `{branch}`: could not switch to `{base}`: {message}"
        ));
        return;
    }
    if let Err(message) = git::delete_local_branch(branch) {
        ctx.warn(&format!(
            "could not delete the local branch `{branch}`: {message}"
        ));
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(args: PrListArgs, ctx: &Context) -> Result<()> {
    if args.assignee.is_some() {
        return Err(CliError::unsupported(
            "`--assignee`",
            "GitFox pull requests have reviewers, not assignees",
        )
        .with_hint("filter by author with --author, or see reviewers with --json reviewRequests"));
    }
    if args.app.is_some() {
        return Err(CliError::unsupported(
            "`--app`",
            "GitFox has no GitHub Apps",
        ));
    }

    let repo = ctx.repo()?;
    if args.web {
        return interact::open_in_browser(ctx, &ctx.web_url(&format!("{}/pulls", repo.full()))?);
    }
    let client = ctx.client()?;

    let author_id = match args.author.as_deref() {
        Some(author) => Some(principal_id(&client, author).await?),
        None => None,
    };
    let mut label_id = Vec::new();
    let mut value_id = Vec::new();
    for name in &args.labels {
        let (label, value, _) = resolve_label(&client, &repo, name).await?;
        label_id.push(label);
        value_id.extend(value);
    }

    let base = PullRequestFilter {
        state: args.state.expand(),
        author_id,
        source_branch: args.head.clone(),
        target_branch: args.base.clone(),
        query: args.search.clone(),
        label_id,
        value_id,
        include_checks: ctx
            .renderer
            .export_spec()
            .is_some_and(|s| s.wants("statusCheckRollup")),
        ..Default::default()
    };

    // A 404 on the repo-scoped path means the repository, not a pull request —
    // GitFox also answers 404 rather than 401 for repositories the caller
    // cannot see, so this is the common case for a typo or a missing grant.
    let (client_ref, repo_ref, base_ref) = (&client, &repo, &base);
    let fetch = move |page, limit| async move {
        let filter = PullRequestFilter {
            page,
            limit,
            ..base_ref.clone()
        };
        client_ref.pull_requests().list(repo_ref, &filter).await
    };
    let paged = if args.draft {
        paginate::collect_filtered(args.limit, |pr: &PullRequest| pr.is_draft, fetch).await
    } else {
        paginate::collect(args.limit, fetch).await
    }
    .map_err(|e| not_found_as_repo(e, &repo))?;

    let mut extras = Vec::with_capacity(paged.items.len());
    for pr in &paged.items {
        extras.push(
            PrExtras::fetch(&client, &repo, pr.number, ctx.renderer.export_spec(), false).await?,
        );
    }

    ctx.renderer.emit(&PullRequestList {
        repo: repo.clone(),
        items: paged.items,
        extras,
        truncated: paged.truncated,
    })
}

// ---------------------------------------------------------------------------
// view
// ---------------------------------------------------------------------------

async fn view(args: PrViewArgs, ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let Target { repo, pr } = resolve(args.target.selector.as_deref(), ctx, &client).await?;
    if args.web {
        return interact::open_in_browser(ctx, &pr_web_url(ctx, &repo, &pr, None)?);
    }
    let extras = PrExtras::fetch(
        &client,
        &repo,
        pr.number,
        ctx.renderer.export_spec(),
        args.comments,
    )
    .await?;
    ctx.renderer.emit(&PullRequestView {
        repo,
        pr,
        extras,
        show_comments: args.comments,
    })
}

/// A pull request's page, or one of its tabs (`changes`, `checks`).
pub fn pr_web_url(
    ctx: &Context,
    repo: &RepoRef,
    pr: &PullRequest,
    section: Option<&str>,
) -> Result<String> {
    let base = match (&pr.web_url, section) {
        (Some(url), None) if !url.is_empty() => return Ok(url.clone()),
        _ => ctx.web_url(&format!("{}/pulls/{}", repo.full(), pr.number))?,
    };
    Ok(match section {
        Some(section) => format!("{}/{section}", base.trim_end_matches('/')),
        None => base,
    })
}

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

async fn create(args: PrCreateArgs, ctx: &Context) -> Result<()> {
    if !args.assignee.is_empty() {
        return Err(CliError::unsupported(
            "`--assignee`",
            "GitFox pull requests have reviewers, not assignees",
        )
        .with_hint("request a review instead with --reviewer"));
    }
    if args.milestone.is_some() {
        return Err(CliError::unsupported(
            "`--milestone`",
            "GitFox has no milestones",
        ));
    }
    if !args.project.is_empty() {
        return Err(CliError::unsupported(
            "`--project`",
            "GitFox has no projects",
        ));
    }
    if args.recover.is_some() {
        return Err(CliError::unsupported(
            "`--recover`",
            "fx keeps no record of a failed create to recover from",
        ));
    }
    if !args.attach.is_empty() {
        return Err(CliError::unsupported(
            "`--attach`",
            "fx cannot upload attachments to a GitFox pull request",
        ));
    }

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

    if args.web {
        let url = ctx.web_url(&format!("{}/pulls/compare/{base}...{head}", repo.full()))?;
        return interact::open_in_browser(ctx, &url);
    }

    let (title, description) = title_and_body(&args, &base, &head, ctx)?;

    if args.dry_run {
        return ctx.renderer.emit(&PullRequestPreview {
            repo: repo.full(),
            title,
            description,
            source_branch: head,
            target_branch: base,
            draft: args.draft,
            reviewers: args.reviewers.clone(),
            labels: args.labels.clone(),
        });
    }

    let mut created = client
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

    // The pull request exists from here on. A reviewer or label that cannot
    // be added is reported, but it does not pretend the create failed.
    for login in &args.reviewers {
        let added = async {
            let id = principal_id(&client, login).await?;
            client
                .pull_requests()
                .add_reviewer(&repo, created.number, id)
                .await
                .map_err(CliError::from)
        }
        .await;
        if let Err(err) = added {
            ctx.warn(&format!("could not request a review from {login}: {err}"));
        }
    }
    if !args.labels.is_empty() {
        for name in &args.labels {
            let added = async {
                let (label, value_id, value) = resolve_label(&client, &repo, name).await?;
                client
                    .pull_requests()
                    .assign_label(&repo, created.number, label, value_id, value.as_deref())
                    .await
                    .map_err(CliError::from)
            }
            .await;
            if let Err(err) = added {
                ctx.warn(&format!("could not add the label {name}: {err}"));
            }
        }
        if let Ok(refreshed) = client.pull_requests().get(&repo, created.number).await {
            created = refreshed;
        }
    }

    ctx.renderer.emit(&PullRequestCreated(created))
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

/// The title and body, from whichever flags supplied them.
///
/// `--title`/`--body` win; `--fill*` read the branch's commits; `--template`
/// seeds the body; `--editor` opens both for editing; with none of them, a
/// person is prompted and a machine is told what to pass.
fn title_and_body(
    args: &PrCreateArgs,
    base: &str,
    head: &str,
    ctx: &Context,
) -> Result<(String, String)> {
    let explicit_body = interact::body_from(args.body.as_deref(), args.body_file.as_deref())?;
    let template = match &args.template {
        Some(path) => Some(interact::read_source(path)?),
        None => None,
    };

    let filled = if args.fill || args.fill_first || args.fill_verbose {
        let commits = git::commits_between(base, head);
        let filled = if args.fill_first {
            git::fill_first(&commits)
        } else if args.fill_verbose {
            git::fill_verbose(&commits)
        } else {
            git::fill_from_commits(&commits)
        };
        Some(filled.ok_or_else(|| {
            CliError::invalid_argument(format!("no commits on `{head}` that are not on `{base}`"))
                .with_hint("push the branch first, or pass --title")
        })?)
    } else {
        None
    };

    let title = args
        .title
        .clone()
        .or_else(|| filled.as_ref().map(|(t, _)| t.clone()));
    let body = explicit_body
        .or_else(|| filled.as_ref().map(|(_, b)| b.clone()))
        .or(template);

    if args.editor {
        let seed = format!(
            "{}\n\n{}",
            title.clone().unwrap_or_default(),
            body.clone().unwrap_or_default()
        );
        let edited = interact::edit_text(ctx, &seed, "PULL_REQUEST.md")?;
        let (first, rest) = edited.split_once('\n').unwrap_or((edited.as_str(), ""));
        let title = first.trim().to_string();
        if title.is_empty() {
            return Err(CliError::invalid_argument("a title is required"));
        }
        return Ok((title, rest.trim().to_string()));
    }

    if let Some(title) = title {
        return Ok((title, body.unwrap_or_default()));
    }

    ctx.require_interactive("a pull request title")
        .map_err(|e| e.with_hint("pass --title, --fill, or --editor"))?;
    let title: String = dialoguer::Input::new()
        .with_prompt("Title")
        .interact_text()
        .map_err(|e| CliError::invalid_argument(format!("could not read the title: {e}")))?;
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(CliError::invalid_argument("a title is required"));
    }
    let body = match body {
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
    if args.auto || args.disable_auto {
        return Err(CliError::unsupported(
            if args.auto {
                "`--auto`"
            } else {
                "`--disable-auto`"
            },
            "GitFox has no auto-merge",
        )
        .with_hint("merge once the checks pass, or bypass the rules with --admin"));
    }
    if args.author_email.is_some() {
        return Err(CliError::unsupported(
            "`--author-email`",
            "GitFox chooses the merge commit's author itself",
        ));
    }
    // `-m` used to take the method; now it is gh's --merge. Catch the old
    // spelling rather than looking for a branch called `squash`.
    if args.merge
        && let Some(selector) = args.target.selector.as_deref()
        && selector.parse::<gitfox_client::MergeMethod>().is_ok()
    {
        return Err(CliError::invalid_argument(format!(
            "`-m` means --merge, as in gh; it no longer takes `{selector}`"
        ))
        .with_hint(format!("use --{selector}, or --method {selector}")));
    }

    let client = ctx.client()?;
    let Target { repo, pr } = resolve(args.target.selector.as_deref(), ctx, &client).await?;
    let message = interact::body_from(args.body.as_deref(), args.body_file.as_deref())?;

    let result = client
        .pull_requests()
        .merge(
            &repo,
            pr.number,
            &MergePullRequest {
                method: Some(args.strategy().into()),
                title: args.subject.clone(),
                message,
                source_sha: args.match_head_commit.clone(),
                bypass_rules: args.admin,
                dry_run: args.dry_run,
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
    if args.delete_branch && !args.dry_run {
        delete_local_branch(ctx, &repo, &pr.source_branch, &pr.target_branch);
    }

    ctx.renderer.emit(&PullRequestMerged {
        number: pr.number,
        title: pr.title.clone(),
        source_branch: pr.source_branch.clone(),
        target_branch: pr.target_branch.clone(),
        dry_run: args.dry_run,
        branch_deleted,
        result,
    })
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

async fn diff(args: PrDiffArgs, ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let Target { repo, pr } = resolve(args.target.selector.as_deref(), ctx, &client).await?;
    if args.web {
        return interact::open_in_browser(ctx, &pr_web_url(ctx, &repo, &pr, Some("changes"))?);
    }

    let color = match args.color {
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => ctx.renderer.color(),
    };

    if args.patch {
        let patches = format_patches(&client, &repo, &pr).await?;
        return ctx.renderer.emit(&PullRequestPatches {
            number: pr.number,
            text: patches,
            color,
        });
    }

    // The endpoint serves either form. A person wants the patch their pager and
    // syntax highlighter understand; a machine wants it split by file. Asking
    // for the one that suits the output mode beats reassembling either.
    let raw = if ctx.renderer.is_machine() || args.name_only {
        None
    } else {
        Some(client.pull_requests().diff_text(&repo, pr.number).await?)
    };
    let mut files = match &raw {
        Some(_) => Vec::new(),
        None => client.pull_requests().diff_files(&repo, pr.number).await?,
    };
    files.retain(|f| !excluded(&f.path, &args.exclude));
    let raw = raw.map(|text| exclude_from_diff(&text, &args.exclude));

    ctx.renderer.emit(&PullRequestDiff {
        number: pr.number,
        source_branch: pr.source_branch.clone(),
        target_branch: pr.target_branch.clone(),
        name_only: args.name_only,
        raw,
        files,
        color,
    })
}

/// The pull request as `git format-patch` would print it: one mail per
/// commit, oldest first.
async fn format_patches(client: &GitFoxClient, repo: &RepoRef, pr: &PullRequest) -> Result<String> {
    let commits = client
        .pull_requests()
        .commits(repo, pr.number, 1, 100)
        .await?;
    let total = commits.len();
    let mut out = String::new();
    for (index, commit) in commits.iter().enumerate() {
        let diff = client.repos().commit_diff(repo, &commit.sha).await?;
        let author = commit.author.clone().unwrap_or_default();
        let subject = if total > 1 {
            format!("[PATCH {}/{total}] {}", index + 1, commit.title)
        } else {
            format!("[PATCH] {}", commit.title)
        };
        out.push_str(&format!(
            "From {} Mon Sep 17 00:00:00 2001\nFrom: {} <{}>\nDate: {}\nSubject: {subject}\n\n",
            commit.sha,
            author.identity.name,
            author.identity.email,
            author.when.unwrap_or_default(),
        ));
        let body = commit.body();
        if !body.is_empty() {
            out.push_str(&body);
            out.push('\n');
        }
        out.push_str("---\n");
        out.push_str(diff.trim_end());
        out.push_str("\n\n");
    }
    Ok(out)
}

/// Whether `path` matches any `--exclude` glob, by full path or file name.
fn excluded(path: &str, patterns: &[String]) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    patterns
        .iter()
        .any(|pattern| glob_match(pattern, path) || glob_match(pattern, name))
}

/// Drop whole file sections whose path is excluded from a unified diff.
fn exclude_from_diff(text: &str, patterns: &[String]) -> String {
    if patterns.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut keep = true;
    for line in text.split_inclusive('\n') {
        if let Some(header) = line.strip_prefix("diff --git ") {
            let path = header.split(" b/").last().unwrap_or_default().trim_end();
            keep = !excluded(path, patterns);
        }
        if keep {
            out.push_str(line);
        }
    }
    out
}

/// `*`, `?` and `**` glob matching, as `--exclude` takes.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) if p.get(1) == Some(&'*') => {
                let rest = &p[2..];
                let rest = rest.strip_prefix(&['/']).unwrap_or(rest);
                (0..=t.len()).any(|i| go(rest, &t[i..]))
            }
            (Some('*'), _) => (0..=t.len())
                .take_while(|&i| i == 0 || t[i - 1] != '/')
                .any(|i| go(&p[1..], &t[i..])),
            (Some('?'), Some(c)) if *c != '/' => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    go(&p, &t)
}

/// Colour a unified diff the way `git diff --color` does.
fn colorize_diff(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.starts_with("diff --git")
                || line.starts_with("index ")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
            {
                format!("\x1b[1m{line}\x1b[0m")
            } else if line.starts_with("@@") {
                format!("\x1b[36m{line}\x1b[0m")
            } else if line.starts_with('+') {
                format!("\x1b[32m{line}\x1b[0m")
            } else if line.starts_with('-') {
                format!("\x1b[31m{line}\x1b[0m")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// checks
// ---------------------------------------------------------------------------

async fn checks(args: PrChecksArgs, ctx: &Context) -> Result<()> {
    if args.watch && ctx.renderer.export_spec().is_some() {
        return Err(CliError::invalid_argument(
            "cannot use `--watch` with `--json` flag",
        ));
    }
    if args.fail_fast && !args.watch {
        return Err(CliError::invalid_argument(
            "cannot use `--fail-fast` flag without `--watch` flag",
        ));
    }
    if args.interval.is_some() && !args.watch {
        return Err(CliError::invalid_argument(
            "cannot use `--interval` flag without `--watch` flag",
        ));
    }

    let client = ctx.client()?;
    let Target { repo, pr } = resolve(args.target.selector.as_deref(), ctx, &client).await?;
    if args.web {
        return interact::open_in_browser(ctx, &pr_web_url(ctx, &repo, &pr, Some("checks"))?);
    }

    let mut checks = fetch_checks(&client, &repo, pr.number, args.required).await?;
    if args.watch {
        let interval = Duration::from_secs(args.interval.unwrap_or(10).max(1));
        let live = ctx.renderer.is_tty() && !ctx.renderer.is_machine();
        loop {
            let view = PullRequestChecksView {
                number: pr.number,
                checks: checks.clone(),
            };
            let pending = view.counts().pending > 0;
            let failed = view.counts().failed > 0;
            if !pending || (args.fail_fast && failed) {
                break;
            }
            if live {
                ctx.renderer.write_str(&format!(
                    "\x1b[H\x1b[2JRefreshing checks status every {} seconds. Press Ctrl+C to quit.\n\n{}\n",
                    interval.as_secs(),
                    view.to_human(ctx.renderer.color())
                ))?;
            }
            tokio::time::sleep(interval).await;
            checks = fetch_checks(&client, &repo, pr.number, args.required).await?;
        }
        if live {
            ctx.renderer.write_str("\x1b[H\x1b[2J")?;
        }
    }

    let view = PullRequestChecksView {
        number: pr.number,
        checks,
    };

    // A person reads the table and the exit code, exactly as with gh. JSON is
    // data, so it exits 0 whatever the checks say — also as gh does.
    if !ctx.renderer.is_machine() {
        if view.checks.checks.is_empty() {
            return Err(CliError::new(
                ErrorCode::NotFound,
                format!(
                    "no {}checks reported on the '{}' branch",
                    if args.required { "required " } else { "" },
                    pr.source_branch
                ),
            ));
        }
        ctx.renderer.emit(&view)?;
        let counts = view.counts();
        if counts.failed > 0 {
            return Err(CliError::new(
                ErrorCode::ChecksFailed,
                format!("{} of {} checks failed", counts.failed, counts.total),
            )
            .silenced());
        }
        if counts.pending > 0 {
            return Err(CliError::new(
                ErrorCode::ChecksPending,
                format!("{} of {} checks are pending", counts.pending, counts.total),
            )
            .silenced());
        }
        return Ok(());
    }
    ctx.renderer.emit(&view)
}

async fn fetch_checks(
    client: &GitFoxClient,
    repo: &RepoRef,
    number: u64,
    required_only: bool,
) -> Result<PullRequestChecks> {
    let mut checks = client.pull_requests().checks(repo, number).await?;
    if required_only {
        checks.checks.retain(|c| c.required.unwrap_or(false));
    }
    Ok(checks)
}

// ---------------------------------------------------------------------------
// checkout
// ---------------------------------------------------------------------------

pub async fn checkout(args: PrCheckoutArgs, ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let Target { pr, .. } = resolve(args.target.selector.as_deref(), ctx, &client).await?;

    let branch = pr.source_branch.clone();
    if branch.is_empty() {
        return Err(CliError::new(
            ErrorCode::ApiError,
            format!("#{} reports no source branch", pr.number),
        ));
    }

    // A fork's branch lives in the fork, which on GitFox is a repository on
    // the same instance — fetch it from there.
    let fork = matches!((pr.source_repo_id, pr.target_repo_id), (Some(s), Some(t)) if s != t);
    let remote = git::remote_name().ok_or_else(|| {
        CliError::new(
            ErrorCode::GitContextError,
            "no git remote to fetch from, or not inside a git repository",
        )
    })?;
    // Where the branch came from: the checkout's remote, or the fork.
    let fetched_from = if fork {
        let source_id = pr.source_repo_id.unwrap_or_default();
        let source = client.repos().get_by_id(source_id).await?;
        let ssh = git::remote_url(&remote).is_some_and(|url| !url.starts_with("http"));
        let url = source.clone_url(ssh).ok_or_else(|| {
            CliError::new(
                ErrorCode::ApiError,
                format!("the fork #{} comes from reports no clone URL", pr.number),
            )
        })?;
        git::fetch_url(url, &branch).map_err(git_failed)?;
        source.reference()
    } else {
        git::fetch(&remote, &branch).map_err(git_failed)?;
        remote.clone()
    };

    let local = args.branch.clone().unwrap_or_else(|| branch.clone());
    let mut existed = false;

    if args.detach {
        git::checkout_detached("FETCH_HEAD").map_err(git_failed)?;
    } else if let Some(path) = &args.worktree {
        existed = git::local_branch_exists(&local);
        git::worktree_add(path, &local, "FETCH_HEAD", !existed).map_err(git_failed)?;
    } else {
        existed = git::local_branch_exists(&local);
        if existed {
            git::checkout(&local).map_err(git_failed)?;
            if args.force {
                git::reset_hard("FETCH_HEAD").map_err(git_failed)?;
            } else {
                // Fast-forward only: a diverged local branch is the user's to
                // resolve, unless --force says to discard it.
                git::merge_ff_only("FETCH_HEAD").map_err(|message| {
                    git_failed(message).with_hint(format!(
                        "`{local}` has diverged from the pull request; reconcile it, or pass --force"
                    ))
                })?;
            }
        } else {
            git::checkout_new(&local, "FETCH_HEAD").map_err(git_failed)?;
            if !fork {
                // Best effort: some remotes have no tracking ref for a fresh branch.
                let _ = git::set_upstream(&local, &remote);
            }
        }
    }

    if args.recurse_submodules {
        git::update_submodules().map_err(git_failed)?;
    }

    ctx.renderer.emit(&PullRequestCheckedOut {
        number: pr.number,
        title: pr.title.clone(),
        branch: if args.detach { None } else { Some(local) },
        remote: fetched_from,
        existed,
        worktree: args.worktree.as_ref().map(|p| p.display().to_string()),
    })
}

fn git_failed(message: String) -> CliError {
    CliError::new(ErrorCode::GitContextError, message)
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

pub fn branch_arrow(pr: &PullRequest) -> String {
    format!("{} → {}", pr.source_branch, pr.target_branch)
}

pub fn state_colour(state: &str) -> &'static str {
    match state {
        "open" => "\x1b[32m",
        "merged" => "\x1b[35m",
        "draft" => "\x1b[2m",
        _ => "\x1b[31m",
    }
}

pub fn painted_state(pr: &PullRequest, color: bool) -> String {
    let state = pr.display_state();
    if color {
        format!("{}{state}\x1b[0m", state_colour(state))
    } else {
        state.to_string()
    }
}

struct PullRequestList {
    repo: RepoRef,
    items: Vec<PullRequest>,
    extras: Vec<PrExtras>,
    /// Whether the server had more than `--limit` allowed through.
    truncated: bool,
}

impl Render for PullRequestList {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.repo.full(),
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

    fn export_fields(&self) -> &'static [&'static str] {
        PR_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        let empty = PrExtras::default();
        Value::Array(
            self.items
                .iter()
                .enumerate()
                .map(|(i, pr)| {
                    let extras = self.extras.get(i).unwrap_or(&empty);
                    export::select(fields, |f| pr_field(pr, extras, &self.repo, f))
                })
                .collect(),
        )
    }

    fn to_human(&self, color: bool) -> String {
        if self.items.is_empty() {
            return format!("No pull requests found in {}", self.repo);
        }
        let rows: Vec<Vec<String>> = self
            .items
            .iter()
            .map(|pr| {
                vec![
                    format!("#{}", pr.number),
                    pr.title.clone(),
                    painted_state(pr, color),
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

struct PullRequestView {
    repo: RepoRef,
    pr: PullRequest,
    extras: PrExtras,
    show_comments: bool,
}

impl Render for PullRequestView {
    fn to_json(&self) -> Value {
        let mut value = serde_json::to_value(&self.pr).unwrap_or(Value::Null);
        if self.show_comments {
            value["comments"] = json!(
                self.extras
                    .comments
                    .iter()
                    .flatten()
                    .map(|c| json!({
                        "id": c.id,
                        "author": c.author,
                        "text": c.text,
                        "created": c.created,
                        "path": c.code_comment.as_ref().map(|cc| cc.path.clone()),
                    }))
                    .collect::<Vec<_>>()
            );
        }
        value
    }

    fn export_fields(&self) -> &'static [&'static str] {
        PR_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        export::select(fields, |f| pr_field(&self.pr, &self.extras, &self.repo, f))
    }

    fn to_human(&self, color: bool) -> String {
        let pr = &self.pr;
        let (bold, dim, reset) = if color {
            ("\x1b[1m", "\x1b[2m", "\x1b[0m")
        } else {
            ("", "", "")
        };

        let mut pairs = vec![
            ("State", painted_state(pr, color)),
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
        if !pr.labels.is_empty() {
            pairs.push((
                "Labels",
                pr.labels
                    .iter()
                    .map(|l| l.name())
                    .collect::<Vec<_>>()
                    .join(", "),
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
        if self.show_comments {
            let comments = self.extras.comments.as_deref().unwrap_or_default();
            if comments.is_empty() {
                out.push_str(&format!("\n\n{dim}No comments{reset}"));
            }
            for c in comments {
                let who = c
                    .author
                    .as_ref()
                    .map(gitfox_client::Principal::label)
                    .unwrap_or_default();
                let when = c.created.map(relative_time).unwrap_or_default();
                let place = c
                    .code_comment
                    .as_ref()
                    .map(|cc| format!(" on {}", cc.path))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "\n\n{bold}{who}{reset} commented{place} {dim}{when}{reset}\n{}",
                    c.text.trim()
                ));
            }
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

struct PullRequestPreview {
    repo: String,
    title: String,
    description: String,
    source_branch: String,
    target_branch: String,
    draft: bool,
    reviewers: Vec<String>,
    labels: Vec<String>,
}

impl Render for PullRequestPreview {
    fn to_json(&self) -> Value {
        json!({
            "dry_run": true,
            "repository": self.repo,
            "title": self.title,
            "description": self.description,
            "source_branch": self.source_branch,
            "target_branch": self.target_branch,
            "is_draft": self.draft,
            "reviewers": self.reviewers,
            "labels": self.labels,
        })
    }

    fn to_human(&self, _color: bool) -> String {
        let mut pairs = vec![
            ("Title", self.title.clone()),
            (
                "Branches",
                format!("{} → {}", self.source_branch, self.target_branch),
            ),
            ("Draft", self.draft.to_string()),
        ];
        if !self.reviewers.is_empty() {
            pairs.push(("Reviewers", self.reviewers.join(", ")));
        }
        if !self.labels.is_empty() {
            pairs.push(("Labels", self.labels.join(", ")));
        }
        let mut out = format!(
            "Would have created a pull request in {}:\n{}",
            self.repo,
            key_values(&pairs)
        );
        if !self.description.trim().is_empty() {
            out.push_str("\n\n");
            out.push_str(self.description.trim());
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

struct PullRequestDiff {
    number: u64,
    source_branch: String,
    target_branch: String,
    name_only: bool,
    /// The raw unified diff, when that is what was fetched.
    raw: Option<String>,
    files: Vec<FileDiff>,
    color: bool,
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
            let raw = raw.trim_end();
            return if self.color {
                colorize_diff(raw)
            } else {
                raw.to_string()
            };
        }
        if self.files.is_empty() {
            return format!("#{} changes nothing", self.number);
        }
        if self.name_only {
            // gh prints bare paths, one per line, for `xargs` and friends.
            return self
                .files
                .iter()
                .map(|f| f.path.clone())
                .collect::<Vec<_>>()
                .join("\n");
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

struct PullRequestPatches {
    number: u64,
    text: String,
    color: bool,
}

impl Render for PullRequestPatches {
    fn to_json(&self) -> Value {
        json!({ "number": self.number, "patch": self.text })
    }

    fn to_human(&self, _color: bool) -> String {
        let text = self.text.trim_end();
        if self.color {
            colorize_diff(text)
        } else {
            text.to_string()
        }
    }
}

#[derive(Debug, Default, PartialEq)]
struct CheckCounts {
    total: usize,
    passed: usize,
    failed: usize,
    pending: usize,
    skipped: usize,
    cancelled: usize,
}

pub struct PullRequestChecksView {
    number: u64,
    checks: PullRequestChecks,
}

impl PullRequestChecksView {
    fn counts(&self) -> CheckCounts {
        let mut counts = CheckCounts {
            total: self.checks.checks.len(),
            ..Default::default()
        };
        for c in &self.checks.checks {
            match gh_check_bucket(c.check.status.as_str()) {
                "pass" => counts.passed += 1,
                "fail" => counts.failed += 1,
                "skipping" => counts.skipped += 1,
                "cancel" => counts.cancelled += 1,
                _ => counts.pending += 1,
            }
        }
        counts
    }
}

const CHECK_FIELDS: &[&str] = &[
    "bucket",
    "completedAt",
    "description",
    "event",
    "link",
    "name",
    "startedAt",
    "state",
    "workflow",
    // fx's own names
    "status",
    "required",
    "summary",
];

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

    fn export_fields(&self) -> &'static [&'static str] {
        CHECK_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        Value::Array(
            self.checks
                .checks
                .iter()
                .map(|c| {
                    let status = c.check.status.as_str();
                    export::select(fields, |field| match field {
                        "bucket" => json!(gh_check_bucket(status)),
                        "completedAt" => time_or_zero(c.check.ended),
                        "startedAt" => time_or_zero(c.check.started),
                        "description" | "summary" => {
                            json!(c.check.summary.clone().unwrap_or_default())
                        }
                        "event" | "workflow" => json!(""),
                        "link" => json!(c.check.link.clone().unwrap_or_default()),
                        "name" => json!(c.check.identifier),
                        "state" => json!(gh_check_state(status)),
                        "status" => json!(status),
                        "required" => json!(c.required.unwrap_or(false)),
                        _ => Value::Null,
                    })
                })
                .collect(),
        )
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

        let counts = self.counts();
        let headline = if counts.failed > 0 {
            "Some checks were not successful"
        } else if counts.pending > 0 {
            "Some checks are still pending"
        } else {
            "All checks were successful"
        };
        let mut out = format!(
            "{headline}\n{} cancelled, {} failing, {} successful, {} skipped, and {} pending checks\n\n{}",
            counts.cancelled,
            counts.failed,
            counts.passed,
            counts.skipped,
            counts.pending,
            plain_table(&["check", "status", "", "summary"], &rows)
        );
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

/// gh prints a check that has not started or finished with Go's zero time.
fn time_or_zero(epoch: Option<i64>) -> Value {
    match export::time_value(epoch) {
        Value::Null => json!("0001-01-01T00:00:00Z"),
        other => other,
    }
}

struct PullRequestCheckedOut {
    number: u64,
    title: String,
    /// `None` for a detached checkout.
    branch: Option<String>,
    remote: String,
    existed: bool,
    worktree: Option<String>,
}

impl Render for PullRequestCheckedOut {
    fn to_json(&self) -> Value {
        json!({
            "number": self.number,
            "title": self.title,
            "branch": self.branch,
            "remote": self.remote,
            "created_branch": self.branch.is_some() && !self.existed,
            "detached": self.branch.is_none(),
            "worktree": self.worktree,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let Some(branch) = &self.branch else {
            return format!(
                "{green}✓{reset} Checked out #{} at a detached HEAD\n  {}",
                self.number, self.title
            );
        };
        let verb = if self.existed { "Updated" } else { "Created" };
        let mut out = format!(
            "{green}✓{reset} {verb} branch {branch} for #{}\n  {}",
            self.number, self.title
        );
        if let Some(worktree) = &self.worktree {
            out.push_str(&format!("\n  in worktree {worktree}"));
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

    fn repo() -> RepoRef {
        RepoRef::parse("ai/backend").unwrap()
    }

    fn list(items: Vec<PullRequest>, truncated: bool) -> PullRequestList {
        PullRequestList {
            repo: repo(),
            extras: items.iter().map(|_| PrExtras::default()).collect(),
            items,
            truncated,
        }
    }

    #[test]
    fn a_selector_is_a_number_a_url_or_a_branch() {
        assert_eq!(parse_selector("12").unwrap(), (None, Selector::Number(12)));
        assert_eq!(parse_selector("#12").unwrap(), (None, Selector::Number(12)));
        assert_eq!(
            parse_selector("feat/oauth").unwrap(),
            (None, Selector::Branch("feat/oauth".into()))
        );
        let (repo, selector) =
            parse_selector("http://git.example.com:3000/ai/backend/pulls/124/changes").unwrap();
        assert_eq!(selector, Selector::Number(124));
        assert_eq!(repo.unwrap().full(), "ai/backend");
        let (repo, _) = parse_selector("https://h/org/team/backend/pulls/3").unwrap();
        assert_eq!(repo.unwrap().full(), "org/team/backend");
        assert!(parse_selector("https://h/ai/backend/issues/3").is_err());
    }

    #[test]
    fn list_json_carries_the_repository_and_a_count() {
        let value = list(vec![pr(12, "open", false), pr(13, "merged", false)], false).to_json();
        assert_eq!(value["repository"], "ai/backend");
        assert_eq!(value["count"], 2);
        assert_eq!(value["items"][1]["number"], 13);
    }

    #[test]
    fn list_export_is_an_array_of_just_the_asked_for_fields() {
        let l = list(vec![pr(12, "open", true)], false);
        let value = l.export(&["number".into(), "state".into(), "isDraft".into()]);
        assert_eq!(
            value,
            json!([{ "isDraft": true, "number": 12, "state": "OPEN" }])
        );
    }

    #[test]
    fn list_jsonl_emits_one_pull_request_per_line() {
        let rows = list(vec![pr(12, "open", false), pr(13, "merged", false)], true).to_jsonl();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["number"], 12);
    }

    #[test]
    fn an_empty_list_says_so_instead_of_printing_a_bare_header() {
        let l = list(vec![], false);
        assert!(l.to_human(false).contains("No pull requests"));
        assert_eq!(l.to_json()["count"], 0);
    }

    #[test]
    fn a_draft_is_shown_as_draft_in_the_table() {
        let text = list(vec![pr(12, "open", true)], false).to_human(false);
        assert!(text.contains("draft"), "{text}");
        assert!(text.contains("feat/oauth → main"), "{text}");
    }

    #[test]
    fn the_human_view_never_emits_colour_when_colour_is_off() {
        let view = PullRequestView {
            repo: repo(),
            pr: pr(12, "open", false),
            extras: PrExtras::default(),
            show_comments: false,
        };
        assert!(!view.to_human(false).contains('\x1b'));
        assert!(view.to_human(true).contains('\x1b'));
    }

    #[test]
    fn view_with_comments_lists_them() {
        let view = PullRequestView {
            repo: repo(),
            pr: pr(12, "open", false),
            extras: PrExtras {
                comments: Some(
                    serde_json::from_value(json!([
                        { "id": 1, "kind": "comment", "type": "comment", "text": "LGTM", "author": { "uid": "alice" } }
                    ]))
                    .unwrap(),
                ),
                ..Default::default()
            },
            show_comments: true,
        };
        let text = view.to_human(false);
        assert!(text.contains("alice commented"), "{text}");
        assert!(text.contains("LGTM"), "{text}");
        assert_eq!(view.to_json()["comments"][0]["text"], "LGTM");
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
        assert!(text.contains("0123456789ab"), "{text}");
        assert!(text.contains("deleted branch feat/oauth"), "{text}");
    }

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
            color: false,
        };
        assert_eq!(with_patch.to_json()["files"][0]["patch"], "@@ -1 +1 @@");

        let without = PullRequestDiff {
            name_only: true,
            files,
            ..with_patch
        };
        assert!(without.to_json()["files"][0]["patch"].is_null());
        // gh prints bare names under --name-only.
        assert_eq!(without.to_human(false), "src/main.rs");
    }

    #[test]
    fn a_raw_diff_is_handed_over_untouched_or_coloured() {
        let diff = PullRequestDiff {
            number: 12,
            source_branch: "feat/x".into(),
            target_branch: "main".into(),
            name_only: false,
            raw: Some("diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n".into()),
            files: vec![],
            color: false,
        };
        let text = diff.to_human(false);
        assert!(text.starts_with("diff --git"), "{text}");
        assert!(
            text.ends_with("+b"),
            "trailing blank lines trimmed: {text:?}"
        );
        let coloured = PullRequestDiff {
            color: true,
            ..diff
        };
        assert!(coloured.to_human(false).contains("\x1b[32m+b\x1b[0m"));
    }

    #[test]
    fn exclude_drops_matching_file_sections() {
        let text = "diff --git a/Cargo.lock b/Cargo.lock\n-a\n+b\ndiff --git a/src/main.rs b/src/main.rs\n-c\n+d\n";
        let kept = exclude_from_diff(text, &["*.lock".into()]);
        assert!(!kept.contains("Cargo.lock"), "{kept}");
        assert!(kept.contains("src/main.rs"), "{kept}");
        assert!(glob_match("src/**/*.rs", "src/a/b/c.rs"));
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "src/main.rs"));
        assert!(
            excluded("src/main.rs", &["*.rs".into()]),
            "matched by file name"
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
    fn checks_report_what_blocks_a_merge_and_count_like_gh() {
        let view = PullRequestChecksView {
            number: 12,
            checks: PullRequestChecks {
                commit_sha: Some("abc".into()),
                checks: vec![
                    check("build", "success", true),
                    check("lint", "failure", true),
                    check("optional-audit", "failure", false),
                    check("deploy", "running", false),
                ],
            },
        };
        let value = view.to_json();
        assert_eq!(value["count"], 4);
        assert_eq!(value["failed"], true);
        // Only the required, non-green ones block.
        assert_eq!(value["blocking"], 1);
        assert_eq!(
            view.counts(),
            CheckCounts {
                total: 4,
                passed: 1,
                failed: 2,
                pending: 1,
                skipped: 0,
                cancelled: 0
            }
        );
        let text = view.to_human(false);
        assert!(text.contains("Some checks were not successful"), "{text}");
        assert!(text.contains("2 failing, 1 successful"), "{text}");
        assert!(text.contains("1 required check is not passing"), "{text}");

        let exported = view.export(&["name".into(), "bucket".into(), "state".into()]);
        assert_eq!(
            exported[1],
            json!({ "bucket": "fail", "name": "lint", "state": "FAILURE" })
        );
        assert_eq!(exported[3]["state"], "IN_PROGRESS");
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
    fn checkout_says_whether_the_branch_was_new_or_detached() {
        let created = PullRequestCheckedOut {
            number: 12,
            title: "feat: add OAuth".into(),
            branch: Some("feat/oauth".into()),
            remote: "origin".into(),
            existed: false,
            worktree: None,
        };
        assert_eq!(created.to_json()["created_branch"], true);
        assert!(created.to_human(false).starts_with("✓ Created branch"));

        let updated = PullRequestCheckedOut {
            existed: true,
            ..created
        };
        assert_eq!(updated.to_json()["created_branch"], false);
        assert!(updated.to_human(false).starts_with("✓ Updated branch"));

        let detached = PullRequestCheckedOut {
            branch: None,
            ..updated
        };
        assert_eq!(detached.to_json()["detached"], true);
        assert!(detached.to_human(false).contains("detached HEAD"));
    }
}
