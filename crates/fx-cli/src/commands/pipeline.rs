//! `fx pipeline` — planned for v0.4.

use crate::cli::{PipelineCommand, PipelineSubcommand};
use crate::context::Context;
use crate::error::{CliError, Result};

pub fn run(cmd: PipelineCommand, _ctx: &Context) -> Result<()> {
    let name = match cmd.command {
        PipelineSubcommand::List(_) => "fx pipeline list",
        PipelineSubcommand::View(_) => "fx pipeline view",
        PipelineSubcommand::Logs(_) => "fx pipeline logs",
        PipelineSubcommand::Run(_) => "fx pipeline run",
        PipelineSubcommand::Retry(_) => "fx pipeline retry",
    };
    Err(CliError::not_implemented(name, "v0.4"))
}
