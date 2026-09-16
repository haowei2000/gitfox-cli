//! The command surface.
//!
//! Environment variables are *not* wired through clap's `env` support on
//! purpose: clap would resolve them before `gf` gets a chance to apply the
//! documented precedence chain. Flags land here as `Option`, and
//! [`crate::config::resolve`] owns the chain.
//!
//! The tree follows gh's: the same command names, the same flags and the same
//! short letters, so a gh invocation works against GitFox unchanged. A gh flag
//! whose feature GitFox lacks still parses — hidden from `--help` — and
//! answers `UNSUPPORTED` with the reason, instead of clap's "unexpected
//! argument" or, worse, being silently ignored.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::Overrides;
use crate::export::Format;
use crate::output::OutputFormat;

mod account;
mod ci;
mod pr;
mod repo;

pub use account::*;
pub use ci::*;
pub use pr::*;
pub use repo::*;

const LONG_ABOUT: &str = "\
gf is a GitFox client with three audiences.

  you    gf pr list
  CI     GITFOX_TOKEN=$TOKEN gf --agent pipeline list
  agent  gf --agent pr list

`--agent` is shorthand for `--output json --non-interactive --no-color`: it
tells gf the caller is a machine. Every command then answers with a stable
envelope ({\"ok\":true,\"data\":…} / {\"ok\":false,\"error\":…}) and a stable exit
code, so nothing has to be parsed out of prose.

The commands, flags and `--json FIELDS` / `--jq` / `--template` output follow
gh, so what works with `gh` works here.";

#[derive(Debug, Parser)]
#[command(
    name = "gf",
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

    /// JSON output. Alone: shorthand for --output json. With fields
    /// (`--json number,title`): only those fields, shaped the way gh prints them
    #[arg(
        long,
        global = true,
        value_name = "FIELDS",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "",
        conflicts_with = "output"
    )]
    pub json: Option<String>,

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

    /// Retries for transient failures on safe-to-repeat requests [env: GITFOX_RETRIES]
    #[arg(long, global = true, value_name = "N")]
    pub retries: Option<u32>,

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
            // Bare `--json` is the envelope. `--json FIELDS` is gh's export,
            // which leaves the format alone — so its errors still go where the
            // resolved format sends them.
            output: self.output.or(match self.json.as_deref() {
                Some("") => Some(OutputFormat::Json),
                _ => None,
            }),
            timeout: self.timeout,
            retries: self.retries,
            insecure: self.insecure,
            agent: self.agent,
            non_interactive: self.non_interactive,
            no_color: self.no_color,
        }
    }
}

/// `--jq` and `--template`, on every command gh gives them to.
#[derive(Debug, Default, Args)]
pub struct FormatArgs {
    /// Filter JSON output using a jq expression (needs --json FIELDS)
    #[arg(short = 'q', long, value_name = "EXPRESSION")]
    pub jq: Option<String>,

    /// Format JSON output using a Go template (needs --json FIELDS)
    #[arg(short = 't', long, value_name = "TEMPLATE")]
    pub template: Option<String>,
}

impl FormatArgs {
    pub fn format(&self) -> Format {
        Format {
            jq: self.jq.clone(),
            template: self.template.clone(),
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Authenticate gf and git with a GitFox host
    Auth(AuthCommand),

    /// Make an authenticated request to any GitFox API endpoint
    #[command(long_about = "\
Make an authenticated request to any GitFox API endpoint.

This is the escape hatch: anything GitFox exposes is reachable from gf on day
one, whether or not a dedicated command exists yet. The flags are gh's.

  gf api /api/v1/user
  gf api -X POST /api/v1/foo -F count=3 -f name=test
  gf api /api/v1/repos/{repo_ref}/pullreq --paginate --jq '.[].title'
  cat payload.json | gf api POST /api/v1/foo --input -

-F/--field sends typed values (integers, true/false/null, @file); -f/--raw-field
always sends strings. They are the JSON body, or the query string on a GET:

  gf api -X GET /api/v1/repos/{repo_ref}/pullreq -f state=merged -F limit=5

{owner}, {repo}, {repo_ref} and {branch} are filled in from the current
checkout, in the path and in -F values.")]
    Api(ApiArgs),

    /// Work with repositories
    Repo(RepoCommand),

    /// Work with pull requests
    #[command(visible_alias = "pull-request")]
    Pr(PrCommand),

    /// Work with pipelines and their runs
    #[command(visible_alias = "ci")]
    Pipeline(PipelineCommand),

    /// View and manage pipeline runs, the way `gh run` does
    Run(RunCommand),

    /// View and manage pipelines, the way `gh workflow` does
    Workflow(WorkflowCommand),

    /// Manage space secrets
    Secret(SecretCommand),

    /// Manage repository labels
    Label(LabelCommand),

    /// Manage SSH keys on your account
    #[command(name = "ssh-key")]
    SshKey(SshKeyCommand),

    /// List the spaces you belong to
    Org(OrgCommand),

    /// View repository protection rules
    #[command(visible_alias = "rs")]
    Ruleset(RulesetCommand),

    /// Manage gitspaces, GitFox's cloud development environments
    #[command(visible_alias = "cs")]
    Codespace(CodespaceCommand),

    /// Open repositories, pull requests and files in the browser
    Browse(BrowseArgs),

    /// Show pull requests that need your attention across a space
    Status(StatusArgs),

    /// Create command shortcuts
    Alias(AliasCommand),

    /// Alias for "pr checkout"
    Co(PrCheckoutArgs),

    /// Read and write gf configuration
    Config(ConfigCommand),

    #[command(flatten)]
    GhOnly(GhOnlyCommand),

    /// Print a shell completion script
    #[command(long_about = "\
Print a shell completion script.

  # zsh
  gf completion zsh > ~/.zfunc/_gf

  # bash
  gf completion -s bash > /usr/local/etc/bash_completion.d/gf

  # fish
  gf completion fish > ~/.config/fish/completions/gf.fish")]
    Completion(CompletionArgs),
}

impl Command {
    /// The `--jq` / `--template` the command was given, for those that take
    /// them.
    pub fn format(&self) -> Format {
        match self {
            Command::Pr(cmd) => match &cmd.command {
                PrSubcommand::List(a) => a.format.format(),
                PrSubcommand::View(a) => a.format.format(),
                PrSubcommand::Checks(a) => a.format.format(),
                PrSubcommand::Status(a) => a.format.format(),
                _ => Format::default(),
            },
            Command::Repo(cmd) => match &cmd.command {
                RepoSubcommand::List(a) => a.format.format(),
                RepoSubcommand::View(a) => a.format.format(),
                RepoSubcommand::ReadDir(a) => a.format.format(),
                RepoSubcommand::ReadFile(a) => a.format.format(),
                _ => Format::default(),
            },
            Command::Run(cmd) => match &cmd.command {
                RunSubcommand::List(a) => a.format.format(),
                RunSubcommand::View(a) => a.format.format(),
                _ => Format::default(),
            },
            Command::Workflow(cmd) => match &cmd.command {
                WorkflowSubcommand::List(a) => a.format.format(),
                _ => Format::default(),
            },
            Command::Secret(cmd) => match &cmd.command {
                SecretSubcommand::List(a) => a.format.format(),
                _ => Format::default(),
            },
            Command::Label(cmd) => match &cmd.command {
                LabelSubcommand::List(a) => a.format.format(),
                _ => Format::default(),
            },
            Command::Codespace(cmd) => match &cmd.command {
                CodespaceSubcommand::List(a) => a.format.format(),
                CodespaceSubcommand::View(a) => a.format.format(),
                _ => Format::default(),
            },
            Command::Auth(cmd) => match &cmd.command {
                AuthSubcommand::Status(a) => Format {
                    jq: a.jq.clone(),
                    template: a.template.clone(),
                },
                _ => Format::default(),
            },
            _ => Format::default(),
        }
    }
}

/// Whatever followed a gh command GitFox has no feature for. Captured so the
/// command can answer with the reason instead of clap's "unrecognized
/// subcommand" — and with `-h` captured too, never a silent exit 0.
#[derive(Debug, Args)]
pub struct GhOnlyArgs {
    #[arg(
        num_args = 0..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS"
    )]
    pub rest: Vec<String>,
}

/// gh commands for features GitFox does not have. Hidden from `--help`; each
/// answers `UNSUPPORTED` saying why, and what to use instead when something
/// comes close.
#[derive(Debug, Subcommand)]
pub enum GhOnlyCommand {
    #[command(hide = true, disable_help_flag = true, aliases = ["agent-tasks", "agent", "agents"])]
    AgentTask(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Attestation(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Cache(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Copilot(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Discussion(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true, aliases = ["extensions", "ext"])]
    Extension(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Gist(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true, name = "gpg-key")]
    GpgKey(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Issue(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Licenses(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Preview(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Project(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Release(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Search(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true, aliases = ["skills"])]
    Skill(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Variable(GhOnlyArgs),
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Shell to generate for
    #[arg(value_name = "SHELL", required_unless_present = "shell_flag")]
    pub shell: Option<clap_complete::Shell>,

    /// Shell to generate for, as gh spells it
    #[arg(
        id = "shell_flag",
        short = 's',
        long = "shell",
        value_name = "SHELL",
        conflicts_with = "shell"
    )]
    pub shell_flag: Option<clap_complete::Shell>,
}

#[cfg(test)]
mod tests;
