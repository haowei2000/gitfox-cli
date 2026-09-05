//! `fx repo` — planned for v0.2, together with git remote detection.

use crate::cli::{RepoCommand, RepoSubcommand};
use crate::context::Context;
use crate::error::{CliError, Result};

pub fn run(cmd: RepoCommand, _ctx: &Context) -> Result<()> {
    let name = match cmd.command {
        RepoSubcommand::List(_) => "fx repo list",
        RepoSubcommand::View(_) => "fx repo view",
        RepoSubcommand::Clone(_) => "fx repo clone",
    };
    Err(CliError::not_implemented(name, "v0.2"))
}
