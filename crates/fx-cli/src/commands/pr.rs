//! `fx pr` — list/view/create/merge land in v0.3; checkout/diff/checks in v0.5.

use crate::cli::{PrCommand, PrSubcommand};
use crate::context::Context;
use crate::error::{CliError, Result};

pub fn run(cmd: PrCommand, _ctx: &Context) -> Result<()> {
    let (name, version) = match cmd.command {
        PrSubcommand::List(_) => ("fx pr list", "v0.3"),
        PrSubcommand::View(_) => ("fx pr view", "v0.3"),
        PrSubcommand::Create(_) => ("fx pr create", "v0.3"),
        PrSubcommand::Merge(_) => ("fx pr merge", "v0.3"),
        PrSubcommand::Checkout(_) => ("fx pr checkout", "v0.5"),
        PrSubcommand::Diff(_) => ("fx pr diff", "v0.5"),
        PrSubcommand::Checks(_) => ("fx pr checks", "v0.5"),
    };
    Err(CliError::not_implemented(name, version))
}
