//! `fx workflow` — pipelines, the way `gh workflow` presents workflows.
//!
//! A pipeline can be named by its identifier, its numeric id, or the path of
//! its YAML file — the three ways gh names a workflow.

use gitfox_client::{Execution, GitFoxClient, Pipeline, RepoRef};
use serde_json::{Value, json};

use super::pipeline::{RunStarted, list_pipelines, not_found_as_pipeline, paint, status_mark};
use crate::cli::{
    WorkflowCommand, WorkflowListArgs, WorkflowRefArgs, WorkflowRunArgs, WorkflowSubcommand,
    WorkflowViewArgs,
};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::export;
use crate::interact;
use crate::output::{Render, key_values, plain_table, relative_time};
use crate::paginate;

pub async fn run(cmd: WorkflowCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        WorkflowSubcommand::List(args) => list(args, ctx).await,
        WorkflowSubcommand::View(args) => view(args, ctx).await,
        WorkflowSubcommand::Run(args) => trigger(args, ctx).await,
        WorkflowSubcommand::Enable(args) => set_enabled(args, true, ctx).await,
        WorkflowSubcommand::Disable(args) => set_enabled(args, false, ctx).await,
    }
}

/// The pipeline a workflow selector names.
async fn find(selector: Option<&str>, repo: &RepoRef, client: &GitFoxClient) -> Result<Pipeline> {
    let pipelines = list_pipelines(client, repo, false, 100).await?;
    let Some(selector) = selector.map(str::trim).filter(|s| !s.is_empty()) else {
        return match pipelines.as_slice() {
            [] => Err(CliError::new(
                ErrorCode::PipelineNotFound,
                format!("{repo} has no pipelines"),
            )),
            [only] => Ok(only.clone()),
            many => Err(CliError::invalid_argument(format!(
                "{repo} has {} pipelines; say which one",
                many.len()
            ))
            .with_hint(
                many.iter()
                    .map(|p| p.identifier.as_str())
                    .collect::<Vec<_>>()
                    .join(" | "),
            )),
        };
    };
    pipelines
        .iter()
        .find(|p| p.identifier == selector)
        .or_else(|| {
            pipelines
                .iter()
                .find(|p| p.id.is_some_and(|id| id.to_string() == selector))
        })
        .or_else(|| {
            pipelines.iter().find(|p| {
                p.config_path.as_deref().is_some_and(|path| {
                    path == selector || path.rsplit('/').next() == Some(selector)
                })
            })
        })
        .or_else(|| {
            pipelines
                .iter()
                .find(|p| p.identifier.eq_ignore_ascii_case(selector))
        })
        .cloned()
        .ok_or_else(|| {
            CliError::new(
                ErrorCode::PipelineNotFound,
                format!("could not find any pipelines named {selector}"),
            )
        })
}

fn pipeline_url(ctx: &Context, repo: &RepoRef, pipeline: &str) -> String {
    ctx.web_url(&format!("{}/pipelines/{pipeline}", repo.full()))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(args: WorkflowListArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let (client_ref, repo_ref) = (&client, &repo);
    let fetch = move |page, limit| async move {
        client_ref
            .pipelines()
            .list(repo_ref, false, page, limit)
            .await
    };
    let all = args.all;
    let paged = paginate::collect_filtered(
        args.limit,
        move |p: &Pipeline| all || !p.disabled.unwrap_or(false),
        fetch,
    )
    .await
    .map_err(|e| super::pr::not_found_as_repo(e, &repo))?;

    let urls = paged
        .items
        .iter()
        .map(|p| pipeline_url(ctx, &repo, &p.identifier))
        .collect();
    ctx.renderer.emit(&WorkflowList {
        repo: repo.full(),
        pipelines: paged.items,
        urls,
        truncated: paged.truncated,
    })
}

// ---------------------------------------------------------------------------
// view
// ---------------------------------------------------------------------------

async fn view(args: WorkflowViewArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pipeline = find(args.target.workflow.as_deref(), &repo, &client).await?;

    if args.web {
        return interact::open_in_browser(ctx, &pipeline_url(ctx, &repo, &pipeline.identifier));
    }

    if args.yaml {
        // The pipeline's own endpoint serves the definition on its default
        // branch; another ref means reading the file itself there.
        let content = match args.git_ref.as_deref() {
            None => client
                .pipelines()
                .content(&repo, &pipeline.identifier)
                .await
                .map_err(|e| not_found_as_pipeline(e, &pipeline.identifier))?,
            Some(git_ref) => {
                let path = pipeline.config_path.clone().ok_or_else(|| {
                    CliError::new(
                        ErrorCode::ApiError,
                        format!("pipeline `{}` reports no config path", pipeline.identifier),
                    )
                })?;
                let file = client.repos().content(&repo, &path, Some(git_ref)).await?;
                file.bytes()
                    .and_then(|b| String::from_utf8(b).ok())
                    .ok_or_else(|| {
                        CliError::new(
                            ErrorCode::ApiError,
                            format!("`{path}` at {git_ref} is not a text file"),
                        )
                    })?
            }
        };
        return ctx.renderer.emit(&WorkflowYaml {
            pipeline: pipeline.identifier.clone(),
            path: pipeline.config_path.clone(),
            git_ref: args.git_ref.clone(),
            content,
        });
    }

    let runs = client
        .pipelines()
        .list_executions(&repo, &pipeline.identifier, 1, 5)
        .await
        .map_err(|e| not_found_as_pipeline(e, &pipeline.identifier))?;
    ctx.renderer.emit(&WorkflowView {
        url: pipeline_url(ctx, &repo, &pipeline.identifier),
        pipeline,
        runs,
    })
}

// ---------------------------------------------------------------------------
// run / enable / disable
// ---------------------------------------------------------------------------

async fn trigger(args: WorkflowRunArgs, ctx: &Context) -> Result<()> {
    if !args.fields.is_empty() || !args.raw_fields.is_empty() {
        return Err(CliError::unsupported(
            "workflow inputs (-f/-F)",
            "a GitFox pipeline run takes a branch and nothing else",
        ));
    }
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pipeline = find(args.target.workflow.as_deref(), &repo, &client).await?;
    if pipeline.disabled == Some(true) {
        return Err(CliError::invalid_argument(format!(
            "pipeline `{}` is disabled",
            pipeline.identifier
        ))
        .with_hint(format!(
            "enable it with `fx workflow enable {}`",
            pipeline.identifier
        )));
    }
    let execution = client
        .pipelines()
        .trigger(&repo, &pipeline.identifier, args.git_ref.as_deref())
        .await
        .map_err(|e| not_found_as_pipeline(e, &pipeline.identifier))?;
    ctx.renderer.emit(&RunStarted {
        pipeline: pipeline.identifier,
        execution,
        verb: "Started",
    })
}

async fn set_enabled(args: WorkflowRefArgs, enabled: bool, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pipeline = find(args.workflow.as_deref(), &repo, &client).await?;
    let already = pipeline.disabled.unwrap_or(false) != enabled;
    let updated = if already {
        pipeline
    } else {
        client
            .pipelines()
            .set_disabled(&repo, &pipeline.identifier, !enabled)
            .await
            .map_err(|e| not_found_as_pipeline(e, &pipeline.identifier))?
    };
    ctx.renderer.emit(&Toggled {
        pipeline: updated,
        enabled,
        changed: !already,
    })
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

const WORKFLOW_FIELDS: &[&str] = &[
    "id",
    "name",
    "path",
    "state",
    // fx's own names
    "identifier",
    "description",
    "disabled",
    "config_path",
    "default_branch",
    "created",
    "updated",
];

fn workflow_field(p: &Pipeline, url: &str, name: &str) -> Value {
    match name {
        "id" => json!(p.id.unwrap_or(0)),
        "name" | "identifier" => json!(p.identifier),
        "path" | "config_path" => json!(p.config_path.clone().unwrap_or_default()),
        "state" => json!(if p.disabled.unwrap_or(false) {
            "disabled_manually"
        } else {
            "active"
        }),
        "disabled" => json!(p.disabled.unwrap_or(false)),
        "description" => json!(p.description.clone().unwrap_or_default()),
        "default_branch" => json!(p.default_branch),
        "created" => json!(p.created),
        "updated" => json!(p.updated),
        "url" => json!(url),
        _ => Value::Null,
    }
}

fn pipeline_json(p: &Pipeline, url: &str) -> Value {
    json!({
        "identifier": p.identifier,
        "id": p.id,
        "description": p.description,
        "config_path": p.config_path,
        "default_branch": p.default_branch,
        "disabled": p.disabled.unwrap_or(false),
        "created": p.created,
        "updated": p.updated,
        "web_url": url,
    })
}

struct WorkflowList {
    repo: String,
    pipelines: Vec<Pipeline>,
    urls: Vec<String>,
    truncated: bool,
}

impl Render for WorkflowList {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.repo,
            "count": self.pipelines.len(),
            "truncated": self.truncated,
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.pipelines
            .iter()
            .zip(&self.urls)
            .map(|(p, url)| pipeline_json(p, url))
            .collect()
    }

    fn export_fields(&self) -> &'static [&'static str] {
        WORKFLOW_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        Value::Array(
            self.pipelines
                .iter()
                .zip(&self.urls)
                .map(|(p, url)| export::select(fields, |f| workflow_field(p, url, f)))
                .collect(),
        )
    }

    fn to_human(&self, _color: bool) -> String {
        if self.pipelines.is_empty() {
            return format!("No pipelines found in {}", self.repo);
        }
        let rows: Vec<Vec<String>> = self
            .pipelines
            .iter()
            .map(|p| {
                vec![
                    p.identifier.clone(),
                    if p.disabled.unwrap_or(false) {
                        "disabled"
                    } else {
                        "active"
                    }
                    .to_string(),
                    p.config_path.clone().unwrap_or_default(),
                    p.id.map(|id| id.to_string()).unwrap_or_default(),
                ]
            })
            .collect();
        let mut out = plain_table(&["name", "state", "path", "id"], &rows);
        if self.truncated {
            out.push_str(&format!(
                "\n\nShowing {} of more; raise --limit to see the rest.",
                self.pipelines.len()
            ));
        }
        out
    }
}

struct WorkflowView {
    pipeline: Pipeline,
    runs: Vec<Execution>,
    url: String,
}

impl Render for WorkflowView {
    fn to_json(&self) -> Value {
        let mut value = pipeline_json(&self.pipeline, &self.url);
        value["recent_runs"] = json!(
            self.runs
                .iter()
                .map(|e| super::pipeline::execution_json(&self.pipeline.identifier, e))
                .collect::<Vec<_>>()
        );
        value
    }

    fn to_human(&self, color: bool) -> String {
        let p = &self.pipeline;
        let (bold, reset) = if color {
            ("\x1b[1m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut pairs = vec![(
            "State",
            if p.disabled.unwrap_or(false) {
                "disabled".to_string()
            } else {
                "active".to_string()
            },
        )];
        if let Some(path) = &p.config_path {
            pairs.push(("Path", path.clone()));
        }
        if let Some(branch) = &p.default_branch {
            pairs.push(("Default", branch.clone()));
        }
        if let Some(id) = p.id {
            pairs.push(("ID", id.to_string()));
        }
        let mut out = format!("{bold}{}{reset}\n{}", p.identifier, key_values(&pairs));
        if let Some(description) = p.description.as_deref().filter(|d| !d.trim().is_empty()) {
            out.push_str(&format!("\n\n{}", description.trim()));
        }
        out.push_str("\n\nRecent runs\n");
        if self.runs.is_empty() {
            out.push_str("  none yet");
        } else {
            let rows: Vec<Vec<String>> = self
                .runs
                .iter()
                .map(|e| {
                    let status = e.status.as_str();
                    vec![
                        format!("#{}", e.number),
                        format!("{} {}", status_mark(status), paint(status, color)),
                        e.branch().unwrap_or_default(),
                        e.summary(),
                        e.started
                            .or(e.created)
                            .map(relative_time)
                            .unwrap_or_default(),
                    ]
                })
                .collect();
            out.push_str(&plain_table(
                &["run", "status", "branch", "message", "started"],
                &rows,
            ));
        }
        if !self.url.is_empty() {
            out.push_str(&format!("\n\nView this pipeline on GitFox: {}", self.url));
        }
        out
    }
}

struct WorkflowYaml {
    pipeline: String,
    path: Option<String>,
    git_ref: Option<String>,
    content: String,
}

impl Render for WorkflowYaml {
    fn to_json(&self) -> Value {
        json!({
            "pipeline": self.pipeline,
            "path": self.path,
            "ref": self.git_ref,
            "content": self.content,
        })
    }

    fn to_human(&self, _color: bool) -> String {
        self.content.trim_end().to_string()
    }
}

struct Toggled {
    pipeline: Pipeline,
    enabled: bool,
    changed: bool,
}

impl Render for Toggled {
    fn to_json(&self) -> Value {
        json!({
            "identifier": self.pipeline.identifier,
            "disabled": !self.enabled,
            "changed": self.changed,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let state = if self.enabled { "Enabled" } else { "Disabled" };
        if self.changed {
            format!("{green}✓{reset} {state} {}", self.pipeline.identifier)
        } else {
            format!(
                "{} is already {}",
                self.pipeline.identifier,
                state.to_ascii_lowercase()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline(identifier: &str, disabled: bool) -> Pipeline {
        serde_json::from_value(json!({
            "id": 24, "identifier": identifier, "disabled": disabled,
            "config_path": ".gitfox/default.yaml", "default_branch": "main"
        }))
        .unwrap()
    }

    #[test]
    fn workflow_fields_follow_gh() {
        let p = pipeline("AgentNexus", false);
        assert_eq!(workflow_field(&p, "u", "id"), 24);
        assert_eq!(workflow_field(&p, "u", "name"), "AgentNexus");
        assert_eq!(workflow_field(&p, "u", "path"), ".gitfox/default.yaml");
        assert_eq!(workflow_field(&p, "u", "state"), "active");
        assert_eq!(
            workflow_field(&pipeline("x", true), "u", "state"),
            "disabled_manually"
        );
    }

    #[test]
    fn the_list_marks_disabled_pipelines() {
        let list = WorkflowList {
            repo: "ai/backend".into(),
            pipelines: vec![pipeline("build", false), pipeline("nightly", true)],
            urls: vec!["a".into(), "b".into()],
            truncated: false,
        };
        let text = list.to_human(false);
        assert!(text.contains("disabled"), "{text}");
        assert_eq!(list.to_json()["items"][1]["disabled"], true);
    }

    #[test]
    fn toggling_to_the_current_state_says_so() {
        let t = Toggled {
            pipeline: pipeline("build", false),
            enabled: true,
            changed: false,
        };
        assert_eq!(t.to_human(false), "build is already enabled");
        assert_eq!(t.to_json()["changed"], false);
    }
}
