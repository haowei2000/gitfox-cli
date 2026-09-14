//! gh-style structured output: `--json FIELDS`, `--jq` and `--template`.
//!
//! `fx --json` on its own keeps its meaning — the `{"ok":…}` envelope. Given
//! field names it switches to the shape gh prints: the resource, or an array of
//! them, restricted to those fields and without an envelope. That is what a
//! script or an agent that learned gh reaches for, so it has to work verbatim:
//!
//! ```text
//! fx pr list --json number,title --jq '.[] | "\(.number) \(.title)"'
//! ```
//!
//! Field names are gh's (`headRefName`, `createdAt`, …) with gh's value shapes
//! — uppercase states, ISO 8601 times, `{"login": …}` authors — plus fx's own
//! snake_case names, so the envelope's keys work too.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::error::{CliError, Result};

/// What `--jq` / `--template` asked for on a command that supports them.
#[derive(Debug, Clone, Default)]
pub struct Format {
    pub jq: Option<String>,
    pub template: Option<String>,
}

/// Everything about a gh-style export request, validated.
#[derive(Debug, Clone)]
pub struct ExportSpec {
    pub fields: Vec<String>,
    pub jq: Option<String>,
    pub template: Option<String>,
}

impl ExportSpec {
    /// Combine `--json`'s value with the command's `--jq` / `--template`.
    ///
    /// Mirrors gh's rules: `--jq` and `--template` need `--json FIELDS`, and
    /// only one of them can shape the output.
    pub fn resolve(json: Option<&str>, format: Format) -> Result<Option<Self>> {
        let fields: Vec<String> = json
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .collect();

        if fields.is_empty() {
            if format.jq.is_some() {
                return Err(CliError::invalid_argument(
                    "cannot use `--jq` without specifying `--json`",
                )
                .with_hint("name the fields to query, e.g. --json number,title --jq '.[].title'"));
            }
            if format.template.is_some() {
                return Err(CliError::invalid_argument(
                    "cannot use `--template` without specifying `--json`",
                ));
            }
            return Ok(None);
        }
        if format.jq.is_some() && format.template.is_some() {
            return Err(CliError::invalid_argument(
                "only one of `--jq` or `--template` may be used",
            ));
        }
        Ok(Some(Self {
            fields,
            jq: format.jq,
            template: format.template,
        }))
    }

    pub fn wants(&self, field: &str) -> bool {
        self.fields.iter().any(|f| f == field)
    }

    pub fn wants_any(&self, fields: &[&str]) -> bool {
        fields.iter().any(|f| self.wants(f))
    }

    /// Reject a field this command cannot export, the way gh does: name it and
    /// list what is available.
    pub fn validate(&self, available: &[&str]) -> Result<()> {
        if available.is_empty() {
            return Err(CliError::invalid_argument(
                "this command does not support `--json FIELDS`",
            )
            .with_hint("use --json on its own for the full JSON envelope"));
        }
        for field in &self.fields {
            if !available.contains(&field.as_str()) {
                let mut listing = available.to_vec();
                listing.sort_unstable();
                listing.dedup();
                return Err(
                    CliError::invalid_argument(format!("Unknown JSON field: \"{field}\""))
                        .with_details(serde_json::json!({ "available": listing }))
                        .with_hint(format!(
                            "for the whole JSON envelope, use --output json; the fields \
                             --json takes here are: {}",
                            listing.join(", ")
                        )),
                );
            }
        }
        Ok(())
    }
}

/// One object holding only `fields`, each looked up through `lookup`.
///
/// A `Map` is ordered by key, so the output matches gh's, whose Go maps
/// marshal sorted.
pub fn select(fields: &[String], lookup: impl Fn(&str) -> Value) -> Value {
    let mut object = Map::new();
    for field in fields {
        object.insert(field.clone(), lookup(field));
    }
    Value::Object(object)
}

/// Render an exported value for stdout: through `--jq` or `--template` when
/// given, otherwise as JSON — indented on a terminal, one line otherwise, as
/// gh prints it.
pub fn render(spec: &ExportSpec, value: &Value, pretty: bool, color: bool) -> Result<String> {
    if let Some(filter) = &spec.jq {
        return jq_lines(filter, value, pretty)
            .map(|lines| lines.into_iter().map(|line| format!("{line}\n")).collect());
    }
    if let Some(template) = &spec.template {
        return template_render(template, value, color);
    }
    Ok(if pretty {
        format!(
            "{}\n",
            serde_json::to_string_pretty(value).unwrap_or_default()
        )
    } else {
        format!("{}\n", serde_json::to_string(value).unwrap_or_default())
    })
}

// ---------------------------------------------------------------------------
// jq
// ---------------------------------------------------------------------------

/// Run a jq program over `input`, one output line per result.
///
/// Strings come out raw, everything else as JSON — gh's `--jq` behaviour,
/// which is what lets `--jq '.[].title'` feed `xargs` directly.
pub fn jq_lines(filter: &str, input: &Value, pretty: bool) -> Result<Vec<String>> {
    use jaq_core::load::{Arena, File, Loader};
    use jaq_core::{Compiler, Ctx, Vars, data, unwrap_valr};
    use jaq_json::Val;

    let invalid = |detail: String| {
        CliError::invalid_argument(format!("invalid jq expression `{filter}`: {detail}"))
    };

    let text = serde_json::to_string(input).map_err(|e| invalid(e.to_string()))?;
    let input =
        jaq_json::read::parse_single(text.as_bytes()).map_err(|e| invalid(e.to_string()))?;

    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let program = File {
        code: filter,
        path: (),
    };
    let modules = loader
        .load(&arena, program)
        .map_err(|errors| invalid(describe_load_errors(&errors)))?;
    let compiled = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|errors| {
            let names = errors
                .iter()
                .flat_map(|(_, errs)| errs.iter().map(|(name, _)| format!("`{name}`")))
                .collect::<Vec<_>>();
            invalid(format!("undefined: {}", names.join(", ")))
        })?;

    let ctx = Ctx::<data::JustLut<Val>>::new(&compiled.lut, Vars::new([]));
    let mut lines = Vec::new();
    for result in compiled.id.run((ctx, input)).map(unwrap_valr) {
        let value = result.map_err(|e| {
            CliError::invalid_argument(format!("jq expression `{filter}` failed: {e}"))
        })?;
        lines.push(match &value {
            Val::TStr(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            other if pretty => {
                // Re-read through serde for indentation: jaq prints compact.
                let compact = other.to_string();
                serde_json::from_str::<Value>(&compact)
                    .ok()
                    .and_then(|v| serde_json::to_string_pretty(&v).ok())
                    .unwrap_or(compact)
            }
            other => other.to_string(),
        });
    }
    Ok(lines)
}

fn describe_load_errors<S: std::fmt::Debug, P>(
    errors: &[(jaq_core::load::File<S, P>, jaq_core::load::Error<S>)],
) -> String {
    use jaq_core::load::Error;
    errors
        .iter()
        .map(|(_, error)| match error {
            Error::Io(_) => "could not read the program".to_string(),
            Error::Lex(lex) => format!(
                "unbalanced or unexpected token{}",
                if lex.is_empty() { "" } else { "s" }
            ),
            Error::Parse(_) => "syntax error".to_string(),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

// ---------------------------------------------------------------------------
// Go templates
// ---------------------------------------------------------------------------

thread_local! {
    /// Rows collected by `tablerow`, flushed by `tablerender` or at the end.
    static TABLE: RefCell<Vec<Vec<String>>> = const { RefCell::new(Vec::new()) };
    static COLOR: RefCell<bool> = const { RefCell::new(false) };
}

/// Render a Go template with gh's helper functions: `autocolor`, `color`,
/// `join`, `pluck`, `tablerow`, `tablerender`, `timeago`, `timefmt`,
/// `truncate` and `hyperlink`.
pub fn template_render(template: &str, input: &Value, color: bool) -> Result<String> {
    use gtmpl_ng::{Context, Template};

    COLOR.with(|c| *c.borrow_mut() = color);
    TABLE.with(|t| t.borrow_mut().clear());

    let mut tmpl = Template::default();
    tmpl.add_funcs(&[
        ("autocolor", autocolor as gtmpl_ng::Func),
        ("color", color_fn as gtmpl_ng::Func),
        ("join", join as gtmpl_ng::Func),
        ("pluck", pluck as gtmpl_ng::Func),
        ("tablerow", tablerow as gtmpl_ng::Func),
        ("tablerender", tablerender as gtmpl_ng::Func),
        ("timeago", timeago as gtmpl_ng::Func),
        ("timefmt", timefmt as gtmpl_ng::Func),
        ("truncate", truncate as gtmpl_ng::Func),
        ("hyperlink", hyperlink as gtmpl_ng::Func),
    ]);
    tmpl.parse(template)
        .map_err(|e| CliError::invalid_argument(format!("invalid template: {e}")))?;
    let context = Context::from(to_gtmpl(input));
    let mut out = tmpl
        .render(&context)
        .map_err(|e| CliError::invalid_argument(format!("template failed: {e}")))?;
    // A table that was started but never rendered still belongs in the output.
    out.push_str(&take_table());
    Ok(out)
}

fn to_gtmpl(value: &Value) -> gtmpl_ng::Value {
    use gtmpl_ng::Value as G;
    match value {
        Value::Null => G::Nil,
        Value::Bool(b) => G::Bool(*b),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => G::from(i),
            (None, Some(u), _) => G::from(u),
            (_, _, Some(f)) => G::from(f),
            _ => G::Nil,
        },
        Value::String(s) => G::String(s.clone()),
        Value::Array(items) => G::Array(items.iter().map(to_gtmpl).collect()),
        Value::Object(map) => G::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), to_gtmpl(v)))
                .collect::<HashMap<_, _>>(),
        ),
    }
}

fn text(value: &gtmpl_ng::Value) -> String {
    use gtmpl_ng::Value as G;
    match value {
        G::String(s) => s.clone(),
        G::Nil | G::NoValue => String::new(),
        other => other.to_string(),
    }
}

fn arg<'a>(
    args: &'a [gtmpl_ng::Value],
    index: usize,
    name: &str,
) -> std::result::Result<&'a gtmpl_ng::Value, gtmpl_ng::FuncError> {
    args.get(index)
        .ok_or_else(|| gtmpl_ng::FuncError::Generic(format!("{name}: missing argument")))
}

fn paint(style: &str, input: &str) -> String {
    let code = match style {
        "black" => "30",
        "red" => "31",
        "green" => "32",
        "yellow" => "33",
        "blue" => "34",
        "magenta" => "35",
        "cyan" => "36",
        "white" => "37",
        "gray" | "grey" => "90",
        "bold" => "1",
        "dim" => "2",
        _ => return input.to_string(),
    };
    format!("\x1b[{code}m{input}\x1b[0m")
}

fn autocolor(
    args: &[gtmpl_ng::Value],
) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    let style = text(arg(args, 0, "autocolor")?);
    let input = text(arg(args, 1, "autocolor")?);
    let on = COLOR.with(|c| *c.borrow());
    Ok(gtmpl_ng::Value::String(if on {
        paint(&style, &input)
    } else {
        input
    }))
}

fn color_fn(args: &[gtmpl_ng::Value]) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    let style = text(arg(args, 0, "color")?);
    let input = text(arg(args, 1, "color")?);
    Ok(gtmpl_ng::Value::String(paint(&style, &input)))
}

fn join(args: &[gtmpl_ng::Value]) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    let separator = text(arg(args, 0, "join")?);
    let items = match arg(args, 1, "join")? {
        gtmpl_ng::Value::Array(items) => items.iter().map(text).collect::<Vec<_>>(),
        other => vec![text(other)],
    };
    Ok(gtmpl_ng::Value::String(items.join(&separator)))
}

fn pluck(args: &[gtmpl_ng::Value]) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    let field = text(arg(args, 0, "pluck")?);
    let items = match arg(args, 1, "pluck")? {
        gtmpl_ng::Value::Array(items) => items.clone(),
        _ => Vec::new(),
    };
    Ok(gtmpl_ng::Value::Array(
        items
            .iter()
            .map(|item| match item {
                gtmpl_ng::Value::Map(map) | gtmpl_ng::Value::Object(map) => {
                    map.get(&field).cloned().unwrap_or(gtmpl_ng::Value::Nil)
                }
                _ => gtmpl_ng::Value::Nil,
            })
            .collect(),
    ))
}

fn tablerow(args: &[gtmpl_ng::Value]) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    TABLE.with(|t| t.borrow_mut().push(args.iter().map(text).collect()));
    Ok(gtmpl_ng::Value::String(String::new()))
}

fn tablerender(_: &[gtmpl_ng::Value]) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    Ok(gtmpl_ng::Value::String(take_table()))
}

fn take_table() -> String {
    let rows = TABLE.with(|t| std::mem::take(&mut *t.borrow_mut()));
    if rows.is_empty() {
        return String::new();
    }
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        let line = row
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                if i + 1 == row.len() {
                    cell.clone()
                } else {
                    format!("{cell:<width$}", width = widths[i])
                }
            })
            .collect::<Vec<_>>()
            .join("  ");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn timeago(args: &[gtmpl_ng::Value]) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    let raw = text(arg(args, 0, "timeago")?);
    let rendered = parse_iso8601(&raw)
        .map(crate::output::relative_time)
        .unwrap_or(raw);
    Ok(gtmpl_ng::Value::String(rendered))
}

fn timefmt(args: &[gtmpl_ng::Value]) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    let layout = text(arg(args, 0, "timefmt")?);
    let raw = text(arg(args, 1, "timefmt")?);
    let rendered = parse_iso8601(&raw)
        .map(|epoch| go_time_format(&layout, epoch))
        .unwrap_or(raw);
    Ok(gtmpl_ng::Value::String(rendered))
}

fn truncate(args: &[gtmpl_ng::Value]) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    let length = text(arg(args, 0, "truncate")?)
        .parse::<usize>()
        .map_err(|_| gtmpl_ng::FuncError::Generic("truncate: length must be a number".into()))?;
    let input = text(arg(args, 1, "truncate")?);
    Ok(gtmpl_ng::Value::String(truncate_text(&input, length)))
}

fn hyperlink(
    args: &[gtmpl_ng::Value],
) -> std::result::Result<gtmpl_ng::Value, gtmpl_ng::FuncError> {
    let url = text(arg(args, 0, "hyperlink")?);
    let label = args.get(1).map(text).unwrap_or_else(|| url.clone());
    let on = COLOR.with(|c| *c.borrow());
    Ok(gtmpl_ng::Value::String(if on {
        format!("\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\")
    } else {
        label
    }))
}

/// Cut `input` to `length` characters, marking the cut the way gh does.
pub fn truncate_text(input: &str, length: usize) -> String {
    let count = input.chars().count();
    if count <= length {
        return input.to_string();
    }
    if length <= 3 {
        return input.chars().take(length).collect();
    }
    let kept: String = input.chars().take(length - 3).collect();
    format!("{kept}...")
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// Seconds since the epoch, from a GitFox timestamp that may be milliseconds.
pub fn epoch_seconds(epoch: i64) -> i64 {
    if epoch.abs() > 100_000_000_000 {
        epoch / 1000
    } else {
        epoch
    }
}

/// `2026-09-14T10:11:12Z`, from a GitFox timestamp. What gh's `createdAt`
/// looks like.
pub fn iso8601(epoch: i64) -> String {
    let seconds = epoch_seconds(epoch);
    let days = seconds.div_euclid(86_400);
    let secs_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        secs_of_day % 3600 / 60,
        secs_of_day % 60
    )
}

/// A GitFox timestamp as gh would print a time field: ISO 8601, or JSON null
/// when GitFox sent nothing (or zero, its "never").
pub fn time_value(epoch: Option<i64>) -> Value {
    match epoch {
        Some(e) if e > 0 => Value::String(iso8601(e)),
        _ => Value::Null,
    }
}

/// Parse `2026-09-14T10:11:12Z` / `2026-09-14T10:11:12.345+08:00` to epoch
/// seconds.
pub fn parse_iso8601(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let (date, rest) = raw.split_once('T').or_else(|| raw.split_once(' '))?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    let (clock, offset_secs) = if let Some(clock) = rest.strip_suffix('Z') {
        (clock, 0)
    } else if let Some(pos) = rest.rfind(['+', '-']) {
        let (clock, offset) = rest.split_at(pos);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let digits: String = offset[1..].chars().filter(char::is_ascii_digit).collect();
        let hours: i64 = digits.get(0..2)?.parse().ok()?;
        let minutes: i64 = digits.get(2..4).unwrap_or("0").parse().ok()?;
        (clock, sign * (hours * 3600 + minutes * 60))
    } else {
        (rest, 0)
    };
    let clock = clock.split('.').next()?;
    let mut clock_parts = clock.split(':');
    let hour: i64 = clock_parts.next()?.parse().ok()?;
    let minute: i64 = clock_parts.next()?.parse().ok()?;
    let second: i64 = clock_parts.next().unwrap_or("0").parse().ok()?;

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs)
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (y, m, d).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The inverse of [`civil_from_days`].
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Format epoch seconds with a Go reference-time layout (`2006-01-02`,
/// `Jan 2, 2006 15:04`), in UTC. Covers the tokens templates actually use.
pub fn go_time_format(layout: &str, epoch: i64) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const DAYS: [&str; 7] = [
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
    ];
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let weekday = DAYS[days.rem_euclid(7) as usize];
    let month_name = MONTHS[(month - 1) as usize];
    let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };

    let tokens: [(&str, String); 22] = [
        ("January", month_name.to_string()),
        ("Monday", weekday.to_string()),
        ("2006", format!("{year:04}")),
        ("Z07:00", "Z".to_string()),
        ("-0700", "+0000".to_string()),
        ("Jan", month_name[..3].to_string()),
        ("Mon", weekday[..3].to_string()),
        ("MST", "UTC".to_string()),
        ("_2", format!("{day:>2}")),
        ("01", format!("{month:02}")),
        ("02", format!("{day:02}")),
        ("15", format!("{hour:02}")),
        ("03", format!("{hour12:02}")),
        ("04", format!("{minute:02}")),
        ("05", format!("{second:02}")),
        ("06", format!("{:02}", year % 100)),
        ("PM", if hour >= 12 { "PM" } else { "AM" }.to_string()),
        ("pm", if hour >= 12 { "pm" } else { "am" }.to_string()),
        ("1", month.to_string()),
        ("2", day.to_string()),
        ("3", hour12.to_string()),
        ("4", minute.to_string()),
    ];

    let mut out = String::new();
    let mut rest = layout;
    'outer: while !rest.is_empty() {
        for (token, value) in &tokens {
            if let Some(after) = rest.strip_prefix(token) {
                out.push_str(value);
                rest = after;
                continue 'outer;
            }
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
        }
        rest = chars.as_str();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(fields: &str, jq: Option<&str>, template: Option<&str>) -> ExportSpec {
        ExportSpec::resolve(
            Some(fields),
            Format {
                jq: jq.map(str::to_string),
                template: template.map(str::to_string),
            },
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn bare_json_is_not_an_export_and_jq_without_fields_is_refused() {
        assert!(
            ExportSpec::resolve(Some(""), Format::default())
                .unwrap()
                .is_none()
        );
        assert!(
            ExportSpec::resolve(None, Format::default())
                .unwrap()
                .is_none()
        );
        let err = ExportSpec::resolve(
            Some(""),
            Format {
                jq: Some(".".into()),
                template: None,
            },
        )
        .unwrap_err();
        assert!(
            err.message.contains("without specifying `--json`"),
            "{}",
            err.message
        );
        let both = ExportSpec::resolve(
            Some("number"),
            Format {
                jq: Some(".".into()),
                template: Some("{{.}}".into()),
            },
        );
        assert!(both.is_err());
    }

    #[test]
    fn fields_are_split_trimmed_and_validated_with_the_available_list() {
        let s = spec("number, title,", None, None);
        assert_eq!(s.fields, vec!["number", "title"]);
        assert!(s.validate(&["number", "title", "state"]).is_ok());
        let err = spec("number,nope", None, None)
            .validate(&["title", "number"])
            .unwrap_err();
        assert_eq!(err.message, "Unknown JSON field: \"nope\"");
        assert_eq!(err.details.unwrap()["available"][0], "number");
        assert!(spec("number", None, None).validate(&[]).is_err());
    }

    #[test]
    fn jq_prints_strings_raw_and_everything_else_as_json() {
        let input =
            json!([{ "number": 12, "title": "feat: OAuth" }, { "number": 13, "title": "x" }]);
        assert_eq!(
            jq_lines(".[].title", &input, false).unwrap(),
            vec!["feat: OAuth", "x"]
        );
        assert_eq!(
            jq_lines("map(.number)", &input, false).unwrap(),
            vec!["[12,13]"]
        );
        assert_eq!(
            jq_lines("[.[] | select(.number > 12)] | length", &input, false).unwrap(),
            vec!["1"]
        );
        let err = jq_lines(".[", &input, false).unwrap_err();
        assert!(
            err.message.starts_with("invalid jq expression"),
            "{}",
            err.message
        );
        let err = jq_lines("nosuchfn", &input, false).unwrap_err();
        assert!(err.message.contains("`nosuchfn`"), "{}", err.message);
    }

    #[test]
    fn templates_get_ghs_helpers() {
        let input = json!([
            { "number": 12, "title": "feat: add OAuth support", "labels": [{ "name": "a" }, { "name": "b" }],
              "createdAt": "2026-09-14T10:11:12Z" }
        ]);
        let out = template_render(
            "{{range .}}#{{.number}} {{truncate 10 .title}} [{{join \",\" (pluck \"name\" .labels)}}] {{timefmt \"2006-01-02\" .createdAt}}\n{{end}}",
            &input,
            false,
        )
        .unwrap();
        assert_eq!(out, "#12 feat: a... [a,b] 2026-09-14\n");

        // A shell can't easily type a newline, so gh users write `{{"\n"}}`.
        assert_eq!(
            template_render(r#"{{range .}}{{.number}}{{"\n"}}{{end}}"#, &input, false).unwrap(),
            "12\n"
        );

        let table = template_render(
            "{{range .}}{{tablerow .number .title}}{{end}}",
            &json!([{ "number": 1, "title": "one" }, { "number": 100, "title": "hundred" }]),
            false,
        )
        .unwrap();
        assert_eq!(table, "1    one\n100  hundred\n");
        assert!(template_render("{{", &input, false).is_err());
    }

    #[test]
    fn autocolor_respects_the_resolved_colour_setting() {
        let input = json!({ "state": "OPEN" });
        assert_eq!(
            template_render("{{autocolor \"green\" .state}}", &input, false).unwrap(),
            "OPEN"
        );
        assert_eq!(
            template_render("{{autocolor \"green\" .state}}", &input, true).unwrap(),
            "\x1b[32mOPEN\x1b[0m"
        );
    }

    #[test]
    fn timestamps_round_trip_through_iso8601_in_either_unit() {
        let seconds = 1_789_346_930;
        assert_eq!(iso8601(seconds), "2026-09-14T00:48:50Z");
        assert_eq!(iso8601(seconds * 1000 + 642), "2026-09-14T00:48:50Z");
        assert_eq!(parse_iso8601("2026-09-14T00:48:50Z"), Some(seconds));
        assert_eq!(parse_iso8601("2026-09-14T08:48:50+08:00"), Some(seconds));
        assert_eq!(parse_iso8601("2026-09-14T00:48:50.123Z"), Some(seconds));
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(time_value(Some(0)), Value::Null);
        assert_eq!(time_value(None), Value::Null);
        // A leap day, to exercise the calendar arithmetic.
        assert_eq!(
            parse_iso8601("2024-02-29T12:00:00Z")
                .map(iso8601)
                .as_deref(),
            Some("2024-02-29T12:00:00Z")
        );
    }

    #[test]
    fn go_layouts_cover_the_common_tokens() {
        let t = parse_iso8601("2026-09-14T15:04:05Z").unwrap();
        assert_eq!(
            go_time_format("2006-01-02 15:04:05", t),
            "2026-09-14 15:04:05"
        );
        assert_eq!(go_time_format("Jan 2, 2006", t), "Sep 14, 2026");
        assert_eq!(go_time_format("Monday 3:04PM", t), "Monday 3:04PM");
    }

    #[test]
    fn truncation_marks_the_cut() {
        assert_eq!(truncate_text("hello", 10), "hello");
        assert_eq!(truncate_text("hello world", 8), "hello...");
        assert_eq!(truncate_text("中文标题很长", 5), "中文...");
    }
}
