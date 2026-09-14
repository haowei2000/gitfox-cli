//! Domain models owned by the client.
//!
//! These are deliberately *not* the raw GitFox API DTOs: keeping a translation
//! layer means an upstream API change is absorbed by a serde attribute here
//! instead of leaking into the CLI's stable JSON schema.

mod activity;
mod commit;
mod content;
mod gitspace;
mod label;
mod pipeline;
mod principal;
mod pull_request;
mod repo_ref;
mod repository;
mod rule;
mod space;
mod user;

pub use activity::{CodeComment, PullRequestActivity, PullRequestReviewer, ReviewDecision};
pub use commit::{Commit, Identity, Signature};
pub use content::{Content, ContentEntry, decode_base64, encode_base64};
pub use gitspace::{CreateGitspace, Gitspace, GitspaceInstance};
pub use label::{
    LABEL_COLORS, Label, LabelAssignment, LabelInput, LabelValue, LabelValueInfo,
    PullRequestLabels, label_color, label_color_hex,
};
pub use pipeline::{CiStatus, Execution, LogLine, Pipeline, Stage, Step};
pub use principal::Principal;
pub use pull_request::{
    Check, CheckCountSummary, CreatePullRequest, FileDiff, MergeMethod, MergePullRequest,
    MergeResult, PullRequest, PullRequestCheck, PullRequestChecks, PullRequestLabel,
    PullRequestState, PullRequestStats, UpdatePullRequest,
};
pub use repo_ref::RepoRef;
pub use repository::{CreateRepository, Repository};
pub use rule::Rule;
pub use space::{LicenseTemplate, Membership, PublicKey, Space, SpaceSecret, SystemConfig};
pub use user::User;
