//! Command implementations. One module per top-level command, matching the
//! tree in [`crate::cli`].

pub mod api;
pub mod auth;
pub mod completion;
pub mod config;
pub mod pipeline;
pub mod pr;
pub mod repo;

use crate::cli::Command;
use crate::context::Context;
use crate::error::Result;

pub async fn dispatch(command: Command, ctx: &Context) -> Result<()> {
    match command {
        Command::Auth(cmd) => auth::run(cmd, ctx).await,
        Command::Api(args) => api::run(args, ctx).await,
        Command::Repo(cmd) => repo::run(cmd, ctx).await,
        Command::Pr(cmd) => pr::run(cmd, ctx).await,
        Command::Pipeline(cmd) => pipeline::run(cmd, ctx).await,
        Command::Config(cmd) => config::run(cmd, ctx),
        Command::Completion(args) => completion::run(args),
    }
}
