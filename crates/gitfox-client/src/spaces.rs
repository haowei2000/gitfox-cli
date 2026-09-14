//! Spaces, and the things that live in them: secrets. Also the caller's own
//! account resources — SSH keys — and the instance's system configuration.
//!
//! Verified against the GitFox API v1.3.0 OpenAPI document:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | memberships    | `GET /api/v1/user/memberships` |
//! | root spaces    | `GET /api/v1/spaces` |
//! | one space      | `GET /api/v1/spaces/{space_ref}` |
//! | list secrets   | `GET /api/v1/spaces/{space_ref}/secrets` |
//! | get secret     | `GET /api/v1/secrets/{secret_ref}` |
//! | create secret  | `POST /api/v1/secrets` |
//! | update secret  | `PATCH /api/v1/secrets/{secret_ref}` |
//! | delete secret  | `DELETE /api/v1/secrets/{secret_ref}` |
//! | SSH keys       | `GET` / `POST /api/v1/user/keys`, `DELETE …/keys/{identifier}` |
//! | system config  | `GET /api/v1/system/config` |
//!
//! `{secret_ref}` is `space/identifier` with the slashes encoded, exactly like
//! a repository reference — checked against a live instance, where the
//! unencoded form answers 404.

use serde_json::json;

use crate::client::{GitFoxClient, Method, Query};
use crate::error::Result;
use crate::models::{Membership, PublicKey, Space, SpaceSecret, SystemConfig};
use crate::pipeline::urlencode;

pub struct SpacesApi<'a> {
    client: &'a GitFoxClient,
}

impl<'a> SpacesApi<'a> {
    pub(crate) fn new(client: &'a GitFoxClient) -> Self {
        Self { client }
    }

    /// `GET /api/v1/user/memberships` — the spaces the caller belongs to.
    pub async fn memberships(&self, page: u32, limit: u32) -> Result<Vec<Membership>> {
        let mut q = Query::new();
        q.push("sort", "identifier")
            .push("order", "asc")
            .push("page", page)
            .push("limit", limit);
        self.client
            .get_json(&q.apply("/api/v1/user/memberships"))
            .await
    }

    /// `GET /api/v1/spaces/{space_ref}`
    pub async fn get(&self, space: &str) -> Result<Space> {
        self.client
            .get_json(&format!("/api/v1/spaces/{}", encode_ref(space)))
            .await
    }

    /// `GET /api/v1/spaces/{space_ref}/secrets`
    pub async fn secrets(
        &self,
        space: &str,
        query: Option<&str>,
        page: u32,
        limit: u32,
    ) -> Result<Vec<SpaceSecret>> {
        let mut q = Query::new();
        q.push_opt("query", query)
            .push("page", page)
            .push("limit", limit);
        self.client
            .get_json(&q.apply(&format!("/api/v1/spaces/{}/secrets", encode_ref(space))))
            .await
    }

    /// `GET /api/v1/secrets/{secret_ref}`
    pub async fn secret(&self, space: &str, identifier: &str) -> Result<SpaceSecret> {
        self.client.get_json(&secret_path(space, identifier)).await
    }

    /// `POST /api/v1/secrets`
    pub async fn create_secret(
        &self,
        space: &str,
        identifier: &str,
        data: &str,
        description: Option<&str>,
    ) -> Result<SpaceSecret> {
        let mut body = json!({ "space_ref": space, "identifier": identifier, "data": data });
        if let Some(description) = description {
            body["description"] = json!(description);
        }
        self.client
            .request(Method::POST, "/api/v1/secrets", Some(&body), &[])
            .await?
            .deserialize()
    }

    /// `PATCH /api/v1/secrets/{secret_ref}`
    pub async fn update_secret(
        &self,
        space: &str,
        identifier: &str,
        data: &str,
        description: Option<&str>,
    ) -> Result<SpaceSecret> {
        let mut body = json!({ "data": data });
        if let Some(description) = description {
            body["description"] = json!(description);
        }
        self.client
            .request(
                Method::PATCH,
                &secret_path(space, identifier),
                Some(&body),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `DELETE /api/v1/secrets/{secret_ref}`
    pub async fn delete_secret(&self, space: &str, identifier: &str) -> Result<()> {
        self.client
            .request(Method::DELETE, &secret_path(space, identifier), None, &[])
            .await
            .map(|_| ())
    }

    /// `GET /api/v1/user/keys`
    pub async fn public_keys(&self, page: u32, limit: u32) -> Result<Vec<PublicKey>> {
        let mut q = Query::new();
        q.push("sort", "created")
            .push("order", "asc")
            .push("page", page)
            .push("limit", limit);
        self.client.get_json(&q.apply("/api/v1/user/keys")).await
    }

    /// `POST /api/v1/user/keys`
    pub async fn add_public_key(&self, identifier: &str, content: &str) -> Result<PublicKey> {
        self.client
            .request(
                Method::POST,
                "/api/v1/user/keys",
                Some(&json!({ "identifier": identifier, "content": content, "usage": "auth" })),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `DELETE /api/v1/user/keys/{public_key_identifier}`
    pub async fn delete_public_key(&self, identifier: &str) -> Result<()> {
        self.client
            .request(
                Method::DELETE,
                &format!("/api/v1/user/keys/{}", urlencode(identifier)),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `GET /api/v1/system/config`
    pub async fn system_config(&self) -> Result<SystemConfig> {
        self.client.get_json("/api/v1/system/config").await
    }
}

/// `org/team` → `org%2Fteam`, as a space or secret reference travels in a path.
pub fn encode_ref(reference: &str) -> String {
    reference
        .trim_matches('/')
        .split('/')
        .map(urlencode)
        .collect::<Vec<_>>()
        .join("%2F")
}

fn secret_path(space: &str, identifier: &str) -> String {
    format!(
        "/api/v1/secrets/{}%2F{}",
        encode_ref(space),
        urlencode(identifier)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_reference_encodes_the_space_and_the_name_as_one_segment() {
        assert_eq!(
            secret_path("ai-repos", "deploy-host"),
            "/api/v1/secrets/ai-repos%2Fdeploy-host"
        );
        assert_eq!(
            secret_path("org/team", "TOKEN"),
            "/api/v1/secrets/org%2Fteam%2FTOKEN"
        );
    }
}
