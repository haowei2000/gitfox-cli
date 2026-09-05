//! Repository endpoints.
//!
//! Lands in **v0.2** together with `fx repo list|view|clone` and git remote
//! detection. The endpoints this module will wrap:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list repositories in a space | `GET /api/v1/spaces/{space_ref}/repos` |
//! | get a repository            | `GET /api/v1/repos/{repo_ref}` |
//! | list branches               | `GET /api/v1/repos/{repo_ref}/branches` |
//!
//! `{repo_ref}` is [`crate::RepoRef::encoded`] (`ai%2Fbackend`).
//!
//! Until then `fx api` is the escape hatch:
//! `fx api GET /api/v1/repos/ai%2Fbackend`.
