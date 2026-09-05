//! Configuration resolution.
//!
//! The precedence chain is the contract:
//!
//! ```text
//! CLI flag  >  environment variable  >  config file  >  git context  >  default
//! ```
//!
//! [`resolve`] is a pure function over its inputs — no environment reads, no
//! file system, no network — so the whole chain is unit-testable. I/O happens
//! in the callers ([`ConfigFile::load`], [`SystemEnv`], [`crate::context`]).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CliError, ErrorCode, Result};
use crate::output::OutputFormat;

pub const ENV_HOST: &str = "GITFOX_HOST";
pub const ENV_TOKEN: &str = "GITFOX_TOKEN";
pub const ENV_REPO: &str = "GITFOX_REPO";
pub const ENV_ORG: &str = "GITFOX_ORG";
pub const ENV_OUTPUT: &str = "GITFOX_OUTPUT";
pub const ENV_CONFIG: &str = "GITFOX_CONFIG";
pub const ENV_TIMEOUT: &str = "GITFOX_TIMEOUT";
pub const ENV_RETRIES: &str = "GITFOX_RETRIES";
pub const ENV_INSECURE: &str = "GITFOX_INSECURE";
pub const ENV_AGENT: &str = "GITFOX_AGENT";
/// Honoured in addition to `--no-color`; see <https://no-color.org>.
pub const ENV_NO_COLOR: &str = "NO_COLOR";

pub const DEFAULT_TIMEOUT_SECS: u64 = gitfox_client::DEFAULT_TIMEOUT_SECS;
pub const DEFAULT_RETRIES: u32 = gitfox_client::DEFAULT_RETRIES;

/// Service name used for the OS keychain entries.
pub const KEYRING_SERVICE: &str = "fx-gitfox";

// ---------------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------------

/// A token that refuses to print itself.
///
/// `Debug` and `Display` are both redacted, so a stray `{:?}` in a log line or
/// an error message cannot leak credentials.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The only way to read the value. Grep for this to audit every use.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Where the token came from — reported by `fx auth status`, never the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Flag,
    Env,
    Keyring,
    None,
}

impl TokenSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Env => "env",
            Self::Keyring => "keyring",
            Self::None => "none",
        }
    }
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// Reads environment variables. Abstracted so tests never touch the real env
/// (which is process-global and makes parallel tests flaky).
pub trait EnvSource {
    fn get(&self, key: &str) -> Option<String>;

    fn is_set(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
}

pub struct SystemEnv;

impl EnvSource for SystemEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.trim().is_empty())
    }
}

/// In-memory environment for tests.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub struct MapEnv(BTreeMap<String, String>);

#[cfg(test)]
impl MapEnv {
    pub fn new<const N: usize>(pairs: [(&str, &str); N]) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }
}

#[cfg(test)]
impl EnvSource for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned().filter(|v| !v.trim().is_empty())
    }
}

/// Values that came from the command line. All optional: `None` means "not
/// given", which is what lets the next tier of the chain win.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub host: Option<String>,
    pub token: Option<String>,
    pub repo: Option<String>,
    pub org: Option<String>,
    pub output: Option<OutputFormat>,
    pub timeout: Option<u64>,
    pub retries: Option<u32>,
    pub insecure: bool,
    pub agent: bool,
    pub non_interactive: bool,
    pub no_color: bool,
}

/// Repository information inferred from the surrounding git checkout.
///
/// Populated in v0.2 by parsing `git remote get-url origin`; the slot exists
/// now so the precedence chain — and its tests — are already complete.
#[derive(Debug, Default, Clone)]
pub struct GitContext {
    pub host: Option<String>,
    pub repo: Option<String>,
}

/// Whether the process is attached to a terminal. Passed in rather than probed
/// so `resolve` stays pure.
#[derive(Debug, Clone, Copy)]
pub struct Tty {
    pub stdin: bool,
    pub stdout: bool,
}

impl Tty {
    pub fn detect() -> Self {
        use std::io::IsTerminal;
        Self {
            stdin: std::io::stdin().is_terminal(),
            stdout: std::io::stdout().is_terminal(),
        }
    }

    /// A terminal on both ends — the only situation where prompting is sane.
    #[cfg(test)]
    pub fn interactive() -> Self {
        Self {
            stdin: true,
            stdout: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Config file
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_host: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hosts: BTreeMap<String, HostConfig>,
}

/// Per-host settings. Tokens are deliberately absent: they live in the OS
/// keychain or in `GITFOX_TOKEN`, never in a plain-text file.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insecure: Option<bool>,
}

impl ConfigFile {
    /// A missing file is not an error — it just means every tier below the
    /// environment is empty.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                Self::parse(&text).map_err(|e| CliError::config(format!("{}: {e}", path.display())))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(CliError::config(format!("{}: {e}", path.display()))),
        }
    }

    pub fn parse(text: &str) -> std::result::Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CliError::config(format!("{}: {e}", parent.display())))?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| CliError::config(format!("could not serialize config: {e}")))?;
        std::fs::write(path, text).map_err(|e| CliError::config(format!("{}: {e}", path.display())))
    }

    /// The API URL configured for a host key, falling back to the key itself.
    pub fn api_url_for(&self, host_key: &str) -> Option<String> {
        self.hosts
            .get(host_key)
            .and_then(|h| h.api_url.clone())
            .or_else(|| Some(host_key.to_string()))
    }
}

/// `$GITFOX_CONFIG`, else `$XDG_CONFIG_HOME/fx/config.toml`, else
/// `~/.config/fx/config.toml` (and the platform config dir on Windows).
pub fn config_path(env: &dyn EnvSource, cli_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = cli_path {
        return Ok(path.to_path_buf());
    }
    if let Some(path) = env.get(ENV_CONFIG) {
        return Ok(PathBuf::from(path));
    }
    if cfg!(windows) {
        let dirs = directories::BaseDirs::new()
            .ok_or_else(|| CliError::config("could not determine the user config directory"))?;
        return Ok(dirs.config_dir().join("fx").join("config.toml"));
    }
    if let Some(xdg) = env.get("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg).join("fx").join("config.toml"));
    }
    let dirs = directories::BaseDirs::new()
        .ok_or_else(|| CliError::config("could not determine the home directory"))?;
    Ok(dirs
        .home_dir()
        .join(".config")
        .join("fx")
        .join("config.toml"))
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// The fully resolved settings a command runs against.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub host: Option<String>,
    /// Hostname only (`git.example.com`) — the key for config and keychain lookups.
    pub host_key: Option<String>,
    pub token: Option<Secret>,
    pub token_source: TokenSource,
    pub repo: Option<String>,
    pub org: Option<String>,
    pub output: OutputFormat,
    pub timeout_secs: u64,
    pub retries: u32,
    pub insecure: bool,
    pub agent: bool,
    pub non_interactive: bool,
    pub color: bool,
}

/// Apply the precedence chain. Pure: same inputs, same output, no I/O.
pub fn resolve(
    cli: &Overrides,
    env: &dyn EnvSource,
    file: &ConfigFile,
    git: &GitContext,
    tty: Tty,
) -> Result<Resolved> {
    let host = cli
        .host
        .clone()
        .or_else(|| env.get(ENV_HOST))
        .or_else(|| {
            file.default_host
                .as_deref()
                .and_then(|key| file.api_url_for(key))
        })
        .or_else(|| git.host.clone());

    let host_key = host.as_deref().and_then(host_key_of);

    let (token, token_source) = match cli.token.clone().filter(|t| !t.trim().is_empty()) {
        Some(t) => (Some(Secret::new(t)), TokenSource::Flag),
        None => match env.get(ENV_TOKEN) {
            // Keychain lookup needs the host key and does I/O, so it is left to
            // `Context`; `TokenSource::None` here means "keep looking".
            Some(t) => (Some(Secret::new(t)), TokenSource::Env),
            None => (None, TokenSource::None),
        },
    };

    let agent = cli.agent || env.get(ENV_AGENT).is_some_and(|v| parse_bool(&v));

    let output = match cli.output {
        Some(format) => format,
        None => match env.get(ENV_OUTPUT) {
            Some(raw) => raw.parse::<OutputFormat>().map_err(|_| {
                CliError::new(
                    ErrorCode::ConfigError,
                    format!("{ENV_OUTPUT}: expected one of table, json, jsonl; got `{raw}`"),
                )
            })?,
            // Agent mode means "the caller is a machine": JSON is the default.
            None if agent => OutputFormat::Json,
            None => OutputFormat::Table,
        },
    };

    let timeout_secs = match cli.timeout {
        Some(secs) => secs,
        None => match env.get(ENV_TIMEOUT) {
            Some(raw) => raw.parse::<u64>().map_err(|_| {
                CliError::new(
                    ErrorCode::ConfigError,
                    format!("{ENV_TIMEOUT}: expected a number of seconds, got `{raw}`"),
                )
            })?,
            None => DEFAULT_TIMEOUT_SECS,
        },
    };
    if timeout_secs == 0 {
        return Err(CliError::config("timeout must be greater than 0 seconds"));
    }

    let retries = match cli.retries {
        Some(retries) => retries,
        None => match env.get(ENV_RETRIES) {
            Some(raw) => raw.parse::<u32>().map_err(|_| {
                CliError::new(
                    ErrorCode::ConfigError,
                    format!("{ENV_RETRIES}: expected a whole number of retries, got `{raw}`"),
                )
            })?,
            None => DEFAULT_RETRIES,
        },
    };

    let insecure = cli.insecure
        || env.get(ENV_INSECURE).is_some_and(|v| parse_bool(&v))
        || host_key
            .as_deref()
            .and_then(|key| file.hosts.get(key))
            .and_then(|h| h.insecure)
            .unwrap_or(false);

    // Anything that is not a live terminal is non-interactive: a prompt there
    // would hang CI forever instead of failing.
    let non_interactive = cli.non_interactive || agent || !tty.stdin || !tty.stdout;

    let color = !(cli.no_color || agent || env.is_set(ENV_NO_COLOR) || !tty.stdout)
        && output == OutputFormat::Table;

    Ok(Resolved {
        host,
        host_key,
        token,
        token_source,
        repo: cli
            .repo
            .clone()
            .or_else(|| env.get(ENV_REPO))
            .or_else(|| git.repo.clone()),
        org: cli.org.clone().or_else(|| env.get(ENV_ORG)),
        output,
        timeout_secs,
        retries,
        insecure,
        agent,
        non_interactive,
        color,
    })
}

/// `https://git.example.com/x` -> `git.example.com`. Bare hostnames pass through.
pub fn host_key_of(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    let candidate = if host.contains("://") {
        host.to_string()
    } else {
        format!("https://{host}")
    };
    url::Url::parse(&candidate)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

/// Truthiness for boolean environment variables.
///
/// Anything unrecognised is false, so a stray `GITFOX_INSECURE=maybe` does not
/// silently disable TLS verification.
pub fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(cli: Overrides, env: MapEnv, file: ConfigFile, git: GitContext) -> Resolved {
        resolve(&cli, &env, &file, &git, Tty::interactive()).expect("resolve should succeed")
    }

    fn file_with_default(host: &str, api_url: &str) -> ConfigFile {
        let mut file = ConfigFile {
            default_host: Some(host.to_string()),
            ..Default::default()
        };
        file.hosts.insert(
            host.to_string(),
            HostConfig {
                api_url: Some(api_url.to_string()),
                ..Default::default()
            },
        );
        file
    }

    // -- host precedence ---------------------------------------------------

    #[test]
    fn cli_host_beats_env_file_and_git() {
        let r = resolved(
            Overrides {
                host: Some("https://git.cli.com".into()),
                ..Default::default()
            },
            MapEnv::new([(ENV_HOST, "https://git.env.com")]),
            file_with_default("git.example.com", "https://git.example.com"),
            GitContext {
                host: Some("https://git.remote.com".into()),
                ..Default::default()
            },
        );
        assert_eq!(r.host.as_deref(), Some("https://git.cli.com"));
        assert_eq!(r.host_key.as_deref(), Some("git.cli.com"));
    }

    #[test]
    fn env_host_beats_file_and_git() {
        let r = resolved(
            Overrides::default(),
            MapEnv::new([(ENV_HOST, "https://git.env.com")]),
            file_with_default("git.example.com", "https://git.example.com"),
            GitContext {
                host: Some("https://git.remote.com".into()),
                ..Default::default()
            },
        );
        assert_eq!(r.host.as_deref(), Some("https://git.env.com"));
    }

    #[test]
    fn file_default_host_beats_git() {
        let r = resolved(
            Overrides::default(),
            MapEnv::default(),
            file_with_default("git.example.com", "https://git.example.com"),
            GitContext {
                host: Some("https://git.remote.com".into()),
                ..Default::default()
            },
        );
        assert_eq!(r.host.as_deref(), Some("https://git.example.com"));
    }

    #[test]
    fn git_context_is_the_last_resort() {
        let r = resolved(
            Overrides::default(),
            MapEnv::default(),
            ConfigFile::default(),
            GitContext {
                host: Some("https://git.remote.com".into()),
                repo: Some("ai/backend".into()),
            },
        );
        assert_eq!(r.host.as_deref(), Some("https://git.remote.com"));
        assert_eq!(r.repo.as_deref(), Some("ai/backend"));
    }

    #[test]
    fn a_host_without_an_api_url_entry_falls_back_to_the_key() {
        let file = ConfigFile {
            default_host: Some("git.example.com".into()),
            ..Default::default()
        };
        let r = resolved(
            Overrides::default(),
            MapEnv::default(),
            file,
            GitContext::default(),
        );
        assert_eq!(r.host.as_deref(), Some("git.example.com"));
    }

    // -- token precedence --------------------------------------------------

    #[test]
    fn cli_token_beats_env() {
        let r = resolved(
            Overrides {
                token: Some("from-cli".into()),
                ..Default::default()
            },
            MapEnv::new([(ENV_TOKEN, "from-env")]),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert_eq!(r.token.as_ref().map(Secret::expose), Some("from-cli"));
        assert_eq!(r.token_source, TokenSource::Flag);
    }

    #[test]
    fn env_token_is_used_when_no_flag_is_given() {
        let r = resolved(
            Overrides::default(),
            MapEnv::new([(ENV_TOKEN, "from-env")]),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert_eq!(r.token.as_ref().map(Secret::expose), Some("from-env"));
        assert_eq!(r.token_source, TokenSource::Env);
    }

    #[test]
    fn no_token_defers_to_the_keychain() {
        let r = resolved(
            Overrides::default(),
            MapEnv::default(),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert!(r.token.is_none());
        assert_eq!(r.token_source, TokenSource::None);
    }

    #[test]
    fn secrets_are_redacted_in_debug_and_display() {
        let secret = Secret::new("super-secret-token");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert_eq!(format!("{secret}"), "***");
        let resolved = resolved(
            Overrides {
                token: Some("super-secret-token".into()),
                ..Default::default()
            },
            MapEnv::default(),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert!(!format!("{resolved:?}").contains("super-secret-token"));
    }

    // -- repo / org --------------------------------------------------------

    #[test]
    fn repo_precedence_is_cli_then_env_then_git() {
        let git = GitContext {
            repo: Some("git/repo".into()),
            ..Default::default()
        };
        let with_cli = resolved(
            Overrides {
                repo: Some("cli/repo".into()),
                ..Default::default()
            },
            MapEnv::new([(ENV_REPO, "env/repo")]),
            ConfigFile::default(),
            git.clone(),
        );
        assert_eq!(with_cli.repo.as_deref(), Some("cli/repo"));

        let with_env = resolved(
            Overrides::default(),
            MapEnv::new([(ENV_REPO, "env/repo")]),
            ConfigFile::default(),
            git.clone(),
        );
        assert_eq!(with_env.repo.as_deref(), Some("env/repo"));

        let with_git = resolved(
            Overrides::default(),
            MapEnv::default(),
            ConfigFile::default(),
            git,
        );
        assert_eq!(with_git.repo.as_deref(), Some("git/repo"));
    }

    // -- output ------------------------------------------------------------

    #[test]
    fn output_defaults_to_a_table_for_humans() {
        let r = resolved(
            Overrides::default(),
            MapEnv::default(),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert_eq!(r.output, OutputFormat::Table);
        assert!(r.color);
        assert!(!r.non_interactive);
    }

    #[test]
    fn agent_mode_implies_json_no_color_and_no_prompts() {
        let r = resolved(
            Overrides {
                agent: true,
                ..Default::default()
            },
            MapEnv::default(),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert_eq!(r.output, OutputFormat::Json);
        assert!(!r.color);
        assert!(r.non_interactive);
    }

    #[test]
    fn gitfox_agent_env_var_enables_agent_mode() {
        for value in ["1", "true", "YES", "on"] {
            let r = resolved(
                Overrides::default(),
                MapEnv::new([(ENV_AGENT, value)]),
                ConfigFile::default(),
                GitContext::default(),
            );
            assert!(r.agent, "expected `{value}` to enable agent mode");
        }
    }

    #[test]
    fn explicit_output_still_wins_over_agent_mode() {
        let r = resolved(
            Overrides {
                agent: true,
                output: Some(OutputFormat::Table),
                ..Default::default()
            },
            MapEnv::default(),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert_eq!(r.output, OutputFormat::Table);
    }

    #[test]
    fn env_output_beats_the_default_but_not_the_flag() {
        let from_env = resolved(
            Overrides::default(),
            MapEnv::new([(ENV_OUTPUT, "jsonl")]),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert_eq!(from_env.output, OutputFormat::Jsonl);

        let from_flag = resolved(
            Overrides {
                output: Some(OutputFormat::Json),
                ..Default::default()
            },
            MapEnv::new([(ENV_OUTPUT, "jsonl")]),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert_eq!(from_flag.output, OutputFormat::Json);
    }

    #[test]
    fn an_unparseable_output_env_var_is_a_config_error() {
        let err = resolve(
            &Overrides::default(),
            &MapEnv::new([(ENV_OUTPUT, "yaml")]),
            &ConfigFile::default(),
            &GitContext::default(),
            Tty::interactive(),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::ConfigError);
        assert_eq!(err.exit_code(), 7);
    }

    // -- terminal detection ------------------------------------------------

    #[test]
    fn a_pipe_disables_color_and_prompts() {
        let r = resolve(
            &Overrides::default(),
            &MapEnv::default(),
            &ConfigFile::default(),
            &GitContext::default(),
            Tty {
                stdin: false,
                stdout: false,
            },
        )
        .unwrap();
        assert!(!r.color);
        assert!(r.non_interactive);
        // Piping does not change the *format*: humans still pipe tables around.
        assert_eq!(r.output, OutputFormat::Table);
    }

    #[test]
    fn no_color_env_var_is_honoured() {
        let r = resolved(
            Overrides::default(),
            MapEnv::new([(ENV_NO_COLOR, "1")]),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert!(!r.color);
    }

    // -- timeout / insecure ------------------------------------------------

    #[test]
    fn timeout_precedence_and_default() {
        assert_eq!(
            resolved(
                Overrides::default(),
                MapEnv::default(),
                ConfigFile::default(),
                GitContext::default()
            )
            .timeout_secs,
            DEFAULT_TIMEOUT_SECS
        );
        assert_eq!(
            resolved(
                Overrides::default(),
                MapEnv::new([(ENV_TIMEOUT, "5")]),
                ConfigFile::default(),
                GitContext::default()
            )
            .timeout_secs,
            5
        );
        assert_eq!(
            resolved(
                Overrides {
                    timeout: Some(9),
                    ..Default::default()
                },
                MapEnv::new([(ENV_TIMEOUT, "5")]),
                ConfigFile::default(),
                GitContext::default()
            )
            .timeout_secs,
            9
        );
    }

    #[test]
    fn retries_follow_the_same_chain_as_everything_else() {
        assert_eq!(
            resolved(
                Overrides::default(),
                MapEnv::default(),
                ConfigFile::default(),
                GitContext::default()
            )
            .retries,
            DEFAULT_RETRIES
        );
        assert_eq!(
            resolved(
                Overrides::default(),
                MapEnv::new([(ENV_RETRIES, "5")]),
                ConfigFile::default(),
                GitContext::default()
            )
            .retries,
            5
        );
        assert_eq!(
            resolved(
                Overrides {
                    retries: Some(0),
                    ..Default::default()
                },
                MapEnv::new([(ENV_RETRIES, "5")]),
                ConfigFile::default(),
                GitContext::default()
            )
            .retries,
            0
        );
    }

    #[test]
    fn a_bad_retry_count_is_a_config_error() {
        let err = resolve(
            &Overrides::default(),
            &MapEnv::new([(ENV_RETRIES, "lots")]),
            &ConfigFile::default(),
            &GitContext::default(),
            Tty::interactive(),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::ConfigError);
    }

    #[test]
    fn a_bad_timeout_is_a_config_error() {
        for bad in ["abc", "-1"] {
            let err = resolve(
                &Overrides::default(),
                &MapEnv::new([(ENV_TIMEOUT, bad)]),
                &ConfigFile::default(),
                &GitContext::default(),
                Tty::interactive(),
            )
            .unwrap_err();
            assert_eq!(err.code, ErrorCode::ConfigError, "for `{bad}`");
        }
    }

    #[test]
    fn insecure_can_come_from_the_flag_the_env_or_the_host_entry() {
        let mut file = ConfigFile::default();
        file.hosts.insert(
            "git.internal.local".into(),
            HostConfig {
                api_url: Some("https://git.internal.local".into()),
                insecure: Some(true),
                ..Default::default()
            },
        );

        assert!(
            resolved(
                Overrides {
                    insecure: true,
                    ..Default::default()
                },
                MapEnv::default(),
                ConfigFile::default(),
                GitContext::default()
            )
            .insecure
        );
        assert!(
            resolved(
                Overrides::default(),
                MapEnv::new([(ENV_INSECURE, "true")]),
                ConfigFile::default(),
                GitContext::default()
            )
            .insecure
        );
        assert!(
            resolved(
                Overrides {
                    host: Some("https://git.internal.local".into()),
                    ..Default::default()
                },
                MapEnv::default(),
                file,
                GitContext::default()
            )
            .insecure
        );
    }

    #[test]
    fn unrecognised_boolean_values_are_false() {
        for value in ["maybe", "0", "false", "no", "off", ""] {
            assert!(!parse_bool(value), "expected `{value}` to be false");
        }
    }

    // -- misc --------------------------------------------------------------

    #[test]
    fn host_key_strips_scheme_port_and_path() {
        assert_eq!(
            host_key_of("https://git.example.com:8443/base").as_deref(),
            Some("git.example.com")
        );
        assert_eq!(
            host_key_of("git.example.com").as_deref(),
            Some("git.example.com")
        );
        assert_eq!(host_key_of("   "), None);
    }

    #[test]
    fn empty_env_values_are_treated_as_unset() {
        let r = resolved(
            Overrides::default(),
            MapEnv::new([(ENV_HOST, "   "), (ENV_TOKEN, "")]),
            ConfigFile::default(),
            GitContext::default(),
        );
        assert!(r.host.is_none());
        assert!(r.token.is_none());
    }

    #[test]
    fn config_file_round_trips_and_never_holds_a_token() {
        let file = file_with_default("git.example.com", "https://git.example.com");
        let text = toml::to_string_pretty(&file).unwrap();
        assert!(!text.to_lowercase().contains("token"), "{text}");
        let parsed = ConfigFile::parse(&text).unwrap();
        assert_eq!(parsed.default_host.as_deref(), Some("git.example.com"));
    }

    #[test]
    fn cli_config_path_beats_the_env_var() {
        let env = MapEnv::new([(ENV_CONFIG, "/from/env.toml")]);
        assert_eq!(
            config_path(&env, Some(Path::new("/from/cli.toml"))).unwrap(),
            PathBuf::from("/from/cli.toml")
        );
        assert_eq!(
            config_path(&env, None).unwrap(),
            PathBuf::from("/from/env.toml")
        );
    }
}
