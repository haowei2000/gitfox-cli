//! End-to-end tests over the real binary.
//!
//! These lock down the three things machines depend on and humans never see:
//! the JSON envelope, the exit code, and the promise that a token never appears
//! in any output stream.

use std::path::Path;
use std::process::Output;

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A `fx` invocation isolated from the developer's real environment: its own
/// config file, no ambient GITFOX_* variables, no colour.
fn fx(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("fx").expect("the fx binary should build");
    for key in [
        "GITFOX_HOST",
        "GITFOX_TOKEN",
        "GITFOX_REPO",
        "GITFOX_ORG",
        "GITFOX_OUTPUT",
        "GITFOX_TIMEOUT",
        "GITFOX_INSECURE",
        "GITFOX_AGENT",
        "RUST_LOG",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("GITFOX_CONFIG", home.join("config.toml"));
    cmd.env("NO_COLOR", "1");
    // Run outside any git checkout so the git tier of the chain stays empty
    // unless a test sets one up on purpose.
    cmd.current_dir(home);
    cmd
}

fn stdout_json(output: &Output) -> Value {
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("stdout was not JSON ({e}): {text}"))
}

fn code(output: &Output) -> i32 {
    output
        .status
        .code()
        .expect("the process should not be signalled")
}

// ---------------------------------------------------------------------------
// surface
// ---------------------------------------------------------------------------

#[test]
fn help_and_version_succeed() {
    let home = TempDir::new().unwrap();
    let help = fx(home.path()).arg("--help").output().unwrap();
    assert_eq!(code(&help), 0);
    let text = String::from_utf8_lossy(&help.stdout);
    for expected in ["auth", "api", "repo", "pr", "pipeline", "config", "--agent"] {
        assert!(
            text.contains(expected),
            "`{expected}` missing from --help:\n{text}"
        );
    }

    let version = fx(home.path()).arg("--version").output().unwrap();
    assert_eq!(code(&version), 0);
    assert!(String::from_utf8_lossy(&version.stdout).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn an_unknown_command_exits_with_the_invalid_argument_code() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path()).arg("nonsense").output().unwrap();
    assert_eq!(code(&output), 2);
}

// ---------------------------------------------------------------------------
// configuration errors
// ---------------------------------------------------------------------------

#[test]
fn a_missing_host_is_a_config_error_with_the_documented_envelope() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .args(["--json", "api", "GET", "/api/v1/user"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 7);
    let body = stdout_json(&output);
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "CONFIG_ERROR");
    assert!(body["error"]["message"].is_string());
    assert!(body["error"].get("details").is_some());
}

#[test]
fn the_same_failure_stays_on_stderr_for_humans() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .args(["api", "GET", "/api/v1/user"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 7);
    assert!(
        output.stdout.is_empty(),
        "human errors must not touch stdout"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("error:"));
}

#[test]
fn an_unparseable_environment_variable_is_reported_not_ignored() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_TIMEOUT", "soon")
        .args(["--json", "api", "GET", "/x"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 7);
    assert_eq!(stdout_json(&output)["error"]["code"], "CONFIG_ERROR");
}

// ---------------------------------------------------------------------------
// roadmap commands
// ---------------------------------------------------------------------------

#[test]
fn commands_still_on_the_roadmap_say_so_in_a_structured_way() {
    let home = TempDir::new().unwrap();
    for (args, command, version) in [
        (vec!["--agent", "repo", "list"], "fx repo list", "v0.2"),
        (vec!["--agent", "pr", "diff"], "fx pr diff", "v0.5"),
        (
            vec!["--agent", "pipeline", "list"],
            "fx pipeline list",
            "v0.4",
        ),
    ] {
        let output = fx(home.path()).args(&args).output().unwrap();
        assert_eq!(code(&output), 9, "for {args:?}");
        let body = stdout_json(&output);
        assert_eq!(body["error"]["code"], "NOT_IMPLEMENTED");
        assert_eq!(body["error"]["details"]["command"], command);
        assert_eq!(body["error"]["details"]["planned_version"], version);
    }
}

// ---------------------------------------------------------------------------
// fx api against a mock GitFox
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn api_get_returns_the_success_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "uid": "whw" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "test-token")
        .args(["--json", "api", "GET", "/api/v1/user"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 0);
    let body = stdout_json(&output);
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["uid"], "whw");
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_mode_needs_no_json_flag_and_prints_no_colour() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "uid": "whw" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    for args in [
        vec!["--agent", "api", "/api/v1/user"],
        vec!["api", "/api/v1/user"],
    ] {
        let output = fx(home.path())
            .env("GITFOX_HOST", server.uri())
            .env("GITFOX_TOKEN", "test-token")
            // The second run gets agent mode from the environment instead.
            .env("GITFOX_AGENT", "1")
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(code(&output), 0);
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            !text.contains('\x1b'),
            "agent output must not contain ANSI escapes"
        );
        assert_eq!(stdout_json(&output)["ok"], true);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bare_path_defaults_to_get_and_fields_imply_post() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "via": "get" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/foo"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "via": "post" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let get = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--json", "api", "/api/v1/user"])
        .output()
        .unwrap();
    assert_eq!(stdout_json(&get)["data"]["via"], "get");

    let post = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--json", "api", "/api/v1/foo", "--field", "name=test"])
        .output()
        .unwrap();
    assert_eq!(code(&post), 0);
    assert_eq!(stdout_json(&post)["data"]["via"], "post");
}

#[tokio::test(flavor = "multi_thread")]
async fn http_failures_map_onto_the_documented_exit_codes() {
    let server = MockServer::start().await;
    for (endpoint, status, expected_code, expected_exit) in [
        ("unauthorized", 401, "AUTH_FAILED", 3),
        ("missing", 404, "NOT_FOUND", 4),
        ("broken", 500, "API_ERROR", 5),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/{endpoint}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({ "message": "nope" })))
            .mount(&server)
            .await;

        let home = TempDir::new().unwrap();
        let output = fx(home.path())
            .env("GITFOX_HOST", server.uri())
            .env("GITFOX_TOKEN", "test-token")
            .args(["--json", "api", "GET", &format!("/api/v1/{endpoint}")])
            .output()
            .unwrap();

        assert_eq!(code(&output), expected_exit, "for HTTP {status}");
        assert_eq!(stdout_json(&output)["error"]["code"], expected_code);
    }
}

#[test]
fn a_bad_method_or_body_fails_before_any_request_is_made() {
    let home = TempDir::new().unwrap();
    for args in [
        vec!["--json", "api", "BREW", "/api/v1/user"],
        vec!["--json", "api", "GET"],
        vec!["--json", "api", "POST", "/x", "--body", "{not json"],
    ] {
        let output = fx(home.path())
            .env("GITFOX_HOST", "https://git.example.com")
            .env("GITFOX_TOKEN", "t")
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(code(&output), 2, "for {args:?}");
        assert_eq!(
            stdout_json(&output)["error"]["code"],
            "INVALID_ARGUMENT",
            "for {args:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn jsonl_streams_one_line_per_element() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "id": 1 }, { "id": 2 }])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--output", "jsonl", "api", "/api/v1/repos"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 0);
    let text = String::from_utf8_lossy(&output.stdout);
    let rows: Vec<Value> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every line must be a JSON value"))
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], 1);
    assert_eq!(rows[1]["id"], 2);
}

// ---------------------------------------------------------------------------
// secrets
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_token_never_appears_in_any_output_stream() {
    const TOKEN: &str = "tok_do_not_leak_me_0123456789";

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "uid": "whw" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    // -vvv is the loudest fx gets, and the mode most likely to leak.
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", TOKEN)
        .args(["-vvv", "--json", "api", "GET", "/api/v1/user"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 0);
    for (name, stream) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        let text = String::from_utf8_lossy(stream);
        assert!(
            !text.contains(TOKEN),
            "the token leaked into {name}:\n{text}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_status_reports_the_token_without_printing_it() {
    const TOKEN: &str = "tok_status_secret_0123456789";

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "uid": "whw" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", TOKEN)
        .args(["--json", "auth", "status"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 0);
    let body = stdout_json(&output);
    assert_eq!(body["data"]["authenticated"], true);
    assert_eq!(body["data"]["token"], "configured");
    assert_eq!(body["data"]["token_source"], "env");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
}

#[test]
fn auth_status_without_a_token_exits_with_the_auth_code() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", "https://git.example.com")
        .args(["--json", "auth", "status"])
        .output()
        .unwrap();
    // 3 = auth error. 7 would mean fx could not even work out which host to ask.
    assert_eq!(
        code(&output),
        3,
        "expected an auth error:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(stdout_json(&output)["error"]["code"], "AUTH_REQUIRED");
}

// ---------------------------------------------------------------------------
// fx config
// ---------------------------------------------------------------------------

#[test]
fn config_set_then_get_round_trips_through_the_file() {
    let home = TempDir::new().unwrap();
    let set = fx(home.path())
        .args(["config", "set", "default_host", "git.example.com"])
        .output()
        .unwrap();
    assert_eq!(code(&set), 0);
    assert!(home.path().join("config.toml").exists());

    let get = fx(home.path())
        .args(["config", "get", "default_host"])
        .output()
        .unwrap();
    assert_eq!(code(&get), 0);
    assert_eq!(
        String::from_utf8_lossy(&get.stdout).trim(),
        "git.example.com"
    );

    // With a default host in the file, the host now resolves without any flag.
    let list = fx(home.path())
        .args(["--json", "config", "list"])
        .output()
        .unwrap();
    assert_eq!(
        stdout_json(&list)["data"]["resolved"]["host"],
        "git.example.com"
    );
}

#[test]
fn config_rejects_anything_token_shaped() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .args([
            "--json",
            "config",
            "set",
            "hosts.git.example.com.token",
            "secret",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    assert_eq!(stdout_json(&output)["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn a_missing_config_key_exits_with_the_not_found_code() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .args(["--json", "config", "get", "default_host"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 4);
    assert_eq!(stdout_json(&output)["error"]["code"], "NOT_FOUND");
}

// ---------------------------------------------------------------------------
// fx pr
// ---------------------------------------------------------------------------

/// One open pull request, as GitFox would return it.
fn sample_pr(number: u64) -> Value {
    json!({
        "number": number,
        "title": "feat: add OAuth",
        "description": "Adds the callback route.",
        "state": "open",
        "is_draft": false,
        "source_branch": "feat/oauth",
        "target_branch": "main",
        "author": { "id": 7, "uid": "whw", "display_name": "Haowei" },
        "created": 1_756_000_000_000i64,
        "updated": 1_756_000_000_000i64,
        "web_url": "https://git.example.com/ai/backend/pulls/12",
        "stats": { "commits": 3, "files_changed": 5, "additions": 120, "deletions": 8 }
    })
}

/// The repo-scoped path, with `ai/backend` encoded as one segment. If fx ever
/// stopped encoding the slash, none of these mocks would match.
const PR_PATH: &str = "/api/v1/repos/ai%2Fbackend/pullreq";

#[tokio::test(flavor = "multi_thread")]
async fn pr_list_renders_a_table_for_humans_and_a_list_for_machines() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([sample_pr(12)])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let base = || {
        let mut c = fx(home.path());
        c.env("GITFOX_HOST", server.uri())
            .env("GITFOX_TOKEN", "t")
            .env("GITFOX_REPO", "ai/backend");
        c
    };

    let human = base().args(["pr", "list"]).output().unwrap();
    assert_eq!(code(&human), 0);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("NUMBER"), "{text}");
    assert!(text.contains("#12"), "{text}");
    assert!(text.contains("feat/oauth → main"), "{text}");

    let machine = base().args(["--agent", "pr", "list"]).output().unwrap();
    let body = stdout_json(&machine);
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["repository"], "ai/backend");
    assert_eq!(body["data"]["count"], 1);
    assert_eq!(body["data"]["items"][0]["number"], 12);

    let lines = base()
        .args(["--output", "jsonl", "pr", "list"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&lines.stdout);
    let rows: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["number"], 12);
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_list_state_filter_reaches_the_server() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PR_PATH))
        .and(query_param("state", "merged"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "list", "--state", "merged"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0);
    assert_eq!(stdout_json(&output)["data"]["count"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_list_resolves_an_author_login_to_a_numeric_filter() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/principals"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 99, "uid": "whwilson", "display_name": "Someone Else" },
            { "id": 7, "uid": "whw", "display_name": "Haowei" }
        ])))
        .mount(&server)
        .await;
    // An exact uid match must win over the first fuzzy hit.
    Mock::given(method("GET"))
        .and(path(PR_PATH))
        .and(query_param("author_id", "7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([sample_pr(12)])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "list", "--author", "whw"])
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(stdout_json(&output)["data"]["count"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_view_by_number_and_a_missing_one() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{PR_PATH}/12")))
        .respond_with(ResponseTemplate::new(200).set_body_json(sample_pr(12)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{PR_PATH}/999")))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "not found" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let found = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "view", "12"])
        .output()
        .unwrap();
    assert_eq!(code(&found), 0);
    assert_eq!(stdout_json(&found)["data"]["title"], "feat: add OAuth");

    let missing = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "view", "999"])
        .output()
        .unwrap();
    // A missing pull request is more specific than a bare 404.
    assert_eq!(code(&missing), 4);
    assert_eq!(stdout_json(&missing)["error"]["code"], "PR_NOT_FOUND");
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_create_sends_the_branches_and_returns_the_new_pull_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(PR_PATH))
        .and(body_json(json!({
            "title": "feat: add OAuth",
            "description": "",
            "source_branch": "feat/oauth",
            "target_branch": "main",
            "is_draft": false
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(sample_pr(12)))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args([
            "--agent",
            "pr",
            "create",
            "-B",
            "main",
            "-H",
            "feat/oauth",
            "-t",
            "feat: add OAuth",
        ])
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(stdout_json(&output)["data"]["number"], 12);
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_create_takes_its_base_from_the_repository_default_branch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fbackend"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "identifier": "backend", "default_branch": "trunk" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(PR_PATH))
        .and(body_json(json!({
            "title": "t",
            "description": "",
            "source_branch": "feat/oauth",
            "target_branch": "trunk",
            "is_draft": false
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(sample_pr(12)))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "create", "-H", "feat/oauth", "-t", "t"])
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn pr_create_without_a_title_fails_instead_of_prompting_a_machine() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", "https://git.example.com")
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "create", "-B", "main", "-H", "feat/oauth"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    assert_eq!(stdout_json(&output)["error"]["code"], "INVALID_ARGUMENT");
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_merge_dry_run_reports_mergeability_without_merging() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{PR_PATH}/12")))
        .respond_with(ResponseTemplate::new(200).set_body_json(sample_pr(12)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{PR_PATH}/12/merge")))
        .and(body_json(json!({ "method": "squash", "dry_run": true })))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "mergeable": true, "dry_run": true, "allowed_methods": ["merge", "squash"] }),
        ))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "merge", "12", "-m", "squash", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let data = &stdout_json(&output)["data"];
    assert_eq!(data["merged"], false);
    assert_eq!(data["mergeable"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_merge_deletes_the_source_branch_with_a_second_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{PR_PATH}/12")))
        .respond_with(ResponseTemplate::new(200).set_body_json(sample_pr(12)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{PR_PATH}/12/merge")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "sha": "0123456789abcdef", "branch_deleted": false })),
        )
        .mount(&server)
        .await;
    // GitFox has no delete-branch flag on merge; this must be its own call.
    Mock::given(method("DELETE"))
        .and(path(format!("{PR_PATH}/12/branch")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "merge", "12", "--delete-branch"])
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let data = &stdout_json(&output)["data"];
    assert_eq!(data["merged"], true);
    assert_eq!(data["branch_deleted"], true);
}

// ---------------------------------------------------------------------------
// git context
// ---------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git should run");
    assert!(
        status.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn inside_a_checkout_the_repository_and_host_come_from_the_remote() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([sample_pr(12)])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let repo = home.path().join("checkout");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "feat/oauth"]);
    // GitFox serves git under /git/<space>/<name>.git.
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            &format!("{}/git/ai/backend.git", server.uri()),
        ],
    );

    // No -R, no GITFOX_REPO, no GITFOX_HOST: everything comes from the remote.
    let mut cmd = fx(home.path());
    cmd.current_dir(&repo)
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "pr", "list"]);
    let output = cmd.output().unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(stdout_json(&output)["data"]["repository"], "ai/backend");
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_number_the_current_branch_selects_the_pull_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PR_PATH))
        .and(query_param("source_branch", "feat/oauth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([sample_pr(12)])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let repo = home.path().join("checkout");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "feat/oauth"]);
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            &format!("{}/git/ai/backend.git", server.uri()),
        ],
    );

    let mut cmd = fx(home.path());
    cmd.current_dir(&repo)
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "pr", "view"]);
    let output = cmd.output().unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(stdout_json(&output)["data"]["number"], 12);
}

#[test]
fn outside_a_checkout_a_repo_scoped_command_says_what_is_missing() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", "https://git.example.com")
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "pr", "list"])
        .output()
        .unwrap();
    // 8 = git context error, distinct from "bad argument" or "not found".
    assert_eq!(code(&output), 8);
    assert_eq!(stdout_json(&output)["error"]["code"], "GIT_CONTEXT_ERROR");
}

#[test]
fn a_malformed_repository_reference_is_an_argument_error() {
    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", "https://git.example.com")
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "backend")
        .args(["--agent", "pr", "list"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    assert_eq!(stdout_json(&output)["error"]["code"], "INVALID_ARGUMENT");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_404_on_the_repository_path_names_the_repository_not_a_pull_request() {
    let server = MockServer::start().await;
    // GitFox answers 404 for a repository that is missing *or* invisible.
    Mock::given(method("GET"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "list"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 4);
    let error = &stdout_json(&output)["error"];
    assert_eq!(error["code"], "REPO_NOT_FOUND");
    assert!(
        error["message"].as_str().unwrap().contains("ai/backend"),
        "{error}"
    );
}
