//! `fx secret` — space secrets.
//!
//! GitHub keeps secrets per repository, environment and organisation; GitFox
//! keeps them per space. So the scope is a space: `-o/--org`, else the current
//! repository's. A secret's value is write-only — GitFox never returns it, and
//! nothing here prints one.

use gitfox_client::{GitFoxClient, SpaceSecret};
use serde_json::{Value, json};

use crate::cli::{
    SecretCommand, SecretDeleteArgs, SecretListArgs, SecretScopeArgs, SecretSetArgs,
    SecretSubcommand,
};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::export::{self, time_value};
use crate::interact;
use crate::output::{Render, plain_table, relative_time};
use crate::paginate;

pub async fn run(cmd: SecretCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        SecretSubcommand::List(args) => list(args, ctx).await,
        SecretSubcommand::Set(args) => set(args, ctx).await,
        SecretSubcommand::Delete(args) => delete(args, ctx).await,
    }
}

/// The space the scope flags name, refusing gh scopes GitFox does not have.
fn space(scope: &SecretScopeArgs, ctx: &Context) -> Result<String> {
    if scope.env.is_some() {
        return Err(CliError::unsupported(
            "`--env`",
            "GitFox has no deployment environments; secrets belong to a space",
        ));
    }
    if scope.user {
        return Err(CliError::unsupported(
            "`--user`",
            "GitFox has no user secrets; secrets belong to a space",
        ));
    }
    if scope.app.is_some() {
        return Err(CliError::unsupported(
            "`--app`",
            "GitFox secrets are shared by every pipeline in the space",
        ));
    }
    ctx.space(scope.space.as_deref())
}

fn not_found_as_space(err: gitfox_client::Error, space: &str) -> CliError {
    match err {
        gitfox_client::Error::NotFound { .. } => CliError::new(
            ErrorCode::NotFound,
            format!("no space `{space}`, or you cannot see it"),
        ),
        other => CliError::from(other),
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(args: SecretListArgs, ctx: &Context) -> Result<()> {
    let space = space(&args.scope, ctx)?;
    let client = ctx.client()?;
    let (client_ref, space_ref) = (&client, space.as_str());
    let paged = paginate::collect(1000, move |page, limit| async move {
        client_ref
            .spaces()
            .secrets(space_ref, None, page, limit)
            .await
    })
    .await
    .map_err(|e| not_found_as_space(e, &space))?;

    ctx.renderer.emit(&SecretList {
        space,
        secrets: paged.items,
        truncated: paged.truncated,
    })
}

// ---------------------------------------------------------------------------
// set
// ---------------------------------------------------------------------------

async fn set(args: SecretSetArgs, ctx: &Context) -> Result<()> {
    for (given, flag, why) in [
        (
            args.visibility.is_some(),
            "`--visibility`",
            "a GitFox secret is visible to every pipeline in its space",
        ),
        (
            !args.repos.is_empty() || args.no_repos_selected,
            "selecting repositories",
            "a GitFox secret is visible to every pipeline in its space",
        ),
        (
            args.no_store,
            "`--no-store`",
            "GitFox encrypts secrets on the server, so there is no client-side ciphertext to print",
        ),
    ] {
        if given {
            return Err(CliError::unsupported(flag, why));
        }
    }
    let space = space(&args.scope, ctx)?;

    let entries: Vec<(String, String)> = match (&args.env_file, &args.name) {
        (Some(file), _) => parse_dotenv(&interact::read_source(file)?)?,
        (None, Some(name)) => {
            let value = match &args.body {
                Some(body) => body.clone(),
                None => match interact::piped_stdin()? {
                    Some(piped) => piped.trim_end_matches(['\n', '\r']).to_string(),
                    None => {
                        ctx.require_interactive("a secret value")
                            .map_err(|e| e.with_hint("pass --body, or pipe the value on stdin"))?;
                        dialoguer::Password::new()
                            .with_prompt(format!("Paste your secret for {name}"))
                            .interact()
                            .map_err(|e| {
                                CliError::invalid_argument(format!(
                                    "could not read the secret: {e}"
                                ))
                            })?
                    }
                },
            };
            vec![(name.clone(), value)]
        }
        (None, None) => {
            return Err(CliError::invalid_argument(
                "must pass a secret name, or --env-file",
            ));
        }
    };
    if entries.is_empty() {
        return Err(CliError::invalid_argument("no secrets found to set"));
    }

    let client = ctx.client()?;
    let mut results = Vec::with_capacity(entries.len());
    for (name, value) in &entries {
        let created = upsert(&client, &space, name, value, args.description.as_deref()).await?;
        results.push((name.clone(), created));
    }

    ctx.renderer.emit(&SecretsSet { space, results })
}

/// Update the secret if it exists, create it otherwise. Returns whether it was
/// created.
async fn upsert(
    client: &GitFoxClient,
    space: &str,
    name: &str,
    value: &str,
    description: Option<&str>,
) -> Result<bool> {
    let spaces = client.spaces();
    match spaces.secret(space, name).await {
        Ok(_) => {
            spaces
                .update_secret(space, name, value, description)
                .await?;
            Ok(false)
        }
        Err(gitfox_client::Error::NotFound { .. }) => {
            spaces
                .create_secret(space, name, value, description)
                .await
                .map_err(|e| not_found_as_space(e, space))?;
            Ok(true)
        }
        Err(other) => Err(other.into()),
    }
}

/// `KEY=VALUE` lines, as a `.env` file writes them: comments, blank lines,
/// `export `, and quoted values with the usual escapes.
fn parse_dotenv(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            return Err(CliError::invalid_argument(format!(
                "line {} of the env file is not KEY=VALUE",
                index + 1
            )));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(CliError::invalid_argument(format!(
                "line {} of the env file has an empty name",
                index + 1
            )));
        }
        let value = value.trim();
        let value = if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            inner
                .replace("\\n", "\n")
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
        } else if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
            inner.to_string()
        } else {
            // An unquoted value ends at a ` #` comment.
            value
                .split(" #")
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        out.push((key.to_string(), value));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------

async fn delete(args: SecretDeleteArgs, ctx: &Context) -> Result<()> {
    let space = space(&args.scope, ctx)?;
    let client = ctx.client()?;
    client
        .spaces()
        .delete_secret(&space, &args.name)
        .await
        .map_err(|e| match e {
            gitfox_client::Error::NotFound { .. } => CliError::new(
                ErrorCode::NotFound,
                format!("no secret `{}` in {space}", args.name),
            ),
            other => CliError::from(other),
        })?;
    ctx.renderer.emit(&SecretDeleted {
        space,
        name: args.name,
    })
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

const SECRET_FIELDS: &[&str] = &[
    "name",
    "numSelectedRepos",
    "selectedReposURL",
    "updatedAt",
    "visibility",
    // fx's own names
    "identifier",
    "description",
    "created",
    "updated",
];

struct SecretList {
    space: String,
    secrets: Vec<SpaceSecret>,
    truncated: bool,
}

impl Render for SecretList {
    fn to_json(&self) -> Value {
        json!({
            "space": self.space,
            "count": self.secrets.len(),
            "truncated": self.truncated,
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.secrets
            .iter()
            .map(|s| {
                json!({
                    "identifier": s.identifier,
                    "description": s.description,
                    "created": s.created,
                    "updated": s.updated,
                })
            })
            .collect()
    }

    fn export_fields(&self) -> &'static [&'static str] {
        SECRET_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        Value::Array(
            self.secrets
                .iter()
                .map(|s| {
                    export::select(fields, |field| match field {
                        "name" | "identifier" => json!(s.identifier),
                        "updatedAt" => time_value(s.updated.or(s.created)),
                        "visibility" | "selectedReposURL" => json!(""),
                        "numSelectedRepos" => json!(0),
                        "description" => json!(s.description.clone().unwrap_or_default()),
                        "created" => json!(s.created),
                        "updated" => json!(s.updated),
                        _ => Value::Null,
                    })
                })
                .collect(),
        )
    }

    fn to_human(&self, _color: bool) -> String {
        if self.secrets.is_empty() {
            return format!("no secrets found in {}", self.space);
        }
        let rows: Vec<Vec<String>> = self
            .secrets
            .iter()
            .map(|s| {
                vec![
                    s.identifier.clone(),
                    s.updated
                        .or(s.created)
                        .map(relative_time)
                        .unwrap_or_default(),
                    s.description.clone().unwrap_or_default(),
                ]
            })
            .collect();
        plain_table(&["name", "updated", "description"], &rows)
    }
}

struct SecretsSet {
    space: String,
    /// (name, whether it was newly created)
    results: Vec<(String, bool)>,
}

impl Render for SecretsSet {
    fn to_json(&self) -> Value {
        json!({
            "space": self.space,
            "count": self.results.len(),
            "items": self.results.iter().map(|(name, created)| json!({
                "name": name,
                "action": if *created { "created" } else { "updated" },
            })).collect::<Vec<_>>(),
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        self.results
            .iter()
            .map(|(name, created)| {
                format!(
                    "{green}✓{reset} {} secret {name} for {}",
                    if *created { "Created" } else { "Updated" },
                    self.space
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct SecretDeleted {
    space: String,
    name: String,
}

impl Render for SecretDeleted {
    fn to_json(&self) -> Value {
        json!({ "space": self.space, "name": self.name, "deleted": true })
    }

    fn to_human(&self, color: bool) -> String {
        let (red, reset) = if color {
            ("\x1b[31m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{red}✓{reset} Deleted secret {} from {}",
            self.name, self.space
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_files_parse_the_way_they_are_written() {
        let parsed = parse_dotenv(
            "# deploy\nexport HOST=git.example.com\nTOKEN=\"a b\\nc\"\nRAW='x#y'\nPLAIN=value # note\n\nEMPTY=\n",
        )
        .unwrap();
        assert_eq!(
            parsed,
            vec![
                ("HOST".to_string(), "git.example.com".to_string()),
                ("TOKEN".to_string(), "a b\nc".to_string()),
                ("RAW".to_string(), "x#y".to_string()),
                ("PLAIN".to_string(), "value".to_string()),
                ("EMPTY".to_string(), String::new()),
            ]
        );
        assert!(parse_dotenv("NOVALUE").is_err());
        assert!(parse_dotenv("=x").is_err());
    }

    #[test]
    fn a_listing_never_carries_a_value() {
        let list = SecretList {
            space: "ai".into(),
            secrets: vec![
                serde_json::from_value(json!({
                    "identifier": "deploy-host", "description": "Release target host",
                    "created": 1_788_499_091_707i64
                }))
                .unwrap(),
            ],
            truncated: false,
        };
        let value = list.to_json();
        assert_eq!(value["items"][0]["identifier"], "deploy-host");
        assert!(value["items"][0].get("data").is_none());
        let exported = list.export(&["name".into(), "updatedAt".into()]);
        assert_eq!(exported[0]["name"], "deploy-host");
        assert!(exported[0]["updatedAt"].as_str().unwrap().ends_with('Z'));
    }
}
