//! Pull request endpoints.
//!
//! Lands in **v0.3** together with `fx pr list|view|create|merge`. The endpoints
//! this module will wrap:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list   | `GET /api/v1/repos/{repo_ref}/pullreq` |
//! | view   | `GET /api/v1/repos/{repo_ref}/pullreq/{number}` |
//! | create | `POST /api/v1/repos/{repo_ref}/pullreq` |
//! | merge  | `POST /api/v1/repos/{repo_ref}/pullreq/{number}/merge` |
//! | diff   | `GET /api/v1/repos/{repo_ref}/pullreq/{number}/diff` |
//!
//! Each one gets a domain model in [`crate::models`] rather than exposing the
//! raw API DTO, so the CLI's JSON schema can stay stable across GitFox versions.
