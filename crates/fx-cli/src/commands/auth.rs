//! `fx auth` — login, logout, status.
//!
//! Tokens are stored in the OS keychain and never written to the config file,
//! never logged, and never printed back. `fx auth status` reports only whether
//! a token exists and where it came from.

use std::io::Read;

use serde_json::{Value, json};

use crate::cli::{AuthCommand, AuthHostArgs, AuthLoginArgs, AuthSubcommand};
use crate::config::{HostConfig, Secret, TokenSource, host_key_of};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::keychain;
use crate::output::{Render, key_values};

pub async fn run(cmd: AuthCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        AuthSubcommand::Login(args) => login(args, ctx).await,
        AuthSubcommand::Logout(args) => logout(args, ctx),
        AuthSubcommand::Status(args) => status(args, ctx).await,
    }
}

async fn login(args: AuthLoginArgs, ctx: &Context) -> Result<()> {
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
    file.default_host.get_or_insert(host_key.clone());
    file.save(&ctx.config_path)?;

    ctx.renderer
        .emit(&LoginResult {
            host: host.clone(),
            host_key,
            user: user.label(),
            config_path: ctx.config_path.display().to_string(),
        })
        .map_err(unexpected)
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

fn logout(args: AuthHostArgs, ctx: &Context) -> Result<()> {
    let host = resolve_host(args.hostname.as_deref(), ctx)?;
    let host_key = host_key_of(&host)
        .ok_or_else(|| CliError::config(format!("could not derive a hostname from `{host}`")))?;
    let removed = keychain::delete(&host_key)?;

    if ctx.config.token_source == TokenSource::Env {
        ctx.warn(
            "GITFOX_TOKEN is still set in this environment and takes precedence over the keychain",
        );
    }

    ctx.renderer
        .emit(&LogoutResult { host_key, removed })
        .map_err(unexpected)
}

async fn status(args: AuthHostArgs, ctx: &Context) -> Result<()> {
    let host = match args.hostname.as_deref() {
        Some(host) => host.to_string(),
        None => ctx.host()?.to_string(),
    };
    let host_key = host_key_of(&host).unwrap_or_else(|| host.clone());

    let Some(token) = ctx.config.token.as_ref() else {
        return Err(CliError::new(
            ErrorCode::AuthRequired,
            format!("not logged in to {host_key}"),
        )
        .with_hint("run `fx auth login`, or set GITFOX_TOKEN"));
    };

    let client = ctx.client_for(&host, Some(token))?;
    let user = client.auth().current_user().await.map_err(|e| {
        // Reaching the host but being rejected is an auth failure, not a
        // generic API error — CI wants exit code 3 here.
        CliError::from(e)
    })?;

    ctx.renderer
        .emit(&StatusResult {
            host,
            host_key,
            user: user.label(),
            token_source: ctx.config.token_source,
            insecure: ctx.config.insecure,
        })
        .map_err(unexpected)
}

fn unexpected(err: std::io::Error) -> CliError {
    CliError::new(ErrorCode::Unexpected, err.to_string())
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
    user: String,
    token_source: TokenSource,
    insecure: bool,
}

impl Render for StatusResult {
    fn to_json(&self) -> Value {
        json!({
            "host": self.host,
            "host_key": self.host_key,
            "user": self.user,
            "authenticated": true,
            // The value itself is never part of any output.
            "token": "configured",
            "token_source": self.token_source.as_str(),
            "insecure": self.insecure,
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
            ("User", self.user.clone()),
            (
                "Token",
                format!("configured ({})", self.token_source.as_str()),
            ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_output_reports_the_token_without_revealing_it() {
        let result = StatusResult {
            host: "https://git.example.com".into(),
            host_key: "git.example.com".into(),
            user: "whw".into(),
            token_source: TokenSource::Keyring,
            insecure: false,
        };
        let json = result.to_json();
        assert_eq!(json["token"], "configured");
        assert_eq!(json["token_source"], "keyring");
        let rendered = format!("{json}{}", result.to_human(false));
        assert!(!rendered.to_lowercase().contains("bearer"), "{rendered}");
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
}
