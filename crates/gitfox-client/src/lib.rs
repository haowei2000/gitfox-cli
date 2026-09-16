//! `gitfox-client` — a Rust client for the GitFox API.
//!
//! This crate knows about HTTP and about GitFox. It knows nothing about
//! terminals, tables, exit codes or configuration files. That separation is
//! what lets `gf` (the CLI) and, later, `gf-mcp` (the MCP server) share one
//! implementation instead of the MCP server shelling out to the CLI.
//!
//! ```no_run
//! # async fn example() -> Result<(), gitfox_client::Error> {
//! use gitfox_client::GitFoxClient;
//!
//! let client = GitFoxClient::builder("https://git.example.com")
//!     .token(Some("…".to_string()))
//!     .build()?;
//! let user = client.auth().current_user().await?;
//! println!("{}", user.label());
//! # Ok(())
//! # }
//! ```

pub mod auth;
pub mod client;
pub mod error;
pub mod labels;
pub mod models;
pub mod pipeline;
pub mod principal;
pub mod pull_request;
pub mod repo;
pub mod rules;
pub mod spaces;

pub use client::{
    DEFAULT_RETRIES, DEFAULT_TIMEOUT_SECS, GitFoxClient, GitFoxClientBuilder, Method, Query,
    RawResponse, is_retryable_method,
};
pub use error::{Error, Result};
pub use models::{
    Check, CheckCountSummary, CiStatus, CodeComment, Commit, Content, ContentEntry, CreateGitspace,
    CreatePullRequest, CreateRepository, Execution, FileDiff, Gitspace, GitspaceInstance, Identity,
    LABEL_COLORS, Label, LabelAssignment, LabelInput, LabelValue, LabelValueInfo, LicenseTemplate,
    LogLine, Membership, MergeMethod, MergePullRequest, MergeResult, Pipeline, Principal,
    PublicKey, PullRequest, PullRequestActivity, PullRequestCheck, PullRequestChecks,
    PullRequestLabel, PullRequestLabels, PullRequestReviewer, PullRequestState, PullRequestStats,
    RepoRef, Repository, ReviewDecision, Rule, Signature, Space, SpaceSecret, Stage, Step,
    SystemConfig, UpdatePullRequest, User, decode_base64, encode_base64, label_color,
    label_color_hex,
};
pub use pull_request::PullRequestFilter;
pub use repo::RepoSort;
