//! Rendering.
//!
//! Every byte `fx` writes to stdout goes through this module, which is what
//! makes "one command, two audiences" tractable: a command produces a value,
//! and [`Renderer`] decides whether a human or a machine is reading it.
//!
//! The machine contract:
//!
//! * `--output json` — exactly one JSON document on stdout,
//!   `{"ok":true,"data":…}` on success and `{"ok":false,"error":{…}}` on
//!   failure. Failures also set a non-zero exit code.
//! * `--output jsonl` — one bare JSON value per line on success (built for
//!   streaming into `jq`/`xargs`); a single enveloped error object on failure.
//! * `--output table` — human text on stdout, errors on stderr.

use std::fmt;
use std::io::{self, Write};
use std::str::FromStr;

use serde_json::{Value, json};

use crate::error::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum OutputFormat {
    /// Aligned columns for people.
    Table,
    /// One JSON document.
    Json,
    /// One JSON value per line.
    Jsonl,
}

impl OutputFormat {
    pub fn is_machine(self) -> bool {
        !matches!(self, Self::Table)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Json => "json",
            Self::Jsonl => "jsonl",
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OutputFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s.trim().to_ascii_lowercase().as_str() {
            "table" | "text" | "human" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            "jsonl" | "ndjson" | "json-lines" => Ok(Self::Jsonl),
            _ => Err(()),
        }
    }
}

/// Anything a command can hand back to the user.
///
/// Implementors describe the value twice — once for people, once for machines —
/// and never decide which one gets used.
pub trait Render {
    /// The payload placed under `data` in the JSON envelope.
    fn to_json(&self) -> Value;

    /// The human rendering. `color` is already resolved; respect it.
    fn to_human(&self, color: bool) -> String;

    /// The rows emitted in `jsonl` mode. Defaults to a single row.
    fn to_jsonl(&self) -> Vec<Value> {
        vec![self.to_json()]
    }
}

/// A bare JSON value, rendered as pretty JSON for humans too.
///
/// Used by `fx api`, where the whole point is to pass the server's answer
/// through untouched.
pub struct Json(pub Value);

impl Render for Json {
    fn to_json(&self) -> Value {
        self.0.clone()
    }

    fn to_human(&self, _color: bool) -> String {
        serde_json::to_string_pretty(&self.0).unwrap_or_else(|_| self.0.to_string())
    }

    fn to_jsonl(&self) -> Vec<Value> {
        // A top-level array streams as one line per element.
        match &self.0 {
            Value::Array(items) => items.clone(),
            other => vec![other.clone()],
        }
    }
}

pub struct Renderer {
    format: OutputFormat,
    color: bool,
}

impl Renderer {
    pub fn new(format: OutputFormat, color: bool) -> Self {
        Self { format, color }
    }

    pub fn emit<T: Render>(&self, value: &T) -> io::Result<()> {
        forgive_broken_pipe(self.write(value))
    }

    fn write<T: Render>(&self, value: &T) -> io::Result<()> {
        let mut out = io::stdout().lock();
        match self.format {
            OutputFormat::Table => {
                let text = value.to_human(self.color);
                if !text.is_empty() {
                    writeln!(out, "{text}")?;
                }
            }
            OutputFormat::Json => {
                let envelope = json!({ "ok": true, "data": value.to_json() });
                writeln!(out, "{}", serde_json::to_string_pretty(&envelope)?)?;
            }
            OutputFormat::Jsonl => {
                for row in value.to_jsonl() {
                    writeln!(out, "{}", serde_json::to_string(&row)?)?;
                }
            }
        }
        out.flush()
    }

    /// Errors go to stdout in machine modes (so the envelope is pipeable) and
    /// to stderr for humans (so it does not pollute a piped table).
    pub fn emit_error(&self, err: &CliError) -> io::Result<()> {
        forgive_broken_pipe(self.write_error(err))
    }

    fn write_error(&self, err: &CliError) -> io::Result<()> {
        if self.format.is_machine() {
            let envelope = json!({ "ok": false, "error": err.to_json() });
            let mut out = io::stdout().lock();
            writeln!(out, "{}", serde_json::to_string_pretty(&envelope)?)?;
            return out.flush();
        }

        let mut err_out = io::stderr().lock();
        let (red, dim, reset) = if self.color {
            ("\x1b[31m", "\x1b[2m", "\x1b[0m")
        } else {
            ("", "", "")
        };
        writeln!(err_out, "{red}error{reset}: {}", err.message)?;
        if let Some(hint) = &err.hint {
            writeln!(err_out, "{dim}hint{reset}: {hint}")?;
        }
        err_out.flush()
    }
}

/// Treat a closed reader as success.
///
/// `fx pr list | head -5` closes the pipe as soon as head has what it wants.
/// Rust ignores `SIGPIPE`, so the write comes back as `BrokenPipe` instead of
/// killing the process the way every other Unix tool dies — and reporting it
/// would turn an ordinary pipeline into a failed command. Restoring the signal
/// handler would need `unsafe`, which this workspace forbids, so the one place
/// that writes to stdout forgives the error instead.
pub fn forgive_broken_pipe(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

/// A borderless, aligned table — the `REPOSITORY  VISIBILITY  DEFAULT` look.
pub fn plain_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    use comfy_table::presets::NOTHING;
    use comfy_table::{Cell, ContentArrangement, Table};

    let mut table = Table::new();
    table
        .load_style(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.iter().map(|h| Cell::new(h.to_uppercase())));
    for row in rows {
        table.add_row(row.iter().map(Cell::new));
    }
    table
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A short "3d ago" for a GitFox timestamp.
///
/// GitFox sends epoch integers without saying which unit, and instances differ.
/// Anything past roughly the year 5138 in seconds must actually be
/// milliseconds, which separates the two cleanly for any date this tool will
/// ever see. The raw value is what goes into JSON; this is for humans only.
pub fn relative_time(epoch: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let seconds = if epoch.abs() > 100_000_000_000 {
        epoch / 1000
    } else {
        epoch
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = now - seconds;
    if delta < 0 {
        return "in the future".to_string();
    }
    match delta {
        d if d < 60 => "just now".to_string(),
        d if d < 3600 => format!("{}m ago", d / 60),
        d if d < 86_400 => format!("{}h ago", d / 3600),
        d if d < 2_592_000 => format!("{}d ago", d / 86_400),
        d if d < 31_536_000 => format!("{}mo ago", d / 2_592_000),
        d => format!("{}y ago", d / 31_536_000),
    }
}

/// Aligned `key: value` lines, for single-record views like `fx auth status`.
pub fn key_values(pairs: &[(&str, String)]) -> String {
    let width = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    pairs
        .iter()
        .map(|(k, v)| format!("{k:<width$}  {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_parses_the_documented_spellings() {
        assert_eq!("json".parse(), Ok(OutputFormat::Json));
        assert_eq!("JSONL".parse(), Ok(OutputFormat::Jsonl));
        assert_eq!("ndjson".parse(), Ok(OutputFormat::Jsonl));
        assert_eq!("  table  ".parse(), Ok(OutputFormat::Table));
        assert_eq!("yaml".parse::<OutputFormat>(), Err(()));
    }

    #[test]
    fn only_the_table_format_is_for_humans() {
        assert!(!OutputFormat::Table.is_machine());
        assert!(OutputFormat::Json.is_machine());
        assert!(OutputFormat::Jsonl.is_machine());
    }

    #[test]
    fn json_render_passes_values_through_untouched() {
        let value = json!({ "id": 12, "title": "Add OAuth" });
        assert_eq!(Json(value.clone()).to_json(), value);
    }

    #[test]
    fn jsonl_flattens_a_top_level_array() {
        let rows = Json(json!([{ "id": 1 }, { "id": 2 }])).to_jsonl();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["id"], 2);
    }

    #[test]
    fn error_envelope_shape_is_stable() {
        let err = CliError::config("no GitFox host configured")
            .with_details(json!({ "checked": ["--host", "GITFOX_HOST"] }))
            .with_hint("run `fx auth login`");
        let envelope = json!({ "ok": false, "error": err.to_json() });
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["error"]["code"], "CONFIG_ERROR");
        assert_eq!(envelope["error"]["details"]["checked"][0], "--host");
        // The human-only hint must not leak into the machine contract.
        assert!(envelope["error"].get("hint").is_none());
    }

    #[test]
    fn a_closed_reader_is_not_a_failure() {
        let broken = Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
        assert!(forgive_broken_pipe(broken).is_ok());
        // Anything else still surfaces.
        let real = Err(io::Error::new(io::ErrorKind::PermissionDenied, "nope"));
        assert!(forgive_broken_pipe(real).is_err());
    }

    #[test]
    fn plain_table_has_uppercase_headers_and_no_borders() {
        let text = plain_table(
            &["repository", "visibility"],
            &[vec!["ai/backend".into(), "private".into()]],
        );
        let lines: Vec<_> = text.lines().collect();
        assert!(lines[0].contains("REPOSITORY"), "{text}");
        assert!(!text.contains('|'), "{text}");
        assert!(lines[1].contains("ai/backend"), "{text}");
    }

    #[test]
    fn relative_time_reads_both_seconds_and_milliseconds() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let three_days = now - 3 * 86_400;
        assert_eq!(relative_time(three_days), "3d ago");
        // The same instant expressed in milliseconds must read the same.
        assert_eq!(relative_time(three_days * 1000), "3d ago");
        assert_eq!(relative_time(now), "just now");
        assert_eq!(relative_time(now - 7200), "2h ago");
    }

    #[test]
    fn key_values_aligns_on_the_longest_key() {
        let text = key_values(&[
            ("Host", "git.example.com".into()),
            ("Token", "configured".into()),
        ]);
        assert_eq!(text, "Host   git.example.com\nToken  configured");
    }
}
