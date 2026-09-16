//! `gf repo`.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::{FormatArgs, GhOnlyArgs};

#[derive(Debug, Args)]
pub struct RepoCommand {
    #[command(subcommand)]
    pub command: RepoSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum RepoSubcommand {
    /// List repositories in a space
    #[command(visible_alias = "ls")]
    List(RepoListArgs),
    /// Show a single repository
    View(RepoViewArgs),
    /// Clone a repository
    Clone(RepoCloneArgs),
    /// Create a repository
    // `-h` is gh's --homepage here, so --help is long-only.
    #[command(disable_help_flag = true)]
    Create(Box<RepoCreateArgs>),
    /// Delete a repository
    Delete(RepoDeleteArgs),
    /// Edit a repository's settings
    #[command(disable_help_flag = true)]
    Edit(Box<RepoEditArgs>),
    /// Rename a repository
    Rename(RepoRenameArgs),
    /// Fork a repository
    Fork(RepoForkArgs),
    /// Sync a repository: a fork or mirror on the server, or your checkout's
    /// branch from its remote
    Sync(RepoSyncArgs),
    /// Set the repository gf uses in this checkout
    #[command(name = "set-default")]
    SetDefault(RepoSetDefaultArgs),
    /// Print a file from a repository
    #[command(name = "read-file")]
    ReadFile(RepoReadFileArgs),
    /// List a directory in a repository
    #[command(name = "read-dir")]
    ReadDir(RepoReadDirArgs),
    /// List the gitignore templates GitFox offers
    Gitignore(RepoGitignoreCommand),
    /// List the licenses GitFox offers
    License(RepoLicenseCommand),
    #[command(hide = true, disable_help_flag = true)]
    Archive(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Unarchive(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true, name = "deploy-key")]
    DeployKey(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Autolink(GhOnlyArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum RepoSort {
    Name,
    Created,
    Updated,
}

impl From<RepoSort> for gitfox_client::RepoSort {
    fn from(value: RepoSort) -> Self {
        match value {
            RepoSort::Name => Self::Name,
            RepoSort::Created => Self::Created,
            RepoSort::Updated => Self::Updated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum Visibility {
    Public,
    Private,
    Internal,
}

#[derive(Debug, Args)]
pub struct RepoListArgs {
    /// Space to list; defaults to --org, then the current repository's space,
    /// then every space you can see
    #[arg(value_name = "SPACE")]
    pub space: Option<String>,

    /// Only repositories whose name matches
    #[arg(short = 'S', long, value_name = "TEXT")]
    pub search: Option<String>,

    /// Sort order
    #[arg(long, value_name = "KEY", default_value = "name")]
    pub sort: RepoSort,

    /// Maximum number of repositories to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 30)]
    pub limit: u32,

    /// Only repositories with this visibility
    #[arg(long, value_name = "VISIBILITY")]
    pub visibility: Option<Visibility>,

    /// Only forks
    #[arg(long, conflicts_with = "source")]
    pub fork: bool,

    /// Only repositories that are not forks
    #[arg(long)]
    pub source: bool,

    /// Accepted for gh compatibility: GitFox repositories are never archived
    #[arg(long, hide = true)]
    pub no_archived: bool,

    #[arg(long, hide = true, conflicts_with = "no_archived")]
    pub archived: bool,

    #[arg(short = 'l', long, value_name = "LANGUAGE", hide = true)]
    pub language: Option<String>,

    #[arg(long, value_name = "TOPIC", hide = true)]
    pub topic: Vec<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct RepoViewArgs {
    /// Repository to show; defaults to the current checkout
    #[arg(value_name = "SPACE/NAME")]
    pub repository: Option<String>,

    /// Show the README from this branch
    #[arg(short, long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Open the repository in the browser
    #[arg(short, long)]
    pub web: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct RepoCloneArgs {
    /// Repository to clone
    #[arg(value_name = "SPACE/NAME")]
    pub repository: String,

    /// Directory to clone into
    #[arg(value_name = "DIRECTORY")]
    pub directory: Option<PathBuf>,

    /// Extra flags for `git clone`, after `--`
    #[arg(last = true, value_name = "GITFLAGS")]
    pub git_flags: Vec<String>,

    /// Clone over SSH instead of HTTP (default: the git_protocol setting)
    #[arg(long)]
    pub ssh: bool,

    /// Name of the remote for a fork's parent
    #[arg(short = 'u', long, value_name = "NAME", default_value = "upstream")]
    pub upstream_remote_name: String,

    /// Do not add a remote for a fork's parent
    #[arg(long)]
    pub no_upstream: bool,
}

#[derive(Debug, Args)]
pub struct RepoCreateArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Name, as space/name or just name inside --org / the current space
    #[arg(value_name = "NAME")]
    pub name: Option<String>,

    /// Description of the repository
    #[arg(short, long)]
    pub description: Option<String>,

    /// Make the repository public
    #[arg(long, conflicts_with_all = ["private", "internal"])]
    pub public: bool,

    /// Make the repository private (the default)
    #[arg(long, conflicts_with = "internal")]
    pub private: bool,

    #[arg(long, hide = true)]
    pub internal: bool,

    /// Add a README file
    #[arg(long)]
    pub add_readme: bool,

    /// Start from a gitignore template; see `gf repo gitignore list`
    #[arg(short, long, value_name = "TEMPLATE")]
    pub gitignore: Option<String>,

    /// Add a license; see `gf repo license list`
    #[arg(short, long, value_name = "LICENSE")]
    pub license: Option<String>,

    /// Clone the new repository into the current directory
    #[arg(short, long)]
    pub clone: bool,

    /// Local repository to add the new repository to as a remote
    #[arg(short, long, value_name = "PATH")]
    pub source: Option<PathBuf>,

    /// Name of the remote added with --source
    #[arg(short, long, value_name = "NAME")]
    pub remote: Option<String>,

    /// Push the --source repository's commits to the new repository
    #[arg(long, requires = "source")]
    pub push: bool,

    #[arg(short = 'h', long, value_name = "URL", hide = true)]
    pub homepage: Option<String>,

    #[arg(short = 'p', long, value_name = "REPOSITORY", hide = true)]
    pub template: Option<String>,

    #[arg(short = 't', long, value_name = "NAME", hide = true)]
    pub team: Option<String>,

    #[arg(long, hide = true)]
    pub include_all_branches: bool,

    /// Accepted for gh compatibility; GitFox repositories have no issues
    #[arg(long, hide = true)]
    pub disable_issues: bool,

    /// Accepted for gh compatibility; GitFox repositories have no wiki
    #[arg(long, hide = true)]
    pub disable_wiki: bool,
}

#[derive(Debug, Args)]
pub struct RepoDeleteArgs {
    /// Repository to delete; defaults to the current checkout
    #[arg(value_name = "SPACE/NAME")]
    pub repository: Option<String>,

    /// Confirm deletion without prompting
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct RepoEditArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Repository to edit; defaults to the current checkout
    #[arg(value_name = "SPACE/NAME")]
    pub repository: Option<String>,

    /// New description
    #[arg(short, long)]
    pub description: Option<String>,

    /// New visibility: public or private
    #[arg(long, value_name = "VISIBILITY")]
    pub visibility: Option<Visibility>,

    /// Required with --visibility, as in gh
    #[arg(long)]
    pub accept_visibility_change_consequences: bool,

    /// New default branch
    #[arg(long, value_name = "BRANCH")]
    pub default_branch: Option<String>,

    #[arg(short = 'h', long, value_name = "URL", hide = true)]
    pub homepage: Option<String>,

    #[arg(long, value_name = "TOPIC", hide = true, value_delimiter = ',')]
    pub add_topic: Vec<String>,

    #[arg(long, value_name = "TOPIC", hide = true, value_delimiter = ',')]
    pub remove_topic: Vec<String>,

    #[arg(long, hide = true)]
    pub template: bool,

    #[arg(long, value_name = "FORMAT", hide = true)]
    pub squash_merge_commit_message: Option<String>,

    /// gh's --enable-*/--allow-*/--delete-branch-on-merge settings, none of
    /// which GitFox has.
    #[arg(
        long = "enable-issues",
        hide = true,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        value_name = "BOOL"
    )]
    pub enable_issues: Option<String>,
    #[arg(long = "enable-wiki", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_wiki: Option<String>,
    #[arg(long = "enable-projects", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_projects: Option<String>,
    #[arg(long = "enable-discussions", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_discussions: Option<String>,
    #[arg(long = "enable-merge-commit", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_merge_commit: Option<String>,
    #[arg(long = "enable-squash-merge", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_squash_merge: Option<String>,
    #[arg(long = "enable-rebase-merge", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_rebase_merge: Option<String>,
    #[arg(long = "enable-auto-merge", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_auto_merge: Option<String>,
    #[arg(long = "enable-advanced-security", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_advanced_security: Option<String>,
    #[arg(long = "enable-secret-scanning", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_secret_scanning: Option<String>,
    #[arg(long = "enable-secret-scanning-push-protection", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub enable_secret_scanning_push_protection: Option<String>,
    #[arg(long = "delete-branch-on-merge", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub delete_branch_on_merge: Option<String>,
    #[arg(long = "allow-forking", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub allow_forking: Option<String>,
    #[arg(long = "allow-update-branch", hide = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub allow_update_branch: Option<String>,
}

impl RepoEditArgs {
    /// The first gh setting given that GitFox has no equivalent for.
    pub fn unsupported_setting(&self) -> Option<&'static str> {
        let settings = [
            ("--homepage", self.homepage.is_some()),
            ("--add-topic", !self.add_topic.is_empty()),
            ("--remove-topic", !self.remove_topic.is_empty()),
            ("--template", self.template),
            (
                "--squash-merge-commit-message",
                self.squash_merge_commit_message.is_some(),
            ),
            ("--enable-issues", self.enable_issues.is_some()),
            ("--enable-wiki", self.enable_wiki.is_some()),
            ("--enable-projects", self.enable_projects.is_some()),
            ("--enable-discussions", self.enable_discussions.is_some()),
            ("--enable-merge-commit", self.enable_merge_commit.is_some()),
            ("--enable-squash-merge", self.enable_squash_merge.is_some()),
            ("--enable-rebase-merge", self.enable_rebase_merge.is_some()),
            ("--enable-auto-merge", self.enable_auto_merge.is_some()),
            (
                "--enable-advanced-security",
                self.enable_advanced_security.is_some(),
            ),
            (
                "--enable-secret-scanning",
                self.enable_secret_scanning.is_some(),
            ),
            (
                "--enable-secret-scanning-push-protection",
                self.enable_secret_scanning_push_protection.is_some(),
            ),
            (
                "--delete-branch-on-merge",
                self.delete_branch_on_merge.is_some(),
            ),
            ("--allow-forking", self.allow_forking.is_some()),
            ("--allow-update-branch", self.allow_update_branch.is_some()),
        ];
        settings
            .into_iter()
            .find(|(_, given)| *given)
            .map(|(flag, _)| flag)
    }
}

#[derive(Debug, Args)]
pub struct RepoRenameArgs {
    /// The new name
    #[arg(value_name = "NEW-NAME")]
    pub new_name: Option<String>,

    /// Skip the confirmation prompt
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct RepoForkArgs {
    /// Repository to fork; defaults to the current checkout's
    #[arg(value_name = "SPACE/NAME")]
    pub repository: Option<String>,

    /// Extra flags for `git clone` with --clone, after `--`
    #[arg(last = true, value_name = "GITFLAGS")]
    pub git_flags: Vec<String>,

    /// Clone the fork
    #[arg(long)]
    pub clone: bool,

    /// Add a git remote for the fork to the current checkout
    #[arg(long)]
    pub remote: bool,

    /// Name of the remote --remote adds
    #[arg(long, value_name = "NAME", default_value = "origin")]
    pub remote_name: String,

    /// Name for the fork; defaults to the source repository's
    #[arg(long, value_name = "NAME")]
    pub fork_name: Option<String>,

    /// Accepted for gh compatibility; GitFox forks every branch
    #[arg(long, hide = true)]
    pub default_branch_only: bool,
}

#[derive(Debug, Args)]
pub struct RepoSyncArgs {
    /// Repository to sync on the server; without it, the checkout's branch is
    /// synced from its remote
    #[arg(value_name = "SPACE/NAME")]
    pub destination: Option<String>,

    /// Branch to sync; defaults to the current branch
    #[arg(short, long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Reset the branch to the remote even when that is not a fast-forward
    #[arg(long)]
    pub force: bool,

    #[arg(short, long, value_name = "REPOSITORY", hide = true)]
    pub source: Option<String>,
}

#[derive(Debug, Args)]
pub struct RepoSetDefaultArgs {
    /// Repository to use; omit with --view or --unset
    #[arg(value_name = "SPACE/NAME")]
    pub repository: Option<String>,

    /// Show the current default
    #[arg(long, conflicts_with_all = ["repository", "unset"])]
    pub view: bool,

    /// Remove the default
    #[arg(short, long, conflicts_with = "repository")]
    pub unset: bool,
}

#[derive(Debug, Args)]
pub struct RepoReadFileArgs {
    /// Path of the file within the repository
    #[arg(value_name = "PATH")]
    pub path: String,

    /// Branch, tag or commit to read from; defaults to the default branch
    #[arg(long = "ref", value_name = "REF")]
    pub git_ref: Option<String>,

    /// Write the file here instead of to stdout (gh's -o; `--output` is gf's
    /// global output format)
    #[arg(id = "output_path", short = 'o', value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Overwrite --output if it exists
    #[arg(long)]
    pub clobber: bool,

    #[arg(long, hide = true)]
    pub allow_escape_sequences: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct RepoReadDirArgs {
    /// Directory within the repository; defaults to the root
    #[arg(value_name = "PATH")]
    pub path: Option<String>,

    /// Branch, tag or commit to list; defaults to the default branch
    #[arg(long = "ref", value_name = "REF")]
    pub git_ref: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct RepoGitignoreCommand {
    #[command(subcommand)]
    pub command: TemplateSubcommand,
}

#[derive(Debug, Args)]
pub struct RepoLicenseCommand {
    #[command(subcommand)]
    pub command: TemplateSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum TemplateSubcommand {
    /// List what GitFox offers
    #[command(visible_alias = "ls")]
    List,
    /// Show one template
    View(TemplateViewArgs),
}

#[derive(Debug, Args)]
pub struct TemplateViewArgs {
    /// Template name or license key
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Open the license's page on choosealicense.com
    #[arg(short, long)]
    pub web: bool,
}
