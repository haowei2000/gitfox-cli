//! `gf auth` — login, logout, status, token, switch, setup-git.
//!
//! Tokens are stored in the OS keychain and never written to the config file
//! or logged. Two commands print one, because that is their whole purpose and
//! gh's equivalents do the same: `gf auth token`, and `gf auth status
//! --show-token`. Nothing else ever does.

use std::io::{BufRead, Read};

use serde_json::{Value, json};

use crate::cli::{
    AuthCommand, AuthGitCredentialArgs, AuthLoginArgs, AuthLogoutArgs, AuthSetupGitArgs,
    AuthStatusArgs, AuthSubcommand, AuthSwitchArgs, AuthTokenArgs,
};
use crate::config::{HostConfig, Secret, TokenSource, host_key_of};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::export;
use crate::git;
use crate::keychain;
use crate::output::{Render, key_values};

pub async fn run(cmd: AuthCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        AuthSubcommand::Login(args) => login(args, ctx).await,
        AuthSubcommand::Logout(args) => logout(args, ctx).await,
        AuthSubcommand::Status(args) => status(args, ctx).await,
        AuthSubcommand::Token(args) => token(args, ctx).await,
        AuthSubcommand::Switch(args) => switch(args, ctx),
        AuthSubcommand::SetupGit(args) => setup_git(args, ctx),
        AuthSubcommand::GitCredential(args) => git_credential(args, ctx),
        AuthSubcommand::Refresh(_) => super::gh_only::refuse(
            "auth refresh",
            "a GitFox token's permissions are fixed when it is created; create a new token and `gf auth login --with-token`",
        ),
    }
}

async fn login(args: AuthLoginArgs, ctx: &Context) -> Result<()> {
    if args.web {
        return Err(CliError::unsupported(
            "`--web`",
            "GitFox logs in with an access token, not a browser flow",
        )
        .with_hint("create a token in GitFox, then `gf auth login --with-token < token.txt`"));
    }
    if !args.scopes.is_empty() {
        return Err(CliError::unsupported(
            "`--scopes`",
            "a GitFox token's permissions are fixed when it is created",
        ));
    }
    if args.clipboard {
        return Err(CliError::unsupported(
            "`--clipboard`",
            "there is no one-time device code to copy",
        ));
    }
    if args.insecure_storage {
        return Err(CliError::unsupported(
            "`--insecure-storage`",
            "gf never writes a token to a plain-text file",
        )
        .with_hint("export GITFOX_TOKEN where no keychain is available"));
    }
    let git_protocol = args
        .git_protocol
        .as_deref()
        .map(parse_git_protocol)
        .transpose()?;

    let host = resolve_host(args.hostname.as_deref(), ctx)?;
    let host_key = host_key_of(&host)
        .ok_or_else(|| CliError::config(format!("could not derive a hostname from `{host}`")))?;

    if !args.force
        && keychain::get(&host_key).is_some()
        && ctx.config.token_source != TokenSource::Flag
        && ctx.config.token_source != TokenSource::Env
    {
        ctx.warn(&format!(
            "a token for {host_key} is already stored; it will be replaced"
        ));
    }

    let token = read_token(&args, ctx)?;

    // Validate before storing: a token that does not work is worse than none.
    let client = ctx.client_for(&host, Some(&token))?;
    let user = client.auth().current_user().await?;

    keychain::set(&host_key, token.expose())?;

    let mut file = ctx.config_file.clone();
    let entry = file.hosts.entry(host_key.clone()).or_insert(HostConfig {
        api_url: Some(host.clone()),
        ..Default::default()
    });
    entry.api_url = Some(host.clone());
    entry.user = user.uid.clone().or_else(|| Some(user.label()));
    if ctx.config.insecure {
        entry.insecure = Some(true);
    }
    if let Some(protocol) = git_protocol {
        entry.git_protocol = Some(protocol.to_string());
    }
    file.default_host.get_or_insert(host_key.clone());
    file.save(&ctx.config_path)?;

    ctx.renderer.emit(&LoginResult {
        host: host.clone(),
        host_key,
        user: user.label(),
        config_path: ctx.config_path.display().to_string(),
    })
}

fn parse_git_protocol(raw: &str) -> Result<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "https" | "http" => Ok("https"),
        "ssh" => Ok("ssh"),
        other => Err(CliError::invalid_argument(format!(
            "`{other}` is not a git protocol; expected https or ssh"
        ))),
    }
}

fn read_token(args: &AuthLoginArgs, ctx: &Context) -> Result<Secret> {
    // --token / GITFOX_TOKEN win, so scripted logins never prompt.
    if let Some(token) = &ctx.config.token
        && matches!(
            ctx.config.token_source,
            TokenSource::Flag | TokenSource::Env
        )
    {
        return Ok(token.clone());
    }

    if args.with_token {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| CliError::invalid_argument(format!("could not read stdin: {e}")))?;
        let token = buf.trim().to_string();
        if token.is_empty() {
            return Err(CliError::invalid_argument("no token was provided on stdin"));
        }
        return Ok(Secret::new(token));
    }

    ctx.require_interactive("reading a token")?;
    let entered = dialoguer::Password::new()
        .with_prompt("GitFox token")
        .interact()
        .map_err(|e| CliError::invalid_argument(format!("could not read the token: {e}")))?;
    let entered = entered.trim().to_string();
    if entered.is_empty() {
        return Err(CliError::invalid_argument("no token was provided"));
    }
    Ok(Secret::new(entered))
}

fn resolve_host(explicit: Option<&str>, ctx: &Context) -> Result<String> {
    if let Some(host) = explicit {
        return Ok(host.to_string());
    }
    if let Some(host) = &ctx.config.host {
        return Ok(host.clone());
    }
    ctx.require_interactive("choosing a host")?;
    let entered: String = dialoguer::Input::new()
        .with_prompt("GitFox hostname")
        .interact_text()
        .map_err(|e| CliError::invalid_argument(format!("could not read the hostname: {e}")))?;
    let entered = entered.trim().to_string();
    if entered.is_empty() {
        return Err(CliError::config("no hostname was provided"));
    }
    Ok(entered)
}

async fn logout(args: AuthLogoutArgs, ctx: &Context) -> Result<()> {
    let host = resolve_host(args.hostname.as_deref(), ctx)?;
    let host_key = host_key_of(&host)
        .ok_or_else(|| CliError::config(format!("could not derive a hostname from `{host}`")))?;

    // `-u` guards against logging out the wrong account: gf keeps one per
    // host, so the only question is whether it is that user's.
    if let Some(wanted) = args.user.as_deref() {
        let stored = ctx
            .config_file
            .hosts
            .get(&host_key)
            .and_then(|h| h.user.clone());
        if stored
            .as_deref()
            .is_some_and(|u| !u.eq_ignore_ascii_case(wanted))
        {
            return Err(CliError::new(
                ErrorCode::NotFound,
                format!("not logged in to {host_key} as {wanted}"),
            ));
        }
    }

    let removed = keychain::delete(&host_key)?;

    if ctx.config.token_source == TokenSource::Env {
        ctx.warn(
            "GITFOX_TOKEN is still set in this environment and takes precedence over the keychain",
        );
    }

    ctx.renderer.emit(&LogoutResult { host_key, removed })
}

async fn status(args: AuthStatusArgs, ctx: &Context) -> Result<()> {
    let (host, token, source) = match args.hostname.as_deref() {
        Some(host) => ctx.host_credentials(host),
        None => (
            ctx.host()?.to_string(),
            ctx.config.token.clone(),
            ctx.config.token_source,
        ),
    };
    let host_key = host_key_of(&host).unwrap_or_else(|| host.clone());

    let Some(token) = token else {
        return Err(CliError::new(
            ErrorCode::AuthRequired,
            format!("not logged in to {host_key}"),
        )
        .with_hint("run `gf auth login`, or set GITFOX_TOKEN"));
    };

    let client = ctx.client_for(&host, Some(&token))?;
    // Reaching the host but being rejected is an auth failure, not a generic
    // API error — CI wants exit code 3 here.
    let user = client.auth().current_user().await?;

    let git_protocol = ctx
        .config_file
        .hosts
        .get(&host_key)
        .and_then(|h| h.git_protocol.clone())
        .or_else(|| ctx.config_file.git_protocol.clone())
        .unwrap_or_else(|| "https".to_string());

    ctx.renderer.emit(&StatusResult {
        host,
        host_key,
        user: user.uid.clone().unwrap_or_else(|| user.label()),
        display_name: user.label(),
        token_source: source,
        insecure: ctx.config.insecure,
        git_protocol,
        token: args.show_token.then(|| token.expose().to_string()),
    })
}

async fn token(args: AuthTokenArgs, ctx: &Context) -> Result<()> {
    let (host, token, _) = match args.hostname.as_deref() {
        Some(host) => ctx.host_credentials(host),
        None => (
            ctx.host()?.to_string(),
            ctx.config.token.clone(),
            ctx.config.token_source,
        ),
    };
    let host_key = host_key_of(&host).unwrap_or_else(|| host.clone());
    let token = token.ok_or_else(|| {
        CliError::new(
            ErrorCode::AuthRequired,
            format!("no oauth token found for {host_key}"),
        )
        .with_hint("run `gf auth login`, or set GITFOX_TOKEN")
    })?;

    if let Some(wanted) = args.user.as_deref() {
        let client = ctx.client_for(&host, Some(&token))?;
        let user = client.auth().current_user().await?;
        if user
            .uid
            .as_deref()
            .is_none_or(|uid| !uid.eq_ignore_ascii_case(wanted))
        {
            return Err(CliError::new(
                ErrorCode::NotFound,
                format!("no token found for {wanted} on {host_key}"),
            ));
        }
    }

    ctx.renderer.emit(&TokenResult {
        host_key,
        token: token.expose().to_string(),
    })
}

fn switch(args: AuthSwitchArgs, ctx: &Context) -> Result<()> {
    let mut file = ctx.config_file.clone();
    let known: Vec<String> = file.hosts.keys().cloned().collect();

    let target = match args.hostname.as_deref() {
        Some(host) => host_key_of(host).unwrap_or_else(|| host.to_string()),
        None => {
            let current = file.default_host.clone();
            let others: Vec<&String> = known
                .iter()
                .filter(|h| Some(*h) != current.as_ref())
                .collect();
            match others.as_slice() {
                [only] => (*only).clone(),
                [] => {
                    return Err(CliError::new(
                        ErrorCode::NotFound,
                        "there is no other host to switch to",
                    )
                    .with_hint("log in to another with `gf auth login --hostname …`"));
                }
                many => {
                    return Err(CliError::invalid_argument(format!(
                        "{} hosts to switch to; say which one",
                        many.len()
                    ))
                    .with_hint(format!(
                        "--hostname {}",
                        many.iter()
                            .map(|h| h.as_str())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    )));
                }
            }
        }
    };

    if !file.hosts.contains_key(&target) && keychain::get(&target).is_none() {
        return Err(CliError::new(
            ErrorCode::AuthRequired,
            format!("not logged in to {target}"),
        )
        .with_hint(format!("run `gf auth login --hostname {target}` first")));
    }

    let previous = file.default_host.replace(target.clone());
    file.save(&ctx.config_path)?;
    ctx.renderer.emit(&SwitchResult {
        host_key: target,
        previous,
    })
}

fn setup_git(args: AuthSetupGitArgs, ctx: &Context) -> Result<()> {
    let hosts: Vec<(String, String)> = match args.hostname.as_deref() {
        Some(host) => {
            let (api_url, token, _) = ctx.host_credentials(host);
            if token.is_none() && !args.force {
                let key = host_key_of(host).unwrap_or_else(|| host.to_string());
                return Err(CliError::new(
                    ErrorCode::AuthRequired,
                    format!("you are not logged in to {key}"),
                )
                .with_hint("run `gf auth login` first, or pass --force"));
            }
            vec![(host.to_string(), api_url)]
        }
        None => ctx
            .config_file
            .hosts
            .iter()
            .map(|(key, entry)| {
                (
                    key.clone(),
                    entry.api_url.clone().unwrap_or_else(|| key.clone()),
                )
            })
            .collect(),
    };
    if hosts.is_empty() {
        return Err(CliError::new(
            ErrorCode::AuthRequired,
            "you are not logged in to any GitFox hosts",
        )
        .with_hint("run `gf auth login` first"));
    }

    let exe = std::env::current_exe()
        .map_err(|e| CliError::config(format!("could not find the gf binary: {e}")))?;
    let helper = format!(
        "!{} auth git-credential",
        shell_quote(&exe.display().to_string())
    );

    let mut configured = Vec::new();
    for (_, api_url) in &hosts {
        let base = gitfox_client::client::normalize_host(api_url).map_err(CliError::from)?;
        let origin = base.origin().ascii_serialization();
        git::set_credential_helper(&origin, &helper)
            .map_err(|message| CliError::config(format!("could not configure git: {message}")))?;
        configured.push(origin);
    }

    ctx.renderer.emit(&SetupGitResult { configured })
}

/// The git credential protocol: read `key=value` lines, answer `get` with a
/// username and password for a host gf has a token for.
///
/// git calls this, not a person, so it prints nothing it was not asked for —
/// an unknown host is simply no answer, which makes git try its next helper.
fn git_credential(args: AuthGitCredentialArgs, ctx: &Context) -> Result<()> {
    if args.operation != "get" {
        // `store` and `erase`: gf owns its tokens, git does not get to change
        // them.
        return Ok(());
    }
    let mut fields = std::collections::BTreeMap::new();
    for line in std::io::stdin().lock().lines() {
        let line =
            line.map_err(|e| CliError::invalid_argument(format!("could not read stdin: {e}")))?;
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.insert(key.to_string(), value.to_string());
        }
    }
    let Some(host) = fields.get("host") else {
        return Ok(());
    };
    let protocol = fields
        .get("protocol")
        .map(String::as_str)
        .unwrap_or("https");
    let (_, token, _) = ctx.host_credentials(&format!("{protocol}://{host}"));
    let Some(token) = token else {
        return Ok(());
    };
    let host_key = host_key_of(&format!("{protocol}://{host}")).unwrap_or_else(|| host.clone());
    let username = ctx
        .config_file
        .hosts
        .get(&host_key)
        .and_then(|h| h.user.clone())
        .unwrap_or_else(|| "gf".to_string());
    ctx.renderer.write_str(&format!(
        "protocol={protocol}\nhost={host}\nusername={username}\npassword={}\n",
        token.expose()
    ))
}

fn shell_quote(text: &str) -> String {
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+:".contains(c))
    {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

struct LoginResult {
    host: String,
    host_key: String,
    user: String,
    config_path: String,
}

impl Render for LoginResult {
    fn to_json(&self) -> Value {
        json!({
            "host": self.host,
            "host_key": self.host_key,
            "user": self.user,
            "token_stored": "keyring",
            "config_path": self.config_path,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} Logged in to {} as {}\n  token stored in the OS keychain\n  config written to {}",
            self.host_key, self.user, self.config_path
        )
    }
}

struct LogoutResult {
    host_key: String,
    removed: bool,
}

impl Render for LogoutResult {
    fn to_json(&self) -> Value {
        json!({ "host_key": self.host_key, "removed": self.removed })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        if self.removed {
            format!(
                "{green}✓{reset} Removed the stored token for {}",
                self.host_key
            )
        } else {
            format!("No stored token for {}", self.host_key)
        }
    }
}

struct StatusResult {
    host: String,
    host_key: String,
    /// The login, which is what gh's `login` and gf's filters take.
    user: String,
    display_name: String,
    token_source: TokenSource,
    insecure: bool,
    git_protocol: String,
    /// Only with `--show-token`.
    token: Option<String>,
}

const STATUS_FIELDS: &[&str] = &["hosts"];

impl Render for StatusResult {
    fn to_json(&self) -> Value {
        let mut value = json!({
            "host": self.host,
            "host_key": self.host_key,
            "user": self.display_name,
            "login": self.user,
            "authenticated": true,
            // The value itself is never part of the output unless asked for.
            "token": "configured",
            "token_source": self.token_source.as_str(),
            "insecure": self.insecure,
            "git_protocol": self.git_protocol,
        });
        if let Some(token) = &self.token {
            value["token"] = json!(token);
        }
        value
    }

    fn export_fields(&self) -> &'static [&'static str] {
        STATUS_FIELDS
    }

    /// gh's shape: hosts keyed by name, each a list of accounts.
    fn export(&self, fields: &[String]) -> Value {
        export::select(fields, |_| {
            json!({
                self.host_key.clone(): [{
                    "state": "success",
                    "active": true,
                    "host": self.host_key,
                    "login": self.user,
                    "tokenSource": self.token_source.as_str(),
                    "scopes": "",
                    "gitProtocol": self.git_protocol,
                    "token": self.token.clone().unwrap_or_else(|| "***".to_string()),
                }]
            })
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut pairs = vec![
            ("Host", self.host.clone()),
            ("User", self.display_name.clone()),
            (
                "Token",
                match &self.token {
                    Some(token) => format!("{token} ({})", self.token_source.as_str()),
                    None => format!("configured ({})", self.token_source.as_str()),
                },
            ),
            ("Git protocol", self.git_protocol.clone()),
        ];
        if self.insecure {
            pairs.push(("TLS", "verification disabled".to_string()));
        }
        format!(
            "{green}✓{reset} Authenticated to {}\n{}",
            self.host_key,
            key_values(&pairs)
        )
    }
}

struct TokenResult {
    host_key: String,
    token: String,
}

impl Render for TokenResult {
    fn to_json(&self) -> Value {
        json!({ "host_key": self.host_key, "token": self.token })
    }

    fn to_human(&self, _color: bool) -> String {
        // Bare, so `$(gf auth token)` is the token and nothing else.
        self.token.clone()
    }
}

struct SwitchResult {
    host_key: String,
    previous: Option<String>,
}

impl Render for SwitchResult {
    fn to_json(&self) -> Value {
        json!({ "default_host": self.host_key, "previous": self.previous })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} Switched the default host to {}",
            self.host_key
        )
    }
}

struct SetupGitResult {
    configured: Vec<String>,
}

impl Render for SetupGitResult {
    fn to_json(&self) -> Value {
        json!({ "configured": self.configured })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        self.configured
            .iter()
            .map(|origin| format!("{green}✓{reset} git uses gf for credentials on {origin}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(token: Option<&str>) -> StatusResult {
        StatusResult {
            host: "https://git.example.com".into(),
            host_key: "git.example.com".into(),
            user: "whw".into(),
            display_name: "Haowei".into(),
            token_source: TokenSource::Keyring,
            insecure: false,
            git_protocol: "https".into(),
            token: token.map(str::to_string),
        }
    }

    #[test]
    fn status_output_reports_the_token_without_revealing_it() {
        let result = status(None);
        let json = result.to_json();
        assert_eq!(json["token"], "configured");
        assert_eq!(json["token_source"], "keyring");
        let rendered = format!("{json}{}", result.to_human(false));
        assert!(!rendered.to_lowercase().contains("bearer"), "{rendered}");
        assert_eq!(
            result.export(&["hosts".into()])["hosts"]["git.example.com"][0]["token"],
            "***"
        );
    }

    #[test]
    fn show_token_is_the_one_way_status_reveals_it() {
        let result = status(Some("s3cret"));
        assert_eq!(result.to_json()["token"], "s3cret");
        assert!(result.to_human(false).contains("s3cret"));
        let hosts = result.export(&["hosts".into()]);
        assert_eq!(hosts["hosts"]["git.example.com"][0]["login"], "whw");
        assert_eq!(hosts["hosts"]["git.example.com"][0]["token"], "s3cret");
    }

    #[test]
    fn login_output_says_where_the_token_went_but_not_what_it_is() {
        let result = LoginResult {
            host: "https://git.example.com".into(),
            host_key: "git.example.com".into(),
            user: "whw".into(),
            config_path: "/tmp/config.toml".into(),
        };
        assert_eq!(result.to_json()["token_stored"], "keyring");
        assert!(result.to_human(false).contains("keychain"));
    }

    #[test]
    fn the_token_command_prints_it_bare_for_command_substitution() {
        let result = TokenResult {
            host_key: "git.example.com".into(),
            token: "s3cret".into(),
        };
        assert_eq!(result.to_human(false), "s3cret");
    }

    #[test]
    fn git_protocols_are_the_two_gh_accepts() {
        assert_eq!(parse_git_protocol("SSH").unwrap(), "ssh");
        assert_eq!(parse_git_protocol("https").unwrap(), "https");
        assert!(parse_git_protocol("ftp").is_err());
        assert_eq!(shell_quote("/usr/local/bin/gf"), "/usr/local/bin/gf");
        assert_eq!(shell_quote("/Users/a b/gf"), "'/Users/a b/gf'");
    }
}
