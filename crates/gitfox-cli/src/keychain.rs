//! Token storage in the OS keychain (Keychain on macOS, Secret Service on
//! Linux, Credential Manager on Windows).
//!
//! Strictly optional: CI and agents pass `GITFOX_TOKEN` and never touch this.
//! A missing or unavailable keychain is therefore never a hard error on read.

use keyring::Entry;

use crate::config::{KEYRING_SERVICE, Secret};
use crate::error::{CliError, ErrorCode, Result};

fn entry(host_key: &str) -> Result<Entry> {
    Entry::new(KEYRING_SERVICE, host_key)
        .map_err(|e| CliError::config(format!("could not open the keychain: {e}")))
}

/// Look up a stored token. Returns `None` when there is no entry *and* when the
/// keychain cannot be reached at all — a headless machine should fall through
/// to the environment, not fail.
pub fn get(host_key: &str) -> Option<Secret> {
    match entry(host_key).ok()?.get_password() {
        Ok(token) if !token.trim().is_empty() => Some(Secret::new(token)),
        Ok(_) => None,
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            tracing::debug!(host = host_key, error = %e, "keychain lookup failed");
            None
        }
    }
}

pub fn set(host_key: &str, token: &str) -> Result<()> {
    entry(host_key)?.set_password(token).map_err(|e| {
        CliError::new(
            ErrorCode::ConfigError,
            format!("could not store the token in the keychain: {e}"),
        )
    })
}

/// Returns whether an entry was actually removed.
pub fn delete(host_key: &str) -> Result<bool> {
    match entry(host_key)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(CliError::new(
            ErrorCode::ConfigError,
            format!("could not remove the token from the keychain: {e}"),
        )),
    }
}
