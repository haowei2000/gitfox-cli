//! Pull request endpoints.
//!
//! Lands in **v0.3** (list/view/create/merge) and **v0.5** (diff/checks).
//! Verified against the GitFox API v1.3.0 OpenAPI document:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list          | `GET /api/v1/repos/{repo_ref}/pullreq` |
//! | list in space | `GET /api/v1/spaces/{space_ref}/pullreq` |
//! | view          | `GET /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}` |
//! | create        | `POST /api/v1/repos/{repo_ref}/pullreq` |
//! | merge         | `POST /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}/merge` |
//! | close/reopen  | `POST /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}/state` |
//! | diff          | `GET /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}/diff` |
//! | checks        | `GET /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}/checks` |
//! | commits       | `GET /api/v1/repos/{repo_ref}/pullreq/{pullreq_number}/commits` |
//!
//! Two of these shape features beyond the obvious CRUD:
//!
//! * `/commits` is what `fx pr create --fill` reads to build a title and body.
//! * `/state` is why closing a pull request is not a `DELETE`; `fx pr close`
//!   and `fx pr reopen` both post there.
//!
//! Each operation gets a domain model in [`crate::models`] rather than exposing
//! the raw API DTO, so the CLI's JSON schema can stay stable across GitFox
//! versions.
