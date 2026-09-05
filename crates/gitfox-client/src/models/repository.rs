use serde::{Deserialize, Serialize};

/// A repository.
///
/// This is the CLI's own model, not the raw API DTO: the serde attributes are
/// the translation layer, so an upstream rename is absorbed with an `alias`
/// instead of breaking the JSON schema agents depend on.
///
/// GitFox returns two shapes here. `GET /repos/{repo_ref}` and
/// `GET /spaces/{space_ref}/repos` answer with `RepoRepositoryOutput`, which
/// carries `is_public` and `importing`; the instance-wide `GET /repos` answers
/// with `TypesRepository`, which does not. Both decode into this one struct,
/// and the fields the narrower shape lacks stay `None` — a repository whose
/// visibility is simply unknown, rather than one wrongly reported as private.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Repository {
    #[serde(default)]
    pub id: Option<i64>,
    /// The repository name on its own, e.g. `backend`.
    #[serde(default)]
    pub identifier: Option<String>,
    /// The full space path, e.g. `ai/backend`.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_branch: Option<String>,
    /// `None` when the endpoint that produced this does not report visibility.
    #[serde(default)]
    pub is_public: Option<bool>,
    #[serde(default)]
    pub importing: Option<bool>,
    #[serde(default)]
    pub git_url: Option<String>,
    #[serde(default)]
    pub git_ssh_url: Option<String>,
    #[serde(default)]
    pub is_empty: Option<bool>,
    #[serde(default)]
    pub num_open_pulls: Option<i64>,
    #[serde(default)]
    pub num_pulls: Option<i64>,
    #[serde(default)]
    pub num_forks: Option<i64>,
    /// Size in KiB, as GitFox reports it.
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
}

impl Repository {
    /// `ai/backend` — the reference a user would type back into `-R`.
    pub fn reference(&self) -> String {
        self.path
            .clone()
            .or_else(|| self.identifier.clone())
            .unwrap_or_default()
    }

    /// `public`, `private`, or `None` when the endpoint did not say.
    pub fn visibility(&self) -> Option<&'static str> {
        self.is_public.map(|p| if p { "public" } else { "private" })
    }

    /// The URL `git clone` should be given.
    pub fn clone_url(&self, ssh: bool) -> Option<&str> {
        let (first, second) = if ssh {
            (&self.git_ssh_url, &self.git_url)
        } else {
            (&self.git_url, &self.git_ssh_url)
        };
        first
            .as_deref()
            .filter(|u| !u.is_empty())
            .or_else(|| second.as_deref().filter(|u| !u.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_narrow_list_shape_decodes_with_visibility_unknown() {
        // `GET /repos` answers with TypesRepository, which has no is_public.
        let repo: Repository = serde_json::from_value(json!({
            "identifier": "backend", "path": "ai/backend", "default_branch": "main"
        }))
        .unwrap();
        assert_eq!(repo.reference(), "ai/backend");
        // Unknown, not "private" — the difference matters in a listing.
        assert_eq!(repo.visibility(), None);
    }

    #[test]
    fn the_wide_shape_reports_visibility() {
        let public: Repository =
            serde_json::from_value(json!({ "identifier": "x", "is_public": true })).unwrap();
        assert_eq!(public.visibility(), Some("public"));
        let private: Repository =
            serde_json::from_value(json!({ "identifier": "x", "is_public": false })).unwrap();
        assert_eq!(private.visibility(), Some("private"));
    }

    #[test]
    fn clone_url_honours_the_requested_protocol_and_falls_back() {
        let both = Repository {
            git_url: Some("http://h:3000/git/ai/backend.git".into()),
            git_ssh_url: Some("git@h:ai/backend.git".into()),
            ..Default::default()
        };
        assert_eq!(
            both.clone_url(false),
            Some("http://h:3000/git/ai/backend.git")
        );
        assert_eq!(both.clone_url(true), Some("git@h:ai/backend.git"));

        // An instance with SSH disabled still clones over HTTP.
        let http_only = Repository {
            git_url: Some("http://h:3000/git/ai/backend.git".into()),
            git_ssh_url: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(
            http_only.clone_url(true),
            Some("http://h:3000/git/ai/backend.git")
        );
        assert_eq!(Repository::default().clone_url(false), None);
    }

    #[test]
    fn reference_falls_back_to_the_identifier() {
        let repo = Repository {
            identifier: Some("backend".into()),
            ..Default::default()
        };
        assert_eq!(repo.reference(), "backend");
    }
}
