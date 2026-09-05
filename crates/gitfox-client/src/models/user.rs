use serde::{Deserialize, Serialize};

/// The authenticated principal, as reported by `GET /api/v1/user`.
///
/// Every field is optional because self-hosted GitFox instances of different
/// versions populate slightly different subsets, and `fx auth status` must not
/// fail just because one of them is missing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct User {
    #[serde(default, alias = "uid", alias = "user_id")]
    pub uid: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, alias = "display_name")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub admin: Option<bool>,
}

impl User {
    /// The best available name for display purposes.
    pub fn label(&self) -> String {
        self.display_name
            .clone()
            .or_else(|| self.uid.clone())
            .or_else(|| self.email.clone())
            .unwrap_or_else(|| "(unknown)".to_string())
    }
}
