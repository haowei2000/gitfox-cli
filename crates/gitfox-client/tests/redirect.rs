//! A request that changes data must not follow a redirect.
//!
//! Following one does not faithfully replay a write: on a 301 or 302 the HTTP
//! client resends a `POST` as a `GET` with no body, and on a 303 it does the
//! same to `PATCH`, `PUT` and `DELETE`. The `GET` answers 200, so a redirected
//! `POST /secrets` came back looking exactly like a secret that was created.

use gitfox_client::{Error, GitFoxClient, Method};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn client(server: &MockServer) -> GitFoxClient {
    GitFoxClient::builder(server.uri())
        .token(Some("test-token".into()))
        .timeout_secs(1)
        .build()
        .unwrap()
}

#[tokio::test]
async fn a_redirected_write_is_reported_rather_than_resent() {
    for status in [301_u16, 302, 303, 307, 308] {
        for verb in ["POST", "PATCH", "PUT", "DELETE"] {
            let server = MockServer::start().await;
            Mock::given(method(verb))
                .and(path("/api/v1/secrets/deploy_key/"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("location", "/api/v1/secrets/deploy_key"),
                )
                .mount(&server)
                .await;
            // Where following the redirect lands. Any request reaching it — a
            // body-less GET or a replayed write — means it was followed.
            Mock::given(path("/api/v1/secrets/deploy_key"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "identifier": "deploy_key",
                    "updated": 1_756_000_000_000_i64
                })))
                .expect(0)
                .mount(&server)
                .await;

            let result = client(&server)
                .await
                .request(
                    Method::from_bytes(verb.as_bytes()).unwrap(),
                    "/api/v1/secrets/deploy_key/",
                    Some(&json!({ "data": "rotated" })),
                    &[],
                )
                .await;

            match result {
                Err(Error::Api {
                    status: got,
                    message,
                    ..
                }) => {
                    assert_eq!(got, status, "{verb}");
                    assert!(
                        message.contains("/api/v1/secrets/deploy_key"),
                        "{verb} answered {status} should say where it was sent: {message}"
                    );
                }
                other => panic!("{verb} answered {status} must be an error, got {other:?}"),
            }
            server.verify().await;
        }
    }
}

#[tokio::test]
async fn a_read_still_follows_a_redirect_with_its_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user/"))
        .respond_with(ResponseTemplate::new(301).insert_header("location", "/api/v1/user"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/user"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "uid": "whw" })))
        .mount(&server)
        .await;

    let response = client(&server)
        .await
        .request(Method::GET, "/api/v1/user/", None, &[])
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.json_or_null()["uid"], "whw");
}
