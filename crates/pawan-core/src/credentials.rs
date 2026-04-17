//! Secure credential storage using OS-native keyring.
//!
//! This module provides a secure way to store and retrieve API keys
//! using the operating system's native credential store:
//! - Linux: libsecret or KWallet
//! - macOS: Keychain
//! - Windows: Credential Manager

use keyring::{Entry, Error};
use thiserror::Error;
use tracing::warn;

const SERVICE_NAME: &str = "pawan";
const USER: &str = "api_keys";

/// Errors that can occur during credential operations.
#[derive(Error, Debug)]
pub enum CredentialError {
    #[error("Failed to access credential store: {0}")]
    StoreError(String),

    #[error("Credential not found")]
    NotFound,

    #[error("Invalid credential data")]
    InvalidData,
}

impl From<Error> for CredentialError {
    fn from(err: Error) -> Self {
        match err {
            Error::PlatformFailure(e) => CredentialError::StoreError(format!("Platform error: {}", e)),
            Error::NoEntry => CredentialError::NotFound,
            _ => CredentialError::StoreError(format!("Credential store error: {}", err)),
        }
    }
}

/// Securely stores an API key in the OS-native credential store.
///
/// # Arguments
/// * `key_name` - The name of the key (e.g., "nvidia_api_key", "openai_api_key")
/// * `api_key` - The API key to store
///
/// # Returns
/// * `Ok(())` if the key was stored successfully
/// * `Err(CredentialError)` if storage failed
pub fn store_api_key(key_name: &str, api_key: &str) -> Result<(), CredentialError> {
    let entry = Entry::new(SERVICE_NAME, &format!("{}_{}", USER, key_name))?;
    entry.set_password(api_key)?;
    warn!("API key '{}' stored securely", key_name);
    Ok(())
}

/// Retrieves an API key from the OS-native credential store.
///
/// # Arguments
/// * `key_name` - The name of the key (e.g., "nvidia_api_key", "openai_api_key")
///
/// # Returns
/// * `Ok(Some(String))` if the key was found
/// * `Ok(None)` if the key was not found
/// * `Err(CredentialError)` if retrieval failed
pub fn get_api_key(key_name: &str) -> Result<Option<String>, CredentialError> {
    let entry = Entry::new(SERVICE_NAME, &format!("{}_{}", USER, key_name))?;
    
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Deletes an API key from the OS-native credential store.
///
/// # Arguments
/// * `key_name` - The name of the key to delete
///
/// # Returns
/// * `Ok(())` if the key was deleted successfully or didn't exist
/// * `Err(CredentialError)` if deletion failed
pub fn delete_api_key(key_name: &str) -> Result<(), CredentialError> {
    let entry = Entry::new(SERVICE_NAME, &format!("{}_{}", USER, key_name))?;
    
    match entry.delete_credential() {
        Ok(()) => {
            warn!("API key '{}' deleted from secure store", key_name);
            Ok(())
        }
        Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Checks if a secure credential store is available on this system.
///
/// # Returns
/// * `true` if a credential store is available
/// * `false` if no credential store is available
pub fn is_secure_store_available() -> bool {
    let test_entry = Entry::new(SERVICE_NAME, "test_check");
    test_entry.is_ok()
}

/// Convenience function for NVIDIA API key operations.
pub fn store_nvidia_api_key(key: &str) -> Result<(), CredentialError> {
    store_api_key("nvidia_api_key", key)
}

pub fn get_nvidia_api_key() -> Result<Option<String>, CredentialError> {
    get_api_key("nvidia_api_key")
}

pub fn delete_nvidia_api_key() -> Result<(), CredentialError> {
    delete_api_key("nvidia_api_key")
}

/// Convenience function for OpenAI API key operations.
pub fn store_openai_api_key(key: &str) -> Result<(), CredentialError> {
    store_api_key("openai_api_key", key)
}

pub fn get_openai_api_key() -> Result<Option<String>, CredentialError> {
    get_api_key("openai_api_key")
}

pub fn delete_openai_api_key() -> Result<(), CredentialError> {
    delete_api_key("openai_api_key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires a working credential store
    fn test_store_and_get_key() {
        let key_name = "test_key_12345";
        let test_key = "test_api_key_value";

        // Store the key
        store_api_key(key_name, test_key).expect("Failed to store key");

        // Retrieve the key
        let retrieved = get_api_key(key_name).expect("Failed to retrieve key");
        assert_eq!(retrieved, Some(test_key.to_string()));

        // Clean up
        delete_api_key(key_name).expect("Failed to delete key");
    }

    #[test]
    #[ignore] // Requires a working credential store
    fn test_get_nonexistent_key() {
        let key_name = "nonexistent_key_12345";
        
        // Delete to ensure clean state
        let _ = delete_api_key(key_name);

        // Try to retrieve
        let retrieved = get_api_key(key_name).expect("Failed to retrieve key");
        assert_eq!(retrieved, None);
    }

    #[test]
    #[ignore] // Requires a working credential store
    fn test_delete_key() {
        let key_name = "test_delete_key_12345";
        let test_key = "test_key_value";

        // Store and verify
        store_api_key(key_name, test_key).expect("Failed to store");
        assert!(get_api_key(key_name).expect("Failed to get") == Some(test_key.to_string()));

        // Delete and verify
        delete_api_key(key_name).expect("Failed to delete");
        assert_eq!(get_api_key(key_name).expect("Failed to get"), None);
    }
}
