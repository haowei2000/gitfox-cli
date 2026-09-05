//! The command surface.
//!
//! Environment variables are *not* wired through clap's `env` support on
//! purpose: clap would resolve them before `fx` gets a chance to apply the
//! documented precedence chain. Flags land here as `Option`, and
//! [`crate::config::resolve`] owns the chain.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::Overrides;
use crate::output::OutputFormat;

const LONG_ABOUT: &str = "\
fx is a GitFox client with three audiences.

  you    fx pr list
  CI     GITFOX_TOKEN=$TOKEN fx --agent pipeline list
  agent  fx --agent pr list

`--agent` is shorthand for `--output json --non-interactive --no-color`: it
tells fx the caller is a machine. Every command then answers with a stable
envelope ({\"ok\":true,\"data\":…} / {\"ok\":false,\"error\":…}) and a stable exit
code, so nothing has to be parsed out of prose.";

#[derive(Debug, Parser)]
#[command(
    name = "fx",
    version,
    about = "GitFox CLI for humans, CI and AI agents",
    long_about = LONG_ABOUT,
    propagate_version = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Default, Args)]
pub struct GlobalArgs {
    /// GitFox host, e.g. https://git.example.com [env: GITFOX_HOST]
    #[arg(long, global = true, value_name = "URL")]
    pub host: Option<String>,

    /// API token [env: GITFOX_TOKEN]
    #[arg(long, global = true, value_name = "TOKEN")]
    pub token: Option<String>,

    /// Repository, as space/name [env: GITFOX_REPO]
    #[arg(short = 'R', long = "repo", global = true, value_name = "SPACE/NAME")]
    pub repo: Option<String>,

    /// Space or organisation [env: GITFOX_ORG]
    #[arg(long, global = true, value_name = "SPACE")]
    pub org: Option<String>,

    /// Output format [env: GITFOX_OUTPUT]
    #[arg(long, global = true, value_name = "FORMAT")]
    pub output: Option<OutputFormat>,

    /// Shorthand for --output json
    #[arg(long, global = true, conflicts_with = "output")]
    pub json: bool,

    /// The caller is a machine: --output json --non-interactive --no-color [env: GITFOX_AGENT]
    #[arg(long, global = true)]
    pub agent: bool,

    /// Never prompt; fail instead of waiting for input
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Disable coloured output [env: NO_COLOR]
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Config file to use [env: GITFOX_CONFIG]
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// HTTP timeout in seconds [env: GITFOX_TIMEOUT]
    #[arg(long, global = true, value_name = "SECONDS")]
    pub timeout: Option<u64>,

    /// Skip TLS certificate verification [env: GITFOX_INSECURE]
    #[arg(long, global = true)]
    pub insecure: bool,

    /// Log to stderr; repeat for more detail
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

impl GlobalArgs {
    pub fn overrides(&self) -> Overrides {
        Overrides {
            host: self.host.clone(),
            token: self.token.clone(),
            repo: self.repo.clone(),
            org: self.org.clone(),
            output: self.output.or(if self.json {
                Some(OutputFormat::Json)
            } else {
                None
            }),
            timeout: self.timeout,
            insecure: self.insecure,
            agent: self.agent,
            non_interactive: self.non_interactive,
            no_color: self.no_color,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Authenticate fx with a GitFox host
    Auth(AuthCommand),

    /// Make an authenticated request to any GitFox API endpoint
    #[command(long_about = "\
Make an authenticated request to any GitFox API endpoint.

This is the escape hatch: anything GitFox exposes is reachable from fx on day
one, whether or not a dedicated command exists yet.

  fx api GET /api/v1/user
  fx api POST /api/v1/foo --field name=test
  fx api POST /api/v1/foo --body '{\"name\":\"test\"}'
  cat payload.json | fx api POST /api/v1/foo --input -")]
    Api(ApiArgs),

    /// Work with repositories
    Repo(RepoCommand),

    /// Work with pull requests
    #[command(visible_alias = "pull-request")]
    Pr(PrCommand),

    /// Work with pipelines and their runs
    #[command(visible_alias = "ci")]
    Pipeline(PipelineCommand),

    /// Read and write fx configuration
    Config(ConfigCommand),
}

// ---------------------------------------------------------------------------
// auth
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct AuthCommand {
    #[command(subcommand)]
    pub command: AuthSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum AuthSubcommand {
    /// Log in to a GitFox host and store the token in the OS keychain
    Login(AuthLoginArgs),
    /// Remove a stored token
    Logout(AuthHostArgs),
    /// Show the active host and whether a token is configured
    Status(AuthHostArgs),
}

#[derive(Debug, Args)]
pub struct AuthLoginArgs {
    /// Host to authenticate against, e.g. git.example.com
    #[arg(long, value_name = "HOST")]
    pub hostname: Option<String>,

    /// Read the token from stdin instead of prompting
    #[arg(long)]
    pub with_token: bool,

    /// Overwrite an existing stored token without asking
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct AuthHostArgs {
    /// Host to act on; defaults to the resolved host
    #[arg(long, value_name = "HOST")]
    pub hostname: Option<String>,
}

// ---------------------------------------------------------------------------
// api
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ApiArgs {
    /// HTTP method, or the path itself when the method is omitted
    #[arg(value_name = "METHOD|PATH")]
    pub method_or_path: String,

    /// Endpoint path, e.g. /api/v1/user
    #[arg(value_name = "PATH")]
    pub path: Option<String>,

    /// Body parameter `key=value`; numbers, booleans and null are detected
    #[arg(short = 'f', long = "field", value_name = "KEY=VALUE")]
    pub fields: Vec<String>,

    /// Body parameter `key=value`, always sent as a string
    #[arg(short = 'F', long = "raw-field", value_name = "KEY=VALUE")]
    pub raw_fields: Vec<String>,

    /// Send this JSON string as the request body
    #[arg(long, value_name = "JSON", conflicts_with_all = ["fields", "raw_fields", "input"])]
    pub body: Option<String>,

    /// Read the JSON request body from a file, or `-` for stdin
    #[arg(long, value_name = "FILE", conflicts_with_all = ["fields", "raw_fields", "body"])]
    pub input: Option<String>,

    /// Extra request header, e.g. -H 'X-Trace: 1'
    #[arg(short = 'H', long = "header", value_name = "NAME: VALUE")]
    pub headers: Vec<String>,

    /// Include the response status and headers in the output
    #[arg(short = 'i', long)]
    pub include: bool,
}

// ---------------------------------------------------------------------------
// repo (v0.2)
// ---------------------------------------------------------------------------

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
}

#[derive(Debug, Args)]
pub struct RepoListArgs {
    /// Space to list; defaults to --org or the current repository's space
    #[arg(value_name = "SPACE")]
    pub space: Option<String>,

    /// Maximum number of repositories to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 30)]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct RepoViewArgs {
    /// Repository to show; defaults to the current checkout
    #[arg(value_name = "SPACE/NAME")]
    pub repository: Option<String>,
}

#[derive(Debug, Args)]
pub struct RepoCloneArgs {
    /// Repository to clone
    #[arg(value_name = "SPACE/NAME")]
    pub repository: String,

    /// Directory to clone into
    #[arg(value_name = "DIRECTORY")]
    pub directory: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// pr (v0.3 / v0.5)
// ---------------------------------------------------------------------------

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
    View(PrNumberArgs),
    /// Open a pull request
    Create(PrCreateArgs),
    /// Merge a pull request
    Merge(PrMergeArgs),
    /// Check out a pull request branch locally
    Checkout(PrNumberArgs),
    /// Show a pull request's diff
    Diff(PrNumberArgs),
    /// Show the status of a pull request's checks
    Checks(PrNumberArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum PrState {
    Open,
    Closed,
    Merged,
    All,
}

#[derive(Debug, Args)]
pub struct PrListArgs {
    /// Filter by state
    #[arg(short, long, value_name = "STATE", default_value = "open")]
    pub state: PrState,

    /// Maximum number of pull requests to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 30)]
    pub limit: u32,

    /// Only pull requests opened by this user
    #[arg(long, value_name = "USER")]
    pub author: Option<String>,
}

#[derive(Debug, Args)]
pub struct PrNumberArgs {
    /// Pull request number; defaults to the one for the current branch
    #[arg(value_name = "NUMBER")]
    pub number: Option<u64>,
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
    #[arg(short, long)]
    pub body: Option<String>,

    /// Take the title and body from the branch's commits
    #[arg(long)]
    pub fill: bool,

    /// Open as a draft
    #[arg(short, long)]
    pub draft: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
    FastForward,
}

#[derive(Debug, Args)]
pub struct PrMergeArgs {
    /// Pull request number; defaults to the one for the current branch
    #[arg(value_name = "NUMBER")]
    pub number: Option<u64>,

    /// Merge strategy
    #[arg(short, long, value_name = "METHOD", default_value = "merge")]
    pub method: MergeMethod,

    /// Delete the source branch after merging
    #[arg(short = 'D', long)]
    pub delete_branch: bool,
}

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
    View(PipelineRefArgs),
    /// Print the logs of a pipeline run
    Logs(PipelineLogsArgs),
    /// Trigger a pipeline run
    Run(PipelineRunArgs),
    /// Retry a pipeline run
    Retry(PipelineRefArgs),
}

#[derive(Debug, Args)]
pub struct PipelineListArgs {
    /// Only runs for this pipeline
    #[arg(long, value_name = "PIPELINE")]
    pub pipeline: Option<String>,

    /// Maximum number of runs to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 20)]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct PipelineRefArgs {
    /// Run number
    #[arg(value_name = "RUN")]
    pub run: u64,

    /// Pipeline the run belongs to
    #[arg(long, value_name = "PIPELINE")]
    pub pipeline: Option<String>,
}

#[derive(Debug, Args)]
pub struct PipelineLogsArgs {
    /// Run number
    #[arg(value_name = "RUN")]
    pub run: u64,

    /// Pipeline the run belongs to
    #[arg(long, value_name = "PIPELINE")]
    pub pipeline: Option<String>,

    /// Only the steps that failed — the fastest path from a red build to a fix
    #[arg(long)]
    pub failed: bool,

    /// Only this step
    #[arg(long, value_name = "STEP")]
    pub step: Option<String>,
}

#[derive(Debug, Args)]
pub struct PipelineRunArgs {
    /// Pipeline to run
    #[arg(value_name = "PIPELINE")]
    pub pipeline: String,

    /// Branch to run against
    #[arg(short, long, value_name = "BRANCH")]
    pub branch: Option<String>,
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigSubcommand {
    /// Print one configuration value
    Get(ConfigGetArgs),
    /// Set one configuration value
    Set(ConfigSetArgs),
    /// Show the resolved configuration and where each value came from
    List,
}

#[derive(Debug, Args)]
pub struct ConfigGetArgs {
    /// Key, e.g. `default_host` or `hosts.git.example.com.api_url`
    pub key: String,
}

#[derive(Debug, Args)]
pub struct ConfigSetArgs {
    /// Key, e.g. `default_host` or `hosts.git.example.com.api_url`
    pub key: String,
    /// Value to store
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn json_flag_is_shorthand_for_output_json() {
        let cli = Cli::try_parse_from(["fx", "--json", "api", "/api/v1/user"]).unwrap();
        assert_eq!(cli.global.overrides().output, Some(OutputFormat::Json));
    }

    #[test]
    fn output_flag_and_json_flag_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["fx", "--json", "--output", "table", "api", "/x"]).is_err());
    }

    #[test]
    fn global_flags_are_accepted_before_and_after_the_subcommand() {
        for argv in [
            vec![
                "fx",
                "--host",
                "https://git.example.com",
                "api",
                "/api/v1/user",
            ],
            vec![
                "fx",
                "api",
                "/api/v1/user",
                "--host",
                "https://git.example.com",
            ],
        ] {
            let cli = Cli::try_parse_from(&argv).unwrap();
            assert_eq!(cli.global.host.as_deref(), Some("https://git.example.com"));
        }
    }

    #[test]
    fn api_accepts_a_bare_path_or_a_method_and_a_path() {
        let bare = Cli::try_parse_from(["fx", "api", "/api/v1/user"]).unwrap();
        let Command::Api(args) = bare.command else {
            panic!("expected the api subcommand")
        };
        assert_eq!(args.method_or_path, "/api/v1/user");
        assert!(args.path.is_none());

        let explicit = Cli::try_parse_from(["fx", "api", "POST", "/api/v1/foo"]).unwrap();
        let Command::Api(args) = explicit.command else {
            panic!("expected the api subcommand")
        };
        assert_eq!(args.method_or_path, "POST");
        assert_eq!(args.path.as_deref(), Some("/api/v1/foo"));
    }

    #[test]
    fn api_body_sources_are_mutually_exclusive() {
        assert!(
            Cli::try_parse_from(["fx", "api", "POST", "/x", "--body", "{}", "--field", "a=b"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from(["fx", "api", "POST", "/x", "--body", "{}", "--input", "-"])
                .is_err()
        );
    }

    #[test]
    fn pr_create_follows_the_gh_short_flag_convention() {
        let cli = Cli::try_parse_from([
            "fx",
            "pr",
            "create",
            "-B",
            "main",
            "-H",
            "feat/oauth",
            "-t",
            "feat: add OAuth",
            "-b",
            "body",
        ])
        .unwrap();
        let Command::Pr(cmd) = cli.command else {
            panic!("expected the pr subcommand")
        };
        let PrSubcommand::Create(args) = cmd.command else {
            panic!("expected pr create")
        };
        assert_eq!(args.base.as_deref(), Some("main"));
        assert_eq!(args.head.as_deref(), Some("feat/oauth"));
        assert_eq!(args.title.as_deref(), Some("feat: add OAuth"));
        assert_eq!(args.body.as_deref(), Some("body"));
    }

    #[test]
    fn pr_has_a_pull_request_alias() {
        assert!(Cli::try_parse_from(["fx", "pull-request", "list"]).is_ok());
        assert!(Cli::try_parse_from(["fx", "pr", "ls"]).is_ok());
    }

    #[test]
    fn agent_flag_is_global() {
        let cli = Cli::try_parse_from(["fx", "--agent", "pr", "list"]).unwrap();
        assert!(cli.global.agent);
    }
}
