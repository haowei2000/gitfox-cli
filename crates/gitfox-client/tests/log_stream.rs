//! The live log stream.
//!
//! GitFox does not persist a step's log until it finishes, so a running step's
//! output is only reachable over this server-sent-event stream. The endpoint is
//! not in the instance's OpenAPI document; these tests pin the shape it was
//! observed to have.

use std::time::{Duration, Instant};

use gitfox_client::{GitFoxClient, LogLine, RepoRef};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const STREAM: &str = "/api/v1/repos/ai%2Fbackend/pipelines/default/executions/132/logs/2/2/stream";

fn client(server: &MockServer) -> GitFoxClient {
    GitFoxClient::builder(server.uri())
        .token(Some("t".into()))
        .timeout_secs(5)
        .build()
        .unwrap()
}

async fn mount(server: &MockServer, body: &str) {
    Mock::given(method("GET"))
        .and(path(STREAM))
        .and(header("accept", "text/event-stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

async fn collect(server: &MockServer) -> Vec<LogLine> {
    client(server)
        .pipelines()
        .step_logs_live(
            &RepoRef::parse("ai/backend").unwrap(),
            "default",
            132,
            2,
            2,
            Duration::from_secs(2),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn data_events_become_log_lines_and_keepalives_are_ignored() {
    let server = MockServer::start().await;
    // `: ping` is the keepalive the server sends; blank lines separate events.
    mount(
        &server,
        ": ping\n\n\
         data: {\"pos\":0,\"out\":\"+ apk add bash\\n\",\"time\":1}\n\n\
         : ping\n\n\
         data: {\"pos\":1,\"out\":\"fetch APKINDEX\\n\",\"time\":2}\n\n",
    )
    .await;

    let lines = collect(&server).await;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].pos, 0);
    assert_eq!(lines[0].out, "+ apk add bash\n");
    assert_eq!(lines[1].out, "fetch APKINDEX\n");
}

#[tokio::test]
async fn the_streams_own_eof_ends_it_without_waiting_for_the_idle_window() {
    let server = MockServer::start().await;
    mount(
        &server,
        "data: {\"pos\":0,\"out\":\"one\",\"time\":1}\n\n\
         event: error\ndata: eof\n\n",
    )
    .await;

    let started = Instant::now();
    let lines = collect(&server).await;
    assert_eq!(lines.len(), 1);
    // `eof` is the ordinary end of a finished step's stream, not a failure —
    // and it must not cost the full idle window.
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_finished_step_streams_nothing_at_all() {
    let server = MockServer::start().await;
    // What the endpoint answers for a step that already completed; its log
    // lives on the static endpoint instead.
    mount(&server, ": ping\n\nevent: error\ndata: eof\n\n").await;
    assert!(collect(&server).await.is_empty());
}

#[tokio::test]
async fn a_line_that_does_not_parse_is_skipped_rather_than_failing_the_stream() {
    let server = MockServer::start().await;
    mount(
        &server,
        "data: not json at all\n\n\
         data: {\"pos\":7,\"out\":\"kept\",\"time\":1}\n\n",
    )
    .await;

    let lines = collect(&server).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].pos, 7);
}

#[tokio::test]
async fn an_event_split_across_chunks_is_still_parsed() {
    let server = MockServer::start().await;
    // Bodies arrive in arbitrary chunks; the parser buffers until a newline.
    let payload = json!({ "pos": 0, "out": "x".repeat(40_000), "time": 1 });
    mount(&server, &format!("data: {payload}\n\n")).await;

    let lines = collect(&server).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].out.len(), 40_000);
}

#[tokio::test]
async fn a_rejected_stream_is_an_error_not_an_empty_log() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(STREAM))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "message": "Forbidden" })))
        .mount(&server)
        .await;

    let result = client(&server)
        .pipelines()
        .step_logs_live(
            &RepoRef::parse("ai/backend").unwrap(),
            "default",
            132,
            2,
            2,
            Duration::from_secs(2),
        )
        .await;
    assert!(matches!(result, Err(gitfox_client::Error::AuthFailed)));
}
