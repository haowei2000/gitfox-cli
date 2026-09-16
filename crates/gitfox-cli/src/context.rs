//! Everything a command needs: resolved configuration, a renderer, and a lazily
//! built API client.

use std::io::Write;
use std::path::PathBuf;

use gitfox_client::{GitFoxClient, RepoRef};

use crate::cli::GlobalArgs;
use crate::config::{
    self, ConfigFile, GitContext, Resolved, Secret, SystemEnv, TokenSource, Tty, host_key_of,
};
use crate::error::{CliError, ErrorCode, Result};
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
    /// `--host` exactly as given on the command line. `gf config` reads it as
    /// the host to scope a setting to, the way gh's `config --host` works.
    pub host_flag: Option<String>,
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
            host_flag: global.host.clone(),
        })
    }

    pub fn host(&self) -> Result<&str> {
        self.config.host.as_deref().ok_or_else(|| {
            CliError::config("no GitFox host configured").with_hint(
                "pass --host, set GITFOX_HOST, or run `gf auth login --hostname git.example.com`",
            )
        })
    }

    /// A client for the resolved host. Requires a host; a token is optional so
    /// that anonymous endpoints still work.
    pub fn client(&self) -> Result<GitFoxClient> {
        self.client_for(self.host()?, self.config.token.as_ref())
    }

    /// A client for an explicit host and token — used by `gf auth login`, which
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

    /// The API URL and token for a host other than the resolved one, as
    /// `--hostname` flags name it. A token from `--token` or `GITFOX_TOKEN`
    /// still wins; otherwise the keychain entry for that host.
    pub fn host_credentials(&self, host: &str) -> (String, Option<Secret>, TokenSource) {
        let host_key = host_key_of(host).unwrap_or_else(|| host.to_string());
        let api_url = if host.contains("://") {
            host.to_string()
        } else {
            self.config_file
                .api_url_for(&host_key)
                .unwrap_or_else(|| host.to_string())
        };
        if matches!(
            self.config.token_source,
            TokenSource::Flag | TokenSource::Env
        ) {
            return (api_url, self.config.token.clone(), self.config.token_source);
        }
        if Some(host_key.as_str()) == self.config.host_key.as_deref() {
            return (api_url, self.config.token.clone(), self.config.token_source);
        }
        match keychain::get(&host_key) {
            Some(token) => (api_url, Some(token), TokenSource::Keyring),
            None => (api_url, None, TokenSource::None),
        }
    }

    /// The repository the command should act on.
    ///
    /// Comes from `-R`, then `GITFOX_REPO`, then the checkout's remote — the
    /// same chain as everything else, resolved in [`crate::config::resolve`].
    pub fn repo(&self) -> Result<RepoRef> {
        let raw = self.config.repo.as_deref().ok_or_else(|| {
            CliError::new(
                ErrorCode::GitContextError,
                "no repository specified and none could be inferred from the current directory",
            )
            .with_hint("pass -R space/name, set GITFOX_REPO, or run gf from inside a checkout")
        })?;
        parse_repo(raw)
    }

    /// The space a space-scoped command acts on: an explicit one, then
    /// `--org` / `GITFOX_ORG`, then the current repository's space.
    pub fn space(&self, explicit: Option<&str>) -> Result<String> {
        if let Some(space) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
            return Ok(space.trim_matches('/').to_string());
        }
        if let Some(org) = &self.config.org {
            return Ok(org.clone());
        }
        if let Some(repo) = self
            .config
            .repo
            .as_deref()
            .and_then(|r| RepoRef::parse(r).ok())
        {
            return Ok(repo.space().to_string());
        }
        Err(CliError::new(
            ErrorCode::GitContextError,
            "no space specified and none could be inferred from the current directory",
        )
        .with_hint("pass -o SPACE or --org SPACE, or run gf from inside a checkout"))
    }

    /// The checked-out branch, for commands that default to "the current one".
    pub fn branch(&self) -> Result<&str> {
        self.git.branch.as_deref().ok_or_else(|| {
            CliError::new(
                ErrorCode::GitContextError,
                "no branch checked out, or not inside a git repository",
            )
            .with_hint("pass the pull request number explicitly")
        })
    }

    /// A page of the GitFox web UI on the resolved host, e.g.
    /// `ai/backend/pulls/12`.
    pub fn web_url(&self, path: &str) -> Result<String> {
        let base = gitfox_client::client::normalize_host(self.host()?).map_err(CliError::from)?;
        let path = gitfox_client::repo::encode_path(path.trim_start_matches('/'));
        base.join(&path)
            .map(|url| url.to_string())
            .map_err(|e| CliError::config(format!("could not build a web URL: {e}")))
    }

    /// Fail instead of blocking when there is nobody at the keyboard.
    pub fn require_interactive(&self, what: &str) -> Result<()> {
        if self.config.non_interactive {
            return Err(CliError::invalid_argument(format!(
                "{what} needs interactive input, but gf is running non-interactively"
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

    /// A progress note for people, on stderr; silent when a machine is reading.
    pub fn note(&self, message: &str) {
        if self.renderer.is_machine() {
            return;
        }
        let _ = writeln!(std::io::stderr().lock(), "{message}");
    }
}

/// `space/name`, or an argument error naming what was given.
pub fn parse_repo(raw: &str) -> Result<RepoRef> {
    RepoRef::parse(raw).map_err(|_| {
        CliError::invalid_argument(format!(
            "`{raw}` is not a repository reference; expected `space/name`"
        ))
    })
}

/// Resolve a login (or a bare principal id) to a principal id.
///
/// GitFox filters and reviewer endpoints take numeric ids, but nobody knows
/// their colleagues by id — so a login costs one lookup. `@me` is the caller.
pub async fn principal_id(client: &GitFoxClient, login: &str) -> Result<i64> {
    let login = login.trim();
    if let Ok(id) = login.parse::<i64>() {
        return Ok(id);
    }
    if login == "@me" {
        let me = client.auth().current_user().await?;
        return me.id.ok_or_else(|| {
            CliError::new(
                ErrorCode::ApiError,
                "GitFox did not report an id for the current user",
            )
        });
    }
    let principal = client
        .principals()
        .find_by_login(login)
        .await?
        .ok_or_else(|| CliError::new(ErrorCode::NotFound, format!("no user matches `{login}`")))?;
    principal.id.ok_or_else(|| {
        CliError::new(
            ErrorCode::ApiError,
            format!("`{login}` resolved to a user with no id"),
        )
    })
}
