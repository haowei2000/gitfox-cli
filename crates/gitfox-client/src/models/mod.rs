//! Domain models owned by the client.
//!
//! These are deliberately *not* the raw GitFox API DTOs: keeping a translation
//! layer means an upstream API change does not have to leak into the CLI's
//! stable JSON schema.

mod repo_ref;
mod user;

pub use repo_ref::RepoRef;
pub use user::User;
