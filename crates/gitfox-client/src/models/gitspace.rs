use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A gitspace — GitFox's cloud development environment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gitspace {
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub name: Option<String>,
    /// `running`, `stopped`, `starting`, `stopping`, `error` or `uninitialized`.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub code_repo_ref: Option<String>,
    #[serde(default)]
    pub code_repo_url: Option<String>,
    #[serde(default)]
    pub code_repo_type: Option<String>,
    #[serde(default)]
    pub ide: Option<String>,
    #[serde(default)]
    pub devcontainer_path: Option<String>,
    #[serde(default)]
    pub space_path: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_display_name: Option<String>,
    #[serde(default)]
    pub resource: Value,
    #[serde(default)]
    pub instance: Option<GitspaceInstance>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitspaceInstance {
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub last_used: Option<i64>,
    #[serde(default)]
    pub created: Option<i64>,
}

/// The body of `POST /api/v1/gitspaces`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateGitspace {
    pub identifier: String,
    pub name: String,
    pub space_ref: String,
    pub code_repo_ref: String,
    pub code_repo_url: String,
    pub code_repo_type: String,
    pub branch: String,
    pub ide: String,
    pub resource_identifier: String,
    pub resource_space_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub devcontainer_path: Option<String>,
}
