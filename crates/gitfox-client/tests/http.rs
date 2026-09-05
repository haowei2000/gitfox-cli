//! HTTP-level behaviour against a mock GitFox, covering the failure modes a
//! self-hosted instance actually produces.

use std::time::Duration;

use gitfox_client::{Error, GitFoxClient, Method};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn client(server: &MockServer) -> GitFoxClient {
    GitFoxClient::builder(server.uri())
        .token(Some("test-token".into()))
        .timeout_secs(1)
        .build()
        .unwrap()
}

#[tokio::test]
async fn sends_a_bearer_token_and_decodes_the_current_user() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "uid": "whw",
            "display_name": "Haowei",
            "email": "whw@example.com",
            "admin": true
        })))
        .mount(&server)
        .await;

    let user = client(&server).await.auth().current_user().await.unwrap();
    assert_eq!(user.uid.as_deref(), Some("whw"));
    assert_eq!(user.label(), "Haowei");
}

#[tokio::test]
async fn a_user_payload_missing_optional_fields_still_decodes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "uid": "whw" })))
        .mount(&server)
        .await;

    let user = client(&server).await.auth().current_user().await.unwrap();
    assert_eq!(user.label(), "whw");
}

#[tokio::test]
async fn unauthenticated_401_is_auth_required_and_authenticated_401_is_auth_failed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({ "message": "unauthorized" })),
        )
        .mount(&server)
        .await;

    let anonymous = GitFoxClient::builder(server.uri())
        .timeout_secs(1)
        .build()
        .unwrap();
    assert!(matches!(
        anonymous.auth().current_user().await,
        Err(Error::AuthRequired)
    ));
    assert!(matches!(
        client(&server).await.auth().current_user().await,
        Err(Error::AuthFailed)
    ));
}

#[tokio::test]
async fn forbidden_is_an_auth_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server).await.auth().current_user().await,
        Err(Error::AuthFailed)
    ));
}

#[tokio::test]
async fn not_found_carries_the_servers_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ai%2Fnope"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({ "message": "repository not found" })),
        )
        .mount(&server)
        .await;

    let err = client(&server)
        .await
        .request(Method::GET, "/api/v1/repos/ai%2Fnope", None, &[])
        .await
        .unwrap_err();
    match err {
        Error::NotFound { reference, .. } => assert_eq!(reference, "repository not found"),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn server_errors_keep_the_status_and_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(
            ResponseTemplate::new(500).set_body_json(json!({ "message": "boom", "trace": "abc" })),
        )
        .mount(&server)
        .await;

    let err = client(&server)
        .await
        .auth()
        .current_user()
        .await
        .unwrap_err();
    match err {
        Error::Api {
            status,
            message,
            body,
        } => {
            assert_eq!(status, 500);
            assert_eq!(message, "boom");
            assert_eq!(body.unwrap()["trace"], "abc");
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

#[tokio::test]
async fn an_error_body_that_is_not_json_still_produces_a_useful_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(502).set_body_string("<html>bad gateway</html>"))
        .mount(&server)
        .await;

    let err = client(&server)
        .await
        .auth()
        .current_user()
        .await
        .unwrap_err();
    match err {
        Error::Api {
            status, message, ..
        } => {
            assert_eq!(status, 502);
            assert!(message.contains("bad gateway"), "{message}");
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

#[tokio::test]
async fn a_success_body_that_is_not_the_expected_shape_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&server)
        .await;

    assert!(matches!(
        client(&server).await.auth().current_user().await,
        Err(Error::Decode(_))
    ));
}

#[tokio::test]
async fn a_slow_server_times_out_rather_than_hanging() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .mount(&server)
        .await;

    assert!(matches!(
        client(&server).await.auth().current_user().await,
        Err(Error::Timeout(1))
    ));
}

#[tokio::test]
async fn an_unreachable_host_is_a_network_error() {
    // Reserved TEST-NET-1 address; nothing answers there.
    let client = GitFoxClient::builder("http://192.0.2.1:9")
        .timeout_secs(1)
        .build()
        .unwrap();
    assert!(matches!(
        client.auth().current_user().await,
        Err(Error::Network(_) | Error::Timeout(_))
    ));
}

#[tokio::test]
async fn post_bodies_and_extra_headers_reach_the_server() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/foo"))
        .and(header("x-trace", "1"))
        .and(body_json(json!({ "name": "test" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 7 })))
        .mount(&server)
        .await;

    let response = client(&server)
        .await
        .request(
            Method::POST,
            "/api/v1/foo",
            Some(&json!({ "name": "test" })),
            &[("x-trace".into(), "1".into())],
        )
        .await
        .unwrap();
    assert_eq!(response.status, 201);
    assert_eq!(response.json_or_null()["id"], 7);
}

#[tokio::test]
async fn an_empty_204_body_is_not_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/foo/1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let response = client(&server)
        .await
        .request(Method::DELETE, "/api/v1/foo/1", None, &[])
        .await
        .unwrap();
    assert_eq!(response.status, 204);
    assert!(response.json.is_none());
}
