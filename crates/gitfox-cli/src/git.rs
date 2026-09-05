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
    /// Every remote, `origin` first. All of them, because the one that points
    /// at GitFox is not always the first — or named `origin`.
    pub remotes: Vec<Remote>,
    /// The checked-out branch, absent when HEAD is detached.
    pub branch: Option<String>,
}

impl GitInfo {
    pub fn to_context(&self) -> GitContext {
        GitContext {
            remotes: self.remotes.clone(),
        }
    }
}

/// Inspect the current directory. Never fails: outside a checkout, or without
/// `git` on PATH, every field is simply empty.
pub fn detect() -> GitInfo {
    if run(&["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return GitInfo::default();
    }
    GitInfo {
        remotes: remotes(),
        branch: current_branch(),
    }
}

/// Every remote that parses, `origin` first.
fn remotes() -> Vec<Remote> {
    let Some(listing) = run(&["remote"]) else {
        return Vec::new();
    };
    let mut names: Vec<&str> = listing
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .collect();
    // `origin` is the convention, so it gets first refusal; the rest keep
    // git's own order.
    names.sort_by_key(|name| *name != "origin");

    names
        .into_iter()
        .filter_map(|name| {
            let url = run(&["remote", "get-url", name])?;
            parse_remote(&url)
        })
        .collect()
}

fn current_branch() -> Option<String> {
    run(&["symbolic-ref", "--short", "HEAD"]).filter(|b| !b.is_empty() && b != "HEAD")
}

/// Run git and return its trimmed stdout, or `None` on any failure.
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

/// The remote to fetch from: `origin` if it exists, else the first one.
pub fn remote_name() -> Option<String> {
    let remotes = run(&["remote"])?;
    let mut names = remotes.lines().map(str::trim).filter(|n| !n.is_empty());
    if remotes.lines().any(|n| n.trim() == "origin") {
        return Some("origin".to_string());
    }
    names.next().map(str::to_string)
}

pub fn local_branch_exists(branch: &str) -> bool {
    run_checked(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/heads/{branch}"),
    ])
    .is_ok()
}

/// `git fetch <remote> <branch>`, leaving the result in `FETCH_HEAD`.
pub fn fetch(remote: &str, branch: &str) -> Result<(), String> {
    run_checked(&["fetch", remote, branch]).map(|_| ())
}

/// Create `branch` at `start` and switch to it.
pub fn checkout_new(branch: &str, start: &str) -> Result<(), String> {
    run_checked(&["checkout", "-b", branch, start]).map(|_| ())
}

pub fn checkout(branch: &str) -> Result<(), String> {
    run_checked(&["checkout", branch]).map(|_| ())
}

/// Fast-forward only: an existing local branch that has diverged is a conflict
/// for the user to resolve, not something to merge behind their back.
pub fn merge_ff_only(rev: &str) -> Result<(), String> {
    run_checked(&["merge", "--ff-only", rev]).map(|_| ())
}

pub fn set_upstream(branch: &str, remote: &str) -> Result<(), String> {
    run_checked(&[
        "branch",
        &format!("--set-upstream-to={remote}/{branch}"),
        branch,
    ])
    .map(|_| ())
}

/// Run git and keep the failure message, unlike [`run`] which discards it.
fn run_checked(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        format!("git {} failed", args.first().copied().unwrap_or("command"))
    } else {
        stderr
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Remote {
    /// The hostname, for any scheme — this is what decides whether the remote
    /// is the GitFox one. The port is deliberately excluded: GitFox serves git
    /// over SSH on one port and its API over HTTP on another.
    pub host_key: Option<String>,
    /// An API base URL, only for HTTP(S) remotes. An SSH remote says nothing
    /// about which scheme or port the API is on.
    pub api_base: Option<String>,
    pub repo: String,
}

/// Pull `space/name` and the host out of a git remote URL.
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
        && let Some((prefix, path)) = url.split_once(':')
        && url.contains('@')
    {
        let host = prefix.rsplit('@').next().filter(|h| !h.is_empty());
        return Some(Remote {
            host_key: host.map(str::to_string),
            api_base: None,
            repo: clean_path(path)?,
        });
    }

    let parsed = url::Url::parse(url).ok()?;
    let repo = clean_path(parsed.path())?;
    let host_key = parsed.host_str().map(str::to_string);
    let api_base = match parsed.scheme() {
        "http" | "https" => {
            let mut base = parsed.clone();
            base.set_path("");
            base.set_query(None);
            base.set_fragment(None);
            Some(base.as_str().trim_end_matches('/').to_string())
        }
        // ssh:// and git:// carry no usable API base.
        _ => None,
    };
    Some(Remote {
        host_key,
        api_base,
        repo,
    })
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
        // The host is known even over SSH — that is what identifies the
        // instance — but the API scheme and port are not.
        assert_eq!(r.host_key.as_deref(), Some("git.example.com"));
        assert_eq!(r.api_base, None);
    }

    #[test]
    fn parses_ssh_url_remotes_and_ignores_the_git_port() {
        // GitFox serves git over SSH on one port and its API over HTTP on
        // another, so the port must not be part of the host key.
        let r = parse_remote("ssh://git@10.1.1.32:3322/ai-repos/GrantNexus.git").unwrap();
        assert_eq!(r.repo, "ai-repos/GrantNexus");
        assert_eq!(r.host_key.as_deref(), Some("10.1.1.32"));
        assert_eq!(r.api_base, None);
    }

    #[test]
    fn parses_http_remotes_and_keeps_the_port_in_the_api_base() {
        let r = parse_remote("http://10.1.1.32:3000/git/ai/backend.git").unwrap();
        assert_eq!(r.repo, "ai/backend");
        assert_eq!(r.host_key.as_deref(), Some("10.1.1.32"));
        assert_eq!(r.api_base.as_deref(), Some("http://10.1.1.32:3000"));
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
            remotes: vec![parse_remote("http://10.1.1.32:3000/git/ai/backend.git").unwrap()],
            branch: Some("feat/oauth".into()),
        };
        let ctx = info.to_context();
        assert_eq!(ctx.api_base().as_deref(), Some("http://10.1.1.32:3000"));
        assert_eq!(
            ctx.repo_for(Some("10.1.1.32")).as_deref(),
            Some("ai/backend")
        );
    }

    #[test]
    fn a_remote_for_another_host_contributes_no_repository() {
        // The case this gate exists for: a GitHub checkout, a GitFox host.
        // `haowei2000/gitfox-cli` is a real repository name that means nothing
        // to the instance being asked.
        let ctx = GitInfo {
            remotes: vec![parse_remote("git@github.com:haowei2000/gitfox-cli.git").unwrap()],
            branch: None,
        }
        .to_context();
        assert_eq!(ctx.repo_for(Some("10.1.1.32")), None);
        assert_eq!(ctx.api_base(), None);
        // It is still the right answer for its own host.
        assert_eq!(
            ctx.repo_for(Some("github.com")).as_deref(),
            Some("haowei2000/gitfox-cli")
        );
    }

    #[test]
    fn the_matching_remote_wins_even_when_it_is_not_first() {
        // A checkout with several remotes, only one of which is GitFox.
        let ctx = GitInfo {
            remotes: vec![
                parse_remote("ssh://aliyun3/srv/git/AgentNexus.git").unwrap(),
                parse_remote("git@github.com:Grant-Huang/AgentNexus.git").unwrap(),
                parse_remote("ssh://git@10.1.1.32:3322/ai-repos/GrantNexus.git").unwrap(),
            ],
            branch: None,
        }
        .to_context();
        assert_eq!(
            ctx.repo_for(Some("10.1.1.32")).as_deref(),
            Some("ai-repos/GrantNexus")
        );
    }

    #[test]
    fn without_a_resolved_host_no_repository_is_inferred() {
        let ctx = GitInfo {
            remotes: vec![parse_remote("git@10.1.1.32:ai/backend.git").unwrap()],
            branch: None,
        }
        .to_context();
        // Nothing to compare against, so nothing is assumed.
        assert_eq!(ctx.repo_for(None), None);
    }
}
