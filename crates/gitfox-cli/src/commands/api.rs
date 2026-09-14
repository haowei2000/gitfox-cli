//! `fx api` — the escape hatch.
//!
//! Every GitFox endpoint is reachable from day one, which is what makes it safe
//! to add typed commands slowly instead of racing the server's API surface.
//!
//! The flags are gh's, with gh's meanings: `-F/--field` is typed and reads
//! `@file`, `-f/--raw-field` is always a string, `-X` names the method,
//! `key[]=v` / `key[sub]=v` build nested bodies, and on a GET the parameters
//! are the query string.

use std::collections::BTreeMap;
use std::io::Read;

use gitfox_client::{GitFoxClient, Method, RawResponse};
use serde_json::{Map, Value, json};

use crate::cli::ApiArgs;
use crate::context::Context;
use crate::error::{CliError, Result};
use crate::export;
use crate::output::{Json, Render};
use crate::paginate::MAX_PAGE_SIZE;

const KNOWN_METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub async fn run(args: ApiArgs, ctx: &Context) -> Result<()> {
    let (explicit_method, path) = split_target(&args)?;
    let path = fill_placeholders(&path, ctx, Spelling::Path)?;
    let params = build_params(&args, |text| fill_placeholders(text, ctx, Spelling::Value))?;
    let payload = read_payload(&args)?;
    let headers = parse_headers(&args.headers)?;

    // `fx api /path` is a GET; sending anything without naming a method means
    // POST.
    let has_payload = args.body.is_some() || args.input.is_some();
    let method = match explicit_method {
        Some(method) => method,
        None if has_payload || !params.is_empty() => Method::POST,
        None => Method::GET,
    };
    // Parameters are the JSON body, except where there is no body for them: a
    // GET or HEAD, or a request whose body is `--input` or `--body`. There
    // they are the query string, as gh sends them — a GET with a body would
    // reach the server with its filters ignored.
    let (path, body) = if params.is_empty() {
        (path, payload)
    } else if has_payload || method == Method::GET || method == Method::HEAD {
        (with_query(&path, &params), payload)
    } else {
        (path, Some(Value::Object(params)))
    };
    if args.paginate && method != Method::GET {
        return Err(CliError::invalid_argument(
            "the `--paginate` option is not supported for non-GET requests",
        ));
    }

    let client = match args.hostname.as_deref() {
        Some(host) => {
            let (api_url, token, _) = ctx.host_credentials(host);
            ctx.client_for(&api_url, token.as_ref())?
        }
        None => ctx.client()?,
    };

    let (response, body) = if args.paginate {
        paginate(&client, &path, &headers, args.slurp).await?
    } else {
        let response = client
            .request(method, &path, body.as_ref(), &headers)
            .await?;
        let body = response_body(&response);
        (response, body)
    };

    if args.silent {
        return if ctx.renderer.format().is_machine() {
            ctx.renderer.emit(&Json(Value::Null))
        } else {
            Ok(())
        };
    }
    if let Some(filter) = &args.jq {
        let lines = export::jq_lines(filter, &body, ctx.renderer.is_tty())?;
        return ctx.renderer.write_str(
            &lines
                .into_iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        );
    }
    if let Some(template) = &args.template {
        let text = export::template_render(template, &body, ctx.renderer.color())?;
        return ctx.renderer.write_str(&text);
    }

    ctx.renderer.emit(&ApiOutput {
        status: response.status,
        headers: response
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().unwrap_or("<binary>").to_string(),
                )
            })
            .collect(),
        body,
        include: args.include,
    })
}

fn response_body(response: &RawResponse) -> Value {
    match &response.json {
        Some(value) => value.clone(),
        None if response.text.trim().is_empty() => Value::Null,
        // Not JSON (a diff, a log, a patch): hand the text back as-is.
        None => Value::String(response.text.clone()),
    }
}

/// Walk a GitFox list endpoint page by page.
///
/// GitFox sends no `Link` header, so the pages are addressed with its own
/// `page` and `limit` parameters: whatever the path already says, starting at
/// its page, until a page comes back shorter than the limit. A response that is
/// not an array has no pages, and is returned as it came.
async fn paginate(
    client: &GitFoxClient,
    path: &str,
    headers: &[(String, String)],
    slurp: bool,
) -> Result<(RawResponse, Value)> {
    let mut url = url::Url::parse("http://fx.invalid/")
        .and_then(|base| base.join(path.trim_start_matches('/')))
        .map_err(|e| CliError::invalid_argument(format!("`{path}` is not a valid path: {e}")))?;
    let mut page: u64 = 1;
    let mut limit: u64 = MAX_PAGE_SIZE as u64;
    let others: Vec<(String, String)> = url
        .query_pairs()
        .filter_map(|(k, v)| match k.as_ref() {
            "page" => {
                page = v.parse().unwrap_or(1);
                None
            }
            "limit" => {
                limit = v.parse().unwrap_or(limit);
                None
            }
            _ => Some((k.into_owned(), v.into_owned())),
        })
        .collect();

    let mut items: Vec<Value> = Vec::new();
    let mut pages: Vec<Value> = Vec::new();
    loop {
        url.query_pairs_mut()
            .clear()
            .extend_pairs(others.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .append_pair("page", &page.to_string())
            .append_pair("limit", &limit.to_string());
        let request_path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
        let response = client
            .request(Method::GET, &request_path, None, headers)
            .await?;
        let body = response_body(&response);
        let Value::Array(batch) = body else {
            // Not a list: nothing to page through.
            return Ok((response, body));
        };
        let received = batch.len() as u64;
        if slurp {
            pages.push(Value::Array(batch));
        } else {
            items.extend(batch);
        }
        if received < limit || received == 0 {
            let merged = if slurp {
                Value::Array(pages)
            } else {
                Value::Array(items)
            };
            return Ok((response, merged));
        }
        page += 1;
    }
}

/// Split `METHOD PATH` from a bare `PATH`, or take the method from `-X`.
fn split_target(args: &ApiArgs) -> Result<(Option<Method>, String)> {
    match (&args.method, &args.path) {
        (Some(_), Some(_)) => Err(CliError::invalid_argument(
            "name the method either positionally or with -X/--method, not both",
        )),
        (Some(method), None) => Ok((Some(parse_method(method)?), args.method_or_path.clone())),
        (None, Some(path)) => Ok((Some(parse_method(&args.method_or_path)?), path.clone())),
        (None, None) => {
            let candidate = args.method_or_path.trim();
            if KNOWN_METHODS.contains(&candidate.to_ascii_uppercase().as_str()) {
                return Err(CliError::invalid_argument(format!(
                    "`{candidate}` is an HTTP method, not a path"
                ))
                .with_hint("usage: fx api [-X METHOD] PATH, e.g. `fx api -X POST /api/v1/foo`"));
            }
            Ok((None, candidate.to_string()))
        }
    }
}

fn parse_method(raw: &str) -> Result<Method> {
    let upper = raw.trim().to_ascii_uppercase();
    if !KNOWN_METHODS.contains(&upper.as_str()) {
        return Err(CliError::invalid_argument(format!(
            "unsupported HTTP method `{raw}`; expected one of {}",
            KNOWN_METHODS.join(", ")
        )));
    }
    Method::from_bytes(upper.as_bytes())
        .map_err(|_| CliError::invalid_argument(format!("unsupported HTTP method `{raw}`")))
}

/// Where a placeholder lands.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Spelling {
    /// In the endpoint path, where GitFox takes `space%2Fname`.
    Path,
    /// In a `-F` value, sent as it reads: `space/name`.
    Value,
}

/// gh's placeholders: `{owner}` and `{repo}` from the current repository,
/// `{branch}` from the checkout — plus `{repo_ref}`, the `space/name` GitFox's
/// repository paths take, encoded in a path.
fn fill_placeholders(text: &str, ctx: &Context, spelling: Spelling) -> Result<String> {
    if !text.contains('{') {
        return Ok(text.to_string());
    }
    let mut out = text.to_string();
    if ["{owner}", "{repo}", "{repo_ref}"]
        .iter()
        .any(|p| out.contains(p))
    {
        let repo = ctx.repo()?;
        let (reference, owner) = match spelling {
            Spelling::Path => (repo.encoded(), repo.space().replace('/', "%2F")),
            Spelling::Value => (repo.full(), repo.space().to_string()),
        };
        out = out
            .replace("{repo_ref}", &reference)
            .replace("{owner}", &owner)
            .replace("{repo}", repo.name());
    }
    if out.contains("{branch}") {
        let branch = ctx
            .branch()
            .map_err(|e| CliError::new(e.code, "`{branch}` needs a checked-out branch"))?;
        out = out.replace("{branch}", branch);
    }
    Ok(out)
}

/// `--body` or `--input`: the request body, as given.
fn read_payload(args: &ApiArgs) -> Result<Option<Value>> {
    if let Some(raw) = &args.body {
        return parse_json(raw, "--body").map(Some);
    }
    if let Some(source) = &args.input {
        let raw = read_input(source)?;
        if raw.trim().is_empty() {
            return Ok(None);
        }
        return parse_json(&raw, "--input").map(Some);
    }
    Ok(None)
}

/// `-f` and `-F`, parsed as gh parses them — raw fields first, as gh does.
/// `fill` fills the placeholders of a typed string value.
fn build_params(
    args: &ApiArgs,
    fill: impl Fn(&str) -> Result<String>,
) -> Result<Map<String, Value>> {
    let mut params = Map::new();
    for field in &args.raw_fields {
        add_field(&mut params, field, "--raw-field", |raw| {
            Ok(Value::String(raw.to_string()))
        })?;
    }
    for field in &args.fields {
        add_field(&mut params, field, "--field", |raw| typed_value(raw, &fill))?;
    }
    Ok(params)
}

/// gh's typed value: `@file` or `@-` is read, an integer or `true`, `false`
/// or `null` becomes that JSON value, and anything else is a string with its
/// placeholders filled. A decimal stays a string, as in gh: `1.10` is not
/// `1.1`.
fn typed_value(raw: &str, fill: impl Fn(&str) -> Result<String>) -> Result<Value> {
    if let Some(source) = raw.strip_prefix('@') {
        return read_input(source).map(Value::String);
    }
    if let Ok(n) = raw.parse::<i64>() {
        return Ok(Value::from(n));
    }
    Ok(match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        text => Value::String(fill(text)?),
    })
}

/// Place one `key=value` in `params`, with gh's key syntax: `a[b]=v` nests an
/// object, `a[]=v` appends to an array, `a[]` alone is an empty array, and
/// `a[][b]=v` fills the array's last object until `b` repeats, which starts
/// the next one. The value is taken as written — not trimmed.
fn add_field(
    params: &mut Map<String, Value>,
    field: &str,
    flag: &str,
    value_of: impl FnOnce(&str) -> Result<Value>,
) -> Result<()> {
    let (key, raw) = match field.split_once('=') {
        Some((key, raw)) => (key, Some(raw)),
        None => (field, None),
    };
    let segments = key_segments(key).ok_or_else(|| {
        CliError::invalid_argument(format!("{flag} has an invalid key: `{field}`"))
    })?;
    let value = raw.map(value_of).transpose()?;

    let mut dest = params;
    let mut in_array = false;
    let mut name = segments[0];
    for &segment in &segments[1..] {
        if segment.is_empty() {
            in_array = true;
            continue;
        }
        dest = if in_array {
            last_object(dest, name, segment, key)?
        } else {
            object_under(dest, name, key)?
        };
        in_array = false;
        name = segment;
    }

    if in_array {
        let slot = dest
            .entry(name.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        match (slot, value) {
            (slot, None) => *slot = Value::Array(Vec::new()),
            (Value::Array(items), Some(value)) => items.push(value),
            _ => {
                return Err(CliError::invalid_argument(format!(
                    "`{key}` appends to `{name}`, which is not an array"
                )));
            }
        }
        return Ok(());
    }
    // Only `key[]` may stand without a value.
    let Some(value) = value else {
        return Err(CliError::invalid_argument(format!(
            "{flag} expects `key=value`, got `{field}`"
        )));
    };
    if dest.contains_key(name) {
        return Err(CliError::invalid_argument(format!(
            "`{key}` is given more than once"
        )));
    }
    dest.insert(name.to_string(), value);
    Ok(())
}

/// `a[b][]` → `["a", "b", ""]`; `None` for a key that is not of that form.
fn key_segments(key: &str) -> Option<Vec<&str>> {
    let (head, mut rest) = key.split_at(key.find('[').unwrap_or(key.len()));
    if head.is_empty() || head.contains(']') {
        return None;
    }
    let mut segments = vec![head];
    while !rest.is_empty() {
        let (segment, after) = rest.strip_prefix('[')?.split_once(']')?;
        if segment.contains('[') {
            return None;
        }
        segments.push(segment);
        rest = after;
    }
    Some(segments)
}

/// The object under `name`, created when absent.
fn object_under<'m>(
    map: &'m mut Map<String, Value>,
    name: &str,
    key: &str,
) -> Result<&'m mut Map<String, Value>> {
    map.entry(name.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            CliError::invalid_argument(format!(
                "`{key}` nests under `{name}`, which is not an object"
            ))
        })
}

/// The object `name[][next]` fills: the array's last object, unless that
/// already has `next`, which starts a new one — gh's rule, so that
/// `-F v[][name]=a -F v[][color]=red -F v[][name]=b` is two objects.
fn last_object<'m>(
    map: &'m mut Map<String, Value>,
    name: &str,
    next: &str,
    key: &str,
) -> Result<&'m mut Map<String, Value>> {
    let not_an_array = || {
        CliError::invalid_argument(format!(
            "`{key}` appends to `{name}`, which is not an array"
        ))
    };
    let items = map
        .entry(name.to_string())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(not_an_array)?;
    let reuse = items
        .last()
        .and_then(Value::as_object)
        .is_some_and(|last| !last.contains_key(next));
    if !reuse {
        items.push(Value::Object(Map::new()));
    }
    items
        .last_mut()
        .and_then(Value::as_object_mut)
        .ok_or_else(not_an_array)
}

/// Parameters as a query string: gh's choice wherever they cannot be the JSON
/// body. An array repeats its key — `label_id=1&label_id=2`, which is how
/// GitFox's list filters take several values — and an object nests as
/// `key[sub]=value`.
fn with_query(path: &str, params: &Map<String, Value>) -> String {
    fn flatten(key: &str, value: &Value, pairs: &mut Vec<(String, String)>) {
        match value {
            Value::Null => pairs.push((key.to_string(), String::new())),
            Value::String(text) => pairs.push((key.to_string(), text.clone())),
            Value::Array(items) => {
                for item in items {
                    flatten(key, item, pairs);
                }
            }
            Value::Object(map) => {
                for (sub, item) in map {
                    flatten(&format!("{key}[{sub}]"), item, pairs);
                }
            }
            other => pairs.push((key.to_string(), other.to_string())),
        }
    }

    let mut pairs = Vec::new();
    for (key, value) in params {
        flatten(key, value, &mut pairs);
    }
    if pairs.is_empty() {
        return path.to_string();
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&pairs)
        .finish();
    let separator = if path.contains('?') { '&' } else { '?' };
    format!("{path}{separator}{query}")
}

fn read_input(source: &str) -> Result<String> {
    if source == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| CliError::invalid_argument(format!("could not read stdin: {e}")))?;
        return Ok(buf);
    }
    std::fs::read_to_string(source)
        .map_err(|e| CliError::invalid_argument(format!("could not read {source}: {e}")))
}

fn parse_json(raw: &str, flag: &str) -> Result<Value> {
    serde_json::from_str(raw)
        .map_err(|e| CliError::invalid_argument(format!("{flag} is not valid JSON: {e}")))
}

/// `Name: value`, trimmed on both sides of the colon.
fn parse_headers(raw: &[String]) -> Result<Vec<(String, String)>> {
    raw.iter()
        .map(|header| {
            let Some((name, value)) = header.split_once(':') else {
                return Err(CliError::invalid_argument(format!(
                    "--header expects `name: value`, got `{header}`"
                )));
            };
            let name = name.trim();
            if name.is_empty() {
                return Err(CliError::invalid_argument(format!(
                    "--header has an empty name: `{header}`"
                )));
            }
            Ok((name.to_string(), value.trim().to_string()))
        })
        .collect()
}

struct ApiOutput {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Value,
    include: bool,
}

impl Render for ApiOutput {
    fn to_json(&self) -> Value {
        if self.include {
            json!({
                "status": self.status,
                "headers": self.headers,
                "body": self.body,
            })
        } else {
            self.body.clone()
        }
    }

    fn to_human(&self, color: bool) -> String {
        let pretty = Json(self.body.clone()).to_human(color);
        if !self.include {
            return pretty;
        }
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("HTTP {}\n{headers}\n\n{pretty}", self.status)
    }

    fn to_jsonl(&self) -> Vec<Value> {
        if self.include {
            return vec![self.to_json()];
        }
        Json(self.body.clone()).to_jsonl()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(method_or_path: &str, path: Option<&str>) -> ApiArgs {
        ApiArgs {
            method_or_path: method_or_path.to_string(),
            path: path.map(str::to_string),
            method: None,
            fields: vec![],
            raw_fields: vec![],
            body: None,
            input: None,
            headers: vec![],
            include: false,
            paginate: false,
            slurp: false,
            jq: None,
            template: None,
            silent: false,
            hostname: None,
            cache: None,
            previews: vec![],
            allow_escape_sequences: false,
        }
    }

    #[test]
    fn a_bare_path_is_a_get() {
        let (method, path) = split_target(&args("/api/v1/user", None)).unwrap();
        assert!(method.is_none());
        assert_eq!(path, "/api/v1/user");
    }

    #[test]
    fn an_explicit_method_is_parsed_case_insensitively() {
        let (method, path) = split_target(&args("post", Some("/api/v1/foo"))).unwrap();
        assert_eq!(method, Some(Method::POST));
        assert_eq!(path, "/api/v1/foo");
    }

    #[test]
    fn dash_x_names_the_method_as_in_gh() {
        let mut a = args("/api/v1/foo", None);
        a.method = Some("patch".into());
        let (method, path) = split_target(&a).unwrap();
        assert_eq!(method, Some(Method::PATCH));
        assert_eq!(path, "/api/v1/foo");

        // Both spellings at once is ambiguous.
        let mut both = args("POST", Some("/x"));
        both.method = Some("PUT".into());
        assert!(split_target(&both).is_err());
    }

    #[test]
    fn a_method_without_a_path_is_rejected_with_a_usage_hint() {
        let err = split_target(&args("GET", None)).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::InvalidArgument);
        assert!(err.hint.is_some());
    }

    #[test]
    fn unsupported_methods_are_rejected() {
        assert!(split_target(&args("BREW", Some("/x"))).is_err());
    }

    /// Parameters with placeholders left as they are.
    fn params(a: &ApiArgs) -> Result<Value> {
        build_params(a, |text| Ok(text.to_string())).map(Value::Object)
    }

    fn fields(list: &[&str]) -> ApiArgs {
        let mut a = args("POST", Some("/x"));
        a.fields = list.iter().map(|f| f.to_string()).collect();
        a
    }

    #[test]
    fn fields_are_typed_and_raw_fields_are_not() {
        let mut a = fields(&[
            "name=test",
            "count=3",
            "draft=true",
            "parent=null",
            // gh converts integers only, so a decimal is not rounded through a float.
            "version=1.10",
        ]);
        a.raw_fields = vec!["tag=3".into(), "flag=true".into()];
        let body = params(&a).unwrap();
        assert_eq!(body["name"], "test");
        assert_eq!(body["count"], 3);
        assert_eq!(body["draft"], true);
        assert!(body["parent"].is_null());
        assert_eq!(body["version"], "1.10");
        assert_eq!(body["tag"], "3");
        assert_eq!(body["flag"], "true");
    }

    #[test]
    fn a_value_is_sent_as_written_not_trimmed() {
        let mut a = args("POST", Some("/x"));
        a.raw_fields = vec!["text=  indented\n".into()];
        assert_eq!(params(&a).unwrap()["text"], "  indented\n");
    }

    #[test]
    fn a_typed_field_can_read_its_value_from_a_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("body.md");
        std::fs::write(&file, "# Hello\n").unwrap();
        let a = fields(&[&format!("description=@{}", file.display())]);
        assert_eq!(params(&a).unwrap()["description"], "# Hello\n");
    }

    #[test]
    fn a_typed_string_has_its_placeholders_filled_and_a_raw_one_does_not() {
        let mut a = fields(&["source_repo_ref={repo_ref}", "count=7"]);
        a.raw_fields = vec!["literal={repo_ref}".into()];
        let body = build_params(&a, |text| Ok(text.replace("{repo_ref}", "ai/backend"))).unwrap();
        assert_eq!(body["source_repo_ref"], "ai/backend");
        assert_eq!(body["literal"], "{repo_ref}");
        assert_eq!(body["count"], 7);
    }

    #[test]
    fn bracketed_keys_build_nested_bodies() {
        let body = params(&fields(&[
            "labels[]=bug",
            "labels[]=ui",
            "pattern[default]=true",
            "pattern[include][]=main",
            "reviewers[]",
        ]))
        .unwrap();
        assert_eq!(
            body,
            json!({
                "labels": ["bug", "ui"],
                "pattern": { "default": true, "include": ["main"] },
                "reviewers": []
            })
        );
    }

    #[test]
    fn an_array_of_objects_fills_each_object_until_a_key_repeats() {
        let body = params(&fields(&[
            "values[][value]=high",
            "values[][color]=red",
            "values[][value]=low",
            "values[][color]=blue",
        ]))
        .unwrap();
        assert_eq!(
            body,
            json!({ "values": [
                { "value": "high", "color": "red" },
                { "value": "low", "color": "blue" }
            ] })
        );
    }

    #[test]
    fn malformed_keys_are_rejected() {
        for bad in [
            "novalue",
            "=empty",
            "[a]=1",
            "a[b=1",
            "a]=1",
            "a[b]c=1",
            "a[b[c]]=1",
            "a[b]",
        ] {
            assert!(params(&fields(&[bad])).is_err(), "{bad} should be rejected");
        }
        // A key given twice is ambiguous, as in gh.
        assert!(params(&fields(&["a=1", "a=2"])).is_err());
        // So is appending to something that is not an array.
        assert!(params(&fields(&["a=1", "a[]=2"])).is_err());
        assert!(params(&fields(&["a=1", "a[b]=2"])).is_err());
    }

    #[test]
    fn parameters_become_a_query_string_where_there_is_no_body_for_them() {
        let body = params(&fields(&[
            "state=open",
            "limit=5",
            "label_id[]=1",
            "label_id[]=2",
            "query=feat oauth",
            "sort[by]=updated",
        ]))
        .unwrap();
        let Value::Object(map) = body else {
            unreachable!()
        };
        assert_eq!(
            with_query("/api/v1/repos/ai%2Fbackend/pullreq", &map),
            "/api/v1/repos/ai%2Fbackend/pullreq?label_id=1&label_id=2&limit=5\
             &query=feat+oauth&sort%5Bby%5D=updated&state=open"
        );
        assert_eq!(
            with_query("/x?page=2", &map).split_once('&').unwrap().0,
            "/x?page=2"
        );
        assert_eq!(with_query("/x", &Map::new()), "/x");
    }

    #[test]
    fn body_is_parsed_as_json_and_bad_json_is_an_argument_error() {
        let mut a = args("POST", Some("/x"));
        a.body = Some(r#"{"name":"test"}"#.into());
        assert_eq!(read_payload(&a).unwrap().unwrap()["name"], "test");

        a.body = Some("{not json".into());
        let err = read_payload(&a).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::InvalidArgument);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn no_body_flags_means_no_body() {
        let a = args("GET", Some("/x"));
        assert!(read_payload(&a).unwrap().is_none());
        assert!(build_params(&a, |t| Ok(t.to_string())).unwrap().is_empty());
    }

    #[test]
    fn malformed_headers_are_rejected() {
        assert!(parse_headers(&["no-colon".to_string()]).is_err());
        assert_eq!(
            parse_headers(&["X-Trace: 1".to_string()]).unwrap(),
            vec![("X-Trace".to_string(), "1".to_string())]
        );
    }

    #[test]
    fn include_wraps_the_body_with_status_and_headers() {
        let output = ApiOutput {
            status: 201,
            headers: BTreeMap::from([("content-type".into(), "application/json".into())]),
            body: json!({ "id": 1 }),
            include: true,
        };
        let value = output.to_json();
        assert_eq!(value["status"], 201);
        assert_eq!(value["body"]["id"], 1);
        assert!(output.to_human(false).starts_with("HTTP 201"));
    }

    #[test]
    fn without_include_the_body_is_passed_through_untouched() {
        let output = ApiOutput {
            status: 200,
            headers: BTreeMap::new(),
            body: json!([{ "id": 1 }, { "id": 2 }]),
            include: false,
        };
        assert_eq!(output.to_json(), json!([{ "id": 1 }, { "id": 2 }]));
        assert_eq!(output.to_jsonl().len(), 2);
    }
}
