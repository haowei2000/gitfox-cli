//! Everything a command needs: resolved configuration, a renderer, and a lazily
//! built API client.

use std::io::Write;
use std::path::PathBuf;

use gitfox_client::GitFoxClient;

use crate::cli::GlobalArgs;
use crate::config::{self, ConfigFile, GitContext, Resolved, Secret, SystemEnv, TokenSource, Tty};
use crate::error::{CliError, Result};
use crate::git::{self, GitInfo};
use crate::keychain;
use crate::output::Renderer;

pub struct Context {
    pub config: Resolved,
    pub config_file: ConfigFile,
    pub config_path: PathBuf,
    pub renderer: Renderer,
    /// What the surrounding checkout said. Empty outside a git repository, and
    /// empty when nothing needed it — see [`Context::build`].
    pub git: GitInfo,
}

impl Context {
    pub fn build(global: &GlobalArgs) -> Result<Self> {
        let env = SystemEnv;
        let config_path = config::config_path(&env, global.config.as_deref())?;
        let config_file = ConfigFile::load(&config_path)?;
        let overrides = global.overrides();
        let tty = Tty::detect();

        // Resolve once without the checkout. The git tier is the last resort,
        // so when the flags, environment and config file already answered, the
        // `git` subprocesses are pure cost — and CI, which sets GITFOX_HOST and
        // GITFOX_REPO, never pays it.
        let mut resolved =
            config::resolve(&overrides, &env, &config_file, &GitContext::default(), tty)?;
        let mut git = GitInfo::default();
        if resolved.host.is_none() || resolved.repo.is_none() {
            git = git::detect();
            resolved = config::resolve(&overrides, &env, &config_file, &git.to_context(), tty)?;
        }

        // The keychain is the last tier of the token chain, and the only one
        // that needs I/O — hence here rather than inside `resolve`.
        if resolved.token.is_none()
            && let Some(host_key) = resolved.host_key.as_deref()
            && let Some(token) = keychain::get(host_key)
        {
            resolved.token = Some(token);
            resolved.token_source = TokenSource::Keyring;
        }

        let renderer = Renderer::new(resolved.output, resolved.color);
        Ok(Self {
            config: resolved,
            config_file,
            config_path,
            renderer,
            git,
        })
    }

    pub fn host(&self) -> Result<&str> {
        self.config.host.as_deref().ok_or_else(|| {
            CliError::config("no GitFox host configured").with_hint(
                "pass --host, set GITFOX_HOST, or run `fx auth login --hostname git.example.com`",
            )
        })
    }

    /// A client for the resolved host. Requires a host; a token is optional so
    /// that anonymous endpoints still work.
    pub fn client(&self) -> Result<GitFoxClient> {
        self.client_for(self.host()?, self.config.token.as_ref())
    }

    /// A client for an explicit host and token — used by `fx auth login`, which
    /// must validate credentials before storing them.
    pub fn client_for(&self, host: &str, token: Option<&Secret>) -> Result<GitFoxClient> {
        if self.config.insecure {
            self.warn(&format!(
                "TLS certificate verification is disabled for {host}"
            ));
        }
        GitFoxClient::builder(host)
            .token(token.map(|t| t.expose().to_string()))
            .timeout_secs(self.config.timeout_secs)
            .retries(self.config.retries)
            .insecure(self.config.insecure)
            .build()
            .map_err(CliError::from)
    }

    /// The repository the command should act on.
    ///
    /// Comes from `-R`, then `GITFOX_REPO`, then the checkout's remote — the
    /// same chain as everything else, resolved in [`crate::config::resolve`].
    pub fn repo(&self) -> Result<gitfox_client::RepoRef> {
        let raw = self.config.repo.as_deref().ok_or_else(|| {
            CliError::new(
                crate::error::ErrorCode::GitContextError,
                "no repository specified and none could be inferred from the current directory",
            )
            .with_hint("pass -R space/name, set GITFOX_REPO, or run fx from inside a checkout")
        })?;
        gitfox_client::RepoRef::parse(raw).map_err(|_| {
            CliError::invalid_argument(format!(
                "`{raw}` is not a repository reference; expected `space/name`"
            ))
        })
    }

    /// The checked-out branch, for commands that default to "the current one".
    pub fn branch(&self) -> Result<&str> {
        self.git.branch.as_deref().ok_or_else(|| {
            CliError::new(
                crate::error::ErrorCode::GitContextError,
                "no branch checked out, or not inside a git repository",
            )
            .with_hint("pass the pull request number explicitly")
        })
    }

    /// Fail instead of blocking when there is nobody at the keyboard.
    pub fn require_interactive(&self, what: &str) -> Result<()> {
        if self.config.non_interactive {
            return Err(CliError::invalid_argument(format!(
                "{what} needs interactive input, but fx is running non-interactively"
            ))
            .with_hint("pass the value as a flag, or use --with-token / GITFOX_TOKEN"));
        }
        Ok(())
    }

    /// Warnings go to stderr in every mode: stdout belongs to the machine
    /// contract and must stay parseable.
    pub fn warn(&self, message: &str) {
        let mut err = std::io::stderr().lock();
        let (yellow, reset) = if self.config.color {
            ("\x1b[33m", "\x1b[0m")
        } else {
            ("", "")
        };
        let _ = writeln!(err, "{yellow}warning{reset}: {message}");
    }
}
