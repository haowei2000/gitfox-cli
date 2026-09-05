//! `fx api` — the escape hatch.
//!
//! Every GitFox endpoint is reachable from day one, which is what makes it safe
//! to add typed commands slowly instead of racing the server's API surface.

use std::collections::BTreeMap;
use std::io::Read;

use gitfox_client::Method;
use serde_json::{Map, Value, json};

use crate::cli::ApiArgs;
use crate::context::Context;
use crate::error::{CliError, Result};
use crate::output::{Json, Render};

const KNOWN_METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub async fn run(args: ApiArgs, ctx: &Context) -> Result<()> {
    let (explicit_method, path) = split_target(&args)?;
    let body = build_body(&args)?;
    let headers = parse_headers(&args.headers)?;

    // `fx api /path` is a GET; adding a body without naming a method means POST.
    let method = match explicit_method {
        Some(method) => method,
        None if body.is_some() => Method::POST,
        None => Method::GET,
    };

    let client = ctx.client()?;
    let response = client
        .request(method, &path, body.as_ref(), &headers)
        .await?;

    let output = ApiOutput {
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
        body: match &response.json {
            Some(value) => value.clone(),
            None if response.text.trim().is_empty() => Value::Null,
            // Not JSON (a diff, a log, a patch): hand the text back as-is.
            None => Value::String(response.text.clone()),
        },
        include: args.include,
    };
    ctx.renderer.emit(&output).map_err(io_error)
}

/// Split `METHOD PATH` from a bare `PATH`.
fn split_target(args: &ApiArgs) -> Result<(Option<Method>, String)> {
    match &args.path {
        Some(path) => Ok((Some(parse_method(&args.method_or_path)?), path.clone())),
        None => {
            let candidate = args.method_or_path.trim();
            if KNOWN_METHODS.contains(&candidate.to_ascii_uppercase().as_str()) {
                return Err(CliError::invalid_argument(format!(
                    "`{candidate}` is an HTTP method, not a path"
                ))
                .with_hint("usage: fx api [METHOD] PATH, e.g. `fx api GET /api/v1/user`"));
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

fn build_body(args: &ApiArgs) -> Result<Option<Value>> {
    if let Some(raw) = &args.body {
        return Ok(Some(parse_json(raw, "--body")?));
    }
    if let Some(source) = &args.input {
        let raw = read_input(source)?;
        if raw.trim().is_empty() {
            return Ok(None);
        }
        return Ok(Some(parse_json(&raw, "--input")?));
    }
    if args.fields.is_empty() && args.raw_fields.is_empty() {
        return Ok(None);
    }

    let mut object = Map::new();
    for field in &args.fields {
        let (key, value) = split_pair(field, '=', "--field")?;
        object.insert(key, infer_scalar(&value));
    }
    for field in &args.raw_fields {
        let (key, value) = split_pair(field, '=', "--raw-field")?;
        object.insert(key, Value::String(value));
    }
    Ok(Some(Value::Object(object)))
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

/// `--field` coerces the obvious scalars so `--field count=3` is a number, not
/// a string. `--raw-field` opts out.
fn infer_scalar(value: &str) -> Value {
    match value {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        other => match other.parse::<i64>() {
            Ok(n) => Value::from(n),
            Err(_) => match other.parse::<f64>() {
                Ok(n) => Value::from(n),
                Err(_) => Value::String(other.to_string()),
            },
        },
    }
}

fn parse_headers(raw: &[String]) -> Result<Vec<(String, String)>> {
    raw.iter()
        .map(|header| split_pair(header, ':', "--header"))
        .collect()
}

fn split_pair(raw: &str, separator: char, flag: &str) -> Result<(String, String)> {
    let Some((key, value)) = raw.split_once(separator) else {
        return Err(CliError::invalid_argument(format!(
            "{flag} expects `key{separator}value`, got `{raw}`"
        )));
    };
    let key = key.trim();
    if key.is_empty() {
        return Err(CliError::invalid_argument(format!(
            "{flag} has an empty key: `{raw}`"
        )));
    }
    Ok((key.to_string(), value.trim().to_string()))
}

fn io_error(err: std::io::Error) -> CliError {
    CliError::new(crate::error::ErrorCode::Unexpected, err.to_string())
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
            fields: vec![],
            raw_fields: vec![],
            body: None,
            input: None,
            headers: vec![],
            include: false,
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
    fn a_method_without_a_path_is_rejected_with_a_usage_hint() {
        let err = split_target(&args("GET", None)).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::InvalidArgument);
        assert!(err.hint.is_some());
    }

    #[test]
    fn unsupported_methods_are_rejected() {
        assert!(split_target(&args("BREW", Some("/x"))).is_err());
    }

    #[test]
    fn fields_are_typed_and_raw_fields_are_not() {
        let mut a = args("POST", Some("/x"));
        a.fields = vec![
            "name=test".into(),
            "count=3".into(),
            "ratio=1.5".into(),
            "draft=true".into(),
            "parent=null".into(),
        ];
        a.raw_fields = vec!["version=3".into()];
        let body = build_body(&a).unwrap().unwrap();
        assert_eq!(body["name"], "test");
        assert_eq!(body["count"], 3);
        assert_eq!(body["ratio"], 1.5);
        assert_eq!(body["draft"], true);
        assert!(body["parent"].is_null());
        assert_eq!(body["version"], "3");
    }

    #[test]
    fn body_is_parsed_as_json_and_bad_json_is_an_argument_error() {
        let mut a = args("POST", Some("/x"));
        a.body = Some(r#"{"name":"test"}"#.into());
        assert_eq!(build_body(&a).unwrap().unwrap()["name"], "test");

        a.body = Some("{not json".into());
        let err = build_body(&a).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::InvalidArgument);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn no_body_flags_means_no_body() {
        assert!(build_body(&args("GET", Some("/x"))).unwrap().is_none());
    }

    #[test]
    fn malformed_fields_and_headers_are_rejected() {
        let mut a = args("POST", Some("/x"));
        a.fields = vec!["novalue".into()];
        assert!(build_body(&a).is_err());

        a.fields = vec!["=empty".into()];
        assert!(build_body(&a).is_err());

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
