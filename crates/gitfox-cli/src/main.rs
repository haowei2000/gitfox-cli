//! `gf` — GitFox CLI for humans, CI and AI agents.
//!
//! The whole binary is a thin shell around three ideas:
//!
//! 1. [`config`] resolves settings through one documented precedence chain.
//! 2. [`commands`] produce values; [`output`] decides how they are rendered.
//! 3. [`error`] turns every failure into a stable code, message and exit status.
//!
//! GitFox itself lives behind `gitfox-client`, so an MCP server can later reuse
//! the same implementation instead of shelling out to this binary.

mod argv;
mod cli;
mod commands;
mod config;
mod context;
mod error;
mod export;
mod git;
mod interact;
mod keychain;
mod output;
mod paginate;

use std::process::ExitCode;

use clap::Parser;

use crate::argv::Invocation;
use crate::cli::{Cli, GlobalArgs};
use crate::config::{ENV_AGENT, ENV_OUTPUT, EnvSource, SystemEnv, parse_bool};
use crate::error::CliError;
use crate::export::ExportSpec;
use crate::output::{OutputFormat, Renderer};

#[tokio::main]
async fn main() -> ExitCode {
    let args = match argv::prepare(std::env::args_os().collect()) {
        Invocation::Args(args) => args,
        Invocation::Shell { script, args } => return run_shell_alias(&script, &args),
    };
    let cli = Cli::parse_from(args);
    init_tracing(cli.global.verbose);

    let mut ctx = match context::Context::build(&cli.global) {
        Ok(ctx) => ctx,
        // Configuration failed, so there is no resolved renderer yet; fall back
        // to what the flags and environment say about the caller.
        Err(err) => return fail(&fallback_renderer(&cli.global), &err),
    };

    match ExportSpec::resolve(cli.global.json.as_deref(), cli.command.format()) {
        Ok(spec) => ctx.renderer.set_export(spec),
        Err(err) => return fail(&ctx.renderer, &err),
    }

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

/// Run a `!` alias through the shell, the alias's arguments as `$1`, `$2`….
fn run_shell_alias(script: &str, args: &[String]) -> ExitCode {
    let (shell, flag) = if cfg!(windows) {
        ("sh.exe", "-c")
    } else {
        ("sh", "-c")
    };
    match std::process::Command::new(shell)
        .arg(flag)
        .arg(script)
        .arg("gf-alias")
        .args(args)
        .status()
    {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(err) => {
            let _ = fail(
                &Renderer::new(OutputFormat::Table, false),
                &CliError::config(format!("could not run the shell for this alias: {err}")),
            );
            ExitCode::from(7)
        }
    }
}

/// A best-effort renderer for errors raised before configuration resolved.
fn fallback_renderer(global: &GlobalArgs) -> Renderer {
    let env = SystemEnv;
    let agent = global.agent || env.get(ENV_AGENT).is_some_and(|v| parse_bool(&v));
    let format = global
        .output
        .or(match global.json.as_deref() {
            Some("") => Some(OutputFormat::Json),
            _ => None,
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
