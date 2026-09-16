//! Changing repositories: create, delete, edit, rename, fork, sync,
//! set-default.

use gitfox_client::{CreateRepository, RepoRef, Repository};
use serde_json::{Value, json};

use super::{fetch, use_ssh};
use crate::cli::{
    RepoCreateArgs, RepoDeleteArgs, RepoEditArgs, RepoForkArgs, RepoRenameArgs, RepoSetDefaultArgs,
    RepoSyncArgs, Visibility,
};
use crate::context::{Context, parse_repo};
use crate::error::{CliError, ErrorCode, Result};
use crate::git;
use crate::interact;
use crate::output::Render;

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

pub async fn create(args: RepoCreateArgs, ctx: &Context) -> Result<()> {
    for (given, flag, why) in [
        (
            args.internal,
            "`--internal`",
            "GitFox repositories are public or private",
        ),
        (
            args.homepage.is_some(),
            "`--homepage`",
            "GitFox repositories have no homepage",
        ),
        (
            args.template.is_some(),
            "`--template`",
            "GitFox has no template repositories",
        ),
        (
            args.team.is_some(),
            "`--team`",
            "GitFox grants access through space membership, not teams",
        ),
        (
            args.include_all_branches,
            "`--include-all-branches`",
            "GitFox has no template repositories",
        ),
    ] {
        if given {
            return Err(CliError::unsupported(flag, why));
        }
    }

    let name = match args.name.clone() {
        Some(name) => name,
        None => {
            ctx.require_interactive("a repository name")
                .map_err(|e| e.with_hint("pass the name, e.g. `gf repo create space/name`"))?;
            dialoguer::Input::<String>::new()
                .with_prompt("Repository name")
                .interact_text()
                .map_err(|e| CliError::invalid_argument(format!("could not read the name: {e}")))?
        }
    };
    let (space, identifier) = match name.trim().trim_matches('/').rsplit_once('/') {
        Some((space, identifier)) => (space.to_string(), identifier.to_string()),
        None => (ctx.space(None)?, name.trim().to_string()),
    };
    if identifier.is_empty() {
        return Err(CliError::invalid_argument("a repository name is required"));
    }

    let client = ctx.client()?;
    let created = client
        .repos()
        .create(&CreateRepository {
            identifier: identifier.clone(),
            parent_ref: space.clone(),
            description: args.description.clone(),
            default_branch: None,
            is_public: args.public,
            readme: args.add_readme,
            license: args.license.clone(),
            git_ignore: args.gitignore.clone(),
            fork_id: None,
        })
        .await?;

    let repo = RepoRef::parse(&format!("{space}/{identifier}")).map_err(|_| {
        CliError::invalid_argument(format!(
            "`{space}/{identifier}` is not a repository reference"
        ))
    })?;
    let ssh = use_ssh(ctx, false);
    let mut cloned_to = None;
    let mut remote_added = None;

    if args.clone {
        let url = created.clone_url(ssh).ok_or_else(|| {
            CliError::new(ErrorCode::ApiError, format!("{repo} reports no clone URL"))
        })?;
        let destination = std::path::PathBuf::from(repo.name());
        git::clone(url, &destination, &[]).map_err(|m| CliError::new(ErrorCode::Unexpected, m))?;
        cloned_to = Some(destination.display().to_string());
    }

    if let Some(source) = &args.source {
        if !git::is_work_tree(source) {
            return Err(CliError::new(
                ErrorCode::GitContextError,
                format!("{} is not a git repository", source.display()),
            )
            .with_hint("run `git init` there first"));
        }
        let url = created.clone_url(ssh).ok_or_else(|| {
            CliError::new(ErrorCode::ApiError, format!("{repo} reports no clone URL"))
        })?;
        let remote = args.remote.clone().unwrap_or_else(|| "origin".to_string());
        git::remote_add_in(source, &remote, url).map_err(|m| {
            CliError::new(
                ErrorCode::GitContextError,
                format!("could not add the `{remote}` remote: {m}"),
            )
        })?;
        if args.push {
            git::push_head_in(source, &remote)
                .map_err(|m| CliError::new(ErrorCode::GitContextError, m))?;
        }
        remote_added = Some(remote);
    }

    ctx.renderer.emit(&RepoChanged {
        verb: "Created",
        reference: repo.full(),
        url: ctx.web_url(&repo.full()).ok(),
        repository: Some(created),
        extra: json!({ "cloned_to": cloned_to, "remote_added": remote_added }),
    })
}

// ---------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------

pub async fn delete(args: RepoDeleteArgs, ctx: &Context) -> Result<()> {
    let repo = match args.repository.as_deref() {
        Some(raw) => parse_repo(raw)?,
        None => ctx.repo()?,
    };
    if !args.yes {
        // gh asks for the name to be typed back, which is worth copying: a
        // `y` is too easy to give to the wrong repository.
        if ctx.config.non_interactive {
            return Err(CliError::invalid_argument(
                "--yes required when not running interactively",
            ));
        }
        let typed: String = dialoguer::Input::new()
            .with_prompt(format!("Type {} to confirm deletion", repo.full()))
            .interact_text()
            .map_err(|e| CliError::invalid_argument(format!("could not read the answer: {e}")))?;
        if typed.trim() != repo.full() {
            return Err(CliError::new(
                ErrorCode::Cancelled,
                "confirmation did not match the repository",
            ));
        }
    }
    let client = ctx.client()?;
    let deleted_at = client.repos().delete(&repo).await.map_err(|e| match e {
        gitfox_client::Error::NotFound { .. } => CliError::new(
            ErrorCode::RepoNotFound,
            format!("no repository {repo}, or you cannot see it"),
        ),
        other => CliError::from(other),
    })?;
    ctx.renderer.emit(&RepoChanged {
        verb: "Deleted",
        reference: repo.full(),
        url: None,
        repository: None,
        extra: json!({ "deleted_at": deleted_at }),
    })
}

// ---------------------------------------------------------------------------
// edit
// ---------------------------------------------------------------------------

pub async fn edit(args: RepoEditArgs, ctx: &Context) -> Result<()> {
    if let Some(flag) = args.unsupported_setting() {
        return Err(CliError::unsupported(
            format!("`{flag}`"),
            "GitFox repositories have no such setting",
        ));
    }
    if args.visibility == Some(Visibility::Internal) {
        return Err(CliError::unsupported(
            "`--visibility internal`",
            "GitFox repositories are public or private",
        ));
    }
    if args.visibility.is_some() && !args.accept_visibility_change_consequences {
        return Err(CliError::invalid_argument(
            "use of --visibility flag requires --accept-visibility-change-consequences flag",
        ));
    }
    if args.description.is_none() && args.visibility.is_none() && args.default_branch.is_none() {
        return Err(CliError::invalid_argument("nothing to edit")
            .with_hint("pass --description, --visibility or --default-branch"));
    }

    let repo = match args.repository.as_deref() {
        Some(raw) => parse_repo(raw)?,
        None => ctx.repo()?,
    };
    let client = ctx.client()?;
    let repos = client.repos();
    let mut latest: Option<Repository> = None;
    if let Some(description) = &args.description {
        latest = Some(repos.update_description(&repo, description).await?);
    }
    if let Some(visibility) = args.visibility {
        latest = Some(
            repos
                .set_public(&repo, visibility == Visibility::Public)
                .await?,
        );
    }
    if let Some(branch) = &args.default_branch {
        latest = Some(repos.set_default_branch(&repo, branch).await?);
    }

    ctx.renderer.emit(&RepoChanged {
        verb: "Edited",
        reference: repo.full(),
        url: ctx.web_url(&repo.full()).ok(),
        repository: latest,
        extra: Value::Null,
    })
}

// ---------------------------------------------------------------------------
// rename
// ---------------------------------------------------------------------------

pub async fn rename(args: RepoRenameArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let new_name = match args.new_name.clone() {
        Some(name) => name,
        None => {
            ctx.require_interactive("a new name").map_err(|_| {
                CliError::invalid_argument(
                    "new name argument required when not running interactively",
                )
            })?;
            dialoguer::Input::<String>::new()
                .with_prompt(format!("Rename {} to", repo.full()))
                .interact_text()
                .map_err(|e| CliError::invalid_argument(format!("could not read the name: {e}")))?
        }
    };
    let new_name = new_name.trim().to_string();
    if new_name.contains('/') {
        return Err(CliError::invalid_argument(
            "the new name is a name, not a path; `gf repo rename` keeps the repository in its space",
        ));
    }
    if !args.yes && !ctx.config.non_interactive {
        interact::confirm(
            ctx,
            &format!("Rename {} to {new_name}?", repo.full()),
            "--yes",
        )?;
    }

    let client = ctx.client()?;
    let renamed = client.repos().rename(&repo, &new_name).await?;
    let new_ref = format!("{}/{new_name}", repo.space());

    // A checkout's remote still points at the old path; follow the rename, as
    // gh does, when it is the remote this repository came from.
    let mut remote_updated = None;
    if let Some(remote) = git::remote_name()
        && let Some(url) = git::remote_url(&remote)
        && let Some(parsed) = git::parse_remote(&url)
        && parsed.repo == repo.full()
    {
        let ssh = !url.starts_with("http");
        if let Some(new_url) = renamed.clone_url(ssh)
            && git::remote_set_url(&remote, new_url).is_ok()
        {
            remote_updated = Some(remote);
        }
    }

    ctx.renderer.emit(&RepoChanged {
        verb: "Renamed",
        reference: new_ref.clone(),
        url: ctx.web_url(&new_ref).ok(),
        repository: Some(renamed),
        extra: json!({ "previous": repo.full(), "remote_updated": remote_updated }),
    })
}

// ---------------------------------------------------------------------------
// fork
// ---------------------------------------------------------------------------

pub async fn fork(args: RepoForkArgs, ctx: &Context) -> Result<()> {
    if args.default_branch_only {
        ctx.warn("GitFox forks every branch; --default-branch-only has no effect");
    }
    let source_ref = match args.repository.as_deref() {
        Some(raw) => parse_repo(raw)?,
        None => ctx.repo()?,
    };
    let client = ctx.client()?;
    let source = fetch(&client, &source_ref).await?;
    let source_id = source.id.ok_or_else(|| {
        CliError::new(
            ErrorCode::ApiError,
            format!("{source_ref} reports no id to fork from"),
        )
    })?;

    let space = match &ctx.config.org {
        Some(org) => org.clone(),
        None => source_ref.space().to_string(),
    };
    let identifier = args
        .fork_name
        .clone()
        .unwrap_or_else(|| source_ref.name().to_string());
    if space == source_ref.space() && identifier == source_ref.name() {
        return Err(CliError::invalid_argument(format!(
            "a fork of {source_ref} in the same space needs another name"
        ))
        .with_hint("pass --fork-name, or --org to fork into another space"));
    }

    let forked = client
        .repos()
        .create(&CreateRepository {
            identifier: identifier.clone(),
            parent_ref: space.clone(),
            description: source.description.clone(),
            default_branch: source.default_branch.clone(),
            is_public: source.is_public.unwrap_or(false),
            fork_id: Some(source_id),
            ..Default::default()
        })
        .await?;
    let fork_ref = format!("{space}/{identifier}");
    let ssh = use_ssh(ctx, false);

    let mut cloned_to = None;
    let mut remote_added = None;
    if args.clone {
        let url = forked.clone_url(ssh).ok_or_else(|| {
            CliError::new(
                ErrorCode::ApiError,
                format!("{fork_ref} reports no clone URL"),
            )
        })?;
        let destination = std::path::PathBuf::from(&identifier);
        git::clone(url, &destination, &args.git_flags)
            .map_err(|m| CliError::new(ErrorCode::Unexpected, m))?;
        if let Some(upstream) = source.clone_url(ssh) {
            let _ = git::remote_add_in(&destination, "upstream", upstream);
        }
        cloned_to = Some(destination.display().to_string());
    } else if args.remote {
        let url = forked.clone_url(ssh).ok_or_else(|| {
            CliError::new(
                ErrorCode::ApiError,
                format!("{fork_ref} reports no clone URL"),
            )
        })?;
        let here = std::path::Path::new(".");
        let name = args.remote_name.as_str();
        // gh renames the existing `origin` to `upstream` before adding the
        // fork as `origin`; do the same when that is the name asked for.
        if name == "origin" && git::remote_url("origin").is_some() {
            let _ = std::process::Command::new("git")
                .args(["remote", "rename", "origin", "upstream"])
                .output();
        }
        git::remote_add_in(here, name, url)
            .map_err(|m| CliError::new(ErrorCode::GitContextError, m))?;
        remote_added = Some(name.to_string());
    }

    ctx.renderer.emit(&RepoChanged {
        verb: "Forked",
        reference: fork_ref.clone(),
        url: ctx.web_url(&fork_ref).ok(),
        repository: Some(forked),
        extra: json!({
            "source": source_ref.full(),
            "cloned_to": cloned_to,
            "remote_added": remote_added,
        }),
    })
}

// ---------------------------------------------------------------------------
// sync
// ---------------------------------------------------------------------------

pub async fn sync(args: RepoSyncArgs, ctx: &Context) -> Result<()> {
    if args.source.is_some() {
        return Err(CliError::unsupported(
            "`--source`",
            "GitFox syncs a fork or mirror from the upstream it already knows",
        ));
    }

    // A named repository is synced on the server; without one, the checkout's
    // branch is brought up to date from its remote, as gh does.
    if let Some(raw) = args.destination.as_deref() {
        if args.branch.is_some() || args.force {
            return Err(CliError::unsupported(
                "`--branch` / `--force` with a repository",
                "GitFox syncs the whole repository from its upstream",
            ));
        }
        let repo = parse_repo(raw)?;
        let client = ctx.client()?;
        client.repos().sync(&repo).await?;
        return ctx.renderer.emit(&Synced {
            target: repo.full(),
            branch: None,
            detail: "synced from its upstream on the server".to_string(),
        });
    }

    let branch = match args.branch.clone() {
        Some(branch) => branch,
        None => ctx.branch()?.to_string(),
    };
    let remote = git::remote_name().ok_or_else(|| {
        CliError::new(
            ErrorCode::GitContextError,
            "no git remote to sync from, or not inside a git repository",
        )
    })?;
    git::fetch(&remote, &branch).map_err(|m| CliError::new(ErrorCode::GitContextError, m))?;

    let current = git::current_branch();
    let fast_forward = if git::local_branch_exists(&branch) {
        git::is_ancestor(&format!("refs/heads/{branch}"), "FETCH_HEAD")
    } else {
        true
    };
    if !fast_forward && !args.force {
        return Err(CliError::new(
            ErrorCode::GitContextError,
            format!("can't sync because there are diverging changes on `{branch}`"),
        )
        .with_hint("pass --force to reset it to the remote"));
    }

    if current.as_deref() == Some(branch.as_str()) {
        let result = if args.force {
            git::reset_hard("FETCH_HEAD")
        } else {
            git::merge_ff_only("FETCH_HEAD")
        };
        result.map_err(|m| CliError::new(ErrorCode::GitContextError, m))?;
    } else {
        git::update_branch_ref(&branch, "FETCH_HEAD")
            .map_err(|m| CliError::new(ErrorCode::GitContextError, m))?;
    }

    ctx.renderer.emit(&Synced {
        target: remote.clone(),
        branch: Some(branch.clone()),
        detail: format!("synced the local {branch} branch from {remote}/{branch}"),
    })
}

// ---------------------------------------------------------------------------
// set-default
// ---------------------------------------------------------------------------

pub async fn set_default(args: RepoSetDefaultArgs, ctx: &Context) -> Result<()> {
    if ctx.git.remotes.is_empty() && !git::is_work_tree(std::path::Path::new(".")) {
        return Err(CliError::new(
            ErrorCode::GitContextError,
            "must be run from inside a git repository",
        ));
    }
    if args.view {
        return match ctx.git.default_repo.clone() {
            Some(repo) => ctx.renderer.emit(&DefaultRepo {
                repo: Some(repo),
                changed: false,
            }),
            None => Err(CliError::new(
                ErrorCode::NotFound,
                "no default repository has been set; use `gf repo set-default` to add one",
            )),
        };
    }
    if args.unset {
        for key in [git::DEFAULT_REPO_KEY, git::LEGACY_DEFAULT_REPO_KEY] {
            git::unset_local_config(key)
                .map_err(|m| CliError::new(ErrorCode::GitContextError, m))?;
        }
        return ctx.renderer.emit(&DefaultRepo {
            repo: None,
            changed: true,
        });
    }

    let repo = match args.repository.as_deref() {
        Some(raw) => parse_repo(raw)?,
        None => {
            let inferred = ctx
                .git
                .to_context()
                .repo_for(ctx.config.host_key.as_deref())
                .ok_or_else(|| {
                    CliError::invalid_argument(
                        "repository required when no remote points at this GitFox host",
                    )
                })?;
            parse_repo(&inferred)?
        }
    };
    // Checked, so a typo is caught now rather than on every later command.
    let client = ctx.client()?;
    fetch(&client, &repo).await?;
    git::set_local_config(git::DEFAULT_REPO_KEY, &repo.full())
        .map_err(|m| CliError::new(ErrorCode::GitContextError, m))?;
    // Nothing reads the pre-0.7 key once this one is set; leaving it would only
    // strand a stale repository in the checkout's git config.
    git::unset_local_config(git::LEGACY_DEFAULT_REPO_KEY)
        .map_err(|m| CliError::new(ErrorCode::GitContextError, m))?;
    ctx.renderer.emit(&DefaultRepo {
        repo: Some(repo.full()),
        changed: true,
    })
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

struct RepoChanged {
    verb: &'static str,
    reference: String,
    url: Option<String>,
    repository: Option<Repository>,
    extra: Value,
}

impl Render for RepoChanged {
    fn to_json(&self) -> Value {
        let mut value = json!({
            "action": self.verb.to_ascii_lowercase(),
            "repository": self.reference,
            "web_url": self.url,
            "git_url": self.repository.as_ref().and_then(|r| r.git_url.clone()),
            "git_ssh_url": self.repository.as_ref().and_then(|r| r.git_ssh_url.clone()),
            "visibility": self.repository.as_ref().and_then(|r| r.visibility()),
            "default_branch": self.repository.as_ref().and_then(|r| r.default_branch.clone()),
            "description": self.repository.as_ref().and_then(|r| r.description.clone()),
        });
        if let Value::Object(extra) = &self.extra {
            for (k, v) in extra {
                value[k] = v.clone();
            }
        }
        value
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut out = format!(
            "{green}✓{reset} {} repository {}",
            self.verb, self.reference
        );
        if let Some(url) = &self.url {
            out.push_str(&format!("\n  {url}"));
        }
        if let Some(dir) = self.extra.get("cloned_to").and_then(Value::as_str) {
            out.push_str(&format!("\n  cloned into {dir}"));
        }
        if let Some(remote) = self.extra.get("remote_added").and_then(Value::as_str) {
            out.push_str(&format!("\n  added remote {remote}"));
        }
        out
    }
}

struct Synced {
    target: String,
    branch: Option<String>,
    detail: String,
}

impl Render for Synced {
    fn to_json(&self) -> Value {
        json!({ "synced": self.target, "branch": self.branch, "detail": self.detail })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        match &self.branch {
            Some(_) => format!("{green}✓{reset} {}", capitalize(&self.detail)),
            None => format!("{green}✓{reset} {} {}", self.target, self.detail),
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

struct DefaultRepo {
    repo: Option<String>,
    changed: bool,
}

impl Render for DefaultRepo {
    fn to_json(&self) -> Value {
        json!({ "default_repository": self.repo, "changed": self.changed })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        match (&self.repo, self.changed) {
            (Some(repo), false) => repo.clone(),
            (Some(repo), true) => {
                format!("{green}✓{reset} Set {repo} as the default repository for this checkout")
            }
            (None, _) => format!("{green}✓{reset} Unset the default repository"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_carries_its_extra_facts_into_the_json() {
        let changed = RepoChanged {
            verb: "Forked",
            reference: "me/backend".into(),
            url: Some("http://h/me/backend".into()),
            repository: None,
            extra: json!({ "source": "ai/backend", "cloned_to": "backend" }),
        };
        let value = changed.to_json();
        assert_eq!(value["action"], "forked");
        assert_eq!(value["source"], "ai/backend");
        let text = changed.to_human(false);
        assert!(text.contains("Forked repository me/backend"), "{text}");
        assert!(text.contains("cloned into backend"), "{text}");
    }

    #[test]
    fn set_default_reads_back_bare_for_scripts() {
        let view = DefaultRepo {
            repo: Some("ai/backend".into()),
            changed: false,
        };
        assert_eq!(view.to_human(false), "ai/backend");
        assert_eq!(capitalize("synced"), "Synced");
    }
}
