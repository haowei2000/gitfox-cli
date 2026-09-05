use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

/// A reference to a repository, e.g. `ai/backend`.
///
/// GitFox spaces can be nested (`org/team/repo`), so everything before the last
/// segment is treated as the space path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRef {
    space: String,
    name: String,
}

impl RepoRef {
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim().trim_matches('/');
        let Some((space, name)) = trimmed.rsplit_once('/') else {
            return Err(Error::InvalidUrl(format!(
                "repository reference must be `space/name`, got `{input}`"
            )));
        };
        if space.is_empty() || name.is_empty() || space.split('/').any(str::is_empty) {
            return Err(Error::InvalidUrl(format!(
                "repository reference must be `space/name`, got `{input}`"
            )));
        }
        Ok(Self {
            space: space.to_string(),
            name: name.to_string(),
        })
    }

    /// The space path (everything before the repository name).
    pub fn space(&self) -> &str {
        &self.space
    }

    /// The repository name (the last segment).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `space/name`, as a user would type it.
    pub fn full(&self) -> String {
        format!("{}/{}", self.space, self.name)
    }

    /// `space%2Fname`, as GitFox expects it inside a URL path segment.
    pub fn encoded(&self) -> String {
        self.full().replace('/', "%2F")
    }
}

impl FromStr for RepoRef {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl fmt::Display for RepoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.full())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_reference() {
        let r = RepoRef::parse("ai/backend").unwrap();
        assert_eq!(r.space(), "ai");
        assert_eq!(r.name(), "backend");
        assert_eq!(r.full(), "ai/backend");
        assert_eq!(r.encoded(), "ai%2Fbackend");
    }

    #[test]
    fn parses_nested_space() {
        let r = RepoRef::parse("org/team/backend").unwrap();
        assert_eq!(r.space(), "org/team");
        assert_eq!(r.name(), "backend");
        assert_eq!(r.encoded(), "org%2Fteam%2Fbackend");
    }

    #[test]
    fn tolerates_surrounding_slashes_and_space() {
        assert_eq!(
            RepoRef::parse("  /ai/backend/ ").unwrap().full(),
            "ai/backend"
        );
    }

    #[test]
    fn rejects_references_without_a_space() {
        for bad in ["backend", "", "/", "ai//backend", "ai/"] {
            assert!(
                RepoRef::parse(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }
}
