//! Repository endpoints.
//!
//! Lands in **v0.2** together with `fx repo list|view|clone` and git remote
//! detection. Verified against the GitFox API v1.3.0 OpenAPI document
//! (`GET /openapi.yaml` on any instance):
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list repositories (all spaces) | `GET /api/v1/repos` |
//! | list repositories in a space   | `GET /api/v1/spaces/{space_ref}/repos` |
//! | get a repository               | `GET /api/v1/repos/{repo_ref}` |
//! | list branches                  | `GET /api/v1/repos/{repo_ref}/branches` |
//!
//! `{repo_ref}` is [`crate::RepoRef::encoded`] (`ai%2Fbackend`).
//!
//! `fx repo list` defaults to `/api/v1/repos`, which spans every space the
//! caller can see; `--org`/`GITFOX_ORG` narrows it to the space endpoint.
//!
//! Until this lands, `fx api` is the escape hatch:
//! `fx api GET /api/v1/repos/ai%2Fbackend`.
