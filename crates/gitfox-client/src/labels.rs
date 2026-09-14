//! Label endpoints.
//!
//! Verified against the GitFox API v1.3.0 OpenAPI document:
//!
//! | Operation | Endpoint |
//! |---|---|
//! | list    | `GET /api/v1/repos/{repo_ref}/labels` (`inherited` adds space labels) |
//! | define  | `POST /api/v1/repos/{repo_ref}/labels` |
//! | update  | `PATCH /api/v1/repos/{repo_ref}/labels/{key}` |
//! | delete  | `DELETE /api/v1/repos/{repo_ref}/labels/{key}` |
//! | values  | `GET /api/v1/repos/{repo_ref}/labels/{key}/values` |

use crate::client::{GitFoxClient, Method, Query};
use crate::error::Result;
use crate::models::{Label, LabelInput, LabelValue, RepoRef};
use crate::pipeline::urlencode;

pub struct LabelsApi<'a> {
    client: &'a GitFoxClient,
}

impl<'a> LabelsApi<'a> {
    pub(crate) fn new(client: &'a GitFoxClient) -> Self {
        Self { client }
    }

    fn path(repo: &RepoRef) -> String {
        format!("/api/v1/repos/{}/labels", repo.encoded())
    }

    /// `GET …/labels`. With `inherited`, labels defined on the enclosing
    /// spaces come too — the full set a pull request can be given.
    pub async fn list(
        &self,
        repo: &RepoRef,
        query: Option<&str>,
        inherited: bool,
        page: u32,
        limit: u32,
    ) -> Result<Vec<Label>> {
        let mut q = Query::new();
        q.push_opt("query", query)
            .push("inherited", inherited)
            .push("page", page)
            .push("limit", limit);
        self.client.get_json(&q.apply(&Self::path(repo))).await
    }

    /// `POST …/labels`
    pub async fn define(&self, repo: &RepoRef, input: &LabelInput) -> Result<Label> {
        let body = serde_json::to_value(input).map_err(|e| crate::Error::Decode(e.to_string()))?;
        self.client
            .request(Method::POST, &Self::path(repo), Some(&body), &[])
            .await?
            .deserialize()
    }

    /// `PATCH …/labels/{key}`
    pub async fn update(&self, repo: &RepoRef, key: &str, input: &LabelInput) -> Result<Label> {
        let body = serde_json::to_value(input).map_err(|e| crate::Error::Decode(e.to_string()))?;
        self.client
            .request(
                Method::PATCH,
                &format!("{}/{}", Self::path(repo), urlencode(key)),
                Some(&body),
                &[],
            )
            .await?
            .deserialize()
    }

    /// `DELETE …/labels/{key}`
    pub async fn delete(&self, repo: &RepoRef, key: &str) -> Result<()> {
        self.client
            .request(
                Method::DELETE,
                &format!("{}/{}", Self::path(repo), urlencode(key)),
                None,
                &[],
            )
            .await
            .map(|_| ())
    }

    /// `GET …/labels/{key}/values`
    pub async fn values(&self, repo: &RepoRef, key: &str) -> Result<Vec<LabelValue>> {
        self.client
            .get_json(&format!("{}/{}/values", Self::path(repo), urlencode(key)))
            .await
    }
}
