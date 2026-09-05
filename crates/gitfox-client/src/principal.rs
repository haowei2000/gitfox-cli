//! Principal (user / service account) lookup.

use crate::client::{GitFoxClient, Query};
use crate::error::Result;
use crate::models::Principal;

pub struct PrincipalsApi<'a> {
    client: &'a GitFoxClient,
}

impl<'a> PrincipalsApi<'a> {
    pub(crate) fn new(client: &'a GitFoxClient) -> Self {
        Self { client }
    }

    /// `GET /api/v1/principals?query=…&type=user`
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<Principal>> {
        let mut q = Query::new();
        q.push("query", query)
            .push("type", "user")
            .push("limit", limit);
        self.client.get_json(&q.apply("/api/v1/principals")).await
    }

    /// Resolve a login to a principal id.
    ///
    /// The pull request filters take numeric ids, but nobody knows their
    /// colleagues by id — so `--author whw` costs one extra lookup. An exact
    /// `uid` match wins over a fuzzy one so a substring hit cannot silently
    /// filter by the wrong person.
    pub async fn find_by_login(&self, login: &str) -> Result<Option<Principal>> {
        let candidates = self.search(login, 20).await?;
        let wanted = login.trim().to_ascii_lowercase();
        Ok(candidates
            .iter()
            .find(|p| {
                p.uid
                    .as_deref()
                    .is_some_and(|uid| uid.eq_ignore_ascii_case(&wanted))
            })
            .or_else(|| candidates.first())
            .cloned())
    }
}
