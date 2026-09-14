//! `fx browse` — open the GitFox web UI at the right page.
//!
//! The routes are the UI's own (`/{repo}/pulls/{n}`, `/{repo}/files/{ref}/~/…`,
//! `/{repo}/pipelines`), read from the GitFox web bundle rather than guessed.

use crate::cli::BrowseArgs;
use crate::context::Context;
use crate::error::{CliError, Result};
use crate::git;
use crate::interact;

pub async fn run(args: BrowseArgs, ctx: &Context) -> Result<()> {
    for (given, flag, why) in [
        (args.projects, "`--projects`", "GitFox has no projects"),
        (args.releases, "`--releases`", "GitFox has no releases"),
        (args.wiki, "`--wiki`", "GitFox repositories have no wiki"),
        (
            args.blame,
            "`--blame`",
            "the GitFox UI has no blame page to link to",
        ),
    ] {
        if given {
            return Err(CliError::unsupported(flag, why));
        }
    }

    let repo = ctx.repo()?;
    let path = page_path(&repo.full(), &args, ctx)?;
    let url = ctx.web_url(&path)?;

    if args.no_browser {
        return interact::print_url(ctx, &url);
    }
    interact::open_in_browser(ctx, &url)
}

/// The UI path for what was asked for.
fn page_path(repo: &str, args: &BrowseArgs, ctx: &Context) -> Result<String> {
    if args.actions {
        return Ok(format!("{repo}/pipelines"));
    }
    if args.settings {
        return Ok(format!("{repo}/settings"));
    }
    if let Some(commit) = &args.commit {
        let sha = if commit == "last" {
            git::rev_parse("HEAD").ok_or_else(|| {
                CliError::new(
                    crate::error::ErrorCode::GitContextError,
                    "no commit to open: not inside a git repository",
                )
            })?
        } else {
            commit.clone()
        };
        return Ok(match &args.target {
            Some(file) => format!("{repo}/files/{sha}/~/{}", strip_line(file)),
            None => format!("{repo}/commit/{sha}"),
        });
    }

    let Some(target) = args.target.as_deref() else {
        return Ok(match &args.branch {
            Some(branch) => format!("{repo}/files/{branch}"),
            None => repo.to_string(),
        });
    };

    let digits = target.strip_prefix('#').unwrap_or(target);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        // GitFox has no issues, so a number is a pull request.
        return Ok(format!("{repo}/pulls/{digits}"));
    }
    if is_commit_sha(target) && !std::path::Path::new(target).exists() {
        return Ok(format!("{repo}/commit/{target}"));
    }

    let git_ref = match &args.branch {
        Some(branch) => branch.clone(),
        None => ctx.git.branch.clone().unwrap_or_else(|| "HEAD".to_string()),
    };
    Ok(format!("{repo}/files/{git_ref}/~/{}", strip_line(target)))
}

/// `src/main.rs:12` → `src/main.rs`. The UI has no line anchors to link to.
fn strip_line(path: &str) -> &str {
    match path.rsplit_once(':') {
        Some((file, line))
            if !line.is_empty() && line.chars().all(|c| c.is_ascii_digit() || c == '-') =>
        {
            file
        }
        _ => path,
    }
}

fn is_commit_sha(text: &str) -> bool {
    (7..=40).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_suffix_is_dropped_and_a_sha_is_recognised() {
        assert_eq!(strip_line("src/main.rs:12"), "src/main.rs");
        assert_eq!(strip_line("src/main.rs:12-20"), "src/main.rs");
        assert_eq!(strip_line("README.md"), "README.md");
        assert!(is_commit_sha("04e53c1c90f5"));
        assert!(!is_commit_sha("readme"));
        assert!(!is_commit_sha("abc"));
    }
}
