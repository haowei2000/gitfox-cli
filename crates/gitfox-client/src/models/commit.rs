use serde::{Deserialize, Serialize};

/// A commit, as GitFox reports it in commit and pull request listings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Commit {
    #[serde(default)]
    pub sha: String,
    /// The subject line.
    #[serde(default)]
    pub title: String,
    /// The whole message, subject included.
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub parent_shas: Vec<String>,
    #[serde(default)]
    pub author: Option<Signature>,
    #[serde(default)]
    pub committer: Option<Signature>,
}

impl Commit {
    /// The message with its subject line removed.
    pub fn body(&self) -> String {
        match self.message.split_once('\n') {
            Some((_, rest)) => rest.trim().to_string(),
            None => String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Signature {
    #[serde(default)]
    pub identity: Identity,
    /// RFC 3339, as GitFox sends it.
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Identity {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_the_message_without_its_subject() {
        let commit = Commit {
            message: "feat: add OAuth\n\nCloses #4\n".into(),
            ..Default::default()
        };
        assert_eq!(commit.body(), "Closes #4");
        let subject_only = Commit {
            message: "fix: typo".into(),
            ..Default::default()
        };
        assert_eq!(subject_only.body(), "");
    }
}
