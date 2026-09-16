//! `gf completion` — shell completion scripts.
//!
//! Written straight to stdout rather than through the renderer: the output is a
//! shell script, not a result, and it is meant to be redirected into a file.

use std::io::Write;

use clap::CommandFactory;

use crate::cli::{Cli, CompletionArgs};
use crate::error::{CliError, ErrorCode, Result};
use crate::output;

pub fn run(args: CompletionArgs) -> Result<()> {
    // `gf completion zsh` and gh's `gf completion -s zsh` both work; clap
    // guarantees exactly one was given.
    let shell = args.shell.or(args.shell_flag).ok_or_else(|| {
        CliError::invalid_argument("name a shell: bash, zsh, fish, powershell or elvish")
    })?;
    let mut command = Cli::command();

    // Generated into memory first: `clap_complete::generate` panics on any
    // write error, and `gf completion zsh | head` closes the pipe long before
    // the script is finished. A Vec cannot fail, so the only fallible write is
    // the one below, where a closed reader is forgiven.
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut command, "gf", &mut script);

    let mut out = std::io::stdout().lock();
    output::forgive_broken_pipe(out.write_all(&script).and_then(|()| out.flush()))
        .map_err(|e| CliError::new(ErrorCode::Unexpected, e.to_string()))
}
