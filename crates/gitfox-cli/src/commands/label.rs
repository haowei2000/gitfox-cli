//! `fx label` — repository labels.
//!
//! GitFox labels differ from GitHub's in two ways the commands absorb: a label
//! has a key (and may carry values, `priority:high`), and its colour is one of
//! a fixed set of names. A hex colour from a gh script is mapped to the
//! nearest name, with a warning saying which.

use gitfox_client::{GitFoxClient, Label, LabelInput, RepoRef, label_color, label_color_hex};
use serde_json::{Value, json};

use crate::cli::{
    LabelCloneArgs, LabelCommand, LabelCreateArgs, LabelDeleteArgs, LabelEditArgs, LabelListArgs,
    LabelSort, LabelSubcommand, SortOrder,
};
use crate::context::{Context, parse_repo};
use crate::error::{CliError, ErrorCode, Result};
use crate::export::{self, time_value};
use crate::interact;
use crate::output::{Render, plain_table};
use crate::paginate;

pub async fn run(cmd: LabelCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        LabelSubcommand::List(args) => list(args, ctx).await,
        LabelSubcommand::Create(args) => create(args, ctx).await,
        LabelSubcommand::Edit(args) => edit(args, ctx).await,
        LabelSubcommand::Delete(args) => delete(args, ctx).await,
        LabelSubcommand::Clone(args) => clone(args, ctx).await,
    }
}

/// Every label on the repository, its own and its space's.
async fn all_labels(client: &GitFoxClient, repo: &RepoRef, inherited: bool) -> Result<Vec<Label>> {
    let paged = paginate::collect(1000, |page, limit| async move {
        client
            .labels()
            .list(repo, None, inherited, page, limit)
            .await
    })
    .await
    .map_err(|e| super::pr::not_found_as_repo(e, repo))?;
    Ok(paged.items)
}

fn find<'a>(labels: &'a [Label], key: &str) -> Option<&'a Label> {
    labels.iter().find(|l| l.key.eq_ignore_ascii_case(key))
}

/// A colour GitFox accepts, from a name or a hex triplet.
fn color(ctx: &Context, input: Option<&str>) -> Result<Option<String>> {
    let Some(input) = input else {
        return Ok(None);
    };
    let name = label_color(input).ok_or_else(|| {
        CliError::invalid_argument(format!("`{input}` is not a colour")).with_hint(format!(
            "use a hex colour or one of: {}",
            gitfox_client::LABEL_COLORS
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    if !input.trim().eq_ignore_ascii_case(name) {
        ctx.warn(&format!(
            "GitFox labels use named colours; {input} is drawn as `{name}`"
        ));
    }
    Ok(Some(name.to_string()))
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn list(args: LabelListArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    if args.web {
        return interact::open_in_browser(
            ctx,
            &ctx.web_url(&format!("{}/settings/labels", repo.full()))?,
        );
    }
    let client = ctx.client()?;
    let mut labels = all_labels(&client, &repo, true).await?;

    if let Some(search) = args.search.as_deref().map(str::to_lowercase) {
        labels.retain(|l| {
            l.key.to_lowercase().contains(&search)
                || l.description
                    .as_deref()
                    .is_some_and(|d| d.to_lowercase().contains(&search))
        });
    }
    match args.sort {
        LabelSort::Name => labels.sort_by_key(|l| l.key.to_lowercase()),
        LabelSort::Created => labels.sort_by_key(|l| (l.created.unwrap_or(0), l.id)),
    }
    if args.order == SortOrder::Desc {
        labels.reverse();
    }
    let truncated = labels.len() > args.limit as usize;
    labels.truncate(args.limit as usize);

    ctx.renderer.emit(&LabelList {
        repo: repo.full(),
        url: ctx
            .web_url(&format!("{}/settings/labels", repo.full()))
            .unwrap_or_default(),
        labels,
        truncated,
    })
}

// ---------------------------------------------------------------------------
// create / edit / delete
// ---------------------------------------------------------------------------

async fn create(args: LabelCreateArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let color = color(ctx, args.color.as_deref())?;
    let existing = all_labels(&client, &repo, false).await?;

    let (label, verb) = match find(&existing, &args.name) {
        Some(found) if !args.force => {
            return Err(CliError::invalid_argument(format!(
                "label with name \"{}\" already exists",
                found.key
            ))
            .with_hint("use --force to update its colour and description"));
        }
        Some(found) => (
            client
                .labels()
                .update(
                    &repo,
                    &found.key,
                    &LabelInput {
                        key: None,
                        description: args.description.clone(),
                        color,
                        kind: None,
                    },
                )
                .await?,
            "Updated",
        ),
        None => (
            client
                .labels()
                .define(
                    &repo,
                    &LabelInput {
                        key: Some(args.name.clone()),
                        description: args.description.clone(),
                        color,
                        kind: Some("static".to_string()),
                    },
                )
                .await?,
            "Created",
        ),
    };
    ctx.renderer.emit(&LabelChanged {
        verb,
        repo: repo.full(),
        label,
    })
}

async fn edit(args: LabelEditArgs, ctx: &Context) -> Result<()> {
    if args.color.is_none() && args.description.is_none() && args.new_name.is_none() {
        return Err(CliError::invalid_argument(
            "specify at least one of `--color`, `--description`, or `--name`",
        ));
    }
    let repo = ctx.repo()?;
    let client = ctx.client()?;
    let color = color(ctx, args.color.as_deref())?;
    let labels = all_labels(&client, &repo, true).await?;
    let found = find(&labels, &args.name).ok_or_else(|| {
        CliError::new(
            ErrorCode::NotFound,
            format!("label \"{}\" not found in {repo}", args.name),
        )
    })?;
    if found.repo_id.is_none() {
        return Err(CliError::invalid_argument(format!(
            "label \"{}\" is defined on the space, not on {repo}",
            found.key
        ))
        .with_hint("edit it in the space's label settings"));
    }
    let label = client
        .labels()
        .update(
            &repo,
            &found.key,
            &LabelInput {
                key: args.new_name.clone(),
                description: args.description.clone(),
                color,
                kind: None,
            },
        )
        .await?;
    ctx.renderer.emit(&LabelChanged {
        verb: "Updated",
        repo: repo.full(),
        label,
    })
}

async fn delete(args: LabelDeleteArgs, ctx: &Context) -> Result<()> {
    let repo = ctx.repo()?;
    if !args.yes {
        interact::confirm(
            ctx,
            &format!("Delete label {} in {repo}?", args.name),
            "--yes",
        )?;
    }
    let client = ctx.client()?;
    client
        .labels()
        .delete(&repo, &args.name)
        .await
        .map_err(|e| match e {
            gitfox_client::Error::NotFound { .. } => CliError::new(
                ErrorCode::NotFound,
                format!("label \"{}\" not found in {repo}", args.name),
            ),
            other => CliError::from(other),
        })?;
    ctx.renderer.emit(&LabelDeleted {
        repo: repo.full(),
        name: args.name,
    })
}

// ---------------------------------------------------------------------------
// clone
// ---------------------------------------------------------------------------

async fn clone(args: LabelCloneArgs, ctx: &Context) -> Result<()> {
    let destination = ctx.repo()?;
    let source = parse_repo(&args.source)?;
    let client = ctx.client()?;
    let from = all_labels(&client, &source, false).await?;
    let existing = all_labels(&client, &destination, false).await?;

    let (mut created, mut updated, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for label in &from {
        let input = LabelInput {
            key: Some(label.key.clone()),
            description: label.description.clone(),
            color: label.color.clone(),
            kind: label.kind.clone(),
        };
        match find(&existing, &label.key) {
            Some(found) if args.force => {
                client
                    .labels()
                    .update(&destination, &found.key, &LabelInput { key: None, ..input })
                    .await?;
                updated.push(label.key.clone());
            }
            Some(_) => skipped.push(label.key.clone()),
            None => {
                client.labels().define(&destination, &input).await?;
                created.push(label.key.clone());
            }
        }
    }

    ctx.renderer.emit(&LabelsCloned {
        source: source.full(),
        destination: destination.full(),
        created,
        updated,
        skipped,
    })
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

const LABEL_FIELDS: &[&str] = &[
    "color",
    "createdAt",
    "description",
    "id",
    "isDefault",
    "name",
    "updatedAt",
    "url",
    // fx's own names
    "key",
    "type",
    "scope",
    "value_count",
];

fn label_json(label: &Label) -> Value {
    json!({
        "id": label.id,
        "key": label.key,
        "description": label.description,
        "color": label.color,
        "type": label.kind,
        "scope": if label.repo_id.is_some() { "repository" } else { "space" },
        "value_count": label.value_count,
        "created": label.created,
        "updated": label.updated,
    })
}

struct LabelList {
    repo: String,
    url: String,
    labels: Vec<Label>,
    truncated: bool,
}

impl Render for LabelList {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.repo,
            "count": self.labels.len(),
            "truncated": self.truncated,
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.labels.iter().map(label_json).collect()
    }

    fn export_fields(&self) -> &'static [&'static str] {
        LABEL_FIELDS
    }

    fn export(&self, fields: &[String]) -> Value {
        Value::Array(
            self.labels
                .iter()
                .map(|l| {
                    export::select(fields, |field| match field {
                        "color" => json!(
                            l.color
                                .as_deref()
                                .and_then(label_color_hex)
                                .unwrap_or_default()
                        ),
                        "createdAt" => time_value(l.created),
                        "updatedAt" => time_value(l.updated),
                        "description" => json!(l.description.clone().unwrap_or_default()),
                        "id" => json!(l.id.to_string()),
                        "isDefault" => json!(false),
                        "name" | "key" => json!(l.key),
                        "url" => json!(self.url),
                        other => label_json(l).get(other).cloned().unwrap_or(Value::Null),
                    })
                })
                .collect(),
        )
    }

    fn to_human(&self, _color: bool) -> String {
        if self.labels.is_empty() {
            return format!("no labels found in {}", self.repo);
        }
        let rows: Vec<Vec<String>> = self
            .labels
            .iter()
            .map(|l| {
                vec![
                    l.key.clone(),
                    l.description.clone().unwrap_or_default(),
                    l.color.clone().unwrap_or_default(),
                    if l.repo_id.is_some() {
                        "repository"
                    } else {
                        "space"
                    }
                    .to_string(),
                ]
            })
            .collect();
        let mut out = plain_table(&["name", "description", "color", "scope"], &rows);
        if self.truncated {
            out.push_str(&format!(
                "\n\nShowing {} of more; raise --limit to see the rest.",
                self.labels.len()
            ));
        }
        out
    }
}

struct LabelChanged {
    verb: &'static str,
    repo: String,
    label: Label,
}

impl Render for LabelChanged {
    fn to_json(&self) -> Value {
        let mut value = label_json(&self.label);
        value["action"] = json!(self.verb.to_ascii_lowercase());
        value["repository"] = json!(self.repo);
        value
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} {} label \"{}\" in {}",
            self.verb, self.label.key, self.repo
        )
    }
}

struct LabelDeleted {
    repo: String,
    name: String,
}

impl Render for LabelDeleted {
    fn to_json(&self) -> Value {
        json!({ "repository": self.repo, "name": self.name, "deleted": true })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} Label \"{}\" deleted from {}",
            self.name, self.repo
        )
    }
}

struct LabelsCloned {
    source: String,
    destination: String,
    created: Vec<String>,
    updated: Vec<String>,
    skipped: Vec<String>,
}

impl Render for LabelsCloned {
    fn to_json(&self) -> Value {
        json!({
            "source": self.source,
            "destination": self.destination,
            "created": self.created,
            "updated": self.updated,
            "skipped": self.skipped,
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut out = format!(
            "{green}✓{reset} Cloned {} labels from {} to {}",
            self.created.len() + self.updated.len(),
            self.source,
            self.destination
        );
        if !self.skipped.is_empty() {
            out.push_str(&format!(
                "\n  skipped {} that already exist (use --force to overwrite): {}",
                self.skipped.len(),
                self.skipped.join(", ")
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(key: &str, color: &str, repo_label: bool) -> Label {
        serde_json::from_value(json!({
            "id": 3, "key": key, "color": color, "description": "Something is broken",
            "type": "static", "repo_id": if repo_label { Some(54) } else { None }
        }))
        .unwrap()
    }

    #[test]
    fn label_fields_speak_ghs_vocabulary() {
        let list = LabelList {
            repo: "ai/backend".into(),
            url: "http://h/ai/backend/settings/labels".into(),
            labels: vec![label("bug", "red", true)],
            truncated: false,
        };
        let exported = list.export(&["name".into(), "color".into(), "id".into(), "scope".into()]);
        assert_eq!(
            exported[0],
            json!({ "color": "ef4444", "id": "3", "name": "bug", "scope": "repository" })
        );
        let text = list.to_human(false);
        assert!(text.contains("bug") && text.contains("red"), "{text}");
    }

    #[test]
    fn a_space_label_is_marked_as_such() {
        assert_eq!(label_json(&label("area", "blue", false))["scope"], "space");
    }
}
