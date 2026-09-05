//! Domain models owned by the client.
//!
//! These are deliberately *not* the raw GitFox API DTOs: keeping a translation
//! layer means an upstream API change is absorbed by a serde attribute here
//! instead of leaking into the CLI's stable JSON schema.

mod pipeline;
mod principal;
mod pull_request;
mod repo_ref;
mod repository;
mod user;

pub use pipeline::{CiStatus, Execution, LogLine, Pipeline, Stage, Step};
pub use principal::Principal;
pub use pull_request::{
    Check, CreatePullRequest, FileDiff, MergeMethod, MergePullRequest, MergeResult, PullRequest,
    PullRequestCheck, PullRequestChecks, PullRequestState, PullRequestStats,
};
pub use repo_ref::RepoRef;
pub use repository::Repository;
pub use user::User;
