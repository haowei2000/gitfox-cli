//! `fx repo` — list, view and clone repositories.

use gitfox_client::{GitFoxClient, RepoRef, Repository};
use serde_json::{Value, json};

use crate::cli::{RepoCloneArgs, RepoCommand, RepoListArgs, RepoSubcommand, RepoViewArgs};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::git;
use crate::output::{Render, key_values, plain_table, relative_time};
use crate::paginate;

pub async fn run(cmd: RepoCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        RepoSubcommand::List(args) => list(args, ctx).await,
        RepoSubcommand::View(args) => view(args, ctx).await,
        RepoSubcommand::Clone(args) => clone(args, ctx).await,
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(args: RepoListArgs, ctx: &Context) -> Result<()> {
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

    let (client, search, scope) = (&client, args.search.as_deref(), space.as_deref());
    let paged = paginate::collect(args.limit, move |page, limit| async move {
        match scope {
            Some(space) => {
                client
                    .repos()
                    .list_in_space(space, search, sort, page, limit)
                    .await
            }
            None => client.repos().list(search, sort, page, limit).await,
        }
    })
    .await
    .map_err(|e| match (e, space.as_deref()) {
        (gitfox_client::Error::NotFound { .. }, Some(space)) => {
            CliError::new(ErrorCode::NotFound, format!("no space `{space}`"))
        }
        (other, _) => CliError::from(other),
    })?;

    ctx.renderer
        .emit(&RepoList {
            space,
            repos: paged.items,
            truncated: paged.truncated,
        })
        .map_err(unexpected)
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
        Some(raw) => parse_reference(raw)?,
        None => ctx.repo()?,
    };
    let client = ctx.client()?;
    let repository = fetch(&client, &repo).await?;
    ctx.renderer
        .emit(&RepoView {
            reference: repo.full(),
            repository,
        })
        .map_err(unexpected)
}

async fn fetch(client: &GitFoxClient, repo: &RepoRef) -> Result<Repository> {
    client.repos().get(repo).await.map_err(|e| match e {
        gitfox_client::Error::NotFound { .. } => CliError::new(
            ErrorCode::RepoNotFound,
            format!("no repository {repo}, or you cannot see it"),
        )
        .with_hint("check the reference, and that the token grants access"),
        other => CliError::from(other),
    })
}

fn parse_reference(raw: &str) -> Result<RepoRef> {
    RepoRef::parse(raw).map_err(|_| {
        CliError::invalid_argument(format!(
            "`{raw}` is not a repository reference; expected `space/name`"
        ))
    })
}

// ---------------------------------------------------------------------------
// clone
// ---------------------------------------------------------------------------

async fn clone(args: RepoCloneArgs, ctx: &Context) -> Result<()> {
    let repo = parse_reference(&args.repository)?;
    let client = ctx.client()?;
    let repository = fetch(&client, &repo).await?;

    let url = repository.clone_url(args.ssh).ok_or_else(|| {
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
    git::clone(url, &destination).map_err(|message| {
        CliError::new(ErrorCode::Unexpected, message)
            .with_hint("check that git can authenticate to the host, or try --ssh")
    })?;

    ctx.renderer
        .emit(&RepoCloned {
            reference: repo.full(),
            url: url.to_string(),
            directory: destination.display().to_string(),
        })
        .map_err(unexpected)
}

fn unexpected(err: std::io::Error) -> CliError {
    CliError::new(ErrorCode::Unexpected, err.to_string())
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
    repository: Repository,
}

impl Render for RepoView {
    fn to_json(&self) -> Value {
        repository_json(&self.repository)
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
        out
    }
}

struct RepoCloned {
    reference: String,
    url: String,
    directory: String,
}

impl Render for RepoCloned {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.reference,
            "url": self.url,
            "directory": self.directory,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} Cloned {} into {}",
            self.reference, self.directory
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str, public: Option<bool>) -> Repository {
        serde_json::from_value(json!({
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

    #[test]
    fn the_visibility_column_appears_only_when_the_endpoint_reported_it() {
        let scoped = RepoList {
            space: Some("ai".into()),
            repos: vec![repo("backend", Some(false))],
            truncated: false,
        };
        let text = scoped.to_human(false);
        assert!(text.contains("VISIBILITY"), "{text}");
        assert!(text.contains("private"), "{text}");

        // `GET /repos` omits is_public; a column of dashes helps nobody.
        let instance_wide = RepoList {
            space: None,
            repos: vec![repo("backend", None)],
            truncated: false,
        };
        let text = instance_wide.to_human(false);
        assert!(!text.contains("VISIBILITY"), "{text}");
        assert!(text.contains("ai/backend"), "{text}");
    }

    #[test]
    fn unknown_visibility_is_null_in_json_not_private() {
        let list = RepoList {
            space: None,
            repos: vec![repo("backend", None)],
            truncated: false,
        };
        let value = list.to_json();
        assert_eq!(value["count"], 1);
        assert!(value["items"][0]["visibility"].is_null());
        assert!(value["items"][0]["is_public"].is_null());
        assert_eq!(value["items"][0]["repository"], "ai/backend");
    }

    #[test]
    fn an_empty_listing_names_the_space_it_searched() {
        let scoped = RepoList {
            space: Some("ai".into()),
            repos: vec![],
            truncated: false,
        };
        assert!(scoped.to_human(false).contains("No repositories in ai"));
        let all = RepoList {
            space: None,
            repos: vec![],
            truncated: false,
        };
        assert!(all.to_human(false).contains("No repositories found"));
    }

    #[test]
    fn list_jsonl_emits_one_repository_per_line() {
        let list = RepoList {
            space: None,
            repos: vec![repo("backend", None), repo("frontend", None)],
            truncated: true,
        };
        let rows = list.to_jsonl();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["repository"], "ai/frontend");
    }

    #[test]
    fn the_view_shows_what_is_known_and_omits_what_is_not() {
        let view = RepoView {
            reference: "ai/backend".into(),
            repository: repo("backend", Some(true)),
        };
        let text = view.to_human(false);
        assert!(text.contains("ai/backend"), "{text}");
        assert!(text.contains("public"), "{text}");
        assert!(text.contains("Open PRs"), "{text}");
        assert!(!text.contains("Size"), "no size was reported: {text}");
        assert!(!text.contains('\x1b'));
    }

    #[test]
    fn the_clone_result_reports_the_url_it_used() {
        let cloned = RepoCloned {
            reference: "ai/backend".into(),
            url: "http://h:3000/git/ai/backend.git".into(),
            directory: "backend".into(),
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
