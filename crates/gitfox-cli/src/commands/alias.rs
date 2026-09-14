//! `fx alias` — shortcuts, stored in the config file's `[aliases]` table and
//! expanded by [`crate::argv`] before anything is parsed.

use clap::CommandFactory;
use serde_json::{Value, json};

use crate::cli::{
    AliasCommand, AliasDeleteArgs, AliasImportArgs, AliasSetArgs, AliasSubcommand, Cli,
};
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::interact;
use crate::output::Render;

pub fn run(cmd: AliasCommand, ctx: &Context) -> Result<()> {
    match cmd.command {
        AliasSubcommand::Set(args) => set(args, ctx),
        AliasSubcommand::List => list(ctx),
        AliasSubcommand::Delete(args) => delete(args, ctx),
        AliasSubcommand::Import(args) => import(args, ctx),
    }
}

/// Why `name` cannot be an alias, if it cannot.
fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.starts_with('-') || name.contains(char::is_whitespace) {
        return Err(CliError::invalid_argument(format!(
            "`{name}` is not a valid alias name"
        )));
    }
    if Cli::command().find_subcommand(name).is_some() {
        return Err(CliError::invalid_argument(format!(
            "could not create alias: \"{name}\" is already an fx command"
        )));
    }
    Ok(())
}

/// A non-shell expansion has to start with something fx can run.
fn check_expansion(expansion: &str, shell: bool) -> Result<String> {
    if shell || expansion.starts_with('!') {
        return Ok(if expansion.starts_with('!') {
            expansion.to_string()
        } else {
            format!("!{expansion}")
        });
    }
    let first = crate::argv::split_words(expansion)
        .into_iter()
        .next()
        .unwrap_or_default();
    if Cli::command().find_subcommand(&first).is_none() {
        return Err(CliError::invalid_argument(format!(
            "could not create alias: {expansion} does not correspond to an fx command"
        ))
        .with_hint("start the expansion with a command, e.g. `pr list`, or pass --shell"));
    }
    Ok(expansion.to_string())
}

fn set(args: AliasSetArgs, ctx: &Context) -> Result<()> {
    check_name(&args.name)?;
    let expansion = check_expansion(&args.expansion, args.shell)?;
    let mut file = ctx.config_file.clone();
    let replaced = file.aliases.get(&args.name).cloned();
    if replaced.is_some() && !args.clobber {
        return Err(CliError::invalid_argument(format!(
            "could not create alias: \"{}\" already exists",
            args.name
        ))
        .with_hint("pass --clobber to overwrite it"));
    }
    file.aliases.insert(args.name.clone(), expansion.clone());
    file.save(&ctx.config_path)?;
    ctx.renderer.emit(&AliasChanged {
        name: args.name,
        expansion,
        replaced,
    })
}

fn list(ctx: &Context) -> Result<()> {
    ctx.renderer.emit(&AliasList(
        ctx.config_file
            .aliases
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
}

fn delete(args: AliasDeleteArgs, ctx: &Context) -> Result<()> {
    let mut file = ctx.config_file.clone();
    let removed: Vec<String> = if args.all {
        std::mem::take(&mut file.aliases).into_keys().collect()
    } else {
        let name = args.name.unwrap_or_default();
        if file.aliases.remove(&name).is_none() {
            return Err(CliError::new(
                ErrorCode::NotFound,
                format!("no such alias {name}"),
            ));
        }
        vec![name]
    };
    file.save(&ctx.config_path)?;
    ctx.renderer.emit(&AliasesDeleted(removed))
}

fn import(args: AliasImportArgs, ctx: &Context) -> Result<()> {
    let text = interact::read_source(args.file.as_deref().unwrap_or("-"))?;
    let entries = parse_yaml_map(&text)?;
    let mut file = ctx.config_file.clone();
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    for (name, expansion) in entries {
        check_name(&name)?;
        let expansion = check_expansion(&expansion, false)?;
        if file.aliases.contains_key(&name) && !args.clobber {
            skipped.push(name);
            continue;
        }
        file.aliases.insert(name.clone(), expansion);
        imported.push(name);
    }
    file.save(&ctx.config_path)?;
    for name in &skipped {
        ctx.warn(&format!(
            "could not import alias {name}: already exists (pass --clobber to overwrite)"
        ));
    }
    ctx.renderer.emit(&AliasesImported { imported, skipped })
}

/// The flat `name: expansion` YAML that `gh alias list` prints and
/// `gh alias import` reads. Quotes are honoured; nesting is not YAML this
/// ever needs.
fn parse_yaml_map(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            return Err(CliError::invalid_argument(format!(
                "line {}: expected `name: expansion`, got `{trimmed}`",
                index + 1
            )));
        };
        out.push((unquote(key.trim()), unquote(value.trim())));
    }
    Ok(out)
}

fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        let inner = &text[1..text.len() - 1];
        return if bytes[0] == b'"' {
            inner.replace("\\\"", "\"").replace("\\\\", "\\")
        } else {
            inner.replace("''", "'")
        };
    }
    text.to_string()
}

fn yaml_value(text: &str) -> String {
    if text.contains([':', '#', '\'', '"']) || text.starts_with(['!', '*', '&', ' ']) {
        format!("'{}'", text.replace('\'', "''"))
    } else {
        text.to_string()
    }
}

struct AliasChanged {
    name: String,
    expansion: String,
    replaced: Option<String>,
}

impl Render for AliasChanged {
    fn to_json(&self) -> Value {
        json!({ "name": self.name, "expansion": self.expansion, "replaced": self.replaced })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        let verb = if self.replaced.is_some() {
            "Changed alias"
        } else {
            "Added alias"
        };
        format!("{green}✓{reset} {verb} {}: {}", self.name, self.expansion)
    }
}

struct AliasList(Vec<(String, String)>);

impl Render for AliasList {
    fn to_json(&self) -> Value {
        json!({
            "count": self.0.len(),
            "items": self.0.iter().map(|(n, e)| json!({ "name": n, "expansion": e })).collect::<Vec<_>>(),
        })
    }

    fn to_jsonl(&self) -> Vec<Value> {
        self.0
            .iter()
            .map(|(n, e)| json!({ "name": n, "expansion": e }))
            .collect()
    }

    fn to_human(&self, _color: bool) -> String {
        if self.0.is_empty() {
            return "no aliases configured".to_string();
        }
        // The same YAML `fx alias import` reads back.
        self.0
            .iter()
            .map(|(name, expansion)| format!("{name}: {}", yaml_value(expansion)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct AliasesDeleted(Vec<String>);

impl Render for AliasesDeleted {
    fn to_json(&self) -> Value {
        json!({ "deleted": self.0 })
    }

    fn to_human(&self, color: bool) -> String {
        let (red, reset) = if color {
            ("\x1b[31m", "\x1b[0m")
        } else {
            ("", "")
        };
        self.0
            .iter()
            .map(|name| format!("{red}✓{reset} Deleted alias {name}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct AliasesImported {
    imported: Vec<String>,
    skipped: Vec<String>,
}

impl Render for AliasesImported {
    fn to_json(&self) -> Value {
        json!({ "imported": self.imported, "skipped": self.skipped })
    }

    fn to_human(&self, color: bool) -> String {
        let (green, reset) = if color {
            ("\x1b[32m", "\x1b[0m")
        } else {
            ("", "")
        };
        format!(
            "{green}✓{reset} Imported {} alias{}",
            self.imported.len(),
            if self.imported.len() == 1 { "" } else { "es" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_in_command_cannot_be_an_alias_name() {
        assert!(check_name("pr").is_err());
        assert!(check_name("co").is_err(), "co is built in");
        assert!(check_name("prs").is_ok());
        assert!(check_name("-x").is_err());
    }

    #[test]
    fn an_expansion_must_run_an_fx_command_unless_it_is_a_shell_alias() {
        assert_eq!(check_expansion("pr list", false).unwrap(), "pr list");
        assert!(check_expansion("rm -rf /", false).is_err());
        assert_eq!(
            check_expansion("fx pr list | head", true).unwrap(),
            "!fx pr list | head"
        );
        assert_eq!(check_expansion("!echo hi", false).unwrap(), "!echo hi");
    }

    #[test]
    fn the_yaml_gh_prints_is_the_yaml_import_reads() {
        let entries = parse_yaml_map(
            "# aliases\nco: pr checkout\nprs: 'pr list --json number,title'\nmine: \"!fx pr list | head\"\n",
        )
        .unwrap();
        assert_eq!(
            entries,
            vec![
                ("co".to_string(), "pr checkout".to_string()),
                ("prs".to_string(), "pr list --json number,title".to_string()),
                ("mine".to_string(), "!fx pr list | head".to_string()),
            ]
        );
        assert!(parse_yaml_map("not yaml at all").is_err());
        assert_eq!(yaml_value("pr list"), "pr list");
        assert_eq!(yaml_value("!fx x | y"), "'!fx x | y'");
        // What list prints, import reads back unchanged.
        let round = parse_yaml_map(&format!("a: {}", yaml_value("it's: odd"))).unwrap();
        assert_eq!(round[0].1, "it's: odd");
    }
}
