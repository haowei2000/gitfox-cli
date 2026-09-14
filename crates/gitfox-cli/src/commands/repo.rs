//! `fx repo` — repositories, with gh's commands and flags.

mod content;
mod manage;

use gitfox_client::{GitFoxClient, RepoRef, Repository};
use serde_json::{Value, json};

use crate::cli::{
    RepoCloneArgs, RepoCommand, RepoListArgs, RepoSubcommand, RepoViewArgs, Visibility,
};
use crate::context::{Context, parse_repo};
use crate::error::{CliError, ErrorCode, Result};
use crate::export::{self, time_value};
use crate::git;
use crate::interact;
use crate::output::{Render, key_values, plain_table, relative_time};
use crate::paginate;

pub async fn run(cmd: RepoCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        RepoSubcommand::List(args) => list(args, ctx).await,
        RepoSubcommand::View(args) => view(args, ctx).await,
        RepoSubcommand::Clone(args) => clone(args, ctx).await,
        RepoSubcommand::Create(args) => manage::create(*args, ctx).await,
        RepoSubcommand::Delete(args) => manage::delete(args, ctx).await,
        RepoSubcommand::Edit(args) => manage::edit(*args, ctx).await,
        RepoSubcommand::Rename(args) => manage::rename(args, ctx).await,
        RepoSubcommand::Fork(args) => manage::fork(args, ctx).await,
        RepoSubcommand::Sync(args) => manage::sync(args, ctx).await,
        RepoSubcommand::SetDefault(args) => manage::set_default(args, ctx).await,
        RepoSubcommand::ReadFile(args) => content::read_file(args, ctx).await,
        RepoSubcommand::ReadDir(args) => content::read_dir(args, ctx).await,
        RepoSubcommand::Gitignore(cmd) => content::gitignore(cmd, ctx).await,
        RepoSubcommand::License(cmd) => content::license(cmd, ctx).await,
        RepoSubcommand::Archive(_) | RepoSubcommand::Unarchive(_) => {
            super::gh_only::refuse("repo archive", "GitFox repositories cannot be archived")
        }
        RepoSubcommand::DeployKey(_) => super::gh_only::refuse(
            "repo deploy-key",
            "GitFox has no deploy keys; use a service account's token, or `fx ssh-key add`",
        ),
        RepoSubcommand::Autolink(_) => {
            super::gh_only::refuse("repo autolink", "GitFox has no autolinks")
        }
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(args: RepoListArgs, ctx: &Context) -> Result<()> {
    if args.archived {
        return Err(CliError::unsupported(
            "`--archived`",
            "GitFox repositories cannot be archived",
        ));
    }
    if args.language.is_some() {
        return Err(CliError::unsupported(
            "`--language`",
            "GitFox does not detect repository languages",
        ));
    }
    if !args.topic.is_empty() {
        return Err(CliError::unsupported(
            "`--topic`",
            "GitFox has no repository topics",
        ));
    }
    if args.visibility == Some(Visibility::Internal) {
        return Err(CliError::unsupported(
            "`--visibility internal`",
            "GitFox repositories are public or private",
        ));
    }

    let client = ctx.client()?;
    let sort = args.sort.into();

    // A space, if one was named or can be inferred, otherwise the whole
    // instance. The two endpoints answer with different shapes — only the
    // space-scoped one reports visibility — which `scope` records so the
    // rendering can say so rather than guessing.
    let space = args
        .space
        .clone()
        .or_else(|| ctx.config.org.clone())
        .or_else(|| current_space(ctx));

    let wants_visibility = args.visibility;
    let (fork, source) = (args.fork, args.source);
    let keep = move |repo: &Repository| {
        let visible = match wants_visibility {
            Some(Visibility::Public) => repo.is_public == Some(true),
            Some(Visibility::Private) => repo.is_public == Some(false),
            _ => true,
        };
        visible && (!fork || repo.is_fork()) && (!source || !repo.is_fork())
    };
    let filtered = args.visibility.is_some() || fork || source;

    let (client_ref, search, scope) = (&client, args.search.as_deref(), space.as_deref());
    let fetch = move |page, limit| async move {
        match scope {
            Some(space) => {
                client_ref
                    .repos()
                    .list_in_space(space, search, sort, page, limit)
                    .await
            }
            None => client_ref.repos().list(search, sort, page, limit).await,
        }
    };
    let paged = if filtered {
        paginate::collect_filtered(args.limit, keep, fetch).await
    } else {
        paginate::collect(args.limit, fetch).await
    }
    .map_err(|e| match (e, space.as_deref()) {
        (gitfox_client::Error::NotFound { .. }, Some(space)) => {
            CliError::new(ErrorCode::NotFound, format!("no space `{space}`"))
        }
        (other, _) => CliError::from(other),
    })?;

    let urls = paged
        .items
        .iter()
        .map(|r| ctx.web_url(&r.reference()).unwrap_or_default())
        .collect();
    ctx.renderer.emit(&RepoList {
        space,
        repos: paged.items,
        urls,
        truncated: paged.truncated,
    })
}

/// The space of the repository the current directory resolves to, if any.
fn current_space(ctx: &Context) -> Option<String> {
    let raw = ctx.config.repo.as_deref()?;
    RepoRef::parse(raw).ok().map(|r| r.space().to_string())
}

// ---------------------------------------------------------------------------
// view
// ---------------------------------------------------------------------------

async fn view(args: RepoViewArgs, ctx: &Context) -> Result<()> {
    let repo = match args.repository.as_deref() {
        Some(raw) => parse_repo(raw)?,
        None => ctx.repo()?,
    };
    if args.web {
        let path = match &args.branch {
            Some(branch) => format!("{}/files/{branch}", repo.full()),
            None => repo.full(),
        };
        return interact::open_in_browser(ctx, &ctx.web_url(&path)?);
    }
    let client = ctx.client()?;
    let repository = fetch(&client, &repo).await?;

    let parent = match ctx.renderer.export_spec() {
        Some(spec) if spec.wants("parent") && repository.is_fork() => client
            .repos()
            .get_by_id(repository.fork_id.unwrap_or_default())
            .await
            .ok(),
        _ => None,
    };

    // A person gets the README, as with gh; a machine asked for data.
    let readme = if ctx.renderer.is_machine() {
        None
    } else {
        readme(&client, &repo, args.branch.as_deref()).await
    };

    ctx.renderer.emit(&RepoView {
        reference: repo.full(),
        url: ctx.web_url(&repo.full()).unwrap_or_default(),
        repository,
        parent,
        readme,
    })
}

async fn readme(client: &GitFoxClient, repo: &RepoRef, branch: Option<&str>) -> Option<String> {
    for name in [
        "README.md",
        "README",
        "readme.md",
        "README.markdown",
        "README.txt",
    ] {
        if let Ok(content) = client.repos().content(repo, name, branch).await
            && let Some(bytes) = content.bytes()
        {
            return String::from_utf8(bytes).ok();
        }
    }
    None
}

pub(crate) async fn fetch(client: &GitFoxClient, repo: &RepoRef) -> Result<Repository> {
    client.repos().get(repo).await.map_err(|e| match e {
        gitfox_client::Error::NotFound { .. } => CliError::new(
            ErrorCode::RepoNotFound,
            format!("no repository {repo}, or you cannot see it"),
        )
        .with_hint("check the reference, and that the token grants access"),
        other => CliError::from(other),
    })
}

// ---------------------------------------------------------------------------
// clone
// ---------------------------------------------------------------------------

async fn clone(args: RepoCloneArgs, ctx: &Context) -> Result<()> {
    let repo = parse_repo(&args.repository)?;
    let client = ctx.client()?;
    let repository = fetch(&client, &repo).await?;
    let ssh = use_ssh(ctx, args.ssh);

    let url = repository.clone_url(ssh).ok_or_else(|| {
        CliError::new(ErrorCode::ApiError, format!("{repo} reports no clone URL"))
    })?;

    // Named after the repository asked for, not after the URL's last segment —
    // those agree on a stock instance, but only one of them is what was meant.
    let destination = args
        .directory
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from(repo.name()));

    // git owns the terminal from here: it prints its own progress and asks for
    // its own credentials. fx does not put the token in the URL — that would
    // write it into .git/config, where it outlives the command.
    git::clone(url, &destination, &args.git_flags).map_err(|message| {
        CliError::new(ErrorCode::Unexpected, message)
            .with_hint("check that git can authenticate to the host, or try --ssh")
    })?;

    // A fork gets its parent as a second remote, the way gh sets one up.
    let mut upstream = None;
    if repository.is_fork() && !args.no_upstream {
        match client
            .repos()
            .get_by_id(repository.fork_id.unwrap_or_default())
            .await
        {
            Ok(parent) => {
                if let Some(parent_url) = parent.clone_url(ssh) {
                    let name = args.upstream_remote_name.as_str();
                    let added = git::remote_add_in(&destination, name, parent_url)
                        .and_then(|()| git::fetch_remote_in(&destination, name));
                    match added {
                        Ok(()) => upstream = Some(parent.reference()),
                        Err(message) => {
                            ctx.warn(&format!("could not add the `{name}` remote: {message}"))
                        }
                    }
                }
            }
            Err(err) => ctx.warn(&format!("could not look up the fork's parent: {err}")),
        }
    }

    ctx.renderer.emit(&RepoCloned {
        reference: repo.full(),
        url: url.to_string(),
        directory: destination.display().to_string(),
        upstream,
    })
}

/// SSH when `--ssh` says so, or when `git_protocol` is `ssh`.
pub(crate) fn use_ssh(ctx: &Context, flag: bool) -> bool {
    flag || ctx
        .config
        .git_protocol
        .as_deref()
        .is_some_and(|p| p.eq_ignore_ascii_case("ssh"))
}

// ---------------------------------------------------------------------------
// fields
// ---------------------------------------------------------------------------

/// Every field `--json` accepts on a repository: gh's names, then fx's own.
pub const REPO_FIELDS: &[&str] = &[
    "archivedAt",
    "assignableUsers",
    "codeOfConduct",
    "contactLinks",
    "createdAt",
    "defaultBranchRef",
    "deleteBranchOnMerge",
    "description",
    "diskUsage",
    "forkCount",
    "fundingLinks",
    "hasDiscussionsEnabled",
    "hasIssuesEnabled",
    "hasProjectsEnabled",
    "hasWikiEnabled",
    "homepageUrl",
    "id",
    "isArchived",
    "isBlankIssuesEnabled",
    "isEmpty",
    "isFork",
    "isInOrganization",
    "isMirror",
    "isPrivate",
    "isSecurityPolicyEnabled",
    "isTemplate",
    "isUserConfigurationRepository",
    "issueTemplates",
    "issues",
    "labels",
    "languages",
    "latestRelease",
    "licenseInfo",
    "mentionableUsers",
    "mergeCommitAllowed",
    "milestones",
    "mirrorUrl",
    "name",
    "nameWithOwner",
    "openGraphImageUrl",
    "owner",
    "parent",
    "primaryLanguage",
    "projects",
    "projectsV2",
    "pullRequestTemplates",
    "pullRequests",
    "pushedAt",
    "rebaseMergeAllowed",
    "repositoryTopics",
    "securityPolicyUrl",
    "squashMergeAllowed",
    "sshUrl",
    "stargazerCount",
    "templateRepository",
    "updatedAt",
    "url",
    "usesCustomOpenGraphImage",
    "viewerCanAdminister",
    "viewerDefaultCommitEmail",
    "viewerDefaultMergeMethod",
    "viewerHasStarred",
    "viewerPermission",
    "viewerPossibleCommitEmails",
    "viewerSubscription",
    "visibility",
    "watchers",
    // fx's own names
    "repository",
    "default_branch",
    "is_public",
    "is_empty",
    "open_pull_requests",
    "size_kib",
    "git_url",
    "git_ssh_url",
    "created",
    "updated",
];

fn repo_field(repo: &Repository, url: &str, parent: Option<&Repository>, name: &str) -> Value {
    let reference = repo.reference();
    let (space, short) = reference
        .rsplit_once('/')
        .map(|(s, n)| (s.to_string(), n.to_string()))
        .unwrap_or_default();
    match name {
        "name" => json!(repo.identifier.clone().unwrap_or(short)),
        "nameWithOwner" | "repository" => json!(reference),
        "owner" => json!({ "id": "", "login": space }),
        "description" => json!(repo.description.clone().unwrap_or_default()),
        "id" => json!(repo.id.map(|id| id.to_string()).unwrap_or_default()),
        "url" => json!(url),
        "sshUrl" | "git_ssh_url" => json!(repo.git_ssh_url.clone().unwrap_or_default()),
        "git_url" => json!(repo.git_url),
        "isPrivate" => json!(repo.is_public.map(|p| !p)),
        "is_public" => json!(repo.is_public),
        "visibility" => json!(repo.visibility().map(str::to_ascii_uppercase)),
        "defaultBranchRef" => json!({ "name": repo.default_branch.clone().unwrap_or_default() }),
        "default_branch" => json!(repo.default_branch),
        "createdAt" => time_value(repo.created),
        "updatedAt" | "pushedAt" => time_value(repo.updated),
        "created" => json!(repo.created),
        "updated" => json!(repo.updated),
        "isEmpty" | "is_empty" => json!(repo.is_empty.unwrap_or(false)),
        "isFork" => json!(repo.is_fork()),
        "isMirror" => json!(repo.mirror.unwrap_or(false)),
        "forkCount" => json!(repo.num_forks.unwrap_or(0)),
        "diskUsage" | "size_kib" => json!(repo.size),
        "pullRequests" => json!({ "totalCount": repo.num_pulls.unwrap_or(0) }),
        "open_pull_requests" => json!(repo.num_open_pulls),
        "parent" => match parent {
            Some(p) => {
                let parent_ref = p.reference();
                let (owner, name) = parent_ref.rsplit_once('/').unwrap_or(("", &parent_ref));
                json!({ "id": p.id.map(|i| i.to_string()).unwrap_or_default(), "name": name, "owner": { "login": owner } })
            }
            None => Value::Null,
        },
        "isInOrganization" => json!(true),
        "isArchived"
        | "isTemplate"
        | "isUserConfigurationRepository"
        | "deleteBranchOnMerge"
        | "hasDiscussionsEnabled"
        | "hasIssuesEnabled"
        | "hasProjectsEnabled"
        | "hasWikiEnabled"
        | "isBlankIssuesEnabled"
        | "isSecurityPolicyEnabled"
        | "usesCustomOpenGraphImage"
        | "viewerCanAdminister"
        | "viewerHasStarred" => json!(false),
        "mergeCommitAllowed" | "rebaseMergeAllowed" | "squashMergeAllowed" => json!(true),
        "stargazerCount" => json!(0),
        "watchers" | "issues" => json!({ "totalCount": 0 }),
        "assignableUsers"
        | "contactLinks"
        | "fundingLinks"
        | "issueTemplates"
        | "labels"
        | "languages"
        | "mentionableUsers"
        | "milestones"
        | "projects"
        | "projectsV2"
        | "pullRequestTemplates"
        | "repositoryTopics"
        | "viewerPossibleCommitEmails" => {
            json!([])
        }
        "homepageUrl"
        | "mirrorUrl"
        | "openGraphImageUrl"
        | "securityPolicyUrl"
        | "viewerDefaultCommitEmail"
        | "viewerDefaultMergeMethod"
        | "viewerPermission"
        | "viewerSubscription" => json!(""),
        // What gh reports for a repository without these: null.
        "archivedAt" | "codeOfConduct" | "latestRelease" | "licenseInfo" | "primaryLanguage"
        | "templateRepository" => Value::Null,
        _ => Value::Null,
    }
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

fn repository_json(repo: &Repository) -> Value {
    json!({
        "repository": repo.reference(),
        "name": repo.identifier,
        "description": repo.description,
        "default_branch": repo.default_branch,
        "visibility": repo.visibility(),
        "is_public": repo.is_public,
        "is_empty": repo.is_empty,
        "is_fork": repo.is_fork(),
        "open_pull_requests": repo.num_open_pulls,
        "size_kib": repo.size,
        "git_url": repo.git_url,
        "git_ssh_url": repo.git_ssh_url,
        "created": repo.created,
        "updated": repo.updated,
    })
}

struct RepoList {
    space: Option<String>,
    repos: Vec<Repository>,
    urls: Vec<String>,
    truncated: bool,
}

impl Render for RepoList {
    fn to_json(&self) -> Value {
        json!({
            "space": self.space,
            "count": self.repos.len(),
            "truncated": self.truncated,
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.repos.iter().map(repository_json).collect()
    }

    fn export_fields(&self) -> &'static [&'static str] {
        REPO_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        Value::Array(
            self.repos
                .iter()
                .zip(&self.urls)
                .map(|(repo, url)| export::select(fields, |f| repo_field(repo, url, None, f)))
                .collect(),
        )
    }

    fn to_human(&self, _color: bool) -> String {
        if self.repos.is_empty() {
            return match &self.space {
                Some(space) => format!("No repositories in {space}"),
                None => "No repositories found".to_string(),
            };
        }

        // The instance-wide listing does not report visibility, so the column
        // is dropped rather than filled with a row of dashes.
        let has_visibility = self.repos.iter().any(|r| r.visibility().is_some());
        let rows: Vec<Vec<String>> = self
            .repos
            .iter()
            .map(|repo| {
                let mut row = vec![repo.reference()];
                if has_visibility {
                    row.push(repo.visibility().unwrap_or("—").to_string());
                }
                row.push(repo.default_branch.clone().unwrap_or_default());
                row.push(repo.updated.map(relative_time).unwrap_or_default());
                row.push(repo.description.clone().unwrap_or_default());
                row
            })
            .collect();

        let mut headers = vec!["repository"];
        if has_visibility {
            headers.push("visibility");
        }
        headers.extend(["default", "updated", "description"]);
        let mut out = plain_table(&headers, &rows);
        if self.truncated {
            out.push_str(&format!(
                "\n\nShowing {} of more; raise --limit to see the rest.",
                self.repos.len()
            ));
        }
        out
    }
}

struct RepoView {
    reference: String,
    url: String,
    repository: Repository,
    parent: Option<Repository>,
    readme: Option<String>,
}

impl Render for RepoView {
    fn to_json(&self) -> Value {
        repository_json(&self.repository)
    }

    fn export_fields(&self) -> &'static [&'static str] {
        REPO_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        export::select(fields, |f| {
            repo_field(&self.repository, &self.url, self.parent.as_ref(), f)
        })
    }

    fn to_human(&self, color: bool) -> String {
        let repo = &self.repository;
        let (bold, reset) = if color {
            ("\x1b[1m", "\x1b[0m")
        } else {
            ("", "")
        };

        let mut pairs = Vec::new();
        if let Some(visibility) = repo.visibility() {
            pairs.push(("Visibility", visibility.to_string()));
        }
        if let Some(branch) = &repo.default_branch {
            pairs.push(("Default", branch.clone()));
        }
        if repo.is_empty == Some(true) {
            pairs.push(("State", "empty".to_string()));
        }
        if repo.is_fork() {
            pairs.push(("Fork", "yes".to_string()));
        }
        if let Some(open) = repo.num_open_pulls {
            pairs.push(("Open PRs", open.to_string()));
        }
        if let Some(size) = repo.size {
            pairs.push(("Size", format!("{size} KiB")));
        }
        if let Some(updated) = repo.updated {
            pairs.push(("Updated", relative_time(updated)));
        }
        if let Some(url) = &repo.git_url {
            pairs.push(("Clone", url.clone()));
        }

        let mut out = format!("{bold}{}{reset}", self.reference);
        if let Some(description) = repo
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            out.push_str(&format!("\n{description}"));
        }
        out.push('\n');
        out.push_str(&key_values(&pairs));
        if let Some(readme) = self
            .readme
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
        {
            out.push_str("\n\n");
            out.push_str(readme);
        }
        out
    }
}

struct RepoCloned {
    reference: String,
    url: String,
    directory: String,
    upstream: Option<String>,
}

impl Render for RepoCloned {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.reference,
            "url": self.url,
            "directory": self.directory,
            "upstream": self.upstream,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut out = format!(
            "{green}✓{reset} Cloned {} into {}",
            self.reference, self.directory
        );
        if let Some(upstream) = &self.upstream {
            out.push_str(&format!("\n  added {upstream} as a remote"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str, public: Option<bool>) -> Repository {
        serde_json::from_value(json!({
            "id": 54,
            "identifier": name,
            "path": format!("ai/{name}"),
            "description": "The backend",
            "default_branch": "main",
            "is_public": public,
            "num_open_pulls": 2,
            "git_url": format!("http://h:3000/git/ai/{name}.git"),
            "updated": 1_756_000_000_000i64
        }))
        .unwrap()
    }

    fn list(repos: Vec<Repository>, space: Option<&str>) -> RepoList {
        RepoList {
            space: space.map(str::to_string),
            urls: repos
                .iter()
                .map(|r| format!("http://h:3000/{}", r.reference()))
                .collect(),
            repos,
            truncated: false,
        }
    }

    #[test]
    fn the_visibility_column_appears_only_when_the_endpoint_reported_it() {
        let text = list(vec![repo("backend", Some(false))], Some("ai")).to_human(false);
        assert!(text.contains("VISIBILITY"), "{text}");
        assert!(text.contains("private"), "{text}");

        // `GET /repos` omits is_public; a column of dashes helps nobody.
        let text = list(vec![repo("backend", None)], None).to_human(false);
        assert!(!text.contains("VISIBILITY"), "{text}");
        assert!(text.contains("ai/backend"), "{text}");
    }

    #[test]
    fn unknown_visibility_is_null_in_json_not_private() {
        let value = list(vec![repo("backend", None)], None).to_json();
        assert_eq!(value["count"], 1);
        assert!(value["items"][0]["visibility"].is_null());
        assert!(value["items"][0]["is_public"].is_null());
        assert_eq!(value["items"][0]["repository"], "ai/backend");
    }

    #[test]
    fn an_empty_listing_names_the_space_it_searched() {
        assert!(
            list(vec![], Some("ai"))
                .to_human(false)
                .contains("No repositories in ai")
        );
        assert!(
            list(vec![], None)
                .to_human(false)
                .contains("No repositories found")
        );
    }

    #[test]
    fn list_jsonl_emits_one_repository_per_line() {
        let rows = list(vec![repo("backend", None), repo("frontend", None)], None).to_jsonl();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["repository"], "ai/frontend");
    }

    #[test]
    fn gh_repository_fields_have_ghs_shapes() {
        let l = list(vec![repo("backend", Some(false))], Some("ai"));
        let value = l.export(&[
            "name".into(),
            "nameWithOwner".into(),
            "isPrivate".into(),
            "visibility".into(),
            "defaultBranchRef".into(),
            "owner".into(),
            "url".into(),
        ]);
        assert_eq!(
            value[0],
            json!({
                "defaultBranchRef": { "name": "main" },
                "isPrivate": true,
                "name": "backend",
                "nameWithOwner": "ai/backend",
                "owner": { "id": "", "login": "ai" },
                "url": "http://h:3000/ai/backend",
                "visibility": "PRIVATE",
            })
        );
    }

    #[test]
    fn every_listed_repository_field_has_a_value_arm() {
        let r = repo("backend", Some(true));
        let null_by_design = [
            "parent",
            "size_kib",
            "diskUsage",
            "created",
            "createdAt",
            "archivedAt",
            "codeOfConduct",
            "latestRelease",
            "licenseInfo",
            "primaryLanguage",
            "templateRepository",
        ];
        for name in REPO_FIELDS {
            if null_by_design.contains(name) {
                continue;
            }
            assert!(
                !repo_field(&r, "u", None, name).is_null(),
                "`{name}` exported null"
            );
        }
    }

    #[test]
    fn the_view_shows_what_is_known_and_the_readme() {
        let view = RepoView {
            reference: "ai/backend".into(),
            url: "http://h:3000/ai/backend".into(),
            repository: repo("backend", Some(true)),
            parent: None,
            readme: Some("# Backend\n\nIt serves.".into()),
        };
        let text = view.to_human(false);
        assert!(text.contains("ai/backend"), "{text}");
        assert!(text.contains("public"), "{text}");
        assert!(text.contains("Open PRs"), "{text}");
        assert!(!text.contains("Size"), "no size was reported: {text}");
        assert!(text.ends_with("It serves."), "{text}");
        assert!(!text.contains('\x1b'));
    }

    #[test]
    fn the_clone_result_reports_the_url_it_used() {
        let cloned = RepoCloned {
            reference: "ai/backend".into(),
            url: "http://h:3000/git/ai/backend.git".into(),
            directory: "backend".into(),
            upstream: None,
        };
        let value = cloned.to_json();
        assert_eq!(value["directory"], "backend");
        // No credential is ever spliced into the URL fx reports or uses.
        assert!(!value["url"].as_str().unwrap().contains('@'));
        assert!(
            cloned
                .to_human(false)
                .contains("Cloned ai/backend into backend")
        );
    }
}
