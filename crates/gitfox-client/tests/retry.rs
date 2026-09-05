//! Retry behaviour.
//!
//! The interesting cases are the ones that must *not* retry: repeating a POST
//! that already reached the server is how a CLI opens two pull requests.

use std::time::Instant;

use gitfox_client::{Error, GitFoxClient, Method};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer, retries: u32) -> GitFoxClient {
    GitFoxClient::builder(server.uri())
        .token(Some("t".into()))
        .timeout_secs(2)
        .retries(retries)
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_transient_503_is_retried_until_it_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "uid": "whw" })))
        .expect(1)
        .mount(&server)
        .await;

    let user = client(&server, 2).auth().current_user().await.unwrap();
    assert_eq!(user.uid.as_deref(), Some("whw"));
}

#[tokio::test]
async fn the_budget_is_finite_and_the_last_error_is_what_surfaces() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({ "message": "down" })))
        // One first attempt plus two retries, and not one request more.
        .expect(3)
        .mount(&server)
        .await;

    let err = client(&server, 2).auth().current_user().await.unwrap_err();
    match err {
        Error::Api { status, .. } => assert_eq!(status, 503),
        other => panic!("expected an API error, got {other:?}"),
    }
}

#[tokio::test]
async fn retries_zero_means_one_attempt() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    assert!(client(&server, 0).auth().current_user().await.is_err());
}

#[tokio::test]
async fn a_post_is_never_repeated() {
    let server = MockServer::start().await;
    // Repeating this could open a second pull request, so one attempt only —
    // even though 503 is transient and the budget allows retries.
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/ai%2Fbackend/pullreq"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    let result = client(&server, 3)
        .request(
            Method::POST,
            "/api/v1/repos/ai%2Fbackend/pullreq",
            Some(&json!({ "title": "x" })),
            &[],
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn a_delete_is_repeated_because_repeating_it_changes_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/x"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/x"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let response = client(&server, 2)
        .request(Method::DELETE, "/api/v1/x", None, &[])
        .await
        .unwrap();
    assert_eq!(response.status, 204);
}

#[tokio::test]
async fn a_500_is_not_retried_because_it_is_usually_a_bug_not_a_blip() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "message": "boom" })))
        .expect(1)
        .mount(&server)
        .await;

    assert!(client(&server, 3).auth().current_user().await.is_err());
}

#[tokio::test]
async fn a_404_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    assert!(client(&server, 3).auth().current_user().await.is_err());
}

#[tokio::test]
async fn an_auth_failure_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;

    assert!(matches!(
        client(&server, 3).auth().current_user().await,
        Err(Error::AuthFailed)
    ));
}

#[tokio::test]
async fn a_429_surfaces_as_rate_limited_with_the_servers_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "7")
                .set_body_json(json!({ "message": "slow down" })),
        )
        .expect(1)
        .mount(&server)
        .await;

    // With no budget the error reaches the caller intact, header and all.
    let err = client(&server, 0).auth().current_user().await.unwrap_err();
    match err {
        Error::RateLimited {
            retry_after,
            message,
        } => {
            assert_eq!(retry_after, Some(7));
            assert_eq!(message, "slow down");
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn a_retry_after_longer_than_the_cap_does_not_stall_the_command() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "3600"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "uid": "whw" })))
        .mount(&server)
        .await;

    let started = Instant::now();
    let user = client(&server, 1).auth().current_user().await.unwrap();
    assert_eq!(user.uid.as_deref(), Some("whw"));
    // An hour was requested; the cap is five seconds.
    let waited = started.elapsed();
    assert!(waited.as_secs() >= 4, "the header was ignored: {waited:?}");
    assert!(waited.as_secs() <= 7, "waited {waited:?}");
}

#[tokio::test]
async fn backoff_actually_waits_between_attempts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(503))
        .expect(3)
        .mount(&server)
        .await;

    let started = Instant::now();
    assert!(client(&server, 2).auth().current_user().await.is_err());
    // 250ms then 500ms, before jitter.
    let waited = started.elapsed();
    assert!(waited.as_millis() >= 700, "retried too fast: {waited:?}");
    assert!(waited.as_secs() < 5, "retried too slowly: {waited:?}");
}
