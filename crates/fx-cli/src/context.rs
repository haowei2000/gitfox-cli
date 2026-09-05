//! Everything a command needs: resolved configuration, a renderer, and a lazily
//! built API client.

use std::io::Write;
use std::path::PathBuf;

use gitfox_client::GitFoxClient;

use crate::cli::GlobalArgs;
use crate::config::{self, ConfigFile, GitContext, Resolved, Secret, SystemEnv, TokenSource, Tty};
use crate::error::{CliError, Result};
use crate::keychain;
use crate::output::Renderer;

pub struct Context {
    pub config: Resolved,
    pub config_file: ConfigFile,
    pub config_path: PathBuf,
    pub renderer: Renderer,
}

impl Context {
    pub fn build(global: &GlobalArgs) -> Result<Self> {
        let env = SystemEnv;
        let config_path = config::config_path(&env, global.config.as_deref())?;
        let config_file = ConfigFile::load(&config_path)?;
        // Populated in v0.2 by inspecting the surrounding git checkout.
        let git = GitContext::default();

        let mut resolved =
            config::resolve(&global.overrides(), &env, &config_file, &git, Tty::detect())?;

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
            .insecure(self.config.insecure)
            .build()
            .map_err(CliError::from)
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
