//! What the surrounding git checkout can tell us.
//!
//! This is the bottom tier of the configuration chain: it is why `cd project &&
//! fx pr list` works without a `-R`.
//!
//! We shell out to `git` rather than linking libgit2. Everything needed here is
//! three plumbing commands, `git` is already present for anyone with a GitFox
//! checkout, and `fx pr checkout` will have to drive the real `git` for its
//! credentials anyway.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::GitContext;

/// Everything read from the checkout in one pass.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct GitInfo {
    /// API base URL, only when the remote is an HTTP(S) one — an SSH remote
    /// says nothing about which port or scheme the API is on.
    pub host: Option<String>,
    /// `space/name`.
    pub repo: Option<String>,
    /// The checked-out branch, absent when HEAD is detached.
    pub branch: Option<String>,
}

impl GitInfo {
    pub fn to_context(&self) -> GitContext {
        GitContext {
            host: self.host.clone(),
            repo: self.repo.clone(),
        }
    }
}

/// Inspect the current directory. Never fails: outside a checkout, or without
/// `git` on PATH, every field is simply `None`.
pub fn detect() -> GitInfo {
    if run(&["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return GitInfo::default();
    }
    let mut info = GitInfo {
        branch: current_branch(),
        ..Default::default()
    };
    if let Some(url) = remote_url()
        && let Some(remote) = parse_remote(&url)
    {
        info.host = remote.host;
        info.repo = Some(remote.repo);
    }
    info
}

/// The checked-out branch, or `None` when HEAD is detached.
///
/// `symbolic-ref` rather than `rev-parse --abbrev-ref`: the latter fails
/// outright on a branch with no commits yet, which is a perfectly ordinary
/// state to run `fx` in. `symbolic-ref` answers there and fails exactly when
/// HEAD really is detached.
fn current_branch() -> Option<String> {
    run(&["symbolic-ref", "--short", "HEAD"]).filter(|b| !b.is_empty() && b != "HEAD")
}

/// `origin` if it exists, otherwise whichever remote is listed first.
fn remote_url() -> Option<String> {
    if let Some(url) = run(&["remote", "get-url", "origin"]) {
        return Some(url);
    }
    let first = run(&["remote"])?.lines().next()?.trim().to_string();
    if first.is_empty() {
        return None;
    }
    run(&["remote", "get-url", &first])
}

fn run(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// One commit, as `--fill` sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub subject: String,
    pub body: String,
}

/// Commits on `head` that are not on `base`, oldest first.
///
/// `base` is tried locally first and then as `origin/<base>`, because the base
/// branch often exists only on the remote in a fresh clone. An empty result
/// means the range could not be resolved — callers treat that as "nothing to
/// fill from" rather than an error.
pub fn commits_between(base: &str, head: &str) -> Vec<Commit> {
    const RECORD: char = '\x1e';
    const FIELD: char = '\x1f';

    for range in [format!("{base}..{head}"), format!("origin/{base}..{head}")] {
        let Some(raw) = run(&[
            "log",
            "--reverse",
            &format!("--format=%s{FIELD}%b{RECORD}"),
            &range,
        ]) else {
            continue;
        };
        let commits: Vec<Commit> = raw
            .split(RECORD)
            .filter_map(|record| {
                let record = record.trim_start_matches('\n');
                let (subject, body) = record.split_once(FIELD)?;
                let subject = subject.trim();
                (!subject.is_empty()).then(|| Commit {
                    subject: subject.to_string(),
                    body: body.trim().to_string(),
                })
            })
            .collect();
        if !commits.is_empty() {
            return commits;
        }
    }
    Vec::new()
}

/// Turn a branch's commits into a pull request title and body.
///
/// One commit means the pull request *is* that commit, so its subject and body
/// carry over verbatim. Several commits get the first subject as a title and a
/// bullet list as the body — the same shape a reviewer would write by hand.
pub fn fill_from_commits(commits: &[Commit]) -> Option<(String, String)> {
    match commits {
        [] => None,
        [only] => Some((only.subject.clone(), only.body.clone())),
        [first, ..] => {
            let bullets = commits
                .iter()
                .map(|c| format!("* {}", c.subject))
                .collect::<Vec<_>>()
                .join("\n");
            Some((first.subject.clone(), bullets))
        }
    }
}

/// Run `git clone` into `destination`, letting git own the terminal.
///
/// stderr is inherited so progress and any credential prompt reach the user;
/// stdout is captured because it is not part of what a caller asked for and
/// would otherwise break the machine output contract.
///
/// `destination` is always explicit. Letting git derive it from the URL would
/// name the directory after whatever the URL happens to end with, which is only
/// incidentally the repository's name — the caller knows the real one.
///
/// The URL is passed through untouched. fx never splices a token into it: that
/// would write the credential into `.git/config`, where it outlives the command
/// and travels with the checkout.
pub fn clone(url: &str, destination: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("clone")
        .arg(url)
        .arg(destination)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;

    if output.status.success() {
        return Ok(());
    }
    Err(match output.status.code() {
        Some(code) => format!("git clone exited with status {code}"),
        None => "git clone was terminated by a signal".to_string(),
    })
}

#[derive(Debug, PartialEq)]
pub struct Remote {
    pub host: Option<String>,
    pub repo: String,
}

/// Pull `space/name` — and, for HTTP remotes, the API base URL — out of a git
/// remote URL.
///
/// GitFox serves git over `<base>/git/<space>/<name>.git`, so the `git/` prefix
/// is stripped when present. Nested spaces (`org/team/repo`) survive intact.
pub fn parse_remote(url: &str) -> Option<Remote> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    // scp-like syntax: git@host:space/repo.git
    if !url.contains("://")
        && let Some((_prefix, path)) = url.split_once(':')
        && url.contains('@')
    {
        return Some(Remote {
            host: None,
            repo: clean_path(path)?,
        });
    }

    let parsed = url::Url::parse(url).ok()?;
    let repo = clean_path(parsed.path())?;
    let host = match parsed.scheme() {
        "http" | "https" => {
            let mut base = parsed.clone();
            base.set_path("");
            base.set_query(None);
            base.set_fragment(None);
            // `Url` renders this as "http://host/"; the trailing slash is what
            // `Url::join` wants anyway.
            Some(base.as_str().trim_end_matches('/').to_string())
        }
        // ssh:// and git:// carry no usable API base.
        _ => None,
    };
    Some(Remote { host, repo })
}

fn clean_path(path: &str) -> Option<String> {
    let path = path.trim().trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    // GitFox's git endpoint lives under /git/.
    let path = path.strip_prefix("git/").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return None;
    }
    Some(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scp_style_ssh_remotes() {
        let r = parse_remote("git@git.example.com:ai/backend.git").unwrap();
        assert_eq!(r.repo, "ai/backend");
        // An SSH remote cannot tell us the API scheme or port.
        assert_eq!(r.host, None);
    }

    #[test]
    fn parses_ssh_url_remotes() {
        let r = parse_remote("ssh://git@git.example.com:22/ai/backend.git").unwrap();
        assert_eq!(r.repo, "ai/backend");
        assert_eq!(r.host, None);
    }

    #[test]
    fn parses_http_remotes_and_keeps_the_port() {
        let r = parse_remote("http://10.1.1.32:3000/git/ai/backend.git").unwrap();
        assert_eq!(r.repo, "ai/backend");
        assert_eq!(r.host.as_deref(), Some("http://10.1.1.32:3000"));
    }

    #[test]
    fn strips_the_gitfox_git_prefix_only_when_present() {
        assert_eq!(
            parse_remote("https://git.example.com/git/ai/backend.git")
                .unwrap()
                .repo,
            "ai/backend"
        );
        assert_eq!(
            parse_remote("https://git.example.com/ai/backend.git")
                .unwrap()
                .repo,
            "ai/backend"
        );
    }

    #[test]
    fn keeps_nested_spaces() {
        assert_eq!(
            parse_remote("git@git.example.com:org/team/backend.git")
                .unwrap()
                .repo,
            "org/team/backend"
        );
        assert_eq!(
            parse_remote("http://h:3000/git/org/team/backend.git")
                .unwrap()
                .repo,
            "org/team/backend"
        );
    }

    #[test]
    fn tolerates_a_missing_dot_git_suffix_and_trailing_slash() {
        assert_eq!(
            parse_remote("https://git.example.com/git/ai/backend/")
                .unwrap()
                .repo,
            "ai/backend"
        );
    }

    #[test]
    fn rejects_remotes_without_a_space_segment() {
        for bad in [
            "",
            "   ",
            "https://git.example.com/",
            "git@host:backend.git",
        ] {
            assert!(
                parse_remote(bad).is_none(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    #[test]
    fn fill_uses_a_lone_commit_verbatim() {
        let commits = [Commit {
            subject: "feat: add OAuth".into(),
            body: "Closes #12".into(),
        }];
        let (title, body) = fill_from_commits(&commits).unwrap();
        assert_eq!(title, "feat: add OAuth");
        assert_eq!(body, "Closes #12");
    }

    #[test]
    fn fill_summarises_several_commits_as_a_list() {
        let commits = [
            Commit {
                subject: "feat: add OAuth".into(),
                body: "x".into(),
            },
            Commit {
                subject: "test: cover the callback".into(),
                body: String::new(),
            },
        ];
        let (title, body) = fill_from_commits(&commits).unwrap();
        assert_eq!(title, "feat: add OAuth");
        assert_eq!(body, "* feat: add OAuth\n* test: cover the callback");
    }

    #[test]
    fn fill_gives_up_on_an_empty_range() {
        assert!(fill_from_commits(&[]).is_none());
    }

    #[test]
    fn a_detected_context_feeds_the_precedence_chain() {
        let info = GitInfo {
            host: Some("http://10.1.1.32:3000".into()),
            repo: Some("ai/backend".into()),
            branch: Some("feat/oauth".into()),
        };
        let ctx = info.to_context();
        assert_eq!(ctx.host.as_deref(), Some("http://10.1.1.32:3000"));
        assert_eq!(ctx.repo.as_deref(), Some("ai/backend"));
    }
}
