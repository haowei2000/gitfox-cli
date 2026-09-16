//! Everything that is not a repository, a pull request or CI: authentication,
//! `gf api`, configuration and aliases, and the space- and account-level
//! resources — secrets, labels, SSH keys, spaces, rules, gitspaces.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::{FormatArgs, GhOnlyArgs};

// ---------------------------------------------------------------------------
// auth
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct AuthCommand {
    #[command(subcommand)]
    pub command: AuthSubcommand,
}

// In gh every `auth` subcommand spells --hostname `-h`, so here `-h` is the
// hostname and --help is long-only. Otherwise `gh auth login -h host` prints
// help, exits 0, and logs nobody in.
#[derive(Debug, Subcommand)]
pub enum AuthSubcommand {
    /// Log in to a GitFox host and store the token in the OS keychain
    #[command(disable_help_flag = true)]
    Login(AuthLoginArgs),
    /// Remove a stored token
    #[command(disable_help_flag = true)]
    Logout(AuthLogoutArgs),
    /// Show the active host and whether a token is configured
    #[command(disable_help_flag = true)]
    Status(AuthStatusArgs),
    /// Print the token gf uses for a host
    #[command(disable_help_flag = true)]
    Token(AuthTokenArgs),
    /// Switch the default host
    #[command(disable_help_flag = true)]
    Switch(AuthSwitchArgs),
    /// Make git use gf for GitFox credentials
    #[command(name = "setup-git", disable_help_flag = true)]
    SetupGit(AuthSetupGitArgs),
    /// The git credential helper `gf auth setup-git` installs
    #[command(name = "git-credential", hide = true)]
    GitCredential(AuthGitCredentialArgs),
    #[command(hide = true, disable_help_flag = true)]
    Refresh(GhOnlyArgs),
}

#[derive(Debug, Args)]
pub struct AuthLoginArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Host to authenticate against, e.g. git.example.com
    #[arg(short = 'h', long, value_name = "HOST")]
    pub hostname: Option<String>,

    /// Read the token from stdin instead of prompting
    #[arg(long)]
    pub with_token: bool,

    /// Overwrite an existing stored token without asking
    #[arg(long)]
    pub force: bool,

    /// The protocol git should use with this host: https or ssh
    #[arg(short = 'p', long, value_name = "PROTOCOL")]
    pub git_protocol: Option<String>,

    /// Accepted for gh compatibility; gf never generates SSH keys
    #[arg(long, hide = true)]
    pub skip_ssh_key: bool,

    #[arg(short = 'w', long, hide = true)]
    pub web: bool,

    #[arg(short = 's', long, value_name = "SCOPES", hide = true)]
    pub scopes: Vec<String>,

    #[arg(short = 'c', long, hide = true)]
    pub clipboard: bool,

    #[arg(long, hide = true)]
    pub insecure_storage: bool,
}

#[derive(Debug, Args)]
pub struct AuthLogoutArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Host to act on; defaults to the resolved host
    #[arg(short = 'h', long, value_name = "HOST")]
    pub hostname: Option<String>,

    /// Only log out if this is the user the token belongs to
    #[arg(short, long, value_name = "LOGIN")]
    pub user: Option<String>,
}

#[derive(Debug, Args)]
pub struct AuthStatusArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Host to check; defaults to the resolved host
    #[arg(short = 'h', long, value_name = "HOST")]
    pub hostname: Option<String>,

    /// Accepted for gh compatibility; gf keeps one account per host
    #[arg(short, long)]
    pub active: bool,

    /// Include the token itself in the output
    #[arg(short = 't', long)]
    pub show_token: bool,

    /// Filter JSON output using a jq expression (needs --json hosts)
    #[arg(short = 'q', long, value_name = "EXPRESSION")]
    pub jq: Option<String>,

    /// Format JSON output using a Go template (needs --json hosts)
    #[arg(long, value_name = "TEMPLATE")]
    pub template: Option<String>,
}

#[derive(Debug, Args)]
pub struct AuthTokenArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Host whose token to print; defaults to the resolved host
    #[arg(short = 'h', long, value_name = "HOST")]
    pub hostname: Option<String>,

    /// Only print it if the token belongs to this user
    #[arg(short, long, value_name = "LOGIN")]
    pub user: Option<String>,
}

#[derive(Debug, Args)]
pub struct AuthSwitchArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Host to make the default
    #[arg(short = 'h', long, value_name = "HOST")]
    pub hostname: Option<String>,

    /// Accepted for gh compatibility; gf keeps one account per host
    #[arg(short, long, value_name = "LOGIN")]
    pub user: Option<String>,
}

#[derive(Debug, Args)]
pub struct AuthSetupGitArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Host to configure; defaults to every host gf knows
    #[arg(short = 'h', long, value_name = "HOST")]
    pub hostname: Option<String>,

    /// Configure --hostname even if gf has no token for it
    #[arg(short, long, requires = "hostname")]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct AuthGitCredentialArgs {
    /// get, store or erase — the git credential protocol's operations
    #[arg(value_name = "OPERATION")]
    pub operation: String,
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

    /// HTTP method (default GET, or POST when a body is given)
    #[arg(short = 'X', long = "method", value_name = "METHOD")]
    pub method: Option<String>,

    /// Typed parameter `key=value`: integers, true/false/null, `@file` or `@-`
    /// for stdin. The JSON body, or the query string on a GET
    #[arg(short = 'F', long = "field", value_name = "KEY=VALUE")]
    pub fields: Vec<String>,

    /// Parameter `key=value`, always sent as a string
    #[arg(short = 'f', long = "raw-field", value_name = "KEY=VALUE")]
    pub raw_fields: Vec<String>,

    /// Send this JSON string as the request body; parameters go in the query
    #[arg(long, value_name = "JSON", conflicts_with = "input")]
    pub body: Option<String>,

    /// Read the JSON request body from a file, or `-` for stdin; parameters go
    /// in the query
    #[arg(long, value_name = "FILE", conflicts_with = "body")]
    pub input: Option<String>,

    /// Extra request header, e.g. -H 'X-Trace: 1'
    #[arg(short = 'H', long = "header", value_name = "NAME: VALUE")]
    pub headers: Vec<String>,

    /// Include the response status and headers in the output
    #[arg(short = 'i', long)]
    pub include: bool,

    /// Fetch every page of a GET list and return one array
    #[arg(long)]
    pub paginate: bool,

    /// With --paginate, return an array of the pages instead of one array
    #[arg(long, requires = "paginate")]
    pub slurp: bool,

    /// Filter the response with a jq expression
    #[arg(short = 'q', long, value_name = "EXPRESSION")]
    pub jq: Option<String>,

    /// Format the response with a Go template
    #[arg(short = 't', long, value_name = "TEMPLATE", conflicts_with = "jq")]
    pub template: Option<String>,

    /// Do not print the response body
    #[arg(long)]
    pub silent: bool,

    /// The GitFox host to send the request to
    #[arg(long, value_name = "HOST")]
    pub hostname: Option<String>,

    /// Accepted for gh compatibility; gf does not cache responses
    #[arg(long, value_name = "DURATION", hide = true)]
    pub cache: Option<String>,

    /// Accepted for gh compatibility; GitFox has no API previews
    #[arg(short = 'p', long = "preview", value_name = "NAMES", hide = true)]
    pub previews: Vec<String>,

    /// Accepted for gh compatibility
    #[arg(long, hide = true)]
    pub allow_escape_sequences: bool,
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: ConfigSubcommand,
}

// As in gh, `-h` is --host on these, so --help is long-only.
#[derive(Debug, Subcommand)]
pub enum ConfigSubcommand {
    /// Print one configuration value
    #[command(disable_help_flag = true)]
    Get(ConfigGetArgs),
    /// Set one configuration value
    #[command(disable_help_flag = true)]
    Set(ConfigSetArgs),
    /// Show the resolved configuration and where each value came from
    #[command(disable_help_flag = true, visible_alias = "ls")]
    List(ConfigListArgs),
    /// Clear the cache — gf keeps none, so this only confirms that
    #[command(name = "clear-cache")]
    ClearCache,
}

#[derive(Debug, Args)]
pub struct ConfigGetArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Read the key for this host (the global --host works too)
    #[arg(id = "scope", short = 'h', value_name = "HOST")]
    pub scope: Option<String>,

    /// Key, e.g. `git_protocol`, `default_host` or `hosts.git.example.com.api_url`
    pub key: String,
}

#[derive(Debug, Args)]
pub struct ConfigSetArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Set the key for this host (the global --host works too)
    #[arg(id = "scope", short = 'h', value_name = "HOST")]
    pub scope: Option<String>,

    /// Key, e.g. `git_protocol`, `default_host` or `hosts.git.example.com.api_url`
    pub key: String,

    /// Value to store
    pub value: String,
}

#[derive(Debug, Args)]
pub struct ConfigListArgs {
    /// Print help
    #[arg(long, action = clap::ArgAction::Help)]
    pub help: Option<bool>,

    /// Only this host's settings
    #[arg(id = "scope", short = 'h', value_name = "HOST")]
    pub scope: Option<String>,
}

// ---------------------------------------------------------------------------
// alias
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct AliasCommand {
    #[command(subcommand)]
    pub command: AliasSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum AliasSubcommand {
    /// Create a shortcut for an gf command
    #[command(long_about = "\
Create a shortcut for an gf command.

  gf alias set co 'pr checkout'
  gf alias set prs 'pr list --author $1'     # $1, $2… take the next arguments
  gf alias set --shell mine 'gf pr list --json number,title | jq .'

An alias never shadows a built-in command.")]
    Set(AliasSetArgs),
    /// List your aliases
    #[command(visible_alias = "ls")]
    List,
    /// Delete an alias
    Delete(AliasDeleteArgs),
    /// Import aliases from a YAML file of `name: expansion` lines
    Import(AliasImportArgs),
}

#[derive(Debug, Args)]
pub struct AliasSetArgs {
    /// Name of the alias
    pub name: String,

    /// What it expands to
    pub expansion: String,

    /// Replace an existing alias of the same name
    #[arg(long)]
    pub clobber: bool,

    /// Run the expansion through the shell
    #[arg(short, long)]
    pub shell: bool,
}

#[derive(Debug, Args)]
pub struct AliasDeleteArgs {
    /// Name of the alias
    #[arg(required_unless_present = "all")]
    pub name: Option<String>,

    /// Delete every alias
    #[arg(long, conflicts_with = "name")]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct AliasImportArgs {
    /// YAML file to read, or `-` for stdin
    #[arg(value_name = "FILE")]
    pub file: Option<String>,

    /// Replace existing aliases of the same name
    #[arg(long)]
    pub clobber: bool,
}

// ---------------------------------------------------------------------------
// secret
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SecretCommand {
    #[command(subcommand)]
    pub command: SecretSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum SecretSubcommand {
    /// List a space's secrets
    #[command(visible_alias = "ls")]
    List(SecretListArgs),
    /// Create or update a secret
    #[command(long_about = "\
Create or update a secret.

GitFox keeps secrets in spaces. The space is -o/--org, else the current
repository's space. The value is --body, else read from stdin, else prompted.

  gf secret set DEPLOY_TOKEN --body \"$TOKEN\"
  gf secret set DEPLOY_TOKEN < token.txt
  gf secret set -f .env              # every KEY=VALUE in the file")]
    Set(SecretSetArgs),
    /// Delete a secret
    #[command(visible_alias = "remove")]
    Delete(SecretDeleteArgs),
}

#[derive(Debug, Args)]
pub struct SecretScopeArgs {
    /// Space holding the secrets; defaults to --org, then the current
    /// repository's space
    #[arg(id = "space", short = 'o', value_name = "SPACE")]
    pub space: Option<String>,

    #[arg(short = 'e', long, value_name = "ENVIRONMENT", hide = true)]
    pub env: Option<String>,

    #[arg(short = 'u', long, hide = true)]
    pub user: bool,

    #[arg(short = 'a', long, value_name = "APP", hide = true)]
    pub app: Option<String>,
}

#[derive(Debug, Args)]
pub struct SecretListArgs {
    #[command(flatten)]
    pub scope: SecretScopeArgs,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct SecretSetArgs {
    /// Name of the secret
    #[arg(value_name = "NAME", required_unless_present = "env_file")]
    pub name: Option<String>,

    /// The value; read from stdin when omitted
    #[arg(short, long, value_name = "VALUE")]
    pub body: Option<String>,

    /// Set every KEY=VALUE in a dotenv file, or `-` for stdin
    #[arg(short = 'f', long, value_name = "FILE", conflicts_with_all = ["name", "body"])]
    pub env_file: Option<String>,

    /// Description stored with the secret
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,

    #[command(flatten)]
    pub scope: SecretScopeArgs,

    // gh's -v; gf's -v is the global --verbose.
    #[arg(long, value_name = "VISIBILITY", hide = true)]
    pub visibility: Option<String>,

    #[arg(short = 'r', long, value_name = "REPOSITORIES", hide = true)]
    pub repos: Vec<String>,

    #[arg(long, hide = true)]
    pub no_repos_selected: bool,

    #[arg(long, hide = true)]
    pub no_store: bool,
}

#[derive(Debug, Args)]
pub struct SecretDeleteArgs {
    /// Name of the secret
    #[arg(value_name = "NAME")]
    pub name: String,

    #[command(flatten)]
    pub scope: SecretScopeArgs,
}

// ---------------------------------------------------------------------------
// label
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct LabelCommand {
    #[command(subcommand)]
    pub command: LabelSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum LabelSubcommand {
    /// List a repository's labels, including those from its space
    #[command(visible_alias = "ls")]
    List(LabelListArgs),
    /// Create a label
    Create(LabelCreateArgs),
    /// Edit a label
    Edit(LabelEditArgs),
    /// Delete a label
    Delete(LabelDeleteArgs),
    /// Copy another repository's labels into this one
    Clone(LabelCloneArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum LabelSort {
    Created,
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug, Args)]
pub struct LabelListArgs {
    /// Maximum number of labels to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 30)]
    pub limit: u32,

    /// Only labels whose key or description matches
    #[arg(short = 'S', long, value_name = "TEXT")]
    pub search: Option<String>,

    /// Sort by
    #[arg(long, value_name = "KEY", default_value = "created")]
    pub sort: LabelSort,

    /// Sort order
    #[arg(long, value_name = "ORDER", default_value = "asc")]
    pub order: SortOrder,

    /// Open the labels page in the browser
    #[arg(short, long)]
    pub web: bool,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct LabelCreateArgs {
    /// The label's key
    pub name: String,

    /// A GitFox colour name, or a hex colour mapped to the nearest one
    #[arg(short, long, value_name = "COLOR")]
    pub color: Option<String>,

    /// Description of the label
    #[arg(short, long)]
    pub description: Option<String>,

    /// Update the label if it already exists
    #[arg(short, long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct LabelEditArgs {
    /// The label's key
    pub name: String,

    /// New colour
    #[arg(short, long, value_name = "COLOR")]
    pub color: Option<String>,

    /// New description
    #[arg(short, long)]
    pub description: Option<String>,

    /// New key
    #[arg(short = 'n', long = "name", value_name = "NAME")]
    pub new_name: Option<String>,
}

#[derive(Debug, Args)]
pub struct LabelDeleteArgs {
    /// The label's key
    pub name: String,

    /// Confirm deletion without prompting
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct LabelCloneArgs {
    /// Repository to copy labels from
    #[arg(value_name = "SPACE/NAME")]
    pub source: String,

    /// Overwrite labels that already exist here
    #[arg(short, long)]
    pub force: bool,
}

// ---------------------------------------------------------------------------
// ssh-key, org, ruleset
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SshKeyCommand {
    #[command(subcommand)]
    pub command: SshKeySubcommand,
}

#[derive(Debug, Subcommand)]
pub enum SshKeySubcommand {
    /// List the SSH keys on your account
    #[command(visible_alias = "ls")]
    List,
    /// Add an SSH key to your account
    Add(SshKeyAddArgs),
    /// Delete an SSH key from your account
    Delete(SshKeyDeleteArgs),
}

#[derive(Debug, Args)]
pub struct SshKeyAddArgs {
    /// Public key file to add, or `-` for stdin
    #[arg(value_name = "KEY-FILE")]
    pub key_file: Option<PathBuf>,

    /// Name for the key; defaults to the key's comment
    #[arg(short, long)]
    pub title: Option<String>,

    /// authentication (GitFox has no signing keys)
    #[arg(long = "type", value_name = "TYPE", default_value = "authentication")]
    pub key_type: String,
}

#[derive(Debug, Args)]
pub struct SshKeyDeleteArgs {
    /// The key's name, as `gf ssh-key list` shows it
    #[arg(value_name = "ID")]
    pub id: String,

    /// Skip the confirmation prompt
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct OrgCommand {
    #[command(subcommand)]
    pub command: OrgSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum OrgSubcommand {
    /// List the spaces you belong to
    #[command(visible_alias = "ls")]
    List(OrgListArgs),
}

#[derive(Debug, Args)]
pub struct OrgListArgs {
    /// Maximum number of spaces to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 30)]
    pub limit: u32,
}

#[derive(Debug, Args)]
pub struct RulesetCommand {
    #[command(subcommand)]
    pub command: RulesetSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum RulesetSubcommand {
    /// List a repository's protection rules
    #[command(visible_alias = "ls")]
    List(RulesetListArgs),
    /// Show a protection rule
    View(RulesetViewArgs),
    #[command(hide = true, disable_help_flag = true)]
    Check(GhOnlyArgs),
}

#[derive(Debug, Args)]
pub struct RulesetListArgs {
    /// Maximum number of rules to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 30)]
    pub limit: u32,

    /// Accepted for gh compatibility; GitFox rules live on repositories
    #[arg(short, long)]
    pub parents: bool,

    /// Open the rules page in the browser
    #[arg(short, long)]
    pub web: bool,

    #[arg(short = 'o', id = "ruleset_org", value_name = "ORG", hide = true)]
    pub org: Option<String>,
}

#[derive(Debug, Args)]
pub struct RulesetViewArgs {
    /// The rule's identifier
    #[arg(value_name = "RULESET-ID")]
    pub id: Option<String>,

    /// Accepted for gh compatibility; GitFox rules live on repositories
    #[arg(short, long)]
    pub parents: bool,

    /// Open the rule in the browser
    #[arg(short, long)]
    pub web: bool,

    #[arg(short = 'o', id = "ruleset_org", value_name = "ORG", hide = true)]
    pub org: Option<String>,
}

// ---------------------------------------------------------------------------
// codespace
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CodespaceCommand {
    #[command(subcommand)]
    pub command: CodespaceSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum CodespaceSubcommand {
    /// List your gitspaces
    #[command(visible_alias = "ls")]
    List(CodespaceListArgs),
    /// Show a gitspace
    View(CodespaceViewArgs),
    /// Create a gitspace
    Create(CodespaceCreateArgs),
    /// Delete a gitspace
    Delete(CodespaceDeleteArgs),
    /// Stop a running gitspace
    Stop(CodespaceSelectArgs),
    /// Print a gitspace's logs
    Logs(CodespaceLogsArgs),
    #[command(hide = true, disable_help_flag = true)]
    Code(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Cp(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Edit(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Jupyter(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Ports(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Rebuild(GhOnlyArgs),
    #[command(hide = true, disable_help_flag = true)]
    Ssh(GhOnlyArgs),
}

#[derive(Debug, Args)]
pub struct CodespaceListArgs {
    /// Maximum number of gitspaces to return
    #[arg(short = 'L', long, value_name = "N", default_value_t = 30)]
    pub limit: u32,

    /// Open the gitspaces page in the browser
    #[arg(short, long)]
    pub web: bool,

    #[arg(short = 'o', id = "codespace_org", value_name = "ORG", hide = true)]
    pub org: Option<String>,

    #[arg(short = 'u', long, value_name = "USER", hide = true)]
    pub user: Option<String>,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct CodespaceSelectArgs {
    /// The gitspace's identifier
    #[arg(short, long, value_name = "NAME")]
    pub codespace: Option<String>,

    #[arg(long, value_name = "OWNER", hide = true)]
    pub repo_owner: Option<String>,
}

#[derive(Debug, Args)]
pub struct CodespaceViewArgs {
    #[command(flatten)]
    pub select: CodespaceSelectArgs,

    #[command(flatten)]
    pub format: FormatArgs,
}

#[derive(Debug, Args)]
pub struct CodespaceCreateArgs {
    /// Branch to open; defaults to the repository's default branch
    #[arg(short, long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Path of the devcontainer.json to use
    #[arg(long, value_name = "PATH")]
    pub devcontainer_path: Option<String>,

    /// Display name
    #[arg(short, long, value_name = "NAME")]
    pub display_name: Option<String>,

    /// Infrastructure resource to run on, as the instance names it
    #[arg(short, long, value_name = "RESOURCE")]
    pub machine: Option<String>,

    /// IDE to open: vs_code or vs_code_web
    #[arg(long, value_name = "IDE", default_value = "vs_code_web")]
    pub ide: String,

    /// Accepted for gh compatibility; GitFox asks for no extra permissions
    #[arg(long, hide = true)]
    pub default_permissions: bool,

    #[arg(long, value_name = "DURATION", hide = true)]
    pub idle_timeout: Option<String>,

    #[arg(short = 'l', long, value_name = "LOCATION", hide = true)]
    pub location: Option<String>,

    #[arg(long, value_name = "DURATION", hide = true)]
    pub retention_period: Option<String>,

    #[arg(short = 's', long, hide = true)]
    pub status: bool,

    #[arg(short = 'w', long, hide = true)]
    pub web: bool,
}

#[derive(Debug, Args)]
pub struct CodespaceDeleteArgs {
    #[command(flatten)]
    pub select: CodespaceSelectArgs,

    /// Delete every gitspace
    #[arg(long, conflicts_with = "codespace")]
    pub all: bool,

    /// Delete gitspaces not used for this many days
    #[arg(long, value_name = "N")]
    pub days: Option<u64>,

    /// Accepted for gh compatibility
    #[arg(short, long)]
    pub force: bool,

    #[arg(short = 'o', id = "codespace_org", value_name = "ORG", hide = true)]
    pub org: Option<String>,

    #[arg(short = 'u', long, value_name = "USER", hide = true)]
    pub user: Option<String>,
}

#[derive(Debug, Args)]
pub struct CodespaceLogsArgs {
    #[command(flatten)]
    pub select: CodespaceSelectArgs,

    /// Keep following the logs
    #[arg(short, long)]
    pub follow: bool,
}

// ---------------------------------------------------------------------------
// browse, status
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct BrowseArgs {
    /// A pull request number, a file path (with optional :LINE), or a commit SHA
    #[arg(value_name = "NUMBER | PATH | COMMIT")]
    pub target: Option<String>,

    /// Open this branch
    #[arg(short, long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Open a commit; `--commit` alone means the last one
    #[arg(
        short,
        long,
        value_name = "SHA",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "last"
    )]
    pub commit: Option<String>,

    /// Print the URL instead of opening a browser
    #[arg(short, long)]
    pub no_browser: bool,

    /// Open the repository's pipelines
    #[arg(short, long)]
    pub actions: bool,

    /// Open the repository's settings
    #[arg(short, long)]
    pub settings: bool,

    #[arg(long, hide = true)]
    pub blame: bool,

    #[arg(short = 'p', long, hide = true)]
    pub projects: bool,

    #[arg(short = 'r', long, hide = true)]
    pub releases: bool,

    #[arg(short = 'w', long, hide = true)]
    pub wiki: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Space to report on; defaults to --org, then the current repository's
    /// space
    #[arg(id = "space", short = 'o', value_name = "SPACE")]
    pub space: Option<String>,

    /// Repositories to leave out, as space/name
    #[arg(short, long, value_name = "SPACE/NAME", value_delimiter = ',')]
    pub exclude: Vec<String>,
}
