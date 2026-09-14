//! `fx ssh-key` — the SSH keys on your GitFox account.

use gitfox_client::PublicKey;
use serde_json::{Value, json};

use crate::cli::{SshKeyAddArgs, SshKeyCommand, SshKeyDeleteArgs, SshKeySubcommand};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::interact;
use crate::output::{Render, plain_table, relative_time};

pub async fn run(cmd: SshKeyCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        SshKeySubcommand::List => list(ctx).await,
        SshKeySubcommand::Add(args) => add(args, ctx).await,
        SshKeySubcommand::Delete(args) => delete(args, ctx).await,
    }
}

async fn list(ctx: &Context) -> Result<()> {
    let client = ctx.client()?;
    let paged = crate::paginate::collect(1000, |page, limit| {
        let client = &client;
        async move { client.spaces().public_keys(page, limit).await }
    })
    .await?;
    ctx.renderer.emit(&KeyList(paged.items))
}

async fn add(args: SshKeyAddArgs, ctx: &Context) -> Result<()> {
    match args.key_type.as_str() {
        "authentication" | "auth" => {}
        "signing" => {
            return Err(CliError::unsupported(
                "`--type signing`",
                "GitFox keys authenticate git over SSH; it has no signing keys",
            ));
        }
        other => {
            return Err(CliError::invalid_argument(format!(
                "`{other}` is not a key type; expected authentication or signing"
            )));
        }
    }

    let (content, file_name) = match args.key_file.as_deref() {
        Some(path) if path.as_os_str() == "-" => (interact::read_source("-")?, None),
        Some(path) => (
            std::fs::read_to_string(path).map_err(|e| {
                CliError::invalid_argument(format!("could not read {}: {e}", path.display()))
            })?,
            path.file_stem().map(|s| s.to_string_lossy().into_owned()),
        ),
        None => match interact::piped_stdin()? {
            Some(text) => (text, None),
            None => {
                return Err(CliError::invalid_argument(
                    "public key file missing; pass a path, or pipe the key on stdin",
                ));
            }
        },
    };
    let content = content.trim().to_string();
    let mut words = content.split_whitespace();
    let (Some(kind), Some(_blob)) = (words.next(), words.next()) else {
        return Err(CliError::invalid_argument(
            "that does not look like an SSH public key",
        ));
    };
    if !kind.starts_with("ssh-") && !kind.starts_with("ecdsa-") && !kind.starts_with("sk-") {
        return Err(
            CliError::invalid_argument(format!("`{kind}` is not an SSH public key type"))
                .with_hint("pass the .pub file, not the private key"),
        );
    }
    let comment = words.collect::<Vec<_>>().join(" ");

    let identifier = args
        .title
        .clone()
        .or_else(|| (!comment.is_empty()).then(|| comment.clone()))
        .or(file_name)
        .map(|name| sanitize(&name))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            CliError::invalid_argument("the key needs a name").with_hint("pass --title")
        })?;

    let client = ctx.client()?;
    let key = client
        .spaces()
        .add_public_key(&identifier, &content)
        .await?;
    ctx.renderer.emit(&KeyAdded(key))
}

/// GitFox identifiers allow letters, digits, `-`, `_` and `.`.
fn sanitize(name: &str) -> String {
    let mapped: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    mapped.trim_matches('-').to_string()
}

async fn delete(args: SshKeyDeleteArgs, ctx: &Context) -> Result<()> {
    if !args.yes {
        interact::confirm(ctx, &format!("Delete the SSH key {}?", args.id), "--yes")?;
    }
    let client = ctx.client()?;
    client
        .spaces()
        .delete_public_key(&args.id)
        .await
        .map_err(|e| match e {
            gitfox_client::Error::NotFound { .. } => {
                CliError::new(ErrorCode::NotFound, format!("no SSH key named {}", args.id))
            }
            other => CliError::from(other),
        })?;
    ctx.renderer.emit(&KeyDeleted(args.id))
}

fn key_json(key: &PublicKey) -> Value {
    json!({
        "id": key.identifier,
        "title": key.identifier,
        "fingerprint": key.fingerprint,
        "type": key.key_type,
        "usage": key.usage,
        "comment": key.comment,
        "created": key.created,
        "verified": key.verified,
    })
}

struct KeyList(Vec<PublicKey>);

impl Render for KeyList {
    fn to_json(&self) -> Value {
        json!({ "count": self.0.len(), "items": self.to_jsonl() })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.0.iter().map(key_json).collect()
    }

    fn to_human(&self, _color: bool) -> String {
        if self.0.is_empty() {
            return "No SSH keys present in the GitFox account.".to_string();
        }
        let rows: Vec<Vec<String>> = self
            .0
            .iter()
            .map(|k| {
                vec![
                    k.identifier.clone(),
                    k.fingerprint.clone().unwrap_or_default(),
                    k.key_type.clone().unwrap_or_default(),
                    "authentication".to_string(),
                    k.created.map(relative_time).unwrap_or_default(),
                ]
            })
            .collect();
        plain_table(
            &["title", "fingerprint", "key type", "type", "added"],
            &rows,
        )
    }
}

struct KeyAdded(PublicKey);

impl Render for KeyAdded {
    fn to_json(&self) -> Value {
        key_json(&self.0)
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} Public key added to your account as {}",
            self.0.identifier
        )
    }
}

struct KeyDeleted(String);

impl Render for KeyDeleted {
    fn to_json(&self) -> Value {
        json!({ "id": self.0, "deleted": true })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} SSH key {} deleted from your account",
            self.0
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_name_is_made_into_a_gitfox_identifier() {
        assert_eq!(sanitize("epichust@skillhub"), "epichust-skillhub");
        assert_eq!(sanitize("  my laptop  "), "my-laptop");
        assert_eq!(sanitize("id_ed25519.pub"), "id_ed25519.pub");
    }
}
