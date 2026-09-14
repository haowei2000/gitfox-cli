//! The command line, before clap sees it.
//!
//! Two gh behaviours cannot be expressed as clap flags:
//!
//! * **`--json FIELDS`.** In gh, `--json` takes a value; in fx it always meant
//!   "JSON output" and takes none, which is how `fx --json pr list` reads. clap
//!   can only offer an optional value greedily — `--json pr` would swallow the
//!   subcommand — so the value is required to be attached (`--json=a,b`), and
//!   this module attaches it: on a command that exports fields (one that takes
//!   `--jq`, as in gh), a `--json` followed by something that looks like a field
//!   list, and is not a subcommand name at that position, becomes
//!   `--json=that`. Anywhere else the next word stays an argument, as it was in
//!   fx 0.6. The walk uses the real command tree, so it knows which flags
//!   consume the next token and which names are subcommands where.
//! * **Aliases.** `fx alias set` shortcuts expand before parsing, and a `!` alias
//!   runs through the shell instead.

use std::ffi::OsString;

use clap::{Arg, Command, CommandFactory};

use crate::cli::Cli;
use crate::config::{ConfigFile, SystemEnv, config_path};

/// What to do with this invocation.
#[derive(Debug, PartialEq)]
pub enum Invocation {
    /// Hand these arguments to clap.
    Args(Vec<OsString>),
    /// A shell alias: run `script` with `args` as its positional parameters.
    Shell { script: String, args: Vec<String> },
}

/// Expand aliases, then normalise `--json`.
pub fn prepare(raw: Vec<OsString>) -> Invocation {
    let root = built_command();
    let aliases = load_aliases(&raw);
    match expand_alias(raw, &root, &aliases) {
        Invocation::Args(args) => Invocation::Args(attach_json_fields(args, &root)),
        shell => shell,
    }
}

fn built_command() -> Command {
    let mut root = Cli::command();
    root.build();
    root
}

// ---------------------------------------------------------------------------
// aliases
// ---------------------------------------------------------------------------

fn load_aliases(raw: &[OsString]) -> std::collections::BTreeMap<String, String> {
    let cli_path = explicit_config(raw);
    config_path(&SystemEnv, cli_path.as_deref())
        .ok()
        .and_then(|path| ConfigFile::load(&path).ok())
        .map(|file| file.aliases)
        .unwrap_or_default()
}

/// `--config PATH` / `--config=PATH`, wherever it appears.
fn explicit_config(raw: &[OsString]) -> Option<std::path::PathBuf> {
    let mut iter = raw.iter().filter_map(|a| a.to_str());
    while let Some(arg) = iter.next() {
        if arg == "--" {
            break;
        }
        if let Some(path) = arg.strip_prefix("--config=") {
            return Some(path.into());
        }
        if arg == "--config" {
            return iter.next().map(Into::into);
        }
    }
    None
}

/// Replace a leading alias with its expansion.
///
/// Only the first argument is considered, and never a built-in command name:
/// an alias cannot change what `fx pr` means.
pub fn expand_alias(
    raw: Vec<OsString>,
    root: &Command,
    aliases: &std::collections::BTreeMap<String, String>,
) -> Invocation {
    let Some(first) = raw.get(1).and_then(|a| a.to_str()) else {
        return Invocation::Args(raw);
    };
    if first.starts_with('-') || root.find_subcommand(first).is_some() {
        return Invocation::Args(raw);
    }
    let Some(expansion) = aliases.get(first) else {
        return Invocation::Args(raw);
    };

    let extra: Vec<String> = raw[2..]
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    if let Some(script) = expansion.strip_prefix('!') {
        return Invocation::Shell {
            script: script.to_string(),
            args: extra,
        };
    }

    let mut used = vec![false; extra.len()];
    let mut expanded: Vec<OsString> = vec![raw[0].clone()];
    for word in split_words(expansion) {
        expanded.push(OsString::from(substitute(&word, &extra, &mut used)));
    }
    for (arg, consumed) in extra.iter().zip(used) {
        if !consumed {
            expanded.push(OsString::from(arg));
        }
    }
    Invocation::Args(expanded)
}

/// Fill `$1`, `$2`… from the arguments that followed the alias.
fn substitute(word: &str, args: &[String], used: &mut [bool]) -> String {
    let mut out = String::with_capacity(word.len());
    let mut chars = word.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek().is_some_and(char::is_ascii_digit) {
            let mut digits = String::new();
            while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                digits.push(*d);
                chars.next();
            }
            let index: usize = digits.parse().unwrap_or(0);
            match index
                .checked_sub(1)
                .and_then(|i| args.get(i).map(|a| (i, a)))
            {
                Some((i, arg)) => {
                    out.push_str(arg);
                    used[i] = true;
                }
                None => {
                    out.push('$');
                    out.push_str(&digits);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split an alias expansion into words, honouring single and double quotes
/// and backslash escapes the way a shell would.
pub fn split_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = input.chars();
    let mut quote: Option<char> = None;

    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                    in_word = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (None, c) => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(current);
    }
    words
}

// ---------------------------------------------------------------------------
// --json FIELDS
// ---------------------------------------------------------------------------

/// Rewrite `--json FIELDS` to `--json=FIELDS` where FIELDS is meant as the
/// flag's value. See the module docs.
pub fn attach_json_fields(args: Vec<OsString>, root: &Command) -> Vec<OsString> {
    let mut out = Vec::with_capacity(args.len());
    let mut iter = args.into_iter();
    if let Some(program) = iter.next() {
        out.push(program);
    }
    let rest: Vec<OsString> = iter.collect();

    let mut command = root;
    let mut i = 0;
    while i < rest.len() {
        let token = &rest[i];
        let Some(text) = token.to_str() else {
            out.push(token.clone());
            i += 1;
            continue;
        };

        if text == "--" {
            out.extend(rest[i..].iter().cloned());
            break;
        }

        if text == "--json" {
            match rest.get(i + 1).and_then(|n| n.to_str()) {
                Some(next)
                    if exports_fields(command)
                        && is_field_list(next)
                        && command.find_subcommand(next).is_none() =>
                {
                    out.push(OsString::from(format!("--json={next}")));
                    i += 2;
                }
                _ => {
                    out.push(token.clone());
                    i += 1;
                }
            }
            continue;
        }

        out.push(token.clone());
        if let Some(long) = text.strip_prefix("--") {
            if !long.contains('=')
                && find_long(command, long).is_some_and(consumes_next)
                && let Some(value) = rest.get(i + 1)
            {
                out.push(value.clone());
                i += 1;
            }
        } else if let Some(cluster) = text.strip_prefix('-').filter(|c| !c.is_empty()) {
            // `-L5` carries its value; `-sL 5` takes the next token.
            let letters: Vec<char> = cluster.chars().collect();
            for (index, letter) in letters.iter().enumerate() {
                match find_short(command, *letter) {
                    Some(arg) if consumes_next(arg) => {
                        if index + 1 == letters.len()
                            && let Some(value) = rest.get(i + 1)
                        {
                            out.push(value.clone());
                            i += 1;
                        }
                        break;
                    }
                    Some(_) => continue,
                    None => break,
                }
            }
        } else if let Some(sub) = command.find_subcommand(text) {
            command = sub;
        }
        i += 1;
    }
    out
}

/// Whether gh's `--json FIELDS` applies here: the commands that export fields
/// are the ones that take `--jq`.
fn exports_fields(command: &Command) -> bool {
    find_long(command, "jq").is_some()
}

fn find_long<'a>(command: &'a Command, name: &str) -> Option<&'a Arg> {
    command.get_arguments().find(|arg| {
        arg.get_long() == Some(name)
            || arg
                .get_all_aliases()
                .is_some_and(|aliases| aliases.contains(&name))
    })
}

fn find_short(command: &Command, letter: char) -> Option<&Arg> {
    command.get_arguments().find(|arg| {
        arg.get_short() == Some(letter)
            || arg
                .get_all_short_aliases()
                .is_some_and(|aliases| aliases.contains(&letter))
    })
}

/// Whether the flag takes the following token as its value.
fn consumes_next(arg: &Arg) -> bool {
    arg.get_action().takes_values()
        && !arg.is_require_equals_set()
        && arg.get_num_args().is_none_or(|n| n.min_values() > 0)
}

/// `number,title,headRefName` — what gh's `--json` takes.
pub fn is_field_list(text: &str) -> bool {
    !text.is_empty()
        && text.split(',').all(|field| {
            let field = field.trim();
            let mut chars = field.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn attach(args: &[&str]) -> Vec<String> {
        strings(&attach_json_fields(os(args), &built_command()))
    }

    #[test]
    fn a_field_list_after_json_becomes_its_value() {
        assert_eq!(
            attach(&["fx", "pr", "list", "--json", "number,title"]),
            ["fx", "pr", "list", "--json=number,title"]
        );
        assert_eq!(
            attach(&["fx", "pr", "view", "12", "--json", "title", "-q", ".title"]),
            ["fx", "pr", "view", "12", "--json=title", "-q", ".title"]
        );
        assert_eq!(
            attach(&["fx", "repo", "view", "--json", "name"]),
            ["fx", "repo", "view", "--json=name"]
        );
    }

    #[test]
    fn a_subcommand_after_json_is_still_a_subcommand() {
        // The original fx spelling must keep working.
        assert_eq!(
            attach(&["fx", "--json", "pr", "list"]),
            ["fx", "--json", "pr", "list"]
        );
        assert_eq!(
            attach(&["fx", "pr", "--json", "status"]),
            ["fx", "pr", "--json", "status"]
        );
        assert_eq!(
            attach(&["fx", "--json", "api", "/api/v1/user"]),
            ["fx", "--json", "api", "/api/v1/user"]
        );
    }

    #[test]
    fn a_flag_value_is_never_mistaken_for_a_subcommand() {
        // `view` is the value of -R here, not a subcommand, so the walk goes
        // on to `pr list` and the `--json` after it is gh's.
        assert_eq!(
            attach(&["fx", "pr", "-R", "view", "list", "--json", "number"]),
            ["fx", "pr", "-R", "view", "list", "--json=number"]
        );
        assert_eq!(
            attach(&["fx", "-R", "ai/backend", "pr", "list", "--json", "state"]),
            ["fx", "-R", "ai/backend", "pr", "list", "--json=state"]
        );
        assert_eq!(
            attach(&["fx", "run", "list", "-L5", "--json", "status"]),
            ["fx", "run", "list", "-L5", "--json=status"]
        );
    }

    #[test]
    fn a_number_or_path_after_json_is_left_alone() {
        assert_eq!(
            attach(&["fx", "pr", "view", "--json", "12"]),
            ["fx", "pr", "view", "--json", "12"]
        );
        assert_eq!(
            attach(&["fx", "pr", "list", "--json", "--", "number"]),
            ["fx", "pr", "list", "--json", "--", "number"]
        );
    }

    #[test]
    fn a_word_after_json_stays_an_argument_where_no_fields_are_exported() {
        // fx 0.6 spellings on commands without gh's `--json FIELDS`.
        assert_eq!(
            attach(&["fx", "pipeline", "run", "--json", "default"]),
            ["fx", "pipeline", "run", "--json", "default"]
        );
        assert_eq!(
            attach(&["fx", "pr", "checkout", "--json", "feat_oauth"]),
            ["fx", "pr", "checkout", "--json", "feat_oauth"]
        );
        assert_eq!(
            attach(&["fx", "--json", "number", "pr", "list"]),
            ["fx", "--json", "number", "pr", "list"]
        );
    }

    #[test]
    fn field_lists_are_recognised_strictly() {
        assert!(is_field_list("number"));
        assert!(is_field_list("number,headRefName,created_at"));
        assert!(!is_field_list("12"));
        assert!(!is_field_list("/api/v1/user"));
        assert!(!is_field_list("a,,b"));
        assert!(!is_field_list(""));
    }

    fn aliases(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn an_alias_expands_with_positional_substitution() {
        let root = built_command();
        let table = aliases(&[
            ("prs", "pr list --author $1"),
            ("co2", "pr checkout"),
            ("mine", "!fx pr list | head -$1"),
        ]);

        assert_eq!(
            expand_alias(os(&["fx", "prs", "whw", "-L", "5"]), &root, &table),
            Invocation::Args(os(&["fx", "pr", "list", "--author", "whw", "-L", "5"]))
        );
        assert_eq!(
            expand_alias(os(&["fx", "co2", "12"]), &root, &table),
            Invocation::Args(os(&["fx", "pr", "checkout", "12"]))
        );
        assert_eq!(
            expand_alias(os(&["fx", "mine", "3"]), &root, &table),
            Invocation::Shell {
                script: "fx pr list | head -$1".into(),
                args: vec!["3".into()],
            }
        );
    }

    #[test]
    fn a_built_in_command_is_never_shadowed_by_an_alias() {
        let root = built_command();
        let table = aliases(&[("pr", "repo list")]);
        assert_eq!(
            expand_alias(os(&["fx", "pr", "list"]), &root, &table),
            Invocation::Args(os(&["fx", "pr", "list"]))
        );
    }

    #[test]
    fn words_split_like_a_shell() {
        assert_eq!(
            split_words(r#"pr create --title "a b" --body 'c "d"' e\ f"#),
            ["pr", "create", "--title", "a b", "--body", "c \"d\"", "e f"]
        );
        assert_eq!(split_words("  "), Vec::<String>::new());
    }
}
