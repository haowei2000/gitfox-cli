//! `fx codespace` — gitspaces, GitFox's cloud development environments,
//! under gh's `codespace` commands.
//!
//! Gitspaces are an instance feature. One with them switched off answers every
//! gitspace endpoint with a bare HTTP 500, so each command asks the system
//! configuration first and says what is actually wrong.

use std::time::Duration;

use gitfox_client::{CreateGitspace, GitFoxClient, Gitspace};
use serde_json::{Value, json};

use crate::cli::{
    CodespaceCommand, CodespaceCreateArgs, CodespaceDeleteArgs, CodespaceListArgs,
    CodespaceLogsArgs, CodespaceSelectArgs, CodespaceSubcommand, CodespaceViewArgs,
};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::export::{self, time_value};
use crate::output::{Render, key_values, plain_table, relative_time};
use crate::paginate;

pub async fn run(cmd: CodespaceCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        CodespaceSubcommand::List(args) => list(args, ctx).await,
        CodespaceSubcommand::View(args) => view(args, ctx).await,
        CodespaceSubcommand::Create(args) => create(args, ctx).await,
        CodespaceSubcommand::Delete(args) => delete(args, ctx).await,
        CodespaceSubcommand::Stop(args) => stop(args, ctx).await,
        CodespaceSubcommand::Logs(args) => logs(args, ctx).await,
        CodespaceSubcommand::Code(_)
        | CodespaceSubcommand::Cp(_)
        | CodespaceSubcommand::Edit(_)
        | CodespaceSubcommand::Jupyter(_)
        | CodespaceSubcommand::Ports(_)
        | CodespaceSubcommand::Rebuild(_)
        | CodespaceSubcommand::Ssh(_) => super::gh_only::refuse(
            "codespace",
            "fx manages gitspaces but cannot connect to one; open it from the GitFox web UI",
        ),
    }
}

async fn enabled_client(ctx: &Context) -> Result<GitFoxClient> {
    let client = ctx.client()?;
    let config = client.spaces().system_config().await?;
    if config.gitspace_enabled == Some(false) {
        return Err(CliError::new(
            ErrorCode::Unsupported,
            "gitspaces are switched off on this GitFox instance",
        )
        .with_hint("an administrator can enable them in the instance configuration"));
    }
    Ok(client)
}

/// The gitspace `-c` names, or the only one there is.
async fn select(select: &CodespaceSelectArgs, client: &GitFoxClient) -> Result<Gitspace> {
    if select.repo_owner.is_some() {
        return Err(CliError::unsupported(
            "`--repo-owner`",
            "gitspaces are selected by name here",
        ));
    }
    if let Some(name) = &select.codespace {
        return client.gitspaces().get(name).await.map_err(|e| match e {
            gitfox_client::Error::NotFound { .. } => {
                CliError::new(ErrorCode::NotFound, format!("no gitspace named {name}"))
            }
            other => CliError::from(other),
        });
    }
    let all = client.gitspaces().list(1, 100).await?;
    match all.as_slice() {
        [] => Err(CliError::new(ErrorCode::NotFound, "you have no gitspaces")),
        [only] => Ok(only.clone()),
        many => Err(
            CliError::invalid_argument("choose a gitspace with -c/--codespace").with_hint(
                many.iter()
                    .map(|g| g.identifier.as_str())
                    .collect::<Vec<_>>()
                    .join(" | "),
            ),
        ),
    }
}

async fn list(args: CodespaceListArgs, ctx: &Context) -> Result<()> {
    if args.org.is_some() || args.user.is_some() {
        return Err(CliError::unsupported(
            "`--org` / `--user`",
            "a GitFox user lists only their own gitspaces",
        ));
    }
    if args.web {
        return Err(CliError::unsupported(
            "`--web`",
            "fx does not know where this instance serves its gitspaces page",
        ));
    }
    let client = enabled_client(ctx).await?;
    let client = &client;
    let paged = paginate::collect(args.limit, move |page, limit| async move {
        client.gitspaces().list(page, limit).await
    })
    .await?;
    ctx.renderer.emit(&GitspaceList {
        gitspaces: paged.items,
        truncated: paged.truncated,
    })
}

async fn view(args: CodespaceViewArgs, ctx: &Context) -> Result<()> {
    let client = enabled_client(ctx).await?;
    let gitspace = select(&args.select, &client).await?;
    ctx.renderer.emit(&GitspaceView(gitspace))
}

async fn create(args: CodespaceCreateArgs, ctx: &Context) -> Result<()> {
    for (given, flag) in [
        (args.idle_timeout.is_some(), "`--idle-timeout`"),
        (args.location.is_some(), "`--location`"),
        (args.retention_period.is_some(), "`--retention-period`"),
        (args.status, "`--status`"),
        (args.web, "`--web`"),
    ] {
        if given {
            return Err(CliError::unsupported(
                flag,
                "GitFox gitspaces are configured by the infrastructure resource they run on",
            ));
        }
    }
    let machine = args.machine.clone().ok_or_else(|| {
        CliError::invalid_argument("a gitspace needs an infrastructure resource")
            .with_hint("pass -m/--machine with the resource's identifier")
    })?;
    let repo = ctx.repo()?;
    let client = enabled_client(ctx).await?;
    let repository = super::repo::fetch(&client, &repo).await?;
    let branch = match args.branch.clone().or(repository.default_branch.clone()) {
        Some(branch) => branch,
        None => {
            return Err(
                CliError::invalid_argument(format!("{repo} has no default branch"))
                    .with_hint("pass --branch"),
            );
        }
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let identifier = format!("{}-{stamp}", repo.name().to_ascii_lowercase());

    let created = client
        .gitspaces()
        .create(&CreateGitspace {
            identifier: identifier.clone(),
            name: args
                .display_name
                .clone()
                .unwrap_or_else(|| repo.name().to_string()),
            space_ref: repo.space().to_string(),
            code_repo_ref: repo.full(),
            code_repo_url: repository.git_url.clone().unwrap_or_default(),
            code_repo_type: "gitness".to_string(),
            branch,
            ide: args.ide.clone(),
            resource_identifier: machine,
            resource_space_ref: repo.space().to_string(),
            devcontainer_path: args.devcontainer_path.clone(),
        })
        .await?;
    ctx.renderer.emit(&GitspaceChanged {
        verb: "Created",
        gitspace: created,
    })
}

async fn delete(args: CodespaceDeleteArgs, ctx: &Context) -> Result<()> {
    if args.org.is_some() || args.user.is_some() {
        return Err(CliError::unsupported(
            "`--org` / `--user`",
            "a GitFox user manages only their own gitspaces",
        ));
    }
    let client = enabled_client(ctx).await?;
    let targets: Vec<Gitspace> = if args.all || args.days.is_some() {
        let cutoff = args.days.map(|days| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            now - days as i64 * 86_400
        });
        client
            .gitspaces()
            .list(1, 100)
            .await?
            .into_iter()
            .filter(|g| {
                cutoff.is_none_or(|cutoff| {
                    let last = g
                        .instance
                        .as_ref()
                        .and_then(|i| i.last_used)
                        .or(g.updated)
                        .unwrap_or(0);
                    export::epoch_seconds(last) < cutoff
                })
            })
            .collect()
    } else {
        vec![select(&args.select, &client).await?]
    };

    let mut deleted = Vec::with_capacity(targets.len());
    for gitspace in targets {
        client.gitspaces().delete(&gitspace.identifier).await?;
        deleted.push(gitspace.identifier);
    }
    ctx.renderer.emit(&GitspacesDeleted(deleted))
}

async fn stop(args: CodespaceSelectArgs, ctx: &Context) -> Result<()> {
    let client = enabled_client(ctx).await?;
    let gitspace = select(&args, &client).await?;
    if gitspace.state.as_deref() == Some("stopped") {
        return Err(CliError::invalid_argument(format!(
            "gitspace {} is not running",
            gitspace.identifier
        )));
    }
    let stopped = client
        .gitspaces()
        .action(&gitspace.identifier, "stop")
        .await?;
    ctx.renderer.emit(&GitspaceChanged {
        verb: "Stopped",
        gitspace: stopped,
    })
}

async fn logs(args: CodespaceLogsArgs, ctx: &Context) -> Result<()> {
    let client = enabled_client(ctx).await?;
    let gitspace = select(&args.select, &client).await?;
    // Following means staying on the stream until it goes quiet for a long
    // time; a snapshot takes what has already arrived.
    let idle = if args.follow {
        Duration::from_secs(60 * 60)
    } else {
        Duration::from_secs(2)
    };
    let lines = client.gitspaces().logs(&gitspace.identifier, idle).await?;
    ctx.renderer.emit(&GitspaceLogs {
        identifier: gitspace.identifier,
        lines: lines.into_iter().map(|l| l.out).collect(),
    })
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

const GITSPACE_FIELDS: &[&str] = &[
    "createdAt",
    "displayName",
    "gitStatus",
    "lastUsedAt",
    "machineName",
    "name",
    "owner",
    "repository",
    "state",
    "vscsTarget",
    // fx's own names
    "identifier",
    "branch",
    "ide",
    "url",
];

/// `gh codespace view` exports more than `list`; GitFox has data for few of
/// the extras, which export gh's empty values.
const GITSPACE_VIEW_FIELDS: &[&str] = &[
    "billableOwner",
    "createdAt",
    "devcontainerPath",
    "displayName",
    "environmentId",
    "gitStatus",
    "idleTimeoutMinutes",
    "lastUsedAt",
    "location",
    "machineDisplayName",
    "machineName",
    "name",
    "owner",
    "prebuild",
    "recentFolders",
    "repository",
    "retentionExpiresAt",
    "retentionPeriodDays",
    "state",
    "vscsTarget",
    // fx's own names
    "identifier",
    "branch",
    "ide",
    "url",
];

fn gh_state(state: Option<&str>) -> &'static str {
    match state {
        Some("running") => "Available",
        Some("stopped") => "Shutdown",
        Some("starting") => "Starting",
        Some("stopping") => "ShuttingDown",
        Some("error") => "Failed",
        _ => "Unknown",
    }
}

fn gitspace_field(g: &Gitspace, name: &str) -> Value {
    match name {
        "name" | "identifier" => json!(g.identifier),
        "displayName" => json!(g.name.clone().unwrap_or_default()),
        "createdAt" => time_value(g.created),
        "lastUsedAt" => time_value(g.instance.as_ref().and_then(|i| i.last_used).or(g.updated)),
        "machineName" => json!(g.resource.get("identifier").cloned().unwrap_or(json!(""))),
        "owner" => json!(g.user_id.clone().unwrap_or_default()),
        "repository" => json!(g.code_repo_ref.clone().unwrap_or_default()),
        "state" => json!(gh_state(g.state.as_deref())),
        "gitStatus" => json!({
            "ref": g.branch.clone().unwrap_or_default(),
            "hasUncommittedChanges": false,
            "hasUnpushedChanges": false,
        }),
        "billableOwner" => {
            json!({ "login": g.user_id.clone().unwrap_or_default(), "type": "User" })
        }
        "devcontainerPath" => json!(g.devcontainer_path.clone().unwrap_or_default()),
        "machineDisplayName" => json!(g.resource.get("name").cloned().unwrap_or(json!(""))),
        "vscsTarget" | "environmentId" | "location" | "retentionExpiresAt" => json!(""),
        "idleTimeoutMinutes" | "retentionPeriodDays" => json!(0),
        "prebuild" => json!(false),
        "recentFolders" => json!([]),
        "branch" => json!(g.branch),
        "ide" => json!(g.ide),
        "url" => json!(g.instance.as_ref().and_then(|i| i.url.clone())),
        _ => Value::Null,
    }
}

fn gitspace_json(g: &Gitspace) -> Value {
    json!({
        "identifier": g.identifier,
        "name": g.name,
        "state": g.state,
        "repository": g.code_repo_ref,
        "branch": g.branch,
        "ide": g.ide,
        "url": g.instance.as_ref().and_then(|i| i.url.clone()),
        "created": g.created,
        "updated": g.updated,
    })
}

struct GitspaceList {
    gitspaces: Vec<Gitspace>,
    truncated: bool,
}

impl Render for GitspaceList {
    fn to_json(&self) -> Value {
        json!({
            "count": self.gitspaces.len(),
            "truncated": self.truncated,
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.gitspaces.iter().map(gitspace_json).collect()
    }

    fn export_fields(&self) -> &'static [&'static str] {
        GITSPACE_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        Value::Array(
            self.gitspaces
                .iter()
                .map(|g| export::select(fields, |f| gitspace_field(g, f)))
                .collect(),
        )
    }

    fn to_human(&self, _color: bool) -> String {
        if self.gitspaces.is_empty() {
            return "No gitspaces found".to_string();
        }
        let rows: Vec<Vec<String>> = self
            .gitspaces
            .iter()
            .map(|g| {
                vec![
                    g.identifier.clone(),
                    g.name.clone().unwrap_or_default(),
                    g.code_repo_ref.clone().unwrap_or_default(),
                    g.branch.clone().unwrap_or_default(),
                    g.state.clone().unwrap_or_default(),
                    g.created.map(relative_time).unwrap_or_default(),
                ]
            })
            .collect();
        plain_table(
            &[
                "name",
                "display name",
                "repository",
                "branch",
                "state",
                "created",
            ],
            &rows,
        )
    }
}

struct GitspaceView(Gitspace);

impl Render for GitspaceView {
    fn to_json(&self) -> Value {
        gitspace_json(&self.0)
    }

    fn export_fields(&self) -> &'static [&'static str] {
        GITSPACE_VIEW_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        export::select(fields, |f| gitspace_field(&self.0, f))
    }

    fn to_human(&self, color: bool) -> String {
        let g = &self.0;
        let (bold, reset) = if color {
            ("\x1b[1m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut pairs = vec![
            ("State", g.state.clone().unwrap_or_default()),
            ("Repository", g.code_repo_ref.clone().unwrap_or_default()),
            ("Branch", g.branch.clone().unwrap_or_default()),
            ("IDE", g.ide.clone().unwrap_or_default()),
        ];
        if let Some(url) = g.instance.as_ref().and_then(|i| i.url.clone()) {
            pairs.push(("URL", url));
        }
        format!("{bold}{}{reset}\n{}", g.identifier, key_values(&pairs))
    }
}

struct GitspaceChanged {
    verb: &'static str,
    gitspace: Gitspace,
}

impl Render for GitspaceChanged {
    fn to_json(&self) -> Value {
        let mut value = gitspace_json(&self.gitspace);
        value["action"] = json!(self.verb.to_ascii_lowercase());
        value
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} {} gitspace {}",
            self.verb, self.gitspace.identifier
        )
    }
}

struct GitspacesDeleted(Vec<String>);

impl Render for GitspacesDeleted {
    fn to_json(&self) -> Value {
        json!({ "deleted": self.0 })
    }

    fn to_human(&self, color: bool) -> String {
        if self.0.is_empty() {
            return "No gitspaces to delete".to_string();
        }
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        self.0
            .iter()
            .map(|id| format!("{green}✓{reset} Deleted gitspace {id}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct GitspaceLogs {
    identifier: String,
    lines: Vec<String>,
}

impl Render for GitspaceLogs {
    fn to_json(&self) -> Value {
        json!({ "identifier": self.identifier, "count": self.lines.len(), "lines": self.lines })
    }

    fn to_human(&self, _color: bool) -> String {
        self.lines
            .iter()
            .map(|l| l.trim_end_matches(['\n', '\r']))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitspace_states_map_onto_ghs() {
        let g: Gitspace = serde_json::from_value(json!({
            "identifier": "backend-1", "name": "Backend", "state": "running",
            "code_repo_ref": "ai/backend", "branch": "main",
            "resource": { "identifier": "small" }
        }))
        .unwrap();
        assert_eq!(gitspace_field(&g, "state"), "Available");
        assert_eq!(gitspace_field(&g, "machineName"), "small");
        assert_eq!(gitspace_field(&g, "repository"), "ai/backend");
        assert_eq!(gh_state(Some("stopped")), "Shutdown");
    }

    #[test]
    fn every_exported_field_has_a_value_of_ghs_type() {
        let g: Gitspace = serde_json::from_value(json!({
            "identifier": "backend-1",
            "created": 1_756_000_000_000i64,
            "updated": 1_756_000_000_000i64
        }))
        .unwrap();
        for field in GITSPACE_VIEW_FIELDS {
            let value = gitspace_field(&g, field);
            // Only the fx names without data may be null; a gh field never is.
            if !["branch", "ide", "url"].contains(field) {
                assert!(!value.is_null(), "{field} exported null");
            }
        }
        assert!(
            GITSPACE_FIELDS
                .iter()
                .all(|f| GITSPACE_VIEW_FIELDS.contains(f)),
            "view exports everything list does"
        );
        assert_eq!(gitspace_field(&g, "recentFolders"), json!([]));
        assert_eq!(gitspace_field(&g, "gitStatus")["hasUnpushedChanges"], false);
    }
}
