//! Domain models owned by the client.
//!
//! These are deliberately *not* the raw GitFox API DTOs: keeping a translation
//! layer means an upstream API change is absorbed by a serde attribute here
//! instead of leaking into the CLI's stable JSON schema.

mod principal;
mod pull_request;
mod repo_ref;
mod repository;
mod user;

pub use principal::Principal;
pub use pull_request::{
    CreatePullRequest, MergeMethod, MergePullRequest, MergeResult, PullRequest, PullRequestState,
    PullRequestStats,
};
pub use repo_ref::RepoRef;
pub use repository::Repository;
pub use user::User;
