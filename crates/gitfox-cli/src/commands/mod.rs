//! Command implementations. One module per top-level command, matching the
//! tree in [`crate::cli`].

pub mod alias;
pub mod api;
pub mod auth;
pub mod browse;
pub mod codespace;
pub mod completion;
pub mod config;
pub mod gh_only;
pub mod label;
pub mod org;
pub mod pipeline;
pub mod pr;
pub mod repo;
pub mod ruleset;
pub mod run;
pub mod secret;
pub mod ssh_key;
pub mod status;
pub mod workflow;

use crate::cli::Command;
use crate::context::Context;
use crate::error::Result;

pub async fn dispatch(command: Command, ctx: &Context) -> Result<()> {
    match command {
        Command::Auth(cmd) => auth::run(cmd, ctx).await,
        Command::Api(args) => api::run(args, ctx).await,
        Command::Repo(cmd) => repo::run(cmd, ctx).await,
        Command::Pr(cmd) => pr::run(cmd, ctx).await,
        Command::Co(args) => pr::checkout(args, ctx).await,
        Command::Pipeline(cmd) => pipeline::run(cmd, ctx).await,
        Command::Run(cmd) => run::run(cmd, ctx).await,
        Command::Workflow(cmd) => workflow::run(cmd, ctx).await,
        Command::Secret(cmd) => secret::run(cmd, ctx).await,
        Command::Label(cmd) => label::run(cmd, ctx).await,
        Command::SshKey(cmd) => ssh_key::run(cmd, ctx).await,
        Command::Org(cmd) => org::run(cmd, ctx).await,
        Command::Ruleset(cmd) => ruleset::run(cmd, ctx).await,
        Command::Codespace(cmd) => codespace::run(cmd, ctx).await,
        Command::Browse(args) => browse::run(args, ctx).await,
        Command::Status(args) => status::run(args, ctx).await,
        Command::Alias(cmd) => alias::run(cmd, ctx),
        Command::Config(cmd) => config::run(cmd, ctx),
        Command::Completion(args) => completion::run(args),
        Command::GhOnly(cmd) => gh_only::run(cmd),
    }
}
