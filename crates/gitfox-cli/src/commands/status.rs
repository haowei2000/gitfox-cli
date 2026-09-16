//! `gf status` — what needs you across a space.
//!
//! gh's status reports issues, pull requests, review requests and mentions
//! across every repository. GitFox has no issues or notifications, so this is
//! the pull request half: the ones waiting on your review and the ones you
//! opened, in the repositories of a space that have any open.

use gitfox_client::{GitFoxClient, PullRequest, PullRequestFilter, RepoRef, Repository};
use serde_json::{Value, json};

use crate::cli::StatusArgs;
use crate::context::Context;
use crate::error::{CliError, ErrorCode, Result};
use crate::output::{Render, relative_time};
use crate::paginate;

pub async fn run(args: StatusArgs, ctx: &Context) -> Result<()> {
    let space = ctx.space(args.space.as_deref())?;
    let client = ctx.client()?;
    let me = client.auth().current_user().await?;
    let my_id = me.id.ok_or_else(|| {
        CliError::new(
            ErrorCode::ApiError,
            "GitFox did not report an id for the current user",
        )
    })?;

    let (client_ref, space_ref) = (&client, space.as_str());
    let repos = paginate::collect(1000, move |page, limit| async move {
        client_ref
            .repos()
            .list_in_space(
                space_ref,
                None,
                gitfox_client::RepoSort::Updated,
                page,
                limit,
            )
            .await
    })
    .await
    .map_err(|e| match e {
        gitfox_client::Error::NotFound { .. } => {
            CliError::new(ErrorCode::NotFound, format!("no space `{space}`"))
        }
        other => CliError::from(other),
    })?;

    // Only repositories with something open cost a request.
    let active: Vec<Repository> = repos
        .items
        .into_iter()
        .filter(|r| r.num_open_pulls.unwrap_or(1) > 0)
        .filter(|r| !args.exclude.iter().any(|x| x == &r.reference()))
        .collect();

    let mut review_requests = Vec::new();
    let mut authored = Vec::new();
    for repository in &active {
        let Ok(repo) = RepoRef::parse(&repository.reference()) else {
            continue;
        };
        review_requests.extend(
            super::pr::awaiting_review(&client, &repo, my_id)
                .await?
                .into_iter()
                .map(|pr| (repo.clone(), pr)),
        );
        authored.extend(
            open(
                &client,
                &repo,
                PullRequestFilter {
                    author_id: Some(my_id),
                    ..Default::default()
                },
            )
            .await?
            .into_iter()
            .map(|pr| (repo.clone(), pr)),
        );
    }

    ctx.renderer.emit(&Status {
        space,
        scanned: active.len(),
        review_requests,
        authored,
    })
}

async fn open(
    client: &GitFoxClient,
    repo: &RepoRef,
    filter: PullRequestFilter,
) -> Result<Vec<PullRequest>> {
    client
        .pull_requests()
        .list(
            repo,
            &PullRequestFilter {
                limit: 50,
                ..filter
            },
        )
        .await
        .map_err(CliError::from)
}

struct Status {
    space: String,
    scanned: usize,
    review_requests: Vec<(RepoRef, PullRequest)>,
    authored: Vec<(RepoRef, PullRequest)>,
}

fn entries(items: &[(RepoRef, PullRequest)]) -> Vec<Value> {
    items
        .iter()
        .map(|(repo, pr)| {
            json!({
                "repository": repo.full(),
                "number": pr.number,
                "title": pr.title,
                "is_draft": pr.is_draft,
                "updated": pr.updated,
                "web_url": pr.web_url,
            })
        })
        .collect()
}

impl Render for Status {
    fn to_json(&self) -> Value {
        json!({
            "space": self.space,
            "repositories_scanned": self.scanned,
            "review_requests": entries(&self.review_requests),
            "pull_requests": entries(&self.authored),
        })
    }

    fn to_human(&self, color: bool) -> String {
        let (bold, dim, reset) = if color {
            ("\x1b[1m", "\x1b[2m", "\x1b[0m")
        } else {
            ("", "", "")
        };
        let section = |title: &str, items: &[(RepoRef, PullRequest)], empty: &str| {
            let mut out = format!("{bold}{title}{reset}\n");
            if items.is_empty() {
                out.push_str(&format!("{dim}{empty}{reset}\n"));
            }
            for (repo, pr) in items {
                out.push_str(&format!(
                    "{repo}#{}  {}  {dim}{}{reset}\n",
                    pr.number,
                    pr.title,
                    pr.updated.map(relative_time).unwrap_or_default()
                ));
            }
            out
        };
        format!(
            "{}\n{}\n{dim}Across {} {} in {} with open pull requests. GitFox has no issues or notifications to report.{reset}",
            section("Review Requests", &self.review_requests, "Nothing here ^_^"),
            section("Your Pull Requests", &self.authored, "Nothing here ^_^"),
            self.scanned,
            if self.scanned == 1 {
                "repository"
            } else {
                "repositories"
            },
            self.space,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lists_both_sections_and_says_what_gitfox_cannot_report() {
        let pr: PullRequest = serde_json::from_value(json!({
            "number": 12, "title": "feat: add OAuth", "state": "open"
        }))
        .unwrap();
        let status = Status {
            space: "ai".into(),
            scanned: 3,
            review_requests: vec![(RepoRef::parse("ai/backend").unwrap(), pr)],
            authored: vec![],
        };
        let text = status.to_human(false);
        assert!(text.contains("ai/backend#12  feat: add OAuth"), "{text}");
        assert!(text.contains("Your Pull Requests\nNothing here"), "{text}");
        assert!(text.contains("no issues or notifications"), "{text}");
        assert_eq!(status.to_json()["review_requests"][0]["number"], 12);
    }
}
