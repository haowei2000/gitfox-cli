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
    // Everything in the v0.1-v0.4 scope is implemented; only v0.5 remains.
    for (args, command, version) in [
        (vec!["--agent", "pr", "diff"], "fx pr diff", "v0.5"),
        (vec!["--agent", "pr", "checks"], "fx pr checks", "v0.5"),
        (vec!["--agent", "pr", "checkout"], "fx pr checkout", "v0.5"),
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

// ---------------------------------------------------------------------------
// fx pipeline
// ---------------------------------------------------------------------------

const PIPE_PATH: &str = "/api/v1/repos/ai%2Fbackend/pipelines";

/// A run with one failed step, one that passed, and one that never ran.
fn failing_execution() -> Value {
    json!({
        "number": 182,
        "status": "failure",
        "message": "feat: add OAuth",
        "target": "main",
        "author_login": "whw",
        "event": "push",
        "started": 1_756_000_000_000i64,
        "stages": [{
            "number": 1, "name": "build", "status": "failure",
            "steps": [
                { "number": 1, "name": "clone", "status": "success" },
                { "number": 2, "name": "cargo test", "status": "failure", "exit_code": 101 },
                { "number": 3, "name": "cargo clippy", "status": "skipped" }
            ]
        }]
    })
}

fn pipeline_env(home: &Path, uri: &str) -> Command {
    let mut cmd = fx(home);
    cmd.env("GITFOX_HOST", uri)
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend");
    cmd
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_list_shows_each_pipelines_latest_run_in_one_request() {
    let server = MockServer::start().await;
    // ?latest=true embeds the most recent execution per pipeline.
    Mock::given(method("GET"))
        .and(path(PIPE_PATH))
        .and(query_param("latest", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "identifier": "default", "execution": failing_execution() },
            { "identifier": "nightly", "execution": null }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = pipeline_env(home.path(), &server.uri())
        .args(["--agent", "pipeline", "list"])
        .output()
        .unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let data = &stdout_json(&output)["data"];
    // The pipeline that has never run contributes no row.
    assert_eq!(data["count"], 1);
    assert_eq!(data["items"][0]["pipeline"], "default");
    assert_eq!(data["items"][0]["number"], 182);
    assert_eq!(data["items"][0]["branch"], "main");
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_list_for_one_pipeline_uses_the_executions_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{PIPE_PATH}/default/executions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([failing_execution()])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = pipeline_env(home.path(), &server.uri())
        .args(["--agent", "pipeline", "list", "--pipeline", "default"])
        .output()
        .unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    // Same schema as the unfiltered form.
    let data = &stdout_json(&output)["data"];
    assert_eq!(data["items"][0]["pipeline"], "default");
    assert_eq!(data["items"][0]["status"], "failure");
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_failed_fetches_only_the_failed_step() {
    let server = MockServer::start().await;
    // The pipeline is inferred: the repository has exactly one.
    Mock::given(method("GET"))
        .and(path(PIPE_PATH))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([{ "identifier": "default" }])),
        )
        .mount(&server)
        .await;
    // The run number is inferred: the most recent one.
    Mock::given(method("GET"))
        .and(path(format!("{PIPE_PATH}/default/executions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([failing_execution()])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{PIPE_PATH}/default/executions/182")))
        .respond_with(ResponseTemplate::new(200).set_body_json(failing_execution()))
        .mount(&server)
        .await;
    // Only stage 1 / step 2 — the one that failed — may be asked for.
    Mock::given(method("GET"))
        .and(path(format!("{PIPE_PATH}/default/executions/182/logs/1/2")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "pos": 0, "out": "error[E0308]: mismatched types\n", "time": 1 },
            { "pos": 1, "out": "error: aborting due to 1 previous error", "time": 2 }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = pipeline_env(home.path(), &server.uri())
        .args(["--agent", "pipeline", "logs", "--failed"])
        .output()
        .unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let data = &stdout_json(&output)["data"];
    assert_eq!(data["run"], 182);
    assert_eq!(data["only_failed"], true);
    assert_eq!(data["count"], 1);
    let step = &data["steps"][0];
    assert_eq!(step["stage"], "build");
    assert_eq!(step["step"], "cargo test");
    assert_eq!(step["exit_code"], 101);
    assert_eq!(step["lines"][0], "error[E0308]: mismatched types");
    assert_eq!(step["lines"][1], "error: aborting due to 1 previous error");
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_for_a_green_run_says_so_and_still_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{PIPE_PATH}/default/executions/9")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "number": 9, "status": "success",
            "stages": [{ "number": 1, "name": "build", "status": "success",
                "steps": [{ "number": 1, "name": "cargo test", "status": "success" }] }]
        })))
        .mount(&server)
        .await;
    // No log endpoint is mounted: asking for one would fail the test.

    let home = TempDir::new().unwrap();
    let output = pipeline_env(home.path(), &server.uri())
        .args(["pipeline", "logs", "9", "--pipeline", "default", "--failed"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 0);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("No failed steps"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn several_pipelines_refuse_to_guess_and_name_the_choices() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PIPE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "identifier": "default" },
            { "identifier": "nightly" }
        ])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = pipeline_env(home.path(), &server.uri())
        .args(["--agent", "pipeline", "logs", "--failed"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 2);
    let error = &stdout_json(&output)["error"];
    assert_eq!(error["code"], "INVALID_ARGUMENT");
    assert!(
        error["message"].as_str().unwrap().contains("2 pipelines"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repository_with_no_pipelines_is_reported_as_such() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PIPE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = pipeline_env(home.path(), &server.uri())
        .args(["--agent", "pipeline", "view"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 4);
    assert_eq!(stdout_json(&output)["error"]["code"], "PIPELINE_NOT_FOUND");
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_view_returns_the_stage_tree() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{PIPE_PATH}/default/executions/182")))
        .respond_with(ResponseTemplate::new(200).set_body_json(failing_execution()))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = pipeline_env(home.path(), &server.uri())
        .args([
            "--agent",
            "pipeline",
            "view",
            "182",
            "--pipeline",
            "default",
        ])
        .output()
        .unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let data = &stdout_json(&output)["data"];
    assert_eq!(data["stages"][0]["name"], "build");
    assert_eq!(data["stages"][0]["steps"][1]["status"], "failure");
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_retry_and_run_start_a_new_execution() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("{PIPE_PATH}/default/executions/182/retry")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "number": 183, "status": "pending" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{PIPE_PATH}/default/executions")))
        .and(query_param("branch", "main"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "number": 184, "status": "pending" })),
        )
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let retried = pipeline_env(home.path(), &server.uri())
        .args([
            "--agent",
            "pipeline",
            "retry",
            "182",
            "--pipeline",
            "default",
        ])
        .output()
        .unwrap();
    assert_eq!(
        code(&retried),
        0,
        "{}",
        String::from_utf8_lossy(&retried.stdout)
    );
    assert_eq!(stdout_json(&retried)["data"]["number"], 183);

    let started = pipeline_env(home.path(), &server.uri())
        .args(["--agent", "pipeline", "run", "default", "-b", "main"])
        .output()
        .unwrap();
    assert_eq!(
        code(&started),
        0,
        "{}",
        String::from_utf8_lossy(&started.stdout)
    );
    assert_eq!(stdout_json(&started)["data"]["number"], 184);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_step_whose_log_is_not_ready_yet_does_not_fail_the_command() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{PIPE_PATH}/default/executions/182")))
        .respond_with(ResponseTemplate::new(200).set_body_json(failing_execution()))
        .mount(&server)
        .await;
    // The step exists in the tree but its log is gone.
    Mock::given(method("GET"))
        .and(path(format!("{PIPE_PATH}/default/executions/182/logs/1/2")))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "not found" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = pipeline_env(home.path(), &server.uri())
        .args([
            "--agent",
            "pipeline",
            "logs",
            "182",
            "--pipeline",
            "default",
            "--failed",
        ])
        .output()
        .unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let step = &stdout_json(&output)["data"]["steps"][0];
    assert_eq!(step["step"], "cargo test");
    // Still reported, with the status and exit code, just without output.
    assert_eq!(step["exit_code"], 101);
    assert_eq!(step["lines"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// fx repo
// ---------------------------------------------------------------------------

fn repo_json(name: &str, is_public: Option<bool>) -> Value {
    let mut value = json!({
        "identifier": name,
        "path": format!("ai/{name}"),
        "description": "The backend",
        "default_branch": "main",
        "num_open_pulls": 2,
        "updated": 1_756_000_000_000i64
    });
    if let Some(public) = is_public {
        value["is_public"] = json!(public);
    }
    value
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_list_without_a_space_spans_the_instance() {
    let server = MockServer::start().await;
    // `GET /repos` answers with the shape that has no is_public.
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("sort", "identifier"))
        .and(query_param("order", "asc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            repo_json("backend", None),
            repo_json("frontend", None)
        ])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let human = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["repo", "list"])
        .output()
        .unwrap();
    assert_eq!(
        code(&human),
        0,
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("ai/backend"), "{text}");
    // No visibility to show, so no column of dashes.
    assert!(!text.contains("VISIBILITY"), "{text}");

    let machine = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "repo", "list"])
        .output()
        .unwrap();
    let data = &stdout_json(&machine)["data"];
    assert_eq!(data["count"], 2);
    assert!(data["space"].is_null());
    assert!(data["items"][0]["visibility"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_list_in_a_space_reports_visibility() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/spaces/ai/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            repo_json("backend", Some(false)),
            repo_json("docs", Some(true))
        ])))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let human = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["repo", "list", "ai"])
        .output()
        .unwrap();
    assert_eq!(code(&human), 0);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("VISIBILITY"), "{text}");
    assert!(text.contains("private"), "{text}");
    assert!(text.contains("public"), "{text}");

    let machine = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "repo", "list", "ai"])
        .output()
        .unwrap();
    let data = &stdout_json(&machine)["data"];
    assert_eq!(data["space"], "ai");
    assert_eq!(data["items"][0]["visibility"], "private");
    assert_eq!(data["items"][1]["is_public"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_list_narrows_to_the_space_of_the_current_repository() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/spaces/ai/repos"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([repo_json("backend", Some(false))])),
        )
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    // GITFOX_REPO says ai/backend, so a bare `repo list` means "this space".
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "repo", "list"])
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(stdout_json(&output)["data"]["space"], "ai");
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_view_uses_the_current_repository_and_reports_a_missing_one() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fbackend"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo_json("backend", Some(false))))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fnope"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "not found" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let found = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "repo", "view"])
        .output()
        .unwrap();
    assert_eq!(
        code(&found),
        0,
        "{}",
        String::from_utf8_lossy(&found.stdout)
    );
    assert_eq!(stdout_json(&found)["data"]["repository"], "ai/backend");
    assert_eq!(stdout_json(&found)["data"]["default_branch"], "main");

    let missing = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "repo", "view", "ai/nope"])
        .output()
        .unwrap();
    assert_eq!(code(&missing), 4);
    assert_eq!(stdout_json(&missing)["error"]["code"], "REPO_NOT_FOUND");
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_clone_really_clones_the_url_the_api_reported() {
    // A real source repository, so `git clone` does real work.
    let home = TempDir::new().unwrap();
    let source = home.path().join("source");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-q", "-b", "main"]);
    git(&source, &["config", "user.email", "t@example.com"]);
    git(&source, &["config", "user.name", "Test"]);
    std::fs::write(source.join("README.md"), "hello").unwrap();
    git(&source, &["add", "README.md"]);
    git(&source, &["commit", "-q", "-m", "initial"]);

    let server = MockServer::start().await;
    let mut repository = repo_json("backend", Some(false));
    repository["git_url"] = json!(format!("file://{}", source.display()));
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fbackend"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repository))
        .mount(&server)
        .await;

    let workdir = home.path().join("work");
    std::fs::create_dir(&workdir).unwrap();
    let mut cmd = fx(home.path());
    cmd.current_dir(&workdir)
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "repo", "clone", "ai/backend"]);
    let output = cmd.output().unwrap();

    assert_eq!(
        code(&output),
        0,
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let data = &stdout_json(&output)["data"];
    assert_eq!(data["repository"], "ai/backend");
    // Named after the repository, not after the URL — the source directory here
    // is called "source", which is what git on its own would have used.
    assert_eq!(data["directory"], "backend");
    assert!(workdir.join("backend/README.md").exists());
    assert!(workdir.join("backend/.git").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_clone_takes_an_explicit_directory() {
    let home = TempDir::new().unwrap();
    let source = home.path().join("source");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-q", "-b", "main"]);
    git(&source, &["config", "user.email", "t@example.com"]);
    git(&source, &["config", "user.name", "Test"]);
    git(&source, &["commit", "-q", "--allow-empty", "-m", "initial"]);

    let server = MockServer::start().await;
    let mut repository = repo_json("backend", Some(false));
    repository["git_url"] = json!(format!("file://{}", source.display()));
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fbackend"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repository))
        .mount(&server)
        .await;

    let workdir = home.path().join("work2");
    std::fs::create_dir(&workdir).unwrap();
    let mut cmd = fx(home.path());
    cmd.current_dir(&workdir)
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "repo", "clone", "ai/backend", "elsewhere"]);
    let output = cmd.output().unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout_json(&output)["data"]["directory"], "elsewhere");
    assert!(workdir.join("elsewhere/.git").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repository_with_no_clone_url_fails_before_running_git() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fbackend"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repo_json("backend", Some(false))))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "repo", "clone", "ai/backend"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 5);
    assert_eq!(stdout_json(&output)["error"]["code"], "API_ERROR");
}

// ---------------------------------------------------------------------------
// pagination
// ---------------------------------------------------------------------------

/// Mount `total` pull requests behind a page/limit endpoint, one mock per page.
async fn mount_paged_pull_requests(server: &MockServer, total: u64, page_size: u64) {
    let pages = total.div_ceil(page_size).max(1);
    for page in 1..=pages {
        let start = (page - 1) * page_size + 1;
        let end = (start + page_size - 1).min(total);
        let items: Vec<Value> = (start..=end).map(sample_pr).collect();
        Mock::given(method("GET"))
            .and(path(PR_PATH))
            .and(query_param("page", page.to_string()))
            .and(query_param("limit", page_size.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(items))
            .mount(server)
            .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_list_that_fits_in_one_page_is_not_marked_truncated() {
    let server = MockServer::start().await;
    // --limit 30 asks for 31; only 12 exist, so that is all of them.
    mount_paged_pull_requests(&server, 12, 31).await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "list"])
        .output()
        .unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let data = &stdout_json(&output)["data"];
    assert_eq!(data["count"], 12);
    assert_eq!(data["truncated"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn more_results_than_the_limit_are_reported_as_truncated() {
    let server = MockServer::start().await;
    // 31 exist and 31 were asked for, which is how fx learns there are more.
    mount_paged_pull_requests(&server, 31, 31).await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "list"])
        .output()
        .unwrap();

    let data = &stdout_json(&output)["data"];
    assert_eq!(data["count"], 30, "the extra one is not handed back");
    assert_eq!(data["truncated"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_beyond_one_page_walks_pages_and_returns_them_in_order() {
    let server = MockServer::start().await;
    // 250 available; --limit 150 needs two pages of the server maximum.
    mount_paged_pull_requests(&server, 250, 100).await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "list", "--limit", "150"])
        .output()
        .unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let data = &stdout_json(&output)["data"];
    assert_eq!(data["count"], 150);
    assert_eq!(data["truncated"], true);
    // The right rows, in the right order: a shrinking last page would have
    // read from the wrong offset and produced low numbers here.
    let items = data["items"].as_array().unwrap();
    assert_eq!(items[0]["number"], 1);
    assert_eq!(items[99]["number"], 100);
    assert_eq!(items[100]["number"], 101);
    assert_eq!(items[149]["number"], 150);
}

#[tokio::test(flavor = "multi_thread")]
async fn paging_stops_when_the_collection_runs_out() {
    let server = MockServer::start().await;
    // 150 available, 500 asked for: page 2 comes back short and that is the end.
    mount_paged_pull_requests(&server, 150, 100).await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["--agent", "pr", "list", "--limit", "500"])
        .output()
        .unwrap();

    let data = &stdout_json(&output)["data"];
    assert_eq!(data["count"], 150);
    assert_eq!(data["truncated"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_human_is_told_when_a_table_is_not_the_whole_story() {
    let server = MockServer::start().await;
    mount_paged_pull_requests(&server, 31, 31).await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .args(["pr", "list"])
        .output()
        .unwrap();

    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("raise --limit"), "{text}");
}

// ---------------------------------------------------------------------------
// retries, through the binary
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_transient_failure_is_retried_without_the_caller_seeing_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "uid": "whw" })))
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .args(["--agent", "api", "GET", "/api/v1/user"])
        .output()
        .unwrap();

    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(stdout_json(&output)["data"]["uid"], "whw");
}

#[tokio::test(flavor = "multi_thread")]
async fn retries_zero_surfaces_the_first_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({ "message": "down" })))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_RETRIES", "0")
        .args(["--agent", "api", "GET", "/api/v1/user"])
        .output()
        .unwrap();

    assert_eq!(code(&output), 5);
    assert_eq!(stdout_json(&output)["error"]["code"], "API_ERROR");
}

#[tokio::test(flavor = "multi_thread")]
async fn rate_limiting_has_its_own_code_and_reports_the_servers_delay() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "12")
                .set_body_json(json!({ "message": "slow down" })),
        )
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path())
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_RETRIES", "0")
        .args(["--agent", "api", "GET", "/api/v1/user"])
        .output()
        .unwrap();

    assert_eq!(
        code(&output),
        6,
        "transient, so the same class as a network error"
    );
    let error = &stdout_json(&output)["error"];
    assert_eq!(error["code"], "RATE_LIMITED");
    assert_eq!(error["details"]["retry_after_secs"], 12);
}
