//! `gitfox-client` — a Rust client for the GitFox API.
//!
//! This crate knows about HTTP and about GitFox. It knows nothing about
//! terminals, tables, exit codes or configuration files. That separation is
//! what lets `fx` (the CLI) and, later, `fx-mcp` (the MCP server) share one
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
pub mod models;
pub mod pipeline;
pub mod principal;
pub mod pull_request;
pub mod repo;

pub use client::{
    DEFAULT_RETRIES, DEFAULT_TIMEOUT_SECS, GitFoxClient, GitFoxClientBuilder, Method, Query,
    RawResponse, is_retryable_method,
};
pub use error::{Error, Result};
pub use models::{
    CiStatus, CreatePullRequest, Execution, LogLine, MergeMethod, MergePullRequest, MergeResult,
    Pipeline, Principal, PullRequest, PullRequestState, PullRequestStats, RepoRef, Repository,
    Stage, Step, User,
};
pub use pull_request::PullRequestFilter;
pub use repo::RepoSort;
