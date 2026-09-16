//! gh commands for features GitFox does not have.
//!
//! They parse — so a gh script, or an agent that learned gh, gets a reason
//! rather than clap's bare "unrecognized subcommand" — and every one answers
//! `UNSUPPORTED` with what GitFox lacks and, where something comes close, what
//! to use instead.

use crate::cli::GhOnlyCommand;
use crate::error::{CliError, Result};

pub fn run(cmd: GhOnlyCommand) -> Result<()> {
    let (command, why) = match cmd {
        GhOnlyCommand::AgentTask(_) => ("agent-task", "that is a GitHub Copilot feature"),
        GhOnlyCommand::Attestation(_) => ("attestation", "GitFox has no artifact attestations"),
        GhOnlyCommand::Cache(_) => (
            "cache",
            "GitFox does not expose its pipeline cache for listing or deletion",
        ),
        GhOnlyCommand::Copilot(_) => ("copilot", "that is a GitHub Copilot feature"),
        GhOnlyCommand::Discussion(_) => ("discussion", "GitFox has no discussions"),
        GhOnlyCommand::Extension(_) => (
            "extension",
            "gf has no extensions; `gf alias set --shell` covers custom commands",
        ),
        GhOnlyCommand::Gist(_) => ("gist", "GitFox has no gists"),
        GhOnlyCommand::GpgKey(_) => (
            "gpg-key",
            "GitFox verifies no commit signatures, so it keeps no GPG keys",
        ),
        GhOnlyCommand::Issue(_) => (
            "issue",
            "GitFox has no issue tracker; pull requests are its unit of work",
        ),
        GhOnlyCommand::Licenses(_) => (
            "licenses",
            "gf does not bundle third-party license texts; gf itself is MIT-licensed",
        ),
        GhOnlyCommand::Preview(_) => ("preview", "gf has no preview features to try"),
        GhOnlyCommand::Project(_) => ("project", "GitFox has no projects"),
        GhOnlyCommand::Release(_) => (
            "release",
            "GitFox has no releases; tag with git and publish artifacts from a pipeline",
        ),
        GhOnlyCommand::Search(_) => (
            "search",
            "GitFox has no cross-repository search; `gf pr list -S` and `gf repo list -S` search within one repository or space",
        ),
        GhOnlyCommand::Skill(_) => ("skill", "that is a GitHub feature"),
        GhOnlyCommand::Variable(_) => (
            "variable",
            "GitFox has no pipeline variables; store values with `gf secret set`",
        ),
    };
    refuse(command, why)
}

/// `gf <command>` is a gh command GitFox cannot back.
pub fn refuse(command: &str, why: &str) -> Result<()> {
    Err(CliError::unsupported(format!("`gf {command}`"), why))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::GhOnlyArgs;
    use crate::error::ErrorCode;

    #[test]
    fn a_gh_only_command_names_itself_and_says_why() {
        let err = run(GhOnlyCommand::Issue(GhOnlyArgs {
            rest: vec!["list".into()],
        }))
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
        assert_eq!(err.exit_code(), 9);
        assert!(
            err.message.starts_with("`gf issue` is not supported"),
            "{}",
            err.message
        );
        assert!(err.message.contains("issue tracker"), "{}", err.message);
    }
}
