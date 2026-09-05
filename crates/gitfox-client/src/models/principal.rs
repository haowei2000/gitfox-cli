use serde::{Deserialize, Serialize};

/// A user or service account, as GitFox reports it inside other resources.
///
/// Every field is optional: which ones an instance populates varies, and no
/// command should fail because a display name was absent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Principal {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

impl Principal {
    /// The most useful name available, preferring the login over the full name
    /// because that is what a caller would type back into `--author`.
    pub fn label(&self) -> String {
        self.uid
            .clone()
            .or_else(|| self.display_name.clone())
            .or_else(|| self.email.clone())
            .unwrap_or_else(|| "(unknown)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_prefers_the_login_then_the_display_name() {
        let full = Principal {
            uid: Some("whw".into()),
            display_name: Some("Haowei".into()),
            ..Default::default()
        };
        assert_eq!(full.label(), "whw");

        let named = Principal {
            display_name: Some("Haowei".into()),
            ..Default::default()
        };
        assert_eq!(named.label(), "Haowei");
        assert_eq!(Principal::default().label(), "(unknown)");
    }
}
