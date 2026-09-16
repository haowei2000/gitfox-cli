use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Principal;

/// A protection rule on a repository.
///
/// `pattern` and `definition` stay JSON: their shape depends on the rule type
/// and grows with every GitFox release, and gf only ever displays them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub description: Option<String>,
    /// `branch` today.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// `active`, `disabled` or `monitor`.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub pattern: Value,
    #[serde(default)]
    pub definition: Value,
    #[serde(default)]
    pub buildin: Option<bool>,
    #[serde(default)]
    pub created_by: Option<Principal>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
}

impl Rule {
    /// The branch patterns the rule includes, `default` meaning the default
    /// branch.
    pub fn targets(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.pattern.get("default").and_then(Value::as_bool) == Some(true) {
            out.push("default branch".to_string());
        }
        if let Some(include) = self.pattern.get("include").and_then(Value::as_array) {
            out.extend(include.iter().filter_map(Value::as_str).map(str::to_string));
        }
        out
    }
}
