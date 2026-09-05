//! `fx config` — inspect and edit the config file.
//!
//! Tokens are not addressable here by design: `fx config set` cannot write a
//! credential into a plain-text file even by accident.

use serde_json::{Value, json};

use crate::cli::{ConfigCommand, ConfigGetArgs, ConfigSetArgs, ConfigSubcommand};
use crate::config::{ConfigFile, parse_bool};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::output::{Render, key_values, plain_table};

pub fn run(cmd: ConfigCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        ConfigSubcommand::Get(args) => get(args, ctx),
        ConfigSubcommand::Set(args) => set(args, ctx),
        ConfigSubcommand::List => list(ctx),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Key {
    DefaultHost,
    Host { host: String, field: HostField },
}

#[derive(Debug, PartialEq, Eq)]
enum HostField {
    ApiUrl,
    User,
    Insecure,
}

/// `default_host` or `hosts.<host>.<field>`.
///
/// Hostnames contain dots, so the field is split off the right-hand side.
fn parse_key(raw: &str) -> Result<Key> {
    let raw = raw.trim();
    if raw == "default_host" {
        return Ok(Key::DefaultHost);
    }
    let Some(rest) = raw.strip_prefix("hosts.") else {
        return Err(unknown_key(raw));
    };
    let Some((host, field)) = rest.rsplit_once('.') else {
        return Err(unknown_key(raw));
    };
    if host.is_empty() {
        return Err(unknown_key(raw));
    }
    let field = match field {
        "api_url" => HostField::ApiUrl,
        "user" => HostField::User,
        "insecure" => HostField::Insecure,
        _ => return Err(unknown_key(raw)),
    };
    Ok(Key::Host {
        host: host.to_string(),
        field,
    })
}

fn unknown_key(raw: &str) -> CliError {
    CliError::invalid_argument(format!("unknown config key `{raw}`")).with_hint(
        "known keys: default_host, hosts.<host>.api_url, hosts.<host>.user, hosts.<host>.insecure",
    )
}

fn lookup(file: &ConfigFile, key: &Key) -> Option<String> {
    match key {
        Key::DefaultHost => file.default_host.clone(),
        Key::Host { host, field } => file.hosts.get(host).and_then(|entry| match field {
            HostField::ApiUrl => entry.api_url.clone(),
            HostField::User => entry.user.clone(),
            HostField::Insecure => entry.insecure.map(|v| v.to_string()),
        }),
    }
}

fn get(args: ConfigGetArgs, ctx: &Context) -> Result<()> {
    let key = parse_key(&args.key)?;
    let value = lookup(&ctx.config_file, &key).ok_or_else(|| {
        CliError::new(
            ErrorCode::NotFound,
            format!("`{}` is not set in {}", args.key, ctx.config_path.display()),
        )
    })?;
    ctx.renderer
        .emit(&Entry {
            key: args.key,
            value,
        })
        .map_err(unexpected)
}

fn set(args: ConfigSetArgs, ctx: &Context) -> Result<()> {
    let key = parse_key(&args.key)?;
    let mut file = ctx.config_file.clone();
    match &key {
        Key::DefaultHost => file.default_host = Some(args.value.clone()),
        Key::Host { host, field } => {
            let entry = file.hosts.entry(host.clone()).or_default();
            match field {
                HostField::ApiUrl => entry.api_url = Some(args.value.clone()),
                HostField::User => entry.user = Some(args.value.clone()),
                HostField::Insecure => entry.insecure = Some(parse_bool(&args.value)),
            }
        }
    }
    file.save(&ctx.config_path)?;
    ctx.renderer
        .emit(&Entry {
            key: args.key,
            value: lookup(&file, &key).unwrap_or_default(),
        })
        .map_err(unexpected)
}

fn list(ctx: &Context) -> Result<()> {
    ctx.renderer
        .emit(&Listing {
            config_path: ctx.config_path.display().to_string(),
            default_host: ctx.config_file.default_host.clone(),
            hosts: ctx
                .config_file
                .hosts
                .iter()
                .map(|(key, entry)| {
                    (
                        key.clone(),
                        entry.api_url.clone().unwrap_or_default(),
                        entry.user.clone().unwrap_or_default(),
                        entry.insecure.unwrap_or(false),
                    )
                })
                .collect(),
            resolved_host: ctx.config.host.clone(),
            resolved_repo: ctx.config.repo.clone(),
            resolved_org: ctx.config.org.clone(),
            output: ctx.config.output.as_str(),
            timeout_secs: ctx.config.timeout_secs,
            insecure: ctx.config.insecure,
            agent: ctx.config.agent,
            token_source: ctx.config.token_source.as_str(),
        })
        .map_err(unexpected)
}

fn unexpected(err: std::io::Error) -> CliError {
    CliError::new(ErrorCode::Unexpected, err.to_string())
}

struct Entry {
    key: String,
    value: String,
}

impl Render for Entry {
    fn to_json(&self) -> Value {
        json!({ "key": self.key, "value": self.value })
    }

    fn to_human(&self, _color: bool) -> String {
        self.value.clone()
    }
}

struct Listing {
    config_path: String,
    default_host: Option<String>,
    hosts: Vec<(String, String, String, bool)>,
    resolved_host: Option<String>,
    resolved_repo: Option<String>,
    resolved_org: Option<String>,
    output: &'static str,
    timeout_secs: u64,
    insecure: bool,
    agent: bool,
    token_source: &'static str,
}

impl Render for Listing {
    fn to_json(&self) -> Value {
        json!({
            "config_path": self.config_path,
            "default_host": self.default_host,
            "hosts": self.hosts.iter().map(|(key, api_url, user, insecure)| json!({
                "host": key,
                "api_url": api_url,
                "user": user,
                "insecure": insecure,
            })).collect::<Vec<_>>(),
            "resolved": {
                "host": self.resolved_host,
                "repo": self.resolved_repo,
                "org": self.resolved_org,
                "output": self.output,
                "timeout_secs": self.timeout_secs,
                "insecure": self.insecure,
                "agent": self.agent,
                "token_source": self.token_source,
            },
        })
    }

    fn to_human(&self, _color: bool) -> String {
        let resolved = key_values(&[
            ("config", self.config_path.clone()),
            (
                "host",
                self.resolved_host
                    .clone()
                    .unwrap_or_else(|| "(unset)".into()),
            ),
            (
                "repo",
                self.resolved_repo
                    .clone()
                    .unwrap_or_else(|| "(unset)".into()),
            ),
            (
                "org",
                self.resolved_org
                    .clone()
                    .unwrap_or_else(|| "(unset)".into()),
            ),
            ("token", format!("from {}", self.token_source)),
            ("output", self.output.to_string()),
            ("timeout", format!("{}s", self.timeout_secs)),
            ("insecure", self.insecure.to_string()),
            ("agent", self.agent.to_string()),
        ]);
        if self.hosts.is_empty() {
            return resolved;
        }
        let rows = self
            .hosts
            .iter()
            .map(|(key, api_url, user, insecure)| {
                let is_default = self.default_host.as_deref() == Some(key.as_str());
                vec![
                    if is_default {
                        format!("{key} *")
                    } else {
                        key.clone()
                    },
                    api_url.clone(),
                    user.clone(),
                    insecure.to_string(),
                ]
            })
            .collect::<Vec<_>>();
        format!(
            "{resolved}\n\n{}",
            plain_table(&["host", "api url", "user", "insecure"], &rows)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HostConfig;

    #[test]
    fn parses_the_known_keys() {
        assert_eq!(parse_key("default_host").unwrap(), Key::DefaultHost);
        assert_eq!(
            parse_key("hosts.git.example.com.api_url").unwrap(),
            Key::Host {
                host: "git.example.com".into(),
                field: HostField::ApiUrl
            }
        );
        assert_eq!(
            parse_key("hosts.localhost.insecure").unwrap(),
            Key::Host {
                host: "localhost".into(),
                field: HostField::Insecure
            }
        );
    }

    #[test]
    fn rejects_unknown_keys_including_anything_token_shaped() {
        for bad in [
            "token",
            "hosts.git.example.com.token",
            "hosts..api_url",
            "hosts.git.example.com",
            "nonsense",
        ] {
            let err = parse_key(bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument, "for `{bad}`");
        }
    }

    #[test]
    fn lookup_reads_back_what_was_written() {
        let mut file = ConfigFile::default();
        file.hosts.insert(
            "git.example.com".into(),
            HostConfig {
                api_url: Some("https://git.example.com".into()),
                user: Some("whw".into()),
                insecure: Some(true),
            },
        );
        let key = parse_key("hosts.git.example.com.api_url").unwrap();
        assert_eq!(
            lookup(&file, &key).as_deref(),
            Some("https://git.example.com")
        );
        let key = parse_key("hosts.git.example.com.insecure").unwrap();
        assert_eq!(lookup(&file, &key).as_deref(), Some("true"));
        let key = parse_key("default_host").unwrap();
        assert_eq!(lookup(&file, &key), None);
    }
}
