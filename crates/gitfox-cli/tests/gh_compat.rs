//! End-to-end tests for gh compatibility.
//!
//! Every test here runs the real binary with the spelling a gh user would type
//! — gh's commands, gh's flags, gh's short letters — against a mock GitFox, and
//! checks that what reaches the server and what comes back are right. The
//! point is the silent failures: a flag that parsed but meant something else.

use std::path::Path;
use std::process::Output;

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::matchers::{
    body_json, body_string, method, path, query_param, query_param_is_missing,
};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `fx` isolated from the developer's environment, pointed at `server`, in
/// `ai/backend`.
fn fx(home: &Path, server: &MockServer) -> Command {
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
        "BROWSER",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("GITFOX_CONFIG", home.join("config.toml"))
        .env("NO_COLOR", "1")
        .env("GITFOX_HOST", server.uri())
        .env("GITFOX_TOKEN", "t")
        .env("GITFOX_REPO", "ai/backend")
        .env("GITFOX_RETRIES", "0")
        .current_dir(home);
    cmd
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stdout_json(output: &Output) -> Value {
    let text = stdout(output);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("stdout was not JSON ({e}): {text}"))
}

fn code(output: &Output) -> i32 {
    output
        .status
        .code()
        .expect("the process should not be signalled")
}

fn describe(output: &Output) -> String {
    format!(
        "exit {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        stdout(output),
        String::from_utf8_lossy(&output.stderr)
    )
}

const PRS: &str = "/api/v1/repos/ai%2Fbackend/pullreq";

fn pr(number: u64) -> Value {
    json!({
        "number": number,
        "title": "feat: add OAuth",
        "description": "Adds the callback route.",
        "state": "open",
        "is_draft": false,
        "source_branch": "feat/oauth",
        "target_branch": "main",
        "source_sha": "abc123",
        "author": { "id": 8, "uid": "alice", "display_name": "Alice" },
        "created": 1_756_000_000_000i64,
        "updated": 1_756_000_000_000i64,
        "web_url": "https://git.example.com/ai/backend/pulls/12",
        "stats": { "commits": 3, "files_changed": 5, "additions": 120, "deletions": 8 }
    })
}

async fn mount_pr(server: &MockServer, value: Value) {
    let number = value["number"].as_u64().unwrap();
    Mock::given(method("GET"))
        .and(path(format!("{PRS}/{number}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(server)
        .await;
}

// ---------------------------------------------------------------------------
// fx api
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn api_capital_f_is_typed_and_lowercase_f_is_a_string_as_in_gh() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/things"))
        .and(body_json(
            json!({ "count": 3, "draft": true, "version": "3", "flag": "true" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": 1 })))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path(), &server)
        .args([
            "api",
            "-X",
            "PUT",
            "/api/v1/things",
            "-F",
            "count=3",
            "-F",
            "draft=true",
            "-f",
            "version=3",
            "-f",
            "flag=true",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn api_paginate_walks_the_pages_and_jq_filters_the_whole() {
    let server = MockServer::start().await;
    for (page, body) in [
        ("1", json!([{ "id": 1 }, { "id": 2 }])),
        ("2", json!([{ "id": 3 }])),
    ] {
        Mock::given(method("GET"))
            .and(path("/api/v1/things"))
            .and(query_param("page", page))
            .and(query_param("limit", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
    }

    let home = TempDir::new().unwrap();
    let output = fx(home.path(), &server)
        .args([
            "api",
            "/api/v1/things?limit=2",
            "--paginate",
            "--jq",
            ".[].id",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(stdout(&output), "1\n2\n3\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn api_placeholders_are_filled_from_the_repository() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PRS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path(), &server)
        .args(["--agent", "api", "/api/v1/repos/{repo_ref}/pullreq"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn api_fields_on_a_get_are_the_query_string_as_in_gh() {
    // Sent as a JSON body instead, the filters would be ignored and the answer
    // would be every pull request.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PRS))
        .and(query_param("state", "merged"))
        .and(query_param("limit", "5"))
        .and(body_string(""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path(), &server)
        .args([
            "api",
            "-X",
            "GET",
            "/api/v1/repos/{repo_ref}/pullreq",
            "-f",
            "state=merged",
            "-F",
            "limit=5",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn api_fields_beside_an_input_body_are_the_query_string() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/things"))
        .and(query_param("dry_run", "true"))
        .and(body_json(json!({ "name": "x" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    std::fs::write(home.path().join("body.json"), r#"{"name":"x"}"#).unwrap();
    let output = fx(home.path(), &server)
        .args([
            "api",
            "/api/v1/things",
            "--input",
            "body.json",
            "-F",
            "dry_run=true",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn api_nested_fields_and_value_placeholders_build_the_body_as_gh_does() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/things"))
        .and(body_json(json!({
            "repo": "ai/backend",
            "values": [
                { "value": "high", "color": "red" },
                { "value": "low", "color": "blue" }
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let output = fx(home.path(), &server)
        .args([
            "api",
            "/api/v1/things",
            "-F",
            "repo={repo_ref}",
            "-F",
            "values[][value]=high",
            "-F",
            "values[][color]=red",
            "-F",
            "values[][value]=low",
            "-F",
            "values[][color]=blue",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
}

// ---------------------------------------------------------------------------
// --json FIELDS, --jq, --template
// ---------------------------------------------------------------------------

async fn mount_pr_list(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(PRS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([pr(12)])))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn json_fields_given_the_gh_way_select_those_fields_without_an_envelope() {
    let server = MockServer::start().await;
    mount_pr_list(&server).await;
    let home = TempDir::new().unwrap();

    // `--json number,title` with a space, exactly as gh takes it.
    let output = fx(home.path(), &server)
        .args([
            "pr",
            "list",
            "--json",
            "number,title,headRefName,state,author",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(
        stdout_json(&output),
        json!([{
            "author": { "id": "8", "is_bot": false, "login": "alice", "name": "Alice" },
            "headRefName": "feat/oauth",
            "number": 12,
            "state": "OPEN",
            "title": "feat: add OAuth",
        }])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn jq_and_template_shape_the_selected_fields() {
    let server = MockServer::start().await;
    mount_pr_list(&server).await;
    let home = TempDir::new().unwrap();

    let jq = fx(home.path(), &server)
        .args([
            "pr",
            "list",
            "--json",
            "number,title",
            "--jq",
            ".[] | \"#\\(.number) \\(.title)\"",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&jq), 0, "{}", describe(&jq));
    assert_eq!(stdout(&jq), "#12 feat: add OAuth\n");

    let template = fx(home.path(), &server)
        .args([
            "pr",
            "list",
            "--json",
            "number,headRefName",
            "-t",
            "{{range .}}{{.number}}:{{.headRefName}}{{end}}",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&template), 0, "{}", describe(&template));
    assert_eq!(stdout(&template), "12:feat/oauth");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_field_is_named_and_the_available_ones_listed() {
    let server = MockServer::start().await;
    mount_pr_list(&server).await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args(["--agent", "pr", "list", "--json", "nope"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 2, "{}", describe(&output));
    let error = &stdout_json(&output)["error"];
    assert_eq!(error["code"], "INVALID_ARGUMENT");
    assert_eq!(error["message"], "Unknown JSON field: \"nope\"");
    let available = error["details"]["available"].as_array().unwrap();
    assert!(available.contains(&json!("headRefName")));
}

#[tokio::test(flavor = "multi_thread")]
async fn jq_without_fields_is_refused_and_plain_json_keeps_its_envelope() {
    let server = MockServer::start().await;
    mount_pr_list(&server).await;
    let home = TempDir::new().unwrap();

    let refused = fx(home.path(), &server)
        .args(["pr", "list", "--jq", "."])
        .output()
        .unwrap();
    assert_eq!(code(&refused), 2, "{}", describe(&refused));

    // fx 0.6's `--json pr list` still means the envelope.
    let envelope = fx(home.path(), &server)
        .args(["--json", "pr", "list"])
        .output()
        .unwrap();
    assert_eq!(code(&envelope), 0, "{}", describe(&envelope));
    let body = stdout_json(&envelope);
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["items"][0]["number"], 12);
}

// ---------------------------------------------------------------------------
// the silent failures
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pr_checks_exits_like_gh_for_a_person_and_zero_for_json() {
    let server = MockServer::start().await;
    mount_pr(&server, pr(12)).await;
    mount_pr(&server, {
        let mut p = pr(13);
        p["number"] = json!(13);
        p
    })
    .await;
    let check = |name: &str, status: &str| json!({ "required": true, "check": { "identifier": name, "status": status } });
    Mock::given(method("GET"))
        .and(path(format!("{PRS}/12/checks")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "checks": [check("build", "success"), check("lint", "failure")]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{PRS}/13/checks")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "checks": [check("build", "running")]
        })))
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    // A red check: exit 1 with the table on stdout, as `gh pr checks` does —
    // so `fx pr checks && deploy` no longer deploys.
    let failed = fx(home.path(), &server)
        .args(["pr", "checks", "12"])
        .output()
        .unwrap();
    assert_eq!(code(&failed), 1, "{}", describe(&failed));
    assert!(stdout(&failed).contains("lint"), "{}", describe(&failed));
    assert!(
        failed.stderr.is_empty(),
        "the table says it all: {}",
        describe(&failed)
    );

    // Still running: exit 8, gh's "pending".
    let pending = fx(home.path(), &server)
        .args(["pr", "checks", "13"])
        .output()
        .unwrap();
    assert_eq!(code(&pending), 8, "{}", describe(&pending));

    // JSON is data, as with gh: exit 0 whatever the checks say.
    let exported = fx(home.path(), &server)
        .args(["pr", "checks", "12", "--json", "name,bucket,state"])
        .output()
        .unwrap();
    assert_eq!(code(&exported), 0, "{}", describe(&exported));
    assert_eq!(
        stdout_json(&exported)[1],
        json!({ "bucket": "fail", "name": "lint", "state": "FAILURE" })
    );
    let enveloped = fx(home.path(), &server)
        .args(["--agent", "pr", "checks", "12"])
        .output()
        .unwrap();
    assert_eq!(code(&enveloped), 0, "{}", describe(&enveloped));
    assert_eq!(stdout_json(&enveloped)["data"]["failed"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn dash_h_on_auth_names_the_host_instead_of_printing_help() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 7, "uid": "whw" })))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let mut cmd = fx(home.path(), &server);
    cmd.env_remove("GITFOX_HOST");
    let output = cmd
        .args(["--agent", "auth", "status", "-h", &server.uri()])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    let data = &stdout_json(&output)["data"];
    assert_eq!(data["host_key"], "127.0.0.1");
    assert_eq!(data["login"], "whw");
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_merge_takes_ghs_strategy_admin_and_commit_flags() {
    let server = MockServer::start().await;
    mount_pr(&server, pr(12)).await;
    Mock::given(method("POST"))
        .and(path(format!("{PRS}/12/merge")))
        .and(body_json(json!({
            "method": "squash",
            "title": "feat: add OAuth (#12)",
            "message": "Squashed.",
            "source_sha": "abc123",
            "bypass_rules": true
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "sha": "0123456789abcdef" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{PRS}/12/branch")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args([
            "--agent",
            "pr",
            "merge",
            "12",
            "--squash",
            "--admin",
            "-t",
            "feat: add OAuth (#12)",
            "-b",
            "Squashed.",
            "--match-head-commit",
            "abc123",
            "-d",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(stdout_json(&output)["data"]["branch_deleted"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gh_flag_gitfox_cannot_honour_says_so_instead_of_being_ignored() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for (args, needle) in [
        (vec!["--agent", "pr", "merge", "12", "--auto"], "auto-merge"),
        (
            vec!["--agent", "pr", "list", "--assignee", "bob"],
            "assignees",
        ),
        (
            vec!["--agent", "pr", "create", "-t", "x", "--milestone", "v1"],
            "milestones",
        ),
        (
            vec!["--agent", "run", "rerun", "7", "--job", "3"],
            "whole run",
        ),
    ] {
        let output = fx(home.path(), &server).args(&args).output().unwrap();
        assert_eq!(code(&output), 9, "{args:?}: {}", describe(&output));
        let error = &stdout_json(&output)["error"];
        assert_eq!(error["code"], "UNSUPPORTED", "{args:?}");
        assert!(
            error["message"].as_str().unwrap().contains(needle),
            "{args:?}: {error}"
        );
    }
}

// ---------------------------------------------------------------------------
// naming a pull request
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_pull_request_can_be_named_by_branch_or_by_url() {
    let server = MockServer::start().await;
    mount_pr(&server, pr(12)).await;
    Mock::given(method("GET"))
        .and(path(PRS))
        .and(query_param("source_branch", "feat/oauth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([pr(12)])))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    for selector in [
        "feat/oauth",
        "https://git.example.com/ai/backend/pulls/12",
        "#12",
    ] {
        let output = fx(home.path(), &server)
            .args(["pr", "view", selector, "--json", "number"])
            .output()
            .unwrap();
        assert_eq!(code(&output), 0, "{selector}: {}", describe(&output));
        assert_eq!(stdout_json(&output), json!({ "number": 12 }), "{selector}");
    }
}

// ---------------------------------------------------------------------------
// new pull request commands
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pr_close_comments_first_then_closes() {
    let server = MockServer::start().await;
    mount_pr(&server, pr(12)).await;
    Mock::given(method("POST"))
        .and(path(format!("{PRS}/12/comments")))
        .and(body_json(json!({ "text": "Superseded by #14" })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": 5, "kind": "comment" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{PRS}/12/state")))
        .and(body_json(json!({ "state": "closed" })))
        .respond_with(ResponseTemplate::new(200).set_body_json({
            let mut p = pr(12);
            p["state"] = json!("closed");
            p
        }))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args(["--agent", "pr", "close", "12", "-c", "Superseded by #14"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(stdout_json(&output)["data"]["state"], "closed");
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_ready_takes_a_draft_out_of_draft() {
    let server = MockServer::start().await;
    mount_pr(&server, {
        let mut p = pr(12);
        p["is_draft"] = json!(true);
        p
    })
    .await;
    Mock::given(method("POST"))
        .and(path(format!("{PRS}/12/state")))
        .and(body_json(json!({ "state": "open", "is_draft": false })))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr(12)))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args(["--agent", "pr", "ready", "12"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(stdout_json(&output)["data"]["changed"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_edit_changes_the_title_a_reviewer_and_a_label() {
    let server = MockServer::start().await;
    mount_pr(&server, pr(12)).await;
    Mock::given(method("PATCH"))
        .and(path(format!("{PRS}/12")))
        .and(body_json(json!({ "title": "feat: add OAuth login" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr(12)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/principals"))
        .and(query_param("query", "bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "id": 9, "uid": "bob" }])))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("{PRS}/12/reviewers")))
        .and(body_json(json!({ "reviewer_id": 9 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "reviewer": { "id": 9 } })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fbackend/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "id": 3, "key": "bug" }])))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("{PRS}/12/labels")))
        .and(body_json(json!({ "label_id": 3 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "label_id": 3 })))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args([
            "--agent",
            "pr",
            "edit",
            "12",
            "-t",
            "feat: add OAuth login",
            "--add-reviewer",
            "bob",
            "--add-label",
            "bug",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_review_posts_the_text_then_the_decision_on_the_head_commit() {
    let server = MockServer::start().await;
    mount_pr(&server, pr(12)).await;
    Mock::given(method("POST"))
        .and(path(format!("{PRS}/12/comments")))
        .and(body_json(json!({ "text": "Needs a test." })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 6 })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{PRS}/12/reviews")))
        .and(body_json(
            json!({ "commit_sha": "abc123", "decision": "changereq" }),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args(["--agent", "pr", "review", "12", "-r", "-b", "Needs a test."])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(stdout_json(&output)["data"]["decision"], "changereq");
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_comment_edit_last_changes_only_your_own_comment() {
    let server = MockServer::start().await;
    mount_pr(&server, pr(12)).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 7, "uid": "whw" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{PRS}/12/activities")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 5, "kind": "comment", "type": "comment", "text": "mine", "created": 1, "author": { "id": 7 } },
            { "id": 6, "kind": "comment", "type": "comment", "text": "theirs", "created": 2, "author": { "id": 8 } }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("{PRS}/12/comments/5")))
        .and(body_json(json!({ "text": "mine, fixed" })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": 5, "text": "mine, fixed" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args([
            "--agent",
            "pr",
            "comment",
            "12",
            "--edit-last",
            "-b",
            "mine, fixed",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(stdout_json(&output)["data"]["action"], "edited");
}

#[tokio::test(flavor = "multi_thread")]
async fn pr_status_falls_back_when_the_reviewer_filter_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 7, "uid": "whw" })))
        .mount(&server)
        .await;
    // Some GitFox instances answer the reviewer filter with a bare 500.
    Mock::given(method("GET"))
        .and(path(PRS))
        .and(query_param("reviewer_id", "7"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(json!({ "message": "Internal error occurred" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(PRS))
        .and(query_param("author_id", "7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(PRS))
        .and(query_param_is_missing("author_id"))
        .and(query_param_is_missing("reviewer_id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([pr(12)])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{PRS}/12/reviewers")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "reviewer": { "id": 7, "uid": "whw" }, "review_decision": "pending" }
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args(["pr", "status", "--json", "number"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(
        stdout_json(&output),
        json!({ "createdBy": [], "currentBranch": null, "needsReview": [{ "number": 12 }] })
    );
}

// ---------------------------------------------------------------------------
// run / workflow
// ---------------------------------------------------------------------------

const PIPELINES: &str = "/api/v1/repos/ai%2Fbackend/pipelines";

async fn mount_pipelines(server: &MockServer, names: &[&str]) {
    let list: Vec<Value> = names
        .iter()
        .enumerate()
        .map(
            |(i, n)| json!({ "id": i + 1, "identifier": n, "config_path": ".gitfox/default.yaml" }),
        )
        .collect();
    Mock::given(method("GET"))
        .and(path(PIPELINES))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(list)))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn run_view_finds_the_one_pipeline_with_that_run_number() {
    let server = MockServer::start().await;
    mount_pipelines(&server, &["build", "deploy"]).await;
    Mock::given(method("GET"))
        .and(path(format!("{PIPELINES}/build/executions/7")))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "not found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{PIPELINES}/deploy/executions/7")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "number": 7, "status": "failure", "event": "push", "target": "main", "pipeline_id": 2,
            "stages": [{ "number": 1, "name": "ship", "status": "failure", "steps": [] }]
        })))
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args([
            "run",
            "view",
            "7",
            "--json",
            "workflowName,conclusion,status,jobs",
            "--exit-status",
        ])
        .output()
        .unwrap();
    // JSON is data: --exit-status does not turn it into a failure.
    assert_eq!(code(&output), 0, "{}", describe(&output));
    let data = stdout_json(&output);
    assert_eq!(data["workflowName"], "deploy");
    assert_eq!(data["conclusion"], "failure");
    assert_eq!(data["status"], "completed");
    assert_eq!(data["jobs"][0]["name"], "ship");

    // For a person, --exit-status is exit 1 on a failed run, as in gh.
    let human = fx(home.path(), &server)
        .args(["run", "view", "deploy/7", "--exit-status"])
        .output()
        .unwrap();
    assert_eq!(code(&human), 1, "{}", describe(&human));
}

#[tokio::test(flavor = "multi_thread")]
async fn run_list_filters_by_ghs_status_words() {
    let server = MockServer::start().await;
    mount_pipelines(&server, &["default"]).await;
    Mock::given(method("GET"))
        .and(path(format!("{PIPELINES}/default/executions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "number": 3, "status": "running", "created": 3 },
            { "number": 2, "status": "success", "created": 2 },
            { "number": 1, "status": "error", "created": 1 }
        ])))
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    for (status, expected) in [
        ("failure", json!([{ "number": 1 }])),
        ("in_progress", json!([{ "number": 3 }])),
    ] {
        let output = fx(home.path(), &server)
            .args(["run", "list", "-s", status, "--json", "number"])
            .output()
            .unwrap();
        assert_eq!(code(&output), 0, "{status}: {}", describe(&output));
        assert_eq!(stdout_json(&output), expected, "{status}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_disable_patches_the_pipeline() {
    let server = MockServer::start().await;
    mount_pipelines(&server, &["nightly"]).await;
    Mock::given(method("PATCH"))
        .and(path(format!("{PIPELINES}/nightly")))
        .and(body_json(json!({ "disabled": true })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "identifier": "nightly", "disabled": true })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args(["--agent", "workflow", "disable", "nightly"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(stdout_json(&output)["data"]["disabled"], true);
}

// ---------------------------------------------------------------------------
// secrets, labels, keys
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn secret_set_creates_a_new_secret_and_updates_an_existing_one() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/secrets/ai%2FNEW_TOKEN"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "not found" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/secrets"))
        .and(body_json(
            json!({ "space_ref": "ai", "identifier": "NEW_TOKEN", "data": "from-stdin" }),
        ))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "identifier": "NEW_TOKEN" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/secrets/ai%2FOLD_TOKEN"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "identifier": "OLD_TOKEN" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/secrets/ai%2FOLD_TOKEN"))
        .and(body_json(json!({ "data": "rotated" })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "identifier": "OLD_TOKEN" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    // The value on stdin, as `gh secret set NAME < file` reads it.
    let created = fx(home.path(), &server)
        .args(["--agent", "secret", "set", "NEW_TOKEN", "-o", "ai"])
        .write_stdin("from-stdin\n")
        .output()
        .unwrap();
    assert_eq!(code(&created), 0, "{}", describe(&created));
    assert_eq!(
        stdout_json(&created)["data"]["items"][0]["action"],
        "created"
    );

    let updated = fx(home.path(), &server)
        .args(["--agent", "secret", "set", "OLD_TOKEN", "--body", "rotated"])
        .output()
        .unwrap();
    assert_eq!(code(&updated), 0, "{}", describe(&updated));
    assert_eq!(
        stdout_json(&updated)["data"]["items"][0]["action"],
        "updated"
    );
    // Neither value is ever echoed back.
    assert!(!stdout(&created).contains("from-stdin") && !stdout(&updated).contains("rotated"));
}

#[tokio::test(flavor = "multi_thread")]
async fn label_create_maps_a_hex_colour_onto_gitfoxs_palette() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fbackend/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/ai%2Fbackend/labels"))
        .and(body_json(json!({ "key": "bug", "color": "red", "description": "Something is broken", "type": "static" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 3, "key": "bug", "color": "red" })))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    // GitHub's default `bug` colour.
    let output = fx(home.path(), &server)
        .args([
            "--agent",
            "label",
            "create",
            "bug",
            "-c",
            "d73a4a",
            "-d",
            "Something is broken",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("`red`"),
        "the mapping is announced: {}",
        describe(&output)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_key_add_names_the_key_after_its_title() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/user/keys"))
        .and(body_json(json!({
            "identifier": "laptop",
            "content": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIG me@laptop",
            "usage": "auth"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "identifier": "laptop" })))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();
    let key = home.path().join("id_ed25519.pub");
    std::fs::write(&key, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIG me@laptop\n").unwrap();

    let output = fx(home.path(), &server)
        .args([
            "--agent",
            "ssh-key",
            "add",
            key.to_str().unwrap(),
            "-t",
            "laptop",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
}

// ---------------------------------------------------------------------------
// repositories
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn repo_create_sends_ghs_options_and_delete_needs_yes_without_a_person() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos"))
        .and(body_json(json!({
            "identifier": "new", "parent_ref": "ai", "description": "A new service",
            "is_public": true, "readme": true, "license": "mit", "git_ignore": "Rust"
        })))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({ "identifier": "new", "path": "ai/new" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/ai%2Fold"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "deleted_at": 1 })))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let created = fx(home.path(), &server)
        .args([
            "--agent",
            "repo",
            "create",
            "ai/new",
            "--public",
            "--add-readme",
            "-l",
            "mit",
            "-g",
            "Rust",
            "-d",
            "A new service",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&created), 0, "{}", describe(&created));

    // Nobody to ask: refused before any request, as gh refuses.
    let refused = fx(home.path(), &server)
        .args(["--agent", "repo", "delete", "ai/old"])
        .output()
        .unwrap();
    assert_eq!(code(&refused), 2, "{}", describe(&refused));

    let deleted = fx(home.path(), &server)
        .args(["--agent", "repo", "delete", "ai/old", "--yes"])
        .output()
        .unwrap();
    assert_eq!(code(&deleted), 0, "{}", describe(&deleted));
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_read_file_prints_the_bytes_and_nothing_else() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fbackend/content/docs/notes.txt"))
        .and(query_param("git_ref", "main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "file", "name": "notes.txt", "path": "docs/notes.txt", "sha": "1",
            "content": { "data": "aGVsbG8K", "encoding": "base64", "size": 6 }
        })))
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args(["repo", "read-file", "docs/notes.txt", "--ref", "main"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert_eq!(output.stdout, b"hello\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn repo_clone_passes_git_flags_through() {
    let home = TempDir::new().unwrap();
    let source = home.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&source)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "Test"]);
    std::fs::write(source.join("README.md"), "hello").unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "-q", "-m", "initial"]);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fbackend"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "identifier": "backend", "path": "ai/backend",
            "git_url": format!("file://{}", source.display())
        })))
        .mount(&server)
        .await;

    let output = fx(home.path(), &server)
        .args([
            "--agent",
            "repo",
            "clone",
            "ai/backend",
            "--",
            "--no-checkout",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", describe(&output));
    assert!(home.path().join("backend/.git").exists());
    assert!(
        !home.path().join("backend/README.md").exists(),
        "--no-checkout reached git"
    );
}

// ---------------------------------------------------------------------------
// local behaviour: aliases, config, completion, browse
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn an_alias_expands_with_its_arguments_before_parsing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PRS))
        .and(query_param("state", "merged"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let set = fx(home.path(), &server)
        .args(["alias", "set", "prs", "pr list --state $1"])
        .output()
        .unwrap();
    assert_eq!(code(&set), 0, "{}", describe(&set));

    let used = fx(home.path(), &server)
        .args(["prs", "merged", "--json", "number"])
        .output()
        .unwrap();
    assert_eq!(code(&used), 0, "{}", describe(&used));
    assert_eq!(stdout_json(&used), json!([]));

    // A built-in name cannot be taken.
    let taken = fx(home.path(), &server)
        .args(["alias", "set", "pr", "repo list"])
        .output()
        .unwrap();
    assert_eq!(code(&taken), 2, "{}", describe(&taken));
}

#[tokio::test(flavor = "multi_thread")]
async fn config_dash_h_scopes_a_key_to_one_host() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();

    let set = fx(home.path(), &server)
        .args([
            "config",
            "set",
            "-h",
            "git.example.com",
            "git_protocol",
            "ssh",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&set), 0, "{}", describe(&set));

    let scoped = fx(home.path(), &server)
        .args(["config", "get", "hosts.git.example.com.git_protocol"])
        .output()
        .unwrap();
    assert_eq!(stdout(&scoped).trim(), "ssh");
    // The top-level key is untouched, and reports gh's default.
    let top = fx(home.path(), &server)
        .args(["config", "get", "git_protocol"])
        .output()
        .unwrap();
    assert_eq!(stdout(&top).trim(), "https");
}

#[tokio::test(flavor = "multi_thread")]
async fn completion_and_browse_take_ghs_flags() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();

    let completion = fx(home.path(), &server)
        .args(["completion", "-s", "zsh"])
        .output()
        .unwrap();
    assert_eq!(code(&completion), 0, "{}", describe(&completion));
    assert!(stdout(&completion).starts_with("#compdef fx"));

    let browse = fx(home.path(), &server)
        .args(["browse", "12", "-n"])
        .output()
        .unwrap();
    assert_eq!(code(&browse), 0, "{}", describe(&browse));
    assert_eq!(
        stdout(&browse).trim(),
        format!("{}/ai/backend/pulls/12", server.uri())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn codespace_explains_that_gitspaces_are_switched_off() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/system/config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "gitspace_enabled": false })),
        )
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args(["--agent", "codespace", "list"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 9, "{}", describe(&output));
    let error = &stdout_json(&output)["error"];
    assert_eq!(error["code"], "UNSUPPORTED");
    assert!(error["message"].as_str().unwrap().contains("switched off"));
}

// ---------------------------------------------------------------------------
// what GitFox lacks, and fx's own commands under gh's names
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_gh_command_for_a_missing_feature_says_why_and_sends_nothing() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let output = fx(home.path(), &server)
        .args(["--agent", "issue", "list", "--state", "open"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 9, "{}", describe(&output));
    let error = &stdout_json(&output)["error"];
    assert_eq!(error["code"], "UNSUPPORTED");
    assert!(
        error["message"].as_str().unwrap().contains("issue tracker"),
        "{error}"
    );

    // gh's `-h` is a host on some of these, and help on others: neither may
    // turn into a silent exit 0.
    for args in [
        vec!["release", "create", "v1.0.0", "-h", "git.example.com"],
        vec!["gist", "--help"],
    ] {
        let output = fx(home.path(), &server).args(&args).output().unwrap();
        assert_eq!(code(&output), 9, "{args:?}: {}", describe(&output));
        assert!(
            stdout(&output).is_empty(),
            "{args:?}: {}",
            describe(&output)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("is not supported"),
            "{args:?}: {}",
            describe(&output)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pipeline_commands_take_ghs_workflow_and_ref_names() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{PIPELINES}/build/executions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{PIPELINES}/build/executions")))
        .and(query_param("branch", "release"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "number": 8, "status": "pending", "target": "release"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();

    let list = fx(home.path(), &server)
        .args(["--agent", "pipeline", "list", "--workflow", "build"])
        .output()
        .unwrap();
    assert_eq!(code(&list), 0, "{}", describe(&list));

    let run = fx(home.path(), &server)
        .args(["--agent", "pipeline", "run", "build", "--ref", "release"])
        .output()
        .unwrap();
    assert_eq!(code(&run), 0, "{}", describe(&run));
    assert_eq!(stdout_json(&run)["data"]["number"], 8);
}
