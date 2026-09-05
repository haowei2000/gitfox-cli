//! Authentication endpoints.

use crate::client::GitFoxClient;
use crate::error::Result;
use crate::models::User;

pub struct AuthApi<'a> {
    client: &'a GitFoxClient,
}

impl<'a> AuthApi<'a> {
    pub(crate) fn new(client: &'a GitFoxClient) -> Self {
        Self { client }
    }

    /// `GET /api/v1/user` — also doubles as the token validity check used by
    /// `fx auth login` and `fx auth status`.
    pub async fn current_user(&self) -> Result<User> {
        self.client.get_json("/api/v1/user").await
    }
}
