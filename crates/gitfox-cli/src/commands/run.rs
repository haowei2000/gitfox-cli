//! `fx run` — pipeline runs, the way `gh run` presents them.
//!
//! GitFox numbers runs per pipeline, where GitHub numbers them per repository.
//! So a run is `NUMBER` when that is unambiguous — one pipeline, or only one
//! pipeline with a run of that number — and otherwise `PIPELINE/NUMBER` or
//! `NUMBER --workflow PIPELINE`. With no run at all, the most recent one.

use std::cmp::Reverse;
use std::time::Duration;

use gitfox_client::{Execution, GitFoxClient, RepoRef};
use serde_json::{Value, json};

use super::pipeline::{
    RunList, RunLogs, RunStarted, RunView, execution_json, fetch_step_logs, list_pipelines,
    not_found_as_pipeline, not_found_as_run, resolve_run,
};
use crate::cli::{
    RunCancelArgs, RunCommand, RunListArgs, RunRefArgs, RunRerunArgs, RunSubcommand, RunViewArgs,
    RunWatchArgs,
};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::export::{self, parse_iso8601, time_value};
use crate::interact;
use crate::output::Render;
use crate::paginate;

pub async fn run(cmd: RunCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        RunSubcommand::List(args) => list(args, ctx).await,
        RunSubcommand::View(args) => view(args, ctx).await,
        RunSubcommand::Rerun(args) => rerun(args, ctx).await,
        RunSubcommand::Watch(args) => watch(args, ctx).await,
        RunSubcommand::Cancel(args) => cancel(args, ctx).await,
        RunSubcommand::Delete(args) => delete(args, ctx).await,
        RunSubcommand::Download(_) => super::gh_only::refuse(
            "run download",
            "GitFox pipeline runs keep no downloadable artifacts through its API",
        ),
    }
}

// ---------------------------------------------------------------------------
// naming a run
// ---------------------------------------------------------------------------

/// `182`, `#182`, `default/182` or `default#182`.
fn parse_run(raw: &str) -> Result<(Option<String>, u64)> {
    let raw = raw.trim();
    if let Some((pipeline, number)) = raw.rsplit_once(['/', '#'])
        && !pipeline.is_empty()
        && let Ok(number) = number.parse::<u64>()
    {
        return Ok((Some(pipeline.to_string()), number));
    }
    raw.trim_start_matches('#')
        .parse::<u64>()
        .map(|n| (None, n))
        .map_err(|_| {
            CliError::invalid_argument(format!("`{raw}` is not a run")).with_hint(
                "a run is NUMBER, or PIPELINE/NUMBER when the repository has several pipelines",
            )
        })
}

/// The pipeline and run number `target` names.
async fn locate(
    target: &RunRefArgs,
    repo: &RepoRef,
    client: &GitFoxClient,
) -> Result<(String, u64)> {
    let (from_ref, number) = match target.run.as_deref() {
        Some(raw) => {
            let (pipeline, number) = parse_run(raw)?;
            (pipeline, Some(number))
        }
        None => (None, None),
    };
    if let Some(pipeline) = target.workflow.clone().or(from_ref) {
        let number = resolve_run(number, repo, &pipeline, client).await?;
        return Ok((pipeline, number));
    }

    let pipelines = list_pipelines(client, repo, number.is_none(), 100).await?;
    match (pipelines.as_slice(), number) {
        ([], _) => Err(CliError::new(
            ErrorCode::PipelineNotFound,
            format!("{repo} has no pipelines"),
        )),
        ([only], number) => {
            let number = resolve_run(number, repo, &only.identifier, client).await?;
            Ok((only.identifier.clone(), number))
        }
        // No run named: the newest across every pipeline, which the
        // `latest=true` listing already embedded.
        (many, None) => many
            .iter()
            .filter_map(|p| p.execution.as_ref().map(|e| (p.identifier.clone(), e)))
            .max_by_key(|(_, e)| e.started.or(e.created).unwrap_or(0))
            .map(|(pipeline, e)| (pipeline, e.number))
            .ok_or_else(|| {
                CliError::new(
                    ErrorCode::PipelineNotFound,
                    format!("no pipeline in {repo} has run yet"),
                )
            }),
        (many, Some(number)) => {
            let mut matches = Vec::new();
            for pipeline in many {
                match client
                    .pipelines()
                    .get_execution(repo, &pipeline.identifier, number)
                    .await
                {
                    Ok(_) => matches.push(pipeline.identifier.clone()),
                    Err(gitfox_client::Error::NotFound { .. }) => {}
                    Err(other) => return Err(other.into()),
                }
            }
            match matches.as_slice() {
                [] => Err(CliError::new(
                    ErrorCode::PipelineNotFound,
                    format!("no pipeline in {repo} has a run #{number}"),
                )),
                [only] => Ok((only.clone(), number)),
                several => Err(CliError::invalid_argument(format!(
                    "{} pipelines have a run #{number}; say which one",
                    several.len()
                ))
                .with_hint(
                    several
                        .iter()
                        .map(|p| format!("{p}/{number}"))
                        .collect::<Vec<_>>()
                        .join(" | "),
                )),
            }
        }
    }
}

fn run_url(ctx: &Context, repo: &RepoRef, pipeline: &str, number: u64) -> String {
    ctx.web_url(&format!(
        "{}/pipelines/{pipeline}/execution/{number}",
        repo.full()
    ))
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(args: RunListArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let filter = RunFilter::from_args(&args)?;

    let pipelines: Vec<String> = match &args.workflow {
        Some(pipeline) => vec![pipeline.clone()],
        None => list_pipelines(&client, &repo, false, 100)
            .await?
            .into_iter()
            .map(|p| p.identifier)
            .collect(),
    };

    let mut runs: Vec<(String, Execution)> = Vec::new();
    let mut truncated = false;
    for pipeline in &pipelines {
        let (client_ref, repo_ref, name) = (&client, &repo, pipeline.as_str());
        let fetch = move |page, limit| async move {
            client_ref
                .pipelines()
                .list_executions(repo_ref, name, page, limit)
                .await
        };
        let paged = if filter.is_empty() {
            paginate::collect(args.limit, fetch).await
        } else {
            paginate::collect_filtered(args.limit, |e: &Execution| filter.matches(e), fetch).await
        }
        .map_err(|e| not_found_as_pipeline(e, pipeline))?;
        truncated |= paged.truncated;
        runs.extend(paged.items.into_iter().map(|e| (pipeline.clone(), e)));
    }

    runs.sort_by_key(|(_, e)| Reverse(e.created.or(e.started).unwrap_or(0)));
    if runs.len() > args.limit as usize {
        runs.truncate(args.limit as usize);
        truncated = true;
    }

    let urls = runs
        .iter()
        .map(|(pipeline, e)| run_url(ctx, &repo, pipeline, e.number))
        .collect();
    ctx.renderer.emit(&GhRunList {
        list: RunList {
            repo: repo.full(),
            runs,
            truncated,
        },
        urls,
    })
}

/// The client-side filters `fx run list` applies — GitFox's executions
/// listing takes none of them.
#[derive(Debug, Default)]
struct RunFilter {
    branch: Option<String>,
    statuses: Option<Vec<&'static str>>,
    raw_status: Option<String>,
    user: Option<String>,
    commit: Option<String>,
    events: Option<Vec<String>>,
    created: Option<(Option<i64>, Option<i64>)>,
}

impl RunFilter {
    fn from_args(args: &RunListArgs) -> Result<Self> {
        let (statuses, raw_status) = match args.status.as_deref() {
            None => (None, None),
            Some(status) => match gh_status_to_gitfox(status) {
                Some(list) => (Some(list), None),
                None => (None, Some(status.to_ascii_lowercase())),
            },
        };
        Ok(Self {
            branch: args.branch.clone(),
            statuses,
            raw_status,
            user: args.user.clone(),
            commit: args.commit.clone(),
            events: args.event.as_deref().map(|e| match e {
                "workflow_dispatch" | "manual" => vec!["manual".to_string()],
                "schedule" | "cron" => vec!["cron".to_string()],
                other => vec![other.to_string()],
            }),
            created: args.created.as_deref().map(parse_date_range).transpose()?,
        })
    }

    fn is_empty(&self) -> bool {
        self.branch.is_none()
            && self.statuses.is_none()
            && self.raw_status.is_none()
            && self.user.is_none()
            && self.commit.is_none()
            && self.events.is_none()
            && self.created.is_none()
    }

    fn matches(&self, e: &Execution) -> bool {
        if let Some(branch) = &self.branch
            && e.branch().as_deref() != Some(branch.as_str())
        {
            return false;
        }
        if let Some(statuses) = &self.statuses
            && !statuses.contains(&e.status.as_str())
        {
            return false;
        }
        if let Some(status) = &self.raw_status
            && e.status.as_str() != status
        {
            return false;
        }
        if let Some(user) = &self.user {
            let login = e.author_login.as_deref().unwrap_or_default();
            let name = e.author_name.as_deref().unwrap_or_default();
            if !login.eq_ignore_ascii_case(user) && !name.eq_ignore_ascii_case(user) {
                return false;
            }
        }
        if let Some(commit) = &self.commit
            && !e
                .after
                .as_deref()
                .is_some_and(|sha| sha.starts_with(commit.as_str()))
        {
            return false;
        }
        if let Some(events) = &self.events
            && !e
                .event
                .as_deref()
                .is_some_and(|ev| events.iter().any(|x| x == ev))
        {
            return false;
        }
        if let Some((from, until)) = self.created {
            let created = export::epoch_seconds(e.created.or(e.started).unwrap_or(0));
            if from.is_some_and(|f| created < f) || until.is_some_and(|u| created >= u) {
                return false;
            }
        }
        true
    }
}

/// gh's run statuses and conclusions, as the GitFox statuses they cover.
/// `None` for a word gh does not use, which is then matched as GitFox's own.
fn gh_status_to_gitfox(status: &str) -> Option<Vec<&'static str>> {
    Some(match status.to_ascii_lowercase().as_str() {
        "queued" | "requested" | "pending" => vec!["pending"],
        "waiting" | "action_required" => vec!["blocked", "waiting_on_dependencies"],
        "in_progress" => vec!["running"],
        "completed" => vec![
            "success", "failure", "error", "killed", "skipped", "declined",
        ],
        "cancelled" => vec!["killed"],
        "failure" | "startup_failure" | "timed_out" => vec!["failure", "error"],
        "success" => vec!["success"],
        "skipped" => vec!["skipped", "declined"],
        "neutral" | "stale" => vec![],
        _ => return None,
    })
}

/// gh's `--created` syntax: `2026-09-14`, `>=2026-09-01`, `<2026-09-14`,
/// `2026-09-01..2026-09-14`, `2026-09-01..*`. Returns a half-open range of
/// epoch seconds, `[from, until)`.
fn parse_date_range(raw: &str) -> Result<(Option<i64>, Option<i64>)> {
    let day = |text: &str| -> Result<i64> {
        parse_iso8601(&format!("{}T00:00:00Z", text.trim())).ok_or_else(|| {
            CliError::invalid_argument(format!(
                "`{raw}` is not a date range; expected YYYY-MM-DD, >=YYYY-MM-DD or YYYY-MM-DD..YYYY-MM-DD"
            ))
        })
    };
    const DAY: i64 = 86_400;
    let raw = raw.trim();
    if let Some((from, until)) = raw.split_once("..") {
        let from = match from.trim() {
            "*" | "" => None,
            d => Some(day(d)?),
        };
        let until = match until.trim() {
            "*" | "" => None,
            d => Some(day(d)? + DAY),
        };
        return Ok((from, until));
    }
    if let Some(d) = raw.strip_prefix(">=") {
        return Ok((Some(day(d)?), None));
    }
    if let Some(d) = raw.strip_prefix('>') {
        return Ok((Some(day(d)? + DAY), None));
    }
    if let Some(d) = raw.strip_prefix("<=") {
        return Ok((None, Some(day(d)? + DAY)));
    }
    if let Some(d) = raw.strip_prefix('<') {
        return Ok((None, Some(day(d)?)));
    }
    let start = day(raw)?;
    Ok((Some(start), Some(start + DAY)))
}

// ---------------------------------------------------------------------------
// view
// ---------------------------------------------------------------------------

async fn view(args: RunViewArgs, ctx: &Context) -> Result<()> {
    if args.attempt.is_some() {
        return Err(CliError::unsupported(
            "`--attempt`",
            "GitFox keeps each retry as a run of its own; view that run instead",
        ));
    }
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let (pipeline, number) = locate(&args.target, &repo, &client).await?;
    let url = run_url(ctx, &repo, &pipeline, number);
    if args.web {
        return interact::open_in_browser(ctx, &url);
    }

    let mut execution = client
        .pipelines()
        .get_execution(&repo, &pipeline, number)
        .await
        .map_err(|e| not_found_as_run(e, &pipeline, number))?;
    if let Some(job) = args.job.as_deref() {
        let before = execution.stages.len();
        execution
            .stages
            .retain(|s| s.name.eq_ignore_ascii_case(job) || s.number.to_string() == job);
        if execution.stages.is_empty() && before > 0 {
            return Err(CliError::new(
                ErrorCode::PipelineNotFound,
                format!("run #{number} has no stage `{job}`"),
            ));
        }
    }
    let failed = execution.status.is_failed();

    if args.log || args.log_failed {
        let steps = fetch_step_logs(
            &client,
            &repo,
            &pipeline,
            number,
            &execution,
            args.log_failed,
            None,
            None,
        )
        .await;
        ctx.renderer.emit(&RunLogs {
            pipeline,
            run: number,
            status: execution.status.as_str().to_string(),
            only_failed: args.log_failed,
            steps,
        })?;
    } else {
        ctx.renderer.emit(&GhRunView {
            view: RunView {
                pipeline,
                execution,
            },
            url,
        })?;
    }

    exit_status(ctx, args.exit_status, failed, number)
}

/// `--exit-status`: a failed run exits 1 once its output is on screen.
///
/// Only for a person. JSON already carried the result — a second, error
/// document would break "stdout is one JSON document", and gh likewise
/// ignores `--exit-status` when exporting JSON — so a machine reads the
/// run's `status` / `conclusion` instead.
fn exit_status(ctx: &Context, wanted: bool, failed: bool, number: u64) -> Result<()> {
    if !wanted || !failed || ctx.renderer.is_machine() {
        return Ok(());
    }
    Err(CliError::new(ErrorCode::RunFailed, format!("run #{number} failed")).silenced())
}

// ---------------------------------------------------------------------------
// rerun / watch / cancel / delete
// ---------------------------------------------------------------------------

async fn rerun(args: RunRerunArgs, ctx: &Context) -> Result<()> {
    if args.job.is_some() {
        return Err(CliError::unsupported(
            "`--job`",
            "GitFox reruns a whole run, not a single stage",
        ));
    }
    if args.debug {
        return Err(CliError::unsupported(
            "`--debug`",
            "GitFox has no debug reruns",
        ));
    }
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let (pipeline, number) = locate(&args.target, &repo, &client).await?;

    if args.failed {
        let execution = client
            .pipelines()
            .get_execution(&repo, &pipeline, number)
            .await
            .map_err(|e| not_found_as_run(e, &pipeline, number))?;
        if !execution.status.is_failed() {
            return Err(CliError::invalid_argument(format!(
                "run #{number} has no failed stages to rerun ({})",
                execution.status
            )));
        }
        ctx.warn("GitFox reruns every stage of the run, not only the failed ones");
    }

    let execution = client
        .pipelines()
        .retry(&repo, &pipeline, number)
        .await
        .map_err(|e| not_found_as_run(e, &pipeline, number))?;
    ctx.renderer.emit(&RunStarted {
        pipeline,
        execution,
        verb: "Requested a rerun as",
    })
}

async fn watch(args: RunWatchArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let (pipeline, number) = locate(&args.target, &repo, &client).await?;
    let interval = Duration::from_secs(args.interval.max(1));
    let live = ctx.renderer.is_tty() && !ctx.renderer.is_machine();

    let execution = loop {
        let execution = client
            .pipelines()
            .get_execution(&repo, &pipeline, number)
            .await
            .map_err(|e| not_found_as_run(e, &pipeline, number))?;
        if !execution.status.is_pending() {
            break execution;
        }
        if live {
            let view = RunView {
                pipeline: pipeline.clone(),
                execution: compacted(execution, args.compact),
            };
            ctx.renderer.write_str(&format!(
                "\x1b[H\x1b[2JRefreshing run status every {} seconds. Press Ctrl+C to quit.\n\n{}\n",
                interval.as_secs(),
                view.to_human(ctx.renderer.color())
            ))?;
        }
        tokio::time::sleep(interval).await;
    };
    if live {
        ctx.renderer.write_str("\x1b[H\x1b[2J")?;
    }

    let failed = execution.status.is_failed();
    ctx.renderer.emit(&GhRunView {
        url: run_url(ctx, &repo, &pipeline, number),
        view: RunView {
            pipeline,
            execution: compacted(execution, args.compact),
        },
    })?;
    exit_status(ctx, args.exit_status, failed, number)
}

/// `--compact`: only the stages and steps that are not simply green.
fn compacted(mut execution: Execution, compact: bool) -> Execution {
    if compact {
        for stage in &mut execution.stages {
            stage.steps.retain(|s| !s.status.is_success());
        }
        execution
            .stages
            .retain(|s| !s.status.is_success() || !s.steps.is_empty());
    }
    execution
}

async fn cancel(args: RunCancelArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let (pipeline, number) = locate(&args.target, &repo, &client).await?;
    let current = client
        .pipelines()
        .get_execution(&repo, &pipeline, number)
        .await
        .map_err(|e| not_found_as_run(e, &pipeline, number))?;
    if !current.status.is_pending() {
        return Err(CliError::invalid_argument(format!(
            "cannot cancel run #{number}: it already finished ({})",
            current.status
        )));
    }
    let execution = client
        .pipelines()
        .cancel(&repo, &pipeline, number)
        .await
        .map_err(|e| not_found_as_run(e, &pipeline, number))?;
    ctx.renderer.emit(&RunStarted {
        pipeline,
        execution,
        verb: "Cancelled",
    })
}

async fn delete(args: RunRefArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let (pipeline, number) = locate(&args, &repo, &client).await?;
    client
        .pipelines()
        .delete_execution(&repo, &pipeline, number)
        .await
        .map_err(|e| not_found_as_run(e, &pipeline, number))?;
    ctx.renderer.emit(&RunDeleted { pipeline, number })
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

/// Every field `--json` accepts on a run: gh's names, then fx's own.
const RUN_FIELDS: &[&str] = &[
    "attempt",
    "conclusion",
    "createdAt",
    "databaseId",
    "displayTitle",
    "event",
    "headBranch",
    "headSha",
    "jobs",
    "name",
    "number",
    "startedAt",
    "status",
    "updatedAt",
    "url",
    "workflowDatabaseId",
    "workflowName",
    // fx's own names
    "pipeline",
    "branch",
    "message",
    "author",
    "commit",
    "created",
    "started",
    "finished",
];

fn gh_run_status(e: &Execution) -> &'static str {
    match e.status.as_str() {
        "running" => "in_progress",
        "pending" => "queued",
        "blocked" | "waiting_on_dependencies" => "waiting",
        _ => "completed",
    }
}

fn gh_conclusion(status: &str) -> &'static str {
    match status {
        "success" => "success",
        "failure" | "error" => "failure",
        "killed" => "cancelled",
        "skipped" | "declined" => "skipped",
        _ => "",
    }
}

fn gh_event(event: Option<&str>) -> String {
    match event {
        Some("manual") => "workflow_dispatch".into(),
        Some("cron") => "schedule".into(),
        Some(other) => other.into(),
        None => String::new(),
    }
}

fn run_field(pipeline: &str, e: &Execution, url: &str, name: &str) -> Value {
    match name {
        "attempt" => json!(e.retry_count.unwrap_or(0) + 1),
        "conclusion" => json!(gh_conclusion(e.status.as_str())),
        "status" => json!(gh_run_status(e)),
        "createdAt" => time_value(e.created),
        "startedAt" => time_value(e.started.or(e.created)),
        "updatedAt" => time_value(e.updated.or(e.finished).or(e.started)),
        "databaseId" | "number" => json!(e.number),
        "displayTitle" => json!(e.summary()),
        "event" => json!(gh_event(e.event.as_deref())),
        "headBranch" => json!(e.branch().unwrap_or_default()),
        "headSha" => json!(e.after.clone().unwrap_or_default()),
        "name" | "workflowName" => json!(pipeline),
        "workflowDatabaseId" => json!(e.pipeline_id.unwrap_or(0)),
        "url" => json!(url),
        "jobs" => json!(
            e.stages
                .iter()
                .map(|stage| {
                    let finished = !stage.status.is_pending();
                    json!({
                        "databaseId": stage.number,
                        "name": stage.name,
                        "status": if finished { "completed" } else if stage.status.as_str() == "running" { "in_progress" } else { "queued" },
                        "conclusion": gh_conclusion(stage.status.as_str()),
                        "startedAt": time_value(stage.started),
                        "completedAt": time_value(stage.stopped),
                        "url": url,
                        "steps": stage.steps.iter().map(|step| json!({
                            "name": step.name,
                            "number": step.number,
                            "status": if step.status.is_pending() { "in_progress" } else { "completed" },
                            "conclusion": gh_conclusion(step.status.as_str()),
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>()
        ),
        other => execution_json(pipeline, e)
            .get(other)
            .cloned()
            .unwrap_or(Value::Null),
    }
}

struct GhRunList {
    list: RunList,
    urls: Vec<String>,
}

impl Render for GhRunList {
    fn to_json(&self) -> Value {
        self.list.to_json()
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.list.to_jsonl()
    }

    fn to_human(&self, color: bool) -> String {
        self.list.to_human(color)
    }

    fn export_fields(&self) -> &'static [&'static str] {
        RUN_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        Value::Array(
            self.list
                .runs
                .iter()
                .zip(&self.urls)
                .map(|((pipeline, e), url)| {
                    export::select(fields, |f| run_field(pipeline, e, url, f))
                })
                .collect(),
        )
    }
}

struct GhRunView {
    view: RunView,
    url: String,
}

impl Render for GhRunView {
    fn to_json(&self) -> Value {
        let mut value = self.view.to_json();
        value["url"] = json!(self.url);
        value
    }

    fn to_human(&self, color: bool) -> String {
        let mut out = self.view.to_human(color);
        if !self.url.is_empty() && self.view.execution.link.is_none() {
            out.push_str(&format!("\n\nView this run on GitFox: {}", self.url));
        }
        out
    }

    fn export_fields(&self) -> &'static [&'static str] {
        RUN_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        export::select(fields, |f| {
            run_field(&self.view.pipeline, &self.view.execution, &self.url, f)
        })
    }
}

struct RunDeleted {
    pipeline: String,
    number: u64,
}

impl Render for RunDeleted {
    fn to_json(&self) -> Value {
        json!({ "pipeline": self.pipeline, "number": self.number, "deleted": true })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} Deleted run #{} of {}",
            self.number, self.pipeline
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution(value: Value) -> Execution {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_run_is_a_number_or_a_pipeline_and_a_number() {
        assert_eq!(parse_run("182").unwrap(), (None, 182));
        assert_eq!(parse_run("#182").unwrap(), (None, 182));
        assert_eq!(
            parse_run("default/182").unwrap(),
            (Some("default".into()), 182)
        );
        assert_eq!(
            parse_run("team/build#7").unwrap(),
            (Some("team/build".into()), 7)
        );
        assert!(parse_run("latest").is_err());
    }

    #[test]
    fn gh_statuses_cover_the_gitfox_statuses_they_mean() {
        assert_eq!(gh_status_to_gitfox("in_progress"), Some(vec!["running"]));
        assert_eq!(gh_status_to_gitfox("cancelled"), Some(vec!["killed"]));
        assert!(
            gh_status_to_gitfox("completed")
                .unwrap()
                .contains(&"failure")
        );
        assert_eq!(gh_status_to_gitfox("waiting_on_dependencies"), None);
    }

    #[test]
    fn created_ranges_follow_ghs_syntax() {
        let day = parse_iso8601("2026-09-14T00:00:00Z").unwrap();
        assert_eq!(
            parse_date_range("2026-09-14").unwrap(),
            (Some(day), Some(day + 86_400))
        );
        assert_eq!(parse_date_range(">=2026-09-14").unwrap(), (Some(day), None));
        assert_eq!(parse_date_range("<2026-09-14").unwrap(), (None, Some(day)));
        assert_eq!(
            parse_date_range("2026-09-13..2026-09-14").unwrap(),
            (Some(day - 86_400), Some(day + 86_400))
        );
        assert_eq!(
            parse_date_range("2026-09-14..*").unwrap(),
            (Some(day), None)
        );
        assert!(parse_date_range("yesterday").is_err());
    }

    #[test]
    fn the_filter_combines_every_criterion() {
        let args = RunListArgs {
            workflow: None,
            branch: Some("main".into()),
            status: Some("failure".into()),
            user: Some("WHW".into()),
            commit: Some("0123".into()),
            event: Some("push".into()),
            created: Some("2025-08-24".into()),
            all: false,
            limit: 20,
            format: Default::default(),
        };
        let filter = RunFilter::from_args(&args).unwrap();
        let matching = execution(json!({
            "number": 1, "status": "error", "target": "main", "author_login": "whw",
            "after": "0123456", "event": "push", "created": 1_756_000_000_000i64
        }));
        assert!(filter.matches(&matching));
        for (key, value) in [
            ("target", json!("dev")),
            ("status", json!("success")),
            ("author_login", json!("someone")),
            ("after", json!("ffff")),
            ("event", json!("cron")),
            ("created", json!(1_700_000_000_000i64)),
        ] {
            let mut v = serde_json::to_value(&matching).unwrap();
            v[key] = value;
            assert!(
                !filter.matches(&execution(v)),
                "{key} should have excluded it"
            );
        }
        assert!(RunFilter::default().is_empty());
    }

    #[test]
    fn run_fields_speak_ghs_vocabulary() {
        let e = execution(json!({
            "number": 451, "status": "running", "event": "manual", "target": "develop",
            "after": "462cb0cb", "pipeline_id": 24, "retry_count": 1,
            "title": "feat(bots): tools", "created": 1_789_348_652_037i64,
            "stages": [{ "number": 1, "name": "build", "status": "success",
                         "steps": [{ "number": 1, "name": "clone", "status": "success" }] }]
        }));
        let field = |name: &str| {
            run_field(
                "AgentNexus",
                &e,
                "http://h/ai/x/pipelines/AgentNexus/execution/451",
                name,
            )
        };
        assert_eq!(field("status"), "in_progress");
        assert_eq!(field("conclusion"), "");
        assert_eq!(field("event"), "workflow_dispatch");
        assert_eq!(field("attempt"), 2);
        assert_eq!(field("headBranch"), "develop");
        assert_eq!(field("workflowDatabaseId"), 24);
        assert_eq!(field("workflowName"), "AgentNexus");
        assert_eq!(field("jobs")[0]["conclusion"], "success");
        // fx's own names read from the envelope's shape.
        assert_eq!(field("branch"), "develop");
        assert_eq!(field("message"), "feat(bots): tools");
    }

    #[test]
    fn compact_keeps_only_what_is_not_green() {
        let e = execution(json!({
            "number": 1, "status": "failure",
            "stages": [
                { "number": 1, "name": "build", "status": "success",
                  "steps": [{ "number": 1, "name": "clone", "status": "success" }] },
                { "number": 2, "name": "test", "status": "failure",
                  "steps": [{ "number": 1, "name": "setup", "status": "success" },
                            { "number": 2, "name": "cargo test", "status": "failure" }] }
            ]
        }));
        let compact = compacted(e, true);
        assert_eq!(compact.stages.len(), 1);
        assert_eq!(compact.stages[0].steps.len(), 1);
        assert_eq!(compact.stages[0].steps[0].name, "cargo test");
    }
}
