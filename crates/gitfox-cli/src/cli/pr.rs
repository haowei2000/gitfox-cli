//! `fx pr`.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::{FormatArgs, GhOnlyArgs};

#[derive(Debug, Args)]
pub struct PrCommand {
    #[command(subcommand)]
    pub command: PrSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum PrSubcommand {
    /// List pull requests
    #[command(visible_alias = "ls")]
    List(PrListArgs),
    /// Show a pull request
    View(PrViewArgs),
    /// Open a pull request
    Create(PrCreateArgs),
    /// Merge a pull request
    Merge(PrMergeArgs),
    /// Check out a pull request branch locally
    Checkout(PrCheckoutArgs),
    /// Show a pull request's diff
    Diff(PrDiffArgs),
    /// Show the status of a pull request's checks
    #[command(long_about = "\
Show the status of a pull request's checks.

Like gh, the exit code says how they stand when a person is reading: 0 when
every check passed, 1 (CHECKS_FAILED) when one failed, 8 (CHECKS_PENDING) while
some are still running. With --json the result is data and the exit code is 0.")]
    Checks(PrChecksArgs),
    /// Close a pull request
    Close(PrCloseArgs),
    /// Reopen a closed pull request
    Reopen(PrReopenArgs),
    /// Mark a pull request as ready for review, or back to draft with --undo
    Ready(PrReadyArgs),
    /// Edit a pull request's title, body, reviewers and labels
    Edit(PrEditArgs),
    /// Comment on a pull request
    Comment(PrCommentArgs),
    /// Approve, request changes on, or comment on a pull request
    Review(PrReviewArgs),
    /// Show the pull requests that concern you in this repository
    Status(PrStatusArgs),
    /// Rebase a pull request's branch onto its base
    #[command(name = "update-branch")]
    UpdateBranch(PrUpdateBranchArgs),
    #[command(hide = true, disable_help_flag = true)]
    Lock(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Unlock(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Revert(GhOnlyArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum PrState {
    Open,
    Closed,
    Merged,
    All,
}

impl PrState {
    /// `all` is a CLI convenience: the API takes a repeatable `state` filter,
    /// so it expands rather than being sent as a value.
    pub fn expand(self) -> Vec<gitfox_client::PullRequestState> {
        use gitfox_client::PullRequestState as S;
        match self {
            Self::Open => vec![S::Open],
            Self::Closed => vec![S::Closed],
            Self::Merged => vec![S::Merged],
            Self::All => vec![S::Open, S::Closed, S::Merged],
        }
    }
}

#[derive(Debug, Args)]
pub struct PrListArgs {
    /// Filter by state
    #[arg(short, long, value_name = "STATE", default_value = "open")]
    pub state: PrState,

    /// Maximum number of pull requests to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 30)]
    pub limit: u32,

    /// Only pull requests opened by this user (a login or a principal id)
    #[arg(short = 'A', long, value_name = "USER")]
    pub author: Option<String>,

    /// Filter by base branch
    #[arg(short = 'B', long, value_name = "BRANCH")]
    pub base: Option<String>,

    /// Filter by head branch
    #[arg(short = 'H', long, value_name = "BRANCH")]
    pub head: Option<String>,

    /// Filter by label: `key`, or `key:value` for a label with values
    #[arg(
        short = 'l',
        long = "label",
        value_name = "LABEL",
        value_delimiter = ','
    )]
    pub labels: Vec<String>,

    /// Search pull request titles
    #[arg(short = 'S', long, value_name = "QUERY")]
    pub search: Option<String>,

    /// Only drafts
    #[arg(short, long)]
    pub draft: bool,

    /// Open the list in the browser
    #[arg(short, long)]
    pub web: bool,

    #[arg(short = 'a', long, value_name = "USER", hide = true)]
    pub assignee: Option<String>,

    #[arg(long, value_name = "APP", hide = true)]
    pub app: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct PrSelectorArgs {
    /// Pull request number, URL or branch; defaults to the current branch's
    #[arg(value_name = "NUMBER | URL | BRANCH")]
    pub selector: Option<String>,
}

#[derive(Debug, Args)]
pub struct PrViewArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// Include the pull request's comments
    #[arg(short, long)]
    pub comments: bool,

    /// Open the pull request in the browser
    #[arg(short, long)]
    pub web: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct PrCreateArgs {
    /// Branch to merge into
    #[arg(short = 'B', long, value_name = "BRANCH")]
    pub base: Option<String>,

    /// Branch to merge from; defaults to the current branch
    #[arg(short = 'H', long, value_name = "BRANCH")]
    pub head: Option<String>,

    /// Pull request title
    #[arg(short, long)]
    pub title: Option<String>,

    /// Pull request description
    #[arg(short, long, conflicts_with = "body_file")]
    pub body: Option<String>,

    /// Read the description from a file, or `-` for stdin
    #[arg(short = 'F', long, value_name = "FILE")]
    pub body_file: Option<String>,

    /// Take the title and body from the branch's commits
    #[arg(short, long)]
    pub fill: bool,

    /// Take the title and body from the first commit only
    #[arg(long, conflicts_with_all = ["fill", "fill_verbose"])]
    pub fill_first: bool,

    /// Take the title from the first commit and the body from every commit's
    /// full message
    #[arg(long, conflicts_with = "fill")]
    pub fill_verbose: bool,

    /// Open as a draft
    #[arg(short, long)]
    pub draft: bool,

    /// Request reviews from these users
    #[arg(short, long = "reviewer", value_name = "LOGIN", value_delimiter = ',')]
    pub reviewers: Vec<String>,

    /// Add labels: `key`, or `key:value`
    #[arg(short, long = "label", value_name = "LABEL", value_delimiter = ',')]
    pub labels: Vec<String>,

    /// Open the browser to create the pull request there
    #[arg(short, long)]
    pub web: bool,

    /// Write the title and body in your editor
    #[arg(short, long)]
    pub editor: bool,

    /// Start the body from this file
    #[arg(short = 'T', long, value_name = "FILE")]
    pub template: Option<String>,

    /// Show what would be created without creating it
    #[arg(long)]
    pub dry_run: bool,

    /// Accepted for gh compatibility; GitFox has no maintainer edits to disable
    #[arg(long, hide = true)]
    pub no_maintainer_edit: bool,

    #[arg(short = 'a', long, value_name = "LOGIN", hide = true)]
    pub assignee: Vec<String>,

    #[arg(short = 'm', long, value_name = "NAME", hide = true)]
    pub milestone: Option<String>,

    #[arg(short = 'p', long, value_name = "TITLE", hide = true)]
    pub project: Vec<String>,

    #[arg(long, value_name = "FILE", hide = true)]
    pub recover: Option<String>,

    #[arg(long, value_name = "FILE", hide = true)]
    pub attach: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
    FastForward,
}

impl From<MergeMethod> for gitfox_client::MergeMethod {
    fn from(value: MergeMethod) -> Self {
        match value {
            MergeMethod::Merge => Self::Merge,
            MergeMethod::Squash => Self::Squash,
            MergeMethod::Rebase => Self::Rebase,
            MergeMethod::FastForward => Self::FastForward,
        }
    }
}

#[derive(Debug, Args)]
pub struct PrMergeArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// Merge the commits with the base branch (the default)
    #[arg(short = 'm', long = "merge", conflicts_with_all = ["squash", "rebase", "method"])]
    pub merge: bool,

    /// Squash the commits into one and merge it
    #[arg(short, long, conflicts_with_all = ["rebase", "method"])]
    pub squash: bool,

    /// Rebase the commits onto the base branch
    #[arg(short, long, conflicts_with = "method")]
    pub rebase: bool,

    /// Merge strategy by name, including fast-forward
    #[arg(long, value_name = "METHOD")]
    pub method: Option<MergeMethod>,

    /// Delete the source branch after merging
    #[arg(short = 'd', short_alias = 'D', long)]
    pub delete_branch: bool,

    /// Report whether the merge would succeed, without performing it
    #[arg(long)]
    pub dry_run: bool,

    /// Merge even when protection rules are not satisfied
    #[arg(long)]
    pub admin: bool,

    /// Subject of the merge commit
    #[arg(short = 't', long, value_name = "TEXT")]
    pub subject: Option<String>,

    /// Body of the merge commit
    #[arg(short, long, value_name = "TEXT", conflicts_with = "body_file")]
    pub body: Option<String>,

    /// Read the merge commit body from a file, or `-` for stdin
    #[arg(short = 'F', long, value_name = "FILE")]
    pub body_file: Option<String>,

    /// Refuse to merge unless the head commit is this one
    #[arg(long, value_name = "SHA")]
    pub match_head_commit: Option<String>,

    #[arg(long, hide = true)]
    pub auto: bool,

    #[arg(long, hide = true)]
    pub disable_auto: bool,

    #[arg(short = 'A', long, value_name = "TEXT", hide = true)]
    pub author_email: Option<String>,
}

impl PrMergeArgs {
    /// The strategy the flags ask for; `merge` when none does.
    pub fn strategy(&self) -> MergeMethod {
        if let Some(method) = self.method {
            method
        } else if self.squash {
            MergeMethod::Squash
        } else if self.rebase {
            MergeMethod::Rebase
        } else {
            MergeMethod::Merge
        }
    }
}

#[derive(Debug, Args)]
pub struct PrCheckoutArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// Local branch name to use; defaults to the pull request's head branch
    #[arg(short, long, value_name = "NAME")]
    pub branch: Option<String>,

    /// Check out the pull request with a detached HEAD
    #[arg(long, conflicts_with_all = ["branch", "worktree"])]
    pub detach: bool,

    /// Reset an existing local branch to the pull request's head
    #[arg(short, long)]
    pub force: bool,

    /// Update all submodules after checkout
    #[arg(long)]
    pub recurse_submodules: bool,

    /// Check the pull request out into a new worktree at this path
    #[arg(long, value_name = "PATH")]
    pub worktree: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum ColorWhen {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Args)]
pub struct PrDiffArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// List the changed files without their patches
    #[arg(long)]
    pub name_only: bool,

    /// Colour the diff: auto, always or never
    #[arg(long, value_name = "WHEN", default_value = "auto")]
    pub color: ColorWhen,

    /// Show the pull request as a series of patches, one per commit
    #[arg(long, conflicts_with = "name_only")]
    pub patch: bool,

    /// Leave out files matching these glob patterns
    #[arg(short, long = "exclude", value_name = "GLOB", value_delimiter = ',')]
    pub exclude: Vec<String>,

    /// Open the pull request's changes in the browser
    #[arg(short, long)]
    pub web: bool,
}

#[derive(Debug, Args)]
pub struct PrChecksArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// Watch the checks until they finish
    #[arg(long)]
    pub watch: bool,

    /// Stop watching at the first failing check
    #[arg(long)]
    pub fail_fast: bool,

    /// Refresh interval in seconds while watching (default 10)
    #[arg(short, long, value_name = "SECONDS")]
    pub interval: Option<u64>,

    /// Only the checks a merge requires
    #[arg(long)]
    pub required: bool,

    /// Open the pull request's checks in the browser
    #[arg(short, long)]
    pub web: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct PrCloseArgs {
    /// Pull request number, URL or branch
    #[arg(value_name = "NUMBER | URL | BRANCH")]
    pub selector: String,

    /// Leave a closing comment
    #[arg(short, long, value_name = "TEXT")]
    pub comment: Option<String>,

    /// Delete the source branch after closing
    #[arg(short, long)]
    pub delete_branch: bool,
}

#[derive(Debug, Args)]
pub struct PrReopenArgs {
    /// Pull request number, URL or branch
    #[arg(value_name = "NUMBER | URL | BRANCH")]
    pub selector: String,

    /// Leave a comment when reopening
    #[arg(short, long, value_name = "TEXT")]
    pub comment: Option<String>,
}

#[derive(Debug, Args)]
pub struct PrReadyArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// Convert the pull request back to a draft
    #[arg(long)]
    pub undo: bool,
}

#[derive(Debug, Args)]
pub struct PrEditArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// New title
    #[arg(short, long)]
    pub title: Option<String>,

    /// New body
    #[arg(short, long, conflicts_with = "body_file")]
    pub body: Option<String>,

    /// Read the new body from a file, or `-` for stdin
    #[arg(short = 'F', long, value_name = "FILE")]
    pub body_file: Option<String>,

    /// Request reviews from these users
    #[arg(long, value_name = "LOGIN", value_delimiter = ',')]
    pub add_reviewer: Vec<String>,

    /// Remove these reviewers
    #[arg(long, value_name = "LOGIN", value_delimiter = ',')]
    pub remove_reviewer: Vec<String>,

    /// Add labels: `key`, or `key:value`
    #[arg(long, value_name = "LABEL", value_delimiter = ',')]
    pub add_label: Vec<String>,

    /// Remove labels
    #[arg(long, value_name = "LABEL", value_delimiter = ',')]
    pub remove_label: Vec<String>,

    #[arg(short = 'B', long, value_name = "BRANCH", hide = true)]
    pub base: Option<String>,

    #[arg(long, value_name = "LOGIN", hide = true)]
    pub add_assignee: Vec<String>,

    #[arg(long, value_name = "LOGIN", hide = true)]
    pub remove_assignee: Vec<String>,

    #[arg(short = 'm', long, value_name = "NAME", hide = true)]
    pub milestone: Option<String>,

    #[arg(long, hide = true)]
    pub remove_milestone: bool,

    #[arg(long, value_name = "TITLE", hide = true)]
    pub add_project: Vec<String>,

    #[arg(long, value_name = "TITLE", hide = true)]
    pub remove_project: Vec<String>,

    #[arg(long, value_name = "FILE", hide = true)]
    pub attach: Vec<String>,
}

#[derive(Debug, Args)]
pub struct PrCommentArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// The comment text
    #[arg(short, long, value_name = "TEXT", conflicts_with = "body_file")]
    pub body: Option<String>,

    /// Read the comment from a file, or `-` for stdin
    #[arg(short = 'F', long, value_name = "FILE")]
    pub body_file: Option<String>,

    /// Write the comment in your editor
    #[arg(short, long)]
    pub editor: bool,

    /// Open the pull request in the browser to comment there
    #[arg(short, long)]
    pub web: bool,

    /// Edit your most recent comment instead of adding one
    #[arg(long, conflicts_with = "delete_last")]
    pub edit_last: bool,

    /// Delete your most recent comment
    #[arg(long)]
    pub delete_last: bool,

    /// With --edit-last, add a comment when you have none to edit
    #[arg(long, requires = "edit_last")]
    pub create_if_none: bool,

    /// Skip the confirmation for --delete-last
    #[arg(long)]
    pub yes: bool,

    #[arg(long, value_name = "FILE", hide = true)]
    pub attach: Vec<String>,
}

#[derive(Debug, Args)]
pub struct PrReviewArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// Approve the pull request
    #[arg(short, long, conflicts_with_all = ["request_changes", "comment"])]
    pub approve: bool,

    /// Request changes
    #[arg(short, long, conflicts_with = "comment")]
    pub request_changes: bool,

    /// Comment without approving or requesting changes
    #[arg(short, long)]
    pub comment: bool,

    /// The review's text, posted as a comment
    #[arg(short, long, value_name = "TEXT", conflicts_with = "body_file")]
    pub body: Option<String>,

    /// Read the review's text from a file, or `-` for stdin
    #[arg(short = 'F', long, value_name = "FILE")]
    pub body_file: Option<String>,
}

#[derive(Debug, Args)]
pub struct PrStatusArgs {
    /// Show whether each pull request can merge cleanly
    #[arg(short, long)]
    pub conflict_status: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct PrUpdateBranchArgs {
    #[command(flatten)]
    pub target: PrSelectorArgs,

    /// Rebase the head branch onto the latest base branch
    #[arg(long)]
    pub rebase: bool,
}
