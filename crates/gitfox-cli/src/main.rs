//! `fx` — GitFox CLI for humans, CI and AI agents.
//!
//! The whole binary is a thin shell around three ideas:
//!
//! 1. [`config`] resolves settings through one documented precedence chain.
//! 2. [`commands`] produce values; [`output`] decides how they are rendered.
//! 3. [`error`] turns every failure into a stable code, message and exit status.
//!
//! GitFox itself lives behind `gitfox-client`, so an MCP server can later reuse
//! the same implementation instead of shelling out to this binary.

mod cli;
mod commands;
mod config;
mod context;
mod error;
mod git;
mod keychain;
mod output;
mod paginate;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, GlobalArgs};
use crate::config::{ENV_AGENT, ENV_OUTPUT, EnvSource, SystemEnv, parse_bool};
use crate::error::CliError;
use crate::output::{OutputFormat, Renderer};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.global.verbose);

    let ctx = match context::Context::build(&cli.global) {
        Ok(ctx) => ctx,
        // Configuration failed, so there is no resolved renderer yet; fall back
        // to what the flags and environment say about the caller.
        Err(err) => return fail(&fallback_renderer(&cli.global), &err),
    };

    match commands::dispatch(cli.command, &ctx).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&ctx.renderer, &err),
    }
}

fn fail(renderer: &Renderer, err: &CliError) -> ExitCode {
    let _ = renderer.emit_error(err);
    // Exit codes are part of the contract; see `docs/exit-codes.md`.
    ExitCode::from(err.exit_code() as u8)
}

/// A best-effort renderer for errors raised before configuration resolved.
fn fallback_renderer(global: &GlobalArgs) -> Renderer {
    let env = SystemEnv;
    let agent = global.agent || env.get(ENV_AGENT).is_some_and(|v| parse_bool(&v));
    let format = global
        .output
        .or(if global.json {
            Some(OutputFormat::Json)
        } else {
            None
        })
        .or_else(|| env.get(ENV_OUTPUT).and_then(|v| v.parse().ok()))
        .unwrap_or(if agent {
            OutputFormat::Json
        } else {
            OutputFormat::Table
        });
    let color = !(global.no_color || agent || env.is_set(config::ENV_NO_COLOR))
        && format == OutputFormat::Table
        && stdout_is_tty();
    Renderer::new(format, color)
}

fn stdout_is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Logging goes to stderr and is off unless asked for: stdout belongs to the
/// output contract. `RUST_LOG` wins over `-v` when both are present.
fn init_tracing(verbose: u8) {
    use tracing_subscriber::EnvFilter;

    let filter = match std::env::var("RUST_LOG") {
        Ok(value) if !value.trim().is_empty() => EnvFilter::new(value),
        _ => EnvFilter::new(match verbose {
            0 => return,
            1 => "gitfox_cli=info,gitfox_client=info",
            2 => "gitfox_cli=debug,gitfox_client=debug",
            _ => "gitfox_cli=trace,gitfox_client=trace,reqwest=debug",
        }),
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .try_init();
}
