//! `fx pipeline` — runs, and the logs that explain them.
//!
//! The command this module exists for is `fx pipeline logs --failed`. GitFox
//! addresses logs per step, and only the single-execution endpoint returns the
//! stage tree, so answering "why is CI red" by hand means reading the run,
//! finding the steps that failed, and fetching each one. That is several
//! requests and a wall of output. Here it is one command and only the output
//! that matters.

use gitfox_client::{Execution, GitFoxClient, LogLine, Pipeline, RepoRef, Stage, Step};
use serde_json::{Value, json};

use crate::cli::{
    PipelineCommand, PipelineListArgs, PipelineLogsArgs, PipelineRefArgs, PipelineRunArgs,
    PipelineSubcommand,
};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::output::{Render, key_values, plain_table, relative_time};
use crate::paginate;

pub async fn run(cmd: PipelineCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        PipelineSubcommand::List(args) => list(args, ctx).await,
        PipelineSubcommand::View(args) => view(args, ctx).await,
        PipelineSubcommand::Logs(args) => logs(args, ctx).await,
        PipelineSubcommand::Run(args) => trigger(args, ctx).await,
        PipelineSubcommand::Retry(args) => retry(args, ctx).await,
    }
}

// ---------------------------------------------------------------------------
// resolution
// ---------------------------------------------------------------------------

/// The pipeline to act on: the one named, or the repository's only one.
///
/// Most repositories have exactly one pipeline, so naming it every time is
/// noise. When there are several, refusing with the list beats guessing.
async fn resolve_pipeline(
    explicit: Option<&str>,
    repo: &RepoRef,
    client: &GitFoxClient,
) -> Result<String> {
    if let Some(name) = explicit {
        return Ok(name.to_string());
    }
    let pipelines = list_pipelines(client, repo, false, 100).await?;
    match pipelines.as_slice() {
        [] => Err(CliError::new(
            ErrorCode::PipelineNotFound,
            format!("{repo} has no pipelines"),
        )),
        [only] => Ok(only.identifier.clone()),
        many => Err(CliError::invalid_argument(format!(
            "{repo} has {} pipelines; say which one",
            many.len()
        ))
        .with_hint(format!(
            "--pipeline {}",
            many.iter()
                .map(|p| p.identifier.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        ))),
    }
}

/// The run to act on: the number given, or the most recent one.
async fn resolve_run(
    explicit: Option<u64>,
    repo: &RepoRef,
    pipeline: &str,
    client: &GitFoxClient,
) -> Result<u64> {
    if let Some(number) = explicit {
        return Ok(number);
    }
    let latest = client
        .pipelines()
        .list_executions(repo, pipeline, 1, 1)
        .await
        .map_err(|e| not_found_as_pipeline(e, pipeline))?;
    latest.first().map(|e| e.number).ok_or_else(|| {
        CliError::new(
            ErrorCode::PipelineNotFound,
            format!("pipeline `{pipeline}` has never run"),
        )
        .with_hint("start one with `fx pipeline run`")
    })
}

async fn list_pipelines(
    client: &GitFoxClient,
    repo: &RepoRef,
    latest: bool,
    limit: u32,
) -> Result<Vec<Pipeline>> {
    client
        .pipelines()
        .list(repo, latest, 1, limit)
        .await
        .map_err(|e| match e {
            gitfox_client::Error::NotFound { .. } => CliError::new(
                ErrorCode::RepoNotFound,
                format!("no repository {repo}, or you cannot see it"),
            ),
            other => CliError::from(other),
        })
}

fn not_found_as_pipeline(err: gitfox_client::Error, pipeline: &str) -> CliError {
    match err {
        gitfox_client::Error::NotFound { .. } => CliError::new(
            ErrorCode::PipelineNotFound,
            format!("no pipeline `{pipeline}`"),
        ),
        other => CliError::from(other),
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(args: PipelineListArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;

    // Either way the output is a list of *runs* with the same shape, so nothing
    // downstream has to branch on which form was asked for.
    let (client_ref, repo_ref) = (&client, &repo);
    let (mut runs, truncated) = match args.pipeline.as_deref() {
        Some(pipeline) => {
            let paged = paginate::collect(args.limit, move |page, limit| async move {
                client_ref
                    .pipelines()
                    .list_executions(repo_ref, pipeline, page, limit)
                    .await
            })
            .await
            .map_err(|e| not_found_as_pipeline(e, pipeline))?;
            let runs = paged
                .items
                .into_iter()
                .map(|e| (pipeline.to_string(), e))
                .collect::<Vec<_>>();
            (runs, paged.truncated)
        }
        // `latest=true` embeds each pipeline's most recent run, so the whole
        // repository's CI state costs one request per page of pipelines rather
        // than one per pipeline. Paging is therefore over pipelines: a
        // pipeline that has never run contributes no row, so the run count can
        // be lower than the limit without anything being hidden.
        None => {
            let paged = paginate::collect(args.limit, move |page, limit| async move {
                client_ref
                    .pipelines()
                    .list(repo_ref, true, page, limit)
                    .await
            })
            .await
            .map_err(|e| match e {
                gitfox_client::Error::NotFound { .. } => CliError::new(
                    ErrorCode::RepoNotFound,
                    format!("no repository {repo_ref}, or you cannot see it"),
                ),
                other => CliError::from(other),
            })?;
            let runs = paged
                .items
                .into_iter()
                .filter_map(|p| p.execution.clone().map(|e| (p.identifier, e)))
                .collect::<Vec<_>>();
            (runs, paged.truncated)
        }
    };

    runs.sort_by_key(|(_, e)| std::cmp::Reverse(e.started.or(e.created).unwrap_or(0)));

    ctx.renderer
        .emit(&RunList {
            repo: repo.full(),
            runs,
            truncated,
        })
        .map_err(unexpected)
}

// ---------------------------------------------------------------------------
// view
// ---------------------------------------------------------------------------

async fn view(args: PipelineRefArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pipeline = resolve_pipeline(args.pipeline.as_deref(), &repo, &client).await?;
    let number = resolve_run(args.run, &repo, &pipeline, &client).await?;
    let execution = client
        .pipelines()
        .get_execution(&repo, &pipeline, number)
        .await
        .map_err(|e| not_found_as_run(e, &pipeline, number))?;

    ctx.renderer
        .emit(&RunView {
            pipeline,
            execution,
        })
        .map_err(unexpected)
}

fn not_found_as_run(err: gitfox_client::Error, pipeline: &str, number: u64) -> CliError {
    match err {
        gitfox_client::Error::NotFound { .. } => CliError::new(
            ErrorCode::PipelineNotFound,
            format!("no run #{number} of pipeline `{pipeline}`"),
        ),
        other => CliError::from(other),
    }
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

async fn logs(args: PipelineLogsArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pipeline = resolve_pipeline(args.pipeline.as_deref(), &repo, &client).await?;
    let number = resolve_run(args.run, &repo, &pipeline, &client).await?;

    // Only this endpoint returns the stage tree, and the tree is what says
    // which (stage, step) pairs have logs worth asking for.
    let execution = client
        .pipelines()
        .get_execution(&repo, &pipeline, number)
        .await
        .map_err(|e| not_found_as_run(e, &pipeline, number))?;

    let wanted = select_steps(&execution, &args);
    let mut steps = Vec::with_capacity(wanted.len());
    for (stage, step) in wanted {
        let lines = client
            .pipelines()
            .step_logs(&repo, &pipeline, number, stage.number, step.number)
            .await
            // A step can be listed before its log exists; an empty log is a
            // better answer than failing the whole command.
            .unwrap_or_default();
        steps.push(StepLogs {
            stage_name: stage.name.clone(),
            stage_number: stage.number,
            step_name: step.name.clone(),
            step_number: step.number,
            status: step.status.as_str().to_string(),
            exit_code: step.exit_code,
            error: step.error.clone().filter(|e| !e.is_empty()),
            total_lines: lines.len(),
            lines: tail(lines, args.tail),
        });
    }

    ctx.renderer
        .emit(&RunLogs {
            pipeline,
            run: number,
            status: execution.status.as_str().to_string(),
            only_failed: args.failed,
            steps,
        })
        .map_err(unexpected)
}

/// Keep only the last `n` lines, when asked.
///
/// A build's log is mostly progress output and the failure is at the end, so
/// this is usually what a caller wants — but the count of what was dropped
/// travels with it, because silently returning part of a log is how an agent
/// concludes the wrong thing.
fn tail(lines: Vec<LogLine>, n: Option<u32>) -> Vec<LogLine> {
    match n {
        Some(n) if lines.len() > n as usize => lines[lines.len() - n as usize..].to_vec(),
        _ => lines,
    }
}

/// Which steps to fetch.
///
/// `--failed` and `--step` compose: `--failed --step test` is the failed steps
/// whose name mentions "test". With neither, every step that actually ran.
fn select_steps<'a>(
    execution: &'a Execution,
    args: &PipelineLogsArgs,
) -> Vec<(&'a Stage, &'a Step)> {
    let needle = args.step.as_ref().map(|s| s.to_lowercase());
    execution
        .steps()
        .filter(|(_, step)| {
            if args.failed && !step.status.is_failed() {
                return false;
            }
            if !args.failed && !step.status.has_run() {
                return false;
            }
            match &needle {
                Some(needle) => step.name.to_lowercase().contains(needle),
                None => true,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// run / retry
// ---------------------------------------------------------------------------

async fn trigger(args: PipelineRunArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let execution = client
        .pipelines()
        .trigger(&repo, &args.pipeline, args.branch.as_deref())
        .await
        .map_err(|e| not_found_as_pipeline(e, &args.pipeline))?;

    ctx.renderer
        .emit(&RunStarted {
            pipeline: args.pipeline,
            execution,
            verb: "Started",
        })
        .map_err(unexpected)
}

async fn retry(args: PipelineRefArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let pipeline = resolve_pipeline(args.pipeline.as_deref(), &repo, &client).await?;
    let number = resolve_run(args.run, &repo, &pipeline, &client).await?;
    let execution = client
        .pipelines()
        .retry(&repo, &pipeline, number)
        .await
        .map_err(|e| not_found_as_run(e, &pipeline, number))?;

    ctx.renderer
        .emit(&RunStarted {
            pipeline,
            execution,
            verb: "Retried",
        })
        .map_err(unexpected)
}

fn unexpected(err: std::io::Error) -> CliError {
    CliError::new(ErrorCode::Unexpected, err.to_string())
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

fn status_mark(status: &str) -> &'static str {
    match status {
        "success" => "✓",
        "failure" | "error" | "killed" => "✗",
        "running" => "●",
        "skipped" | "declined" => "-",
        _ => "·",
    }
}

fn status_colour(status: &str) -> &'static str {
    match status {
        "success" => "\x1b[32m",
        "failure" | "error" | "killed" => "\x1b[31m",
        "running" | "pending" | "blocked" | "waiting_on_dependencies" => "\x1b[33m",
        _ => "\x1b[2m",
    }
}

fn paint(status: &str, color: bool) -> String {
    if color {
        format!("{}{status}\x1b[0m", status_colour(status))
    } else {
        status.to_string()
    }
}

fn execution_json(pipeline: &str, execution: &Execution) -> Value {
    json!({
        "pipeline": pipeline,
        "number": execution.number,
        "status": execution.status.as_str(),
        "branch": execution.branch(),
        "message": execution.summary(),
        "author": execution.author(),
        "event": execution.event,
        "commit": execution.after,
        "created": execution.created,
        "started": execution.started,
        "finished": execution.finished,
        "error": execution.error,
        "link": execution.link,
    })
}

struct RunList {
    repo: String,
    runs: Vec<(String, Execution)>,
    truncated: bool,
}

impl Render for RunList {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.repo,
            "count": self.runs.len(),
            "truncated": self.truncated,
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.runs
            .iter()
            .map(|(pipeline, execution)| execution_json(pipeline, execution))
            .collect()
    }

    fn to_human(&self, color: bool) -> String {
        if self.runs.is_empty() {
            return format!("No pipeline runs in {}", self.repo);
        }
        let rows: Vec<Vec<String>> = self
            .runs
            .iter()
            .map(|(pipeline, e)| {
                let status = e.status.as_str();
                vec![
                    format!("#{}", e.number),
                    pipeline.clone(),
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
        let mut out = plain_table(
            &["run", "pipeline", "status", "branch", "message", "started"],
            &rows,
        );
        if self.truncated {
            out.push_str(&format!(
                "\n\nShowing {} of more; raise --limit to see the rest.",
                self.runs.len()
            ));
        }
        out
    }
}

struct RunView {
    pipeline: String,
    execution: Execution,
}

impl Render for RunView {
    fn to_json(&self) -> Value {
        let mut value = execution_json(&self.pipeline, &self.execution);
        value["stages"] = json!(
            self.execution
                .stages
                .iter()
                .map(|stage| json!({
                    "number": stage.number,
                    "name": stage.name,
                    "status": stage.status.as_str(),
                    "error": stage.error,
                    "steps": stage.steps.iter().map(|step| json!({
                        "number": step.number,
                        "name": step.name,
                        "status": step.status.as_str(),
                        "exit_code": step.exit_code,
                        "error": step.error,
                    })).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>()
        );
        value
    }

    fn to_human(&self, color: bool) -> String {
        let e = &self.execution;
        let (bold, reset) = if color {
            ("\x1b[1m", "\x1b[0m")
        } else {
            ("", "")
        };
        let status = e.status.as_str();

        let mut pairs = vec![
            ("Pipeline", self.pipeline.clone()),
            (
                "Status",
                format!("{} {}", status_mark(status), paint(status, color)),
            ),
        ];
        if let Some(branch) = e.branch() {
            pairs.push(("Branch", branch));
        }
        if let Some(event) = &e.event {
            pairs.push(("Event", event.clone()));
        }
        let author = e.author();
        if !author.is_empty() {
            pairs.push(("Author", author));
        }
        if let Some(started) = e.started.or(e.created) {
            pairs.push(("Started", relative_time(started)));
        }
        if let Some(error) = &e.error.clone().filter(|s| !s.is_empty()) {
            pairs.push(("Error", error.clone()));
        }
        if let Some(link) = &e.link {
            pairs.push(("Web", link.clone()));
        }

        let mut out = format!(
            "{bold}#{} {}{reset}\n{}",
            e.number,
            e.summary(),
            key_values(&pairs)
        );
        for stage in &e.stages {
            let s = stage.status.as_str();
            out.push_str(&format!(
                "\n\n{} {} ({})",
                status_mark(s),
                stage.name,
                paint(s, color)
            ));
            for step in &stage.steps {
                let s = step.status.as_str();
                let exit = match step.exit_code {
                    Some(code) if code != 0 => format!(" exit {code}"),
                    _ => String::new(),
                };
                out.push_str(&format!(
                    "\n    {} {} ({}{exit})",
                    status_mark(s),
                    step.name,
                    paint(s, color)
                ));
            }
        }
        out
    }
}

struct StepLogs {
    stage_name: String,
    stage_number: i64,
    step_name: String,
    step_number: i64,
    status: String,
    exit_code: Option<i64>,
    error: Option<String>,
    /// How many lines the step produced, before `--tail` trimmed anything.
    total_lines: usize,
    lines: Vec<LogLine>,
}

impl StepLogs {
    fn text(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|line| line.out.trim_end_matches(['\n', '\r']).to_string())
            .collect()
    }

    fn to_json(&self) -> Value {
        json!({
            "stage": self.stage_name,
            "stage_number": self.stage_number,
            "step": self.step_name,
            "step_number": self.step_number,
            "status": self.status,
            "exit_code": self.exit_code,
            "error": self.error,
            "total_lines": self.total_lines,
            "lines": self.text(),
        })
    }
}

struct RunLogs {
    pipeline: String,
    run: u64,
    status: String,
    only_failed: bool,
    steps: Vec<StepLogs>,
}

impl Render for RunLogs {
    fn to_json(&self) -> Value {
        json!({
            "pipeline": self.pipeline,
            "run": self.run,
            "status": self.status,
            "only_failed": self.only_failed,
            "count": self.steps.len(),
            "steps": self.steps.iter().map(StepLogs::to_json).collect::<Vec<_>>(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.steps.iter().map(StepLogs::to_json).collect()
    }

    fn to_human(&self, color: bool) -> String {
        if self.steps.is_empty() {
            return if self.only_failed {
                format!("No failed steps in run #{} ({})", self.run, self.status)
            } else {
                format!("No step output for run #{}", self.run)
            };
        }
        let (bold, dim, reset) = if color {
            ("\x1b[1m", "\x1b[2m", "\x1b[0m")
        } else {
            ("", "", "")
        };
        self.steps
            .iter()
            .map(|step| {
                let exit = match step.exit_code {
                    Some(code) if code != 0 => format!(" exit {code}"),
                    _ => String::new(),
                };
                let mut block = format!(
                    "{bold}{} {} / {}{reset} ({}{exit})",
                    status_mark(&step.status),
                    step.stage_name,
                    step.step_name,
                    paint(&step.status, color),
                );
                if let Some(error) = &step.error {
                    block.push_str(&format!("\n{dim}  {error}{reset}"));
                }
                let text = step.text();
                let omitted = step.total_lines.saturating_sub(text.len());
                if omitted > 0 {
                    block.push_str(&format!(
                        "\n{dim}  … {omitted} earlier lines omitted{reset}"
                    ));
                }
                if text.is_empty() {
                    block.push_str(&format!("\n{dim}  (no output){reset}"));
                } else {
                    for line in text {
                        block.push_str(&format!("\n  {line}"));
                    }
                }
                block
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

struct RunStarted {
    pipeline: String,
    execution: Execution,
    verb: &'static str,
}

impl Render for RunStarted {
    fn to_json(&self) -> Value {
        execution_json(&self.pipeline, &self.execution)
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let e = &self.execution;
        let mut out = format!(
            "{green}✓{reset} {} run #{} of {}",
            self.verb, e.number, self.pipeline
        );
        if let Some(branch) = e.branch() {
            out.push_str(&format!(" on {branch}"));
        }
        if let Some(link) = &e.link {
            out.push_str(&format!("\n  {link}"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution() -> Execution {
        serde_json::from_value(json!({
            "number": 182,
            "status": "failure",
            "message": "feat: add OAuth\n\nbody",
            "target": "main",
            "author_login": "whw",
            "event": "push",
            "started": 1_756_000_000_000i64,
            "stages": [{
                "number": 1, "name": "build", "status": "failure",
                "steps": [
                    { "number": 1, "name": "clone", "status": "success" },
                    { "number": 2, "name": "cargo test", "status": "failure", "exit_code": 101 },
                    { "number": 3, "name": "cargo clippy", "status": "skipped" }
                ]
            }, {
                "number": 2, "name": "deploy", "status": "skipped",
                "steps": [{ "number": 1, "name": "ship", "status": "skipped" }]
            }]
        }))
        .unwrap()
    }

    fn args(failed: bool, step: Option<&str>) -> PipelineLogsArgs {
        PipelineLogsArgs {
            run: None,
            pipeline: None,
            failed,
            step: step.map(str::to_string),
            tail: None,
        }
    }

    #[test]
    fn failed_selects_only_steps_that_actually_failed() {
        let e = execution();
        let picked = select_steps(&e, &args(true, None));
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].0.name, "build");
        assert_eq!(picked[0].1.name, "cargo test");
    }

    #[test]
    fn without_failed_the_steps_that_never_ran_are_still_left_out() {
        let e = execution();
        let picked = select_steps(&e, &args(false, None));
        let names: Vec<_> = picked.iter().map(|(_, s)| s.name.as_str()).collect();
        // clone and cargo test ran; cargo clippy and ship were skipped.
        assert_eq!(names, vec!["clone", "cargo test"]);
    }

    #[test]
    fn step_filter_is_a_case_insensitive_substring_and_composes_with_failed() {
        let e = execution();
        let by_name = select_steps(&e, &args(false, Some("CARGO")));
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].1.name, "cargo test");

        let both = select_steps(&e, &args(true, Some("clone")));
        assert!(both.is_empty(), "clone succeeded, so --failed excludes it");
    }

    #[test]
    fn logs_json_carries_the_run_status_and_the_lines() {
        let logs = RunLogs {
            pipeline: "default".into(),
            run: 182,
            status: "failure".into(),
            only_failed: true,
            steps: vec![StepLogs {
                stage_name: "build".into(),
                stage_number: 1,
                step_name: "cargo test".into(),
                step_number: 2,
                status: "failure".into(),
                exit_code: Some(101),
                error: None,
                total_lines: 2,
                lines: vec![
                    LogLine {
                        pos: 0,
                        out: "error[E0308]\n".into(),
                        time: 1,
                    },
                    LogLine {
                        pos: 1,
                        out: "aborting".into(),
                        time: 2,
                    },
                ],
            }],
        };
        let value = logs.to_json();
        assert_eq!(value["run"], 182);
        assert_eq!(value["count"], 1);
        assert_eq!(value["steps"][0]["exit_code"], 101);
        // Trailing newlines are stripped so a consumer gets clean lines.
        assert_eq!(value["steps"][0]["lines"][0], "error[E0308]");
        assert_eq!(value["steps"][0]["lines"][1], "aborting");
        assert_eq!(logs.to_jsonl().len(), 1);
    }

    fn line(text: &str) -> LogLine {
        LogLine {
            pos: 0,
            out: text.to_string(),
            time: 0,
        }
    }

    #[test]
    fn tail_keeps_the_end_where_the_failure_is() {
        let lines: Vec<LogLine> = (1..=100).map(|i| line(&format!("line {i}"))).collect();
        let kept = tail(lines.clone(), Some(3));
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0].out, "line 98");
        assert_eq!(kept[2].out, "line 100");
        // Asking for more than there is keeps everything.
        assert_eq!(tail(lines.clone(), Some(500)).len(), 100);
        assert_eq!(tail(lines, None).len(), 100);
    }

    #[test]
    fn a_tailed_step_says_how_much_it_dropped() {
        let logs = RunLogs {
            pipeline: "default".into(),
            run: 182,
            status: "failure".into(),
            only_failed: true,
            steps: vec![StepLogs {
                stage_name: "build".into(),
                stage_number: 1,
                step_name: "mvn deploy".into(),
                step_number: 2,
                status: "failure".into(),
                exit_code: Some(1),
                error: None,
                total_lines: 1658,
                lines: vec![line("BUILD FAILURE")],
            }],
        };
        let value = logs.to_json();
        // Both numbers travel, so truncation is never silent.
        assert_eq!(value["steps"][0]["total_lines"], 1658);
        assert_eq!(value["steps"][0]["lines"].as_array().unwrap().len(), 1);
        let text = logs.to_human(false);
        assert!(text.contains("1657 earlier lines omitted"), "{text}");
        assert!(text.contains("BUILD FAILURE"), "{text}");
    }

    #[test]
    fn nothing_failed_is_a_sentence_not_an_empty_screen() {
        let logs = RunLogs {
            pipeline: "default".into(),
            run: 182,
            status: "success".into(),
            only_failed: true,
            steps: vec![],
        };
        let text = logs.to_human(false);
        assert!(text.contains("No failed steps"), "{text}");
        assert!(text.contains("success"), "{text}");
        assert_eq!(logs.to_json()["count"], 0);
    }

    #[test]
    fn the_run_table_shows_status_branch_and_message() {
        let list = RunList {
            repo: "ai/backend".into(),
            runs: vec![("default".into(), execution())],
            truncated: false,
        };
        let text = list.to_human(false);
        assert!(text.contains("#182"), "{text}");
        assert!(text.contains("failure"), "{text}");
        assert!(text.contains("main"), "{text}");
        assert!(text.contains("feat: add OAuth"), "{text}");
        assert!(!text.contains('\x1b'));

        let value = list.to_json();
        assert_eq!(value["count"], 1);
        assert_eq!(value["items"][0]["pipeline"], "default");
        assert_eq!(value["items"][0]["branch"], "main");
    }

    #[test]
    fn the_run_view_includes_the_stage_tree() {
        let view = RunView {
            pipeline: "default".into(),
            execution: execution(),
        };
        let value = view.to_json();
        assert_eq!(value["stages"][0]["name"], "build");
        assert_eq!(value["stages"][0]["steps"][1]["exit_code"], 101);
        let text = view.to_human(false);
        assert!(text.contains("cargo test"), "{text}");
        assert!(text.contains("exit 101"), "{text}");
    }
}
