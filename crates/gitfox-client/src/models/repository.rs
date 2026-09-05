use serde::{Deserialize, Serialize};

/// A repository.
///
/// This is the CLI's own model, not the raw API DTO: the serde attributes are
/// the translation layer, so an upstream rename is absorbed with an `alias`
/// instead of breaking the JSON schema agents depend on.
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
    #[serde(default)]
    pub git_url: Option<String>,
    #[serde(default)]
    pub git_ssh_url: Option<String>,
    #[serde(default)]
    pub is_empty: Option<bool>,
    #[serde(default)]
    pub num_open_pulls: Option<i64>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
}
