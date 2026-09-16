//! `gf pipeline`, and gh's `gf run` / `gf workflow` over the same runs.

use clap::{Args, Subcommand};

use super::{FormatArgs, GhOnlyArgs};

// ---------------------------------------------------------------------------
// pipeline (v0.4)
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct PipelineCommand {
    #[command(subcommand)]
    pub command: PipelineSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum PipelineSubcommand {
    /// List pipeline runs
    #[command(visible_alias = "ls")]
    List(PipelineListArgs),
    /// Show a pipeline run
    View(PipelineViewArgs),
    /// Print the logs of a pipeline run
    #[command(long_about = "\
Print the logs of a pipeline run.

Logs are addressed per step, so this walks the run and fetches the steps you
asked for. Inside a checkout with a single pipeline, none of it needs naming:

  gf pipeline logs --failed      the failed steps of the most recent run
  gf pipeline logs 182 --failed  the failed steps of run 182
  gf pipeline logs --step test   steps whose name contains \"test\"

A failed build's log is mostly progress output, and the reason it failed is at
the end. --tail keeps that end; the response says how many lines there were in
total, so nothing is dropped silently:

  gf --agent pipeline logs --failed --tail 50")]
    Logs(PipelineLogsArgs),
    /// Trigger a pipeline run
    Run(PipelineRunArgs),
    /// Retry a pipeline run
    Retry(PipelineRefArgs),
}

#[derive(Debug, Args)]
pub struct PipelineListArgs {
    /// Only runs for this pipeline
    #[arg(short = 'w', long, value_name = "PIPELINE", visible_alias = "workflow")]
    pub pipeline: Option<String>,

    /// Maximum number of runs to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 20)]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct PipelineRefArgs {
    /// Run number; defaults to the most recent run
    #[arg(value_name = "RUN")]
    pub run: Option<u64>,

    /// Pipeline the run belongs to; inferred when the repository has only one
    #[arg(long, value_name = "PIPELINE")]
    pub pipeline: Option<String>,
}

#[derive(Debug, Args)]
pub struct PipelineLogsArgs {
    /// Run number; defaults to the most recent run
    #[arg(value_name = "RUN")]
    pub run: Option<u64>,

    /// Pipeline the run belongs to; inferred when the repository has only one
    #[arg(long, value_name = "PIPELINE")]
    pub pipeline: Option<String>,

    /// Only the steps that failed — the fastest path from a red build to a fix
    #[arg(long)]
    pub failed: bool,

    /// Only steps whose name contains this
    #[arg(long, value_name = "STEP")]
    pub step: Option<String>,

    /// Only the last N lines of each step — build failures are at the end
    #[arg(long, value_name = "N")]
    pub tail: Option<u32>,
}

#[derive(Debug, Args)]
pub struct PipelineRunArgs {
    /// Pipeline to run
    #[arg(value_name = "PIPELINE")]
    pub pipeline: String,

    /// Branch to run against
    #[arg(
        short,
        long,
        value_name = "BRANCH",
        short_alias = 'r',
        visible_alias = "ref"
    )]
    pub branch: Option<String>,
}

#[derive(Debug, Args)]
pub struct PipelineViewArgs {
    #[command(flatten)]
    pub target: PipelineRefArgs,

    /// Exit with status 1 when the run did not succeed
    #[arg(long)]
    pub exit_status: bool,
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct RunCommand {
    #[command(subcommand)]
    pub command: RunSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum RunSubcommand {
    /// List recent runs across the repository's pipelines
    #[command(visible_alias = "ls")]
    List(RunListArgs),
    /// Show a run, its stages and steps, and optionally its logs
    View(RunViewArgs),
    /// Run a finished run again
    Rerun(RunRerunArgs),
    /// Follow a run until it finishes
    Watch(RunWatchArgs),
    /// Cancel a run
    Cancel(RunCancelArgs),
    /// Delete a run
    Delete(RunRefArgs),
    #[command(hide = true, disable_help_flag = true)]
    Download(GhOnlyArgs),
}

/// A run. GitFox numbers runs per pipeline, so `RUN` is `number`,
/// `pipeline/number`, or `number` with `--workflow` when the repository has
/// several pipelines.
#[derive(Debug, Args)]
pub struct RunRefArgs {
    /// Run number, or pipeline/number; defaults to the most recent run
    #[arg(value_name = "RUN")]
    pub run: Option<String>,

    /// Pipeline the run belongs to; found automatically when it is unambiguous
    #[arg(long, value_name = "PIPELINE", visible_alias = "pipeline")]
    pub workflow: Option<String>,
}

#[derive(Debug, Args)]
pub struct RunListArgs {
    /// Only runs of this pipeline
    #[arg(short, long, value_name = "PIPELINE", visible_alias = "pipeline")]
    pub workflow: Option<String>,

    /// Only runs for this branch
    #[arg(short, long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Only runs in this state: queued, in_progress, completed, success,
    /// failure, cancelled, skipped — or GitFox's own status word
    #[arg(short, long, value_name = "STATUS")]
    pub status: Option<String>,

    /// Only runs triggered by this user
    #[arg(short, long, value_name = "LOGIN")]
    pub user: Option<String>,

    /// Only runs for this commit
    #[arg(short, long, value_name = "SHA")]
    pub commit: Option<String>,

    /// Only runs triggered by this event: push, pull_request, manual, cron, tag
    #[arg(short, long, value_name = "EVENT")]
    pub event: Option<String>,

    /// Only runs created on a date: 2026-09-14, >=2026-09-01, 2026-09-01..2026-09-14
    #[arg(long, value_name = "DATE")]
    pub created: Option<String>,

    /// Accepted for gh compatibility; runs of disabled pipelines are always listed
    #[arg(short, long)]
    pub all: bool,

    /// Maximum number of runs to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 20)]
    pub limit: u32,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct RunViewArgs {
    #[command(flatten)]
    pub target: RunRefArgs,

    /// Print every step's log
    #[arg(long, conflicts_with = "log_failed")]
    pub log: bool,

    /// Print the logs of the steps that failed
    #[arg(long)]
    pub log_failed: bool,

    /// Exit with status 1 when the run did not succeed
    #[arg(long)]
    pub exit_status: bool,

    /// Only this stage, by number or name
    #[arg(short, long, value_name = "STAGE")]
    pub job: Option<String>,

    /// Open the run in the browser
    #[arg(short, long)]
    pub web: bool,

    #[arg(short = 'a', long, value_name = "N", hide = true)]
    pub attempt: Option<u64>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct RunRerunArgs {
    #[command(flatten)]
    pub target: RunRefArgs,

    /// Rerun a failed run (GitFox reruns every stage)
    #[arg(long)]
    pub failed: bool,

    #[arg(short, long, value_name = "JOB", hide = true)]
    pub job: Option<String>,

    #[arg(short, long, hide = true)]
    pub debug: bool,
}

#[derive(Debug, Args)]
pub struct RunWatchArgs {
    #[command(flatten)]
    pub target: RunRefArgs,

    /// Exit with status 1 when the run does not succeed
    #[arg(long)]
    pub exit_status: bool,

    /// Refresh interval in seconds
    #[arg(short, long, value_name = "SECONDS", default_value_t = 3)]
    pub interval: u64,

    /// Show only the stages and steps that are running or failed
    #[arg(long)]
    pub compact: bool,
}

#[derive(Debug, Args)]
pub struct RunCancelArgs {
    #[command(flatten)]
    pub target: RunRefArgs,

    /// Accepted for gh compatibility; a GitFox cancel is already forceful
    #[arg(long)]
    pub force: bool,
}

// ---------------------------------------------------------------------------
// workflow
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct WorkflowCommand {
    #[command(subcommand)]
    pub command: WorkflowSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum WorkflowSubcommand {
    /// List the repository's pipelines
    #[command(visible_alias = "ls")]
    List(WorkflowListArgs),
    /// Show a pipeline and its recent runs
    View(WorkflowViewArgs),
    /// Start a run of a pipeline
    Run(WorkflowRunArgs),
    /// Enable a pipeline
    Enable(WorkflowRefArgs),
    /// Disable a pipeline
    Disable(WorkflowRefArgs),
}

#[derive(Debug, Args)]
pub struct WorkflowListArgs {
    /// Include disabled pipelines
    #[arg(short, long)]
    pub all: bool,

    /// Maximum number of pipelines to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 50)]
    pub limit: u32,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct WorkflowRefArgs {
    /// Pipeline identifier; inferred when the repository has only one
    #[arg(value_name = "PIPELINE")]
    pub workflow: Option<String>,
}

#[derive(Debug, Args)]
pub struct WorkflowViewArgs {
    #[command(flatten)]
    pub target: WorkflowRefArgs,

    /// With --yaml, the branch or tag to read the definition from
    #[arg(short = 'r', long = "ref", value_name = "REF")]
    pub git_ref: Option<String>,

    /// Open the pipeline in the browser
    #[arg(short, long)]
    pub web: bool,

    /// Print the pipeline's YAML definition
    #[arg(short, long)]
    pub yaml: bool,
}

#[derive(Debug, Args)]
pub struct WorkflowRunArgs {
    #[command(flatten)]
    pub target: WorkflowRefArgs,

    /// Branch or tag to run against
    #[arg(short = 'r', long = "ref", value_name = "REF")]
    pub git_ref: Option<String>,

    #[arg(short = 'f', long = "raw-field", value_name = "KEY=VALUE", hide = true)]
    pub raw_fields: Vec<String>,

    #[arg(short = 'F', long = "field", value_name = "KEY=VALUE", hide = true)]
    pub fields: Vec<String>,
}
