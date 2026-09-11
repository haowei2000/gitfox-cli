# gitfox-client

A Rust client for the [GitFox](https://github.com/harness/gitness) API.

This crate knows about HTTP and about GitFox. It knows nothing about terminals,
tables, exit codes or configuration files — that separation is what lets
[`fx`](https://crates.io/crates/gitfox-cli), and later an MCP server, share one
implementation instead of shelling out to a binary.

```rust,no_run
use gitfox_client::GitFoxClient;

# async fn example() -> Result<(), gitfox_client::Error> {
let client = GitFoxClient::builder("https://git.example.com")
    .token(Some("…".to_string()))
    .build()?;

let user = client.auth().current_user().await?;
let repos = client.repos().list(None, Default::default(), 1, 30).await?;
let prs = client
    .pull_requests()
    .list(&"ai/backend".parse()?, &Default::default())
    .await?;
# Ok(())
# }
```

## What it handles for you

* **Base URL normalisation** — `git.example.com`, `https://git.example.com` and
  `https://example.com/gitfox` all work, and repository references travel as one
  percent-encoded path segment.
* **Typed errors** — authentication, not-found, rate limiting, timeouts and
  transport failures are distinguishable without inspecting status codes.
* **Retries** — transient failures back off exponentially, but only for methods
  that are safe to repeat. `POST` and `PATCH` are never retried: repeating a
  `POST /pullreq` that already reached the server opens a second pull request.
* **Redirects** — followed for `GET` and `HEAD` only. Following one can resend a
  write as a `GET` with no body (a `POST` on 301/302, any write on 303), which
  answers 200 while changing nothing, so a redirected write is an `Error::Api`
  naming the new location.
* **Redaction** — the token is not in `Debug` output, and the `Authorization`
  header is marked sensitive.

Domain models are the crate's own rather than the raw API DTOs, so an upstream
rename is absorbed by a serde attribute here instead of reaching callers.

Endpoints were verified against a live instance's own `/openapi.yaml`
(GitFox API v1.3.0), which every instance serves unauthenticated.

## Status

Pre-1.0: the API may still change. See the
[repository](https://github.com/haowei2000/gitfox-cli) for the roadmap.

## License

MIT
