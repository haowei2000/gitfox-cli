//! `gf ruleset` — a repository's protection rules, GitFox's counterpart to
//! GitHub rulesets.

use gitfox_client::Rule;
use serde_json::{Value, json};

use crate::cli::{RulesetCommand, RulesetListArgs, RulesetSubcommand, RulesetViewArgs};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::interact;
use crate::output::{Render, key_values, plain_table, relative_time};
use crate::paginate;

pub async fn run(cmd: RulesetCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        RulesetSubcommand::List(args) => list(args, ctx).await,
        RulesetSubcommand::View(args) => view(args, ctx).await,
        RulesetSubcommand::Check(_) => super::gh_only::refuse(
            "ruleset check",
            "GitFox does not report which rules apply to a branch; see `gf ruleset list`",
        ),
    }
}

fn org_rules_unsupported(org: Option<&String>) -> Result<()> {
    match org {
        Some(_) => Err(CliError::unsupported(
            "`--org`",
            "GitFox protection rules belong to repositories",
        )),
        None => Ok(()),
    }
}

async fn list(args: RulesetListArgs, ctx: &Context) -> Result<()> {
    org_rules_unsupported(args.org.as_ref())?;
    let repo = ctx.repo()?;
    if args.web {
        return interact::open_in_browser(
            ctx,
            &ctx.web_url(&format!("{}/settings/rules", repo.full()))?,
        );
    }
    let client = ctx.client()?;
    let (client_ref, repo_ref) = (&client, &repo);
    let paged = paginate::collect(args.limit, move |page, limit| async move {
        client_ref.rules().list(repo_ref, page, limit).await
    })
    .await
    .map_err(|e| super::pr::not_found_as_repo(e, &repo))?;
    ctx.renderer.emit(&RuleList {
        repo: repo.full(),
        rules: paged.items,
        truncated: paged.truncated,
    })
}

async fn view(args: RulesetViewArgs, ctx: &Context) -> Result<()> {
    org_rules_unsupported(args.org.as_ref())?;
    let repo = ctx.repo()?;
    let client = ctx.client()?;

    let id = match args.id.clone() {
        Some(id) => id,
        None => {
            // gh prompts; with one rule there is nothing to ask.
            let rules = client.rules().list(&repo, 1, 100).await?;
            match rules.as_slice() {
                [only] => only.identifier.clone(),
                [] => {
                    return Err(CliError::new(
                        ErrorCode::NotFound,
                        format!("no rulesets found in {repo}"),
                    ));
                }
                many => {
                    return Err(CliError::invalid_argument(
                        "ruleset ID required when the repository has several",
                    )
                    .with_hint(
                        many.iter()
                            .map(|r| r.identifier.as_str())
                            .collect::<Vec<_>>()
                            .join(" | "),
                    ));
                }
            }
        }
    };
    let url = ctx.web_url(&format!("{}/settings/rules/{id}", repo.full()))?;
    if args.web {
        return interact::open_in_browser(ctx, &url);
    }
    let rule = client.rules().get(&repo, &id).await.map_err(|e| match e {
        gitfox_client::Error::NotFound { .. } => {
            CliError::new(ErrorCode::NotFound, format!("no ruleset `{id}` in {repo}"))
        }
        other => CliError::from(other),
    })?;
    ctx.renderer.emit(&RuleView { rule, url })
}

fn rule_json(rule: &Rule) -> Value {
    json!({
        "identifier": rule.identifier,
        "description": rule.description,
        "type": rule.kind,
        "state": rule.state,
        "targets": rule.targets(),
        "pattern": rule.pattern,
        "definition": rule.definition,
        "created_by": rule.created_by.as_ref().map(gitfox_client::Principal::label),
        "created": rule.created,
        "updated": rule.updated,
    })
}

struct RuleList {
    repo: String,
    rules: Vec<Rule>,
    truncated: bool,
}

impl Render for RuleList {
    fn to_json(&self) -> Value {
        json!({
            "repository": self.repo,
            "count": self.rules.len(),
            "truncated": self.truncated,
            "items": self.to_jsonl(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.rules.iter().map(rule_json).collect()
    }

    fn to_human(&self, _color: bool) -> String {
        if self.rules.is_empty() {
            return format!("no rulesets found in {}", self.repo);
        }
        let rows: Vec<Vec<String>> = self
            .rules
            .iter()
            .map(|r| {
                vec![
                    r.identifier.clone(),
                    r.state.clone().unwrap_or_default(),
                    r.kind.clone().unwrap_or_default(),
                    r.targets().join(", "),
                    r.updated.map(relative_time).unwrap_or_default(),
                ]
            })
            .collect();
        plain_table(&["id", "status", "type", "targets", "updated"], &rows)
    }
}

struct RuleView {
    rule: Rule,
    url: String,
}

impl Render for RuleView {
    fn to_json(&self) -> Value {
        let mut value = rule_json(&self.rule);
        value["web_url"] = json!(self.url);
        value
    }

    fn to_human(&self, color: bool) -> String {
        let r = &self.rule;
        let (bold, reset) = if color {
            ("\x1b[1m", "\x1b[0m")
        } else {
            ("", "")
        };
        let mut pairs = vec![
            ("Status", r.state.clone().unwrap_or_default()),
            ("Type", r.kind.clone().unwrap_or_default()),
            ("Targets", r.targets().join(", ")),
        ];
        if let Some(by) = &r.created_by {
            pairs.push(("Created by", by.label()));
        }
        if let Some(updated) = r.updated {
            pairs.push(("Updated", relative_time(updated)));
        }
        let mut out = format!("{bold}{}{reset}\n{}", r.identifier, key_values(&pairs));
        if let Some(description) = r.description.as_deref().filter(|d| !d.trim().is_empty()) {
            out.push_str(&format!("\n\n{}", description.trim()));
        }
        if !r.definition.is_null() {
            out.push_str("\n\nRules\n");
            out.push_str(&serde_json::to_string_pretty(&r.definition).unwrap_or_default());
        }
        out.push_str(&format!("\n\nView this ruleset on GitFox: {}", self.url));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_lists_its_targets() {
        let rule: Rule = serde_json::from_value(json!({
            "identifier": "CI_Check", "type": "branch", "state": "disabled",
            "pattern": { "default": true, "include": ["release/*"] },
            "definition": { "pullreq": { "status_checks": { "require_identifiers": ["build"] } } }
        }))
        .unwrap();
        assert_eq!(rule.targets(), vec!["default branch", "release/*"]);
        let list = RuleList {
            repo: "ai/backend".into(),
            rules: vec![rule.clone()],
            truncated: false,
        };
        assert!(list.to_human(false).contains("CI_Check"));
        let view = RuleView {
            rule,
            url: "http://h/ai/backend/settings/rules/CI_Check".into(),
        };
        let text = view.to_human(false);
        assert!(text.contains("require_identifiers"), "{text}");
    }
}
