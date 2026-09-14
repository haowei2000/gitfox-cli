use serde::{Deserialize, Serialize};

use super::Principal;

/// A space — GitFox's organisation, which can nest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Space {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub identifier: String,
    /// The full path, e.g. `org/team`.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_public: Option<bool>,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
}

impl Space {
    pub fn reference(&self) -> String {
        if self.path.is_empty() {
            self.identifier.clone()
        } else {
            self.path.clone()
        }
    }
}

/// A space the caller belongs to, and in what role.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Membership {
    #[serde(default)]
    pub space: Space,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub added_by: Option<Principal>,
    #[serde(default)]
    pub created: Option<i64>,
}

/// A secret stored in a space. GitFox never returns the value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpaceSecret {
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub space_id: Option<i64>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
}

/// An SSH public key on the caller's account.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PublicKey {
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// The key algorithm, e.g. `ssh-ed25519`.
    #[serde(default, rename = "type")]
    pub key_type: Option<String>,
    /// `auth` — GitFox has no signing keys.
    #[serde(default)]
    pub usage: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub verified: Option<i64>,
}

/// `GET /api/v1/system/config` — what this instance has switched on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemConfig {
    #[serde(default)]
    pub gitspace_enabled: Option<bool>,
    #[serde(default)]
    pub ssh_enabled: Option<bool>,
    #[serde(default)]
    pub user_signup_allowed: Option<bool>,
    #[serde(default)]
    pub public_resource_creation_enabled: Option<bool>,
    #[serde(default)]
    pub artifact_registry_enabled: Option<bool>,
}

/// One entry of `GET /api/v1/resources/license`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LicenseTemplate {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub value: String,
}
