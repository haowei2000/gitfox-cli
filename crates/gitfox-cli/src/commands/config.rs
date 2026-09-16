//! `gf config` — inspect and edit the config file.
//!
//! The keys include gh's (`git_protocol`, `editor`, `browser`, `pager`,
//! `prompt`), and `-h HOST` scopes one to a host the way `gh config --host`
//! does. Tokens are not addressable here by design: `gf config set` cannot
//! write a credential into a plain-text file even by accident.

use serde_json::{Value, json};

use crate::cli::{ConfigCommand, ConfigGetArgs, ConfigListArgs, ConfigSetArgs, ConfigSubcommand};
use crate::config::{ConfigFile, host_key_of, parse_bool};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::output::{Render, key_values, plain_table};

pub fn run(cmd: ConfigCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        ConfigSubcommand::Get(args) => get(args, ctx),
        ConfigSubcommand::Set(args) => set(args, ctx),
        ConfigSubcommand::List(args) => list(args, ctx),
        ConfigSubcommand::ClearCache => ctx.renderer.emit(&ClearedCache),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Key {
    DefaultHost,
    Top(TopField),
    Host { host: String, field: HostField },
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TopField {
    GitProtocol,
    Editor,
    Browser,
    Pager,
    Prompt,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum HostField {
    ApiUrl,
    User,
    Insecure,
    GitProtocol,
}

const KNOWN_KEYS: &str = "default_host, git_protocol, editor, browser, pager, prompt, \
    hosts.<host>.api_url, hosts.<host>.user, hosts.<host>.insecure, hosts.<host>.git_protocol";

fn host_field(name: &str) -> Option<HostField> {
    Some(match name {
        "api_url" => HostField::ApiUrl,
        "user" => HostField::User,
        "insecure" => HostField::Insecure,
        "git_protocol" => HostField::GitProtocol,
        _ => return None,
    })
}

/// `default_host`, a top-level key, or `hosts.<host>.<field>` — or, with a
/// host scope, a bare host field.
///
/// Hostnames contain dots, so the field is split off the right-hand side.
fn parse_key(raw: &str, scope: Option<&str>) -> Result<Key> {
    let raw = raw.trim();
    if let Some(scope) = scope {
        let host = host_key_of(scope).unwrap_or_else(|| scope.to_string());
        return match host_field(raw) {
            Some(field) => Ok(Key::Host { host, field }),
            None => Err(unknown_key(raw).with_hint(
                "with -h/--host, the key is one of: api_url, user, insecure, git_protocol",
            )),
        };
    }
    match raw {
        "default_host" => return Ok(Key::DefaultHost),
        "git_protocol" => return Ok(Key::Top(TopField::GitProtocol)),
        "editor" => return Ok(Key::Top(TopField::Editor)),
        "browser" => return Ok(Key::Top(TopField::Browser)),
        "pager" => return Ok(Key::Top(TopField::Pager)),
        "prompt" => return Ok(Key::Top(TopField::Prompt)),
        _ => {}
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
    let field = host_field(field).ok_or_else(|| unknown_key(raw))?;
    Ok(Key::Host {
        host: host.to_string(),
        field,
    })
}

fn unknown_key(raw: &str) -> CliError {
    CliError::invalid_argument(format!("unknown config key `{raw}`"))
        .with_hint(format!("known keys: {KNOWN_KEYS}"))
}

fn lookup(file: &ConfigFile, key: &Key) -> Option<String> {
    match key {
        Key::DefaultHost => file.default_host.clone(),
        Key::Top(field) => match field {
            TopField::GitProtocol => file.git_protocol.clone(),
            TopField::Editor => file.editor.clone(),
            TopField::Browser => file.browser.clone(),
            TopField::Pager => file.pager.clone(),
            TopField::Prompt => file.prompt.clone(),
        },
        Key::Host { host, field } => file.hosts.get(host).and_then(|entry| match field {
            HostField::ApiUrl => entry.api_url.clone(),
            HostField::User => entry.user.clone(),
            HostField::Insecure => entry.insecure.map(|v| v.to_string()),
            HostField::GitProtocol => entry.git_protocol.clone(),
        }),
    }
}

/// What gh reports for a key nobody set.
fn default_for(key: &Key) -> Option<&'static str> {
    match key {
        Key::Top(TopField::GitProtocol)
        | Key::Host {
            field: HostField::GitProtocol,
            ..
        } => Some("https"),
        Key::Top(TopField::Prompt) => Some("enabled"),
        _ => None,
    }
}

/// Reject a value gh would reject, so a typo cannot quietly disable prompts
/// or pick a protocol nothing understands.
fn validate(key: &Key, value: &str) -> Result<String> {
    let lower = value.trim().to_ascii_lowercase();
    match key {
        Key::Top(TopField::GitProtocol)
        | Key::Host {
            field: HostField::GitProtocol,
            ..
        } => match lower.as_str() {
            "https" | "ssh" => Ok(lower),
            _ => Err(CliError::invalid_argument(format!(
                "invalid value `{value}` for git_protocol; expected https or ssh"
            ))),
        },
        Key::Top(TopField::Prompt) => match lower.as_str() {
            "enabled" | "disabled" => Ok(lower),
            _ => Err(CliError::invalid_argument(format!(
                "invalid value `{value}` for prompt; expected enabled or disabled"
            ))),
        },
        _ => Ok(value.to_string()),
    }
}

fn scope<'a>(explicit: Option<&'a str>, ctx: &'a Context) -> Option<&'a str> {
    explicit.or(ctx.host_flag.as_deref())
}

fn get(args: ConfigGetArgs, ctx: &Context) -> Result<()> {
    let key = parse_key(&args.key, scope(args.scope.as_deref(), ctx))?;
    let value = lookup(&ctx.config_file, &key)
        .or_else(|| default_for(&key).map(str::to_string))
        .ok_or_else(|| {
            CliError::new(
                ErrorCode::NotFound,
                format!("`{}` is not set in {}", args.key, ctx.config_path.display()),
            )
        })?;
    ctx.renderer.emit(&Entry {
        key: args.key,
        value,
    })
}

fn set(args: ConfigSetArgs, ctx: &Context) -> Result<()> {
    let key = parse_key(&args.key, scope(args.scope.as_deref(), ctx))?;
    let value = validate(&key, &args.value)?;
    let mut file = ctx.config_file.clone();
    match &key {
        Key::DefaultHost => file.default_host = Some(value.clone()),
        Key::Top(field) => {
            let slot = match field {
                TopField::GitProtocol => &mut file.git_protocol,
                TopField::Editor => &mut file.editor,
                TopField::Browser => &mut file.browser,
                TopField::Pager => &mut file.pager,
                TopField::Prompt => &mut file.prompt,
            };
            *slot = Some(value.clone());
        }
        Key::Host { host, field } => {
            let entry = file.hosts.entry(host.clone()).or_default();
            match field {
                HostField::ApiUrl => entry.api_url = Some(value.clone()),
                HostField::User => entry.user = Some(value.clone()),
                HostField::Insecure => entry.insecure = Some(parse_bool(&value)),
                HostField::GitProtocol => entry.git_protocol = Some(value.clone()),
            }
        }
    }
    file.save(&ctx.config_path)?;
    if matches!(key, Key::Top(TopField::Pager)) {
        // Saved so a gh setup script runs, but saying nothing would let it look
        // like it took effect.
        ctx.warn("gf does not page its output; `pager` is saved but has no effect");
    }
    ctx.renderer.emit(&Entry {
        key: args.key,
        value: lookup(&file, &key).unwrap_or_default(),
    })
}

fn list(args: ConfigListArgs, ctx: &Context) -> Result<()> {
    let scope =
        scope(args.scope.as_deref(), ctx).map(|s| host_key_of(s).unwrap_or_else(|| s.to_string()));
    let file = &ctx.config_file;
    ctx.renderer.emit(&Listing {
        config_path: ctx.config_path.display().to_string(),
        default_host: file.default_host.clone(),
        settings: vec![
            (
                "git_protocol",
                file.git_protocol.clone().unwrap_or_else(|| "https".into()),
            ),
            ("editor", file.editor.clone().unwrap_or_default()),
            ("browser", file.browser.clone().unwrap_or_default()),
            ("pager", file.pager.clone().unwrap_or_default()),
            (
                "prompt",
                file.prompt.clone().unwrap_or_else(|| "enabled".into()),
            ),
        ],
        hosts: file
            .hosts
            .iter()
            .filter(|(key, _)| scope.as_deref().is_none_or(|s| s == key.as_str()))
            .map(|(key, entry)| HostRow {
                host: key.clone(),
                api_url: entry.api_url.clone().unwrap_or_default(),
                user: entry.user.clone().unwrap_or_default(),
                insecure: entry.insecure.unwrap_or(false),
                git_protocol: entry.git_protocol.clone(),
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

struct HostRow {
    host: String,
    api_url: String,
    user: String,
    insecure: bool,
    git_protocol: Option<String>,
}

struct Listing {
    config_path: String,
    default_host: Option<String>,
    settings: Vec<(&'static str, String)>,
    hosts: Vec<HostRow>,
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
        let mut value = json!({
            "config_path": self.config_path,
            "default_host": self.default_host,
            "hosts": self.hosts.iter().map(|h| json!({
                "host": h.host,
                "api_url": h.api_url,
                "user": h.user,
                "insecure": h.insecure,
                "git_protocol": h.git_protocol,
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
        });
        for (key, setting) in &self.settings {
            value[*key] = json!(setting);
        }
        value
    }

    fn to_human(&self, _color: bool) -> String {
        let mut resolved = vec![
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
        ];
        for (key, setting) in &self.settings {
            if !setting.is_empty() {
                resolved.push((key, setting.clone()));
            }
        }
        let resolved = key_values(&resolved);
        if self.hosts.is_empty() {
            return resolved;
        }
        let rows = self
            .hosts
            .iter()
            .map(|h| {
                let is_default = self.default_host.as_deref() == Some(h.host.as_str());
                vec![
                    if is_default {
                        format!("{} *", h.host)
                    } else {
                        h.host.clone()
                    },
                    h.api_url.clone(),
                    h.user.clone(),
                    h.insecure.to_string(),
                    h.git_protocol.clone().unwrap_or_default(),
                ]
            })
            .collect::<Vec<_>>();
        format!(
            "{resolved}\n\n{}",
            plain_table(
                &["host", "api url", "user", "insecure", "git protocol"],
                &rows
            )
        )
    }
}

struct ClearedCache;

impl Render for ClearedCache {
    fn to_json(&self) -> Value {
        json!({ "cleared": true, "entries": 0 })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!("{green}✓{reset} Cleared the cache (gf keeps none)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HostConfig;

    #[test]
    fn parses_the_known_keys() {
        assert_eq!(parse_key("default_host", None).unwrap(), Key::DefaultHost);
        assert_eq!(
            parse_key("git_protocol", None).unwrap(),
            Key::Top(TopField::GitProtocol)
        );
        assert_eq!(
            parse_key("hosts.git.example.com.api_url", None).unwrap(),
            Key::Host {
                host: "git.example.com".into(),
                field: HostField::ApiUrl
            }
        );
        assert_eq!(
            parse_key("hosts.localhost.insecure", None).unwrap(),
            Key::Host {
                host: "localhost".into(),
                field: HostField::Insecure
            }
        );
    }

    #[test]
    fn a_host_scope_turns_a_bare_field_into_that_hosts_key() {
        assert_eq!(
            parse_key("git_protocol", Some("https://git.example.com")).unwrap(),
            Key::Host {
                host: "git.example.com".into(),
                field: HostField::GitProtocol
            }
        );
        assert!(parse_key("editor", Some("git.example.com")).is_err());
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
            let err = parse_key(bad, None).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument, "for `{bad}`");
        }
        assert!(parse_key("token", Some("git.example.com")).is_err());
    }

    #[test]
    fn values_gh_would_reject_are_rejected() {
        let protocol = Key::Top(TopField::GitProtocol);
        assert_eq!(validate(&protocol, "SSH").unwrap(), "ssh");
        assert!(validate(&protocol, "ftp").is_err());
        assert!(validate(&Key::Top(TopField::Prompt), "maybe").is_err());
        assert_eq!(default_for(&protocol), Some("https"));
        assert_eq!(default_for(&Key::DefaultHost), None);
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
                git_protocol: Some("ssh".into()),
            },
        );
        let key = parse_key("hosts.git.example.com.api_url", None).unwrap();
        assert_eq!(
            lookup(&file, &key).as_deref(),
            Some("https://git.example.com")
        );
        let key = parse_key("hosts.git.example.com.insecure", None).unwrap();
        assert_eq!(lookup(&file, &key).as_deref(), Some("true"));
        let key = parse_key("git_protocol", Some("git.example.com")).unwrap();
        assert_eq!(lookup(&file, &key).as_deref(), Some("ssh"));
        let key = parse_key("default_host", None).unwrap();
        assert_eq!(lookup(&file, &key), None);
    }
}
