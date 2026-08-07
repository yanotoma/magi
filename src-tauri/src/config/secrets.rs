//! API keys, kept out of `config.toml` and out of the tests.
//!
//! Two reasons this is a trait rather than direct `keyring` calls. CI has no
//! keychain, so a test that reached for the real one would fail on the runner;
//! and a test that reached for the developer's would leave entries behind in it.
//! Everything here is exercised through [`InMemoryStore`].

use std::collections::HashMap;
use std::sync::Mutex;

/// The keychain service name. Entries are keyed by provider id.
const SERVICE: &str = "dev.magi.app";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("the OS keychain refused the request: {0}")]
    Keychain(String),

    #[error("the secret store is unusable: {0}")]
    Poisoned(String),
}

/// A string that does not print itself.
///
/// Provider state ends up in logs and in pasted bug reports. Deriving `Debug`
/// on a struct holding a bare `String` key would undo the whole point of keeping
/// keys out of `config.toml`, and it would do so silently — nobody reviews a
/// log line for a secret that was never supposed to be there.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Deliberately verbose. Reading a secret should be visible at the call site.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// A fingerprint that identifies a stored key without revealing it.
///
/// Computed here rather than in the frontend on purpose. Masking in the UI
/// would mean the real key had already crossed into the webview, the DOM and
/// devtools, and the asterisks would be decoration over a secret that is
/// already exposed. Only this string ever leaves the backend.
///
/// Short keys are masked entirely. Showing the ends of an eight-character
/// secret reveals most of it, and the point is to tell two keys apart, not to
/// display one.
pub fn fingerprint(secret: &str) -> String {
    const REVEAL_ENDS_ABOVE: usize = 20;
    const HEAD: usize = 4;
    const TAIL: usize = 4;

    let characters: Vec<char> = secret.chars().collect();

    if characters.len() <= REVEAL_ENDS_ABOVE {
        return "•".repeat(characters.len().clamp(4, 12));
    }

    let head: String = characters.iter().take(HEAD).collect();
    let tail: String = characters.iter().skip(characters.len() - TAIL).collect();
    format!("{head}…{tail}")
}

pub trait SecretStore: Send + Sync {
    /// `Ok(None)` for a provider with no key — the normal case for Ollama and
    /// LM Studio, not a failure.
    fn get(&self, provider_id: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, provider_id: &str, secret: &str) -> Result<(), SecretError>;
    /// Succeeds whether or not a key was there, so removing a provider does not
    /// have to check first.
    fn delete(&self, provider_id: &str) -> Result<(), SecretError>;
}

/// The real store. Never constructed in a test.
pub struct KeyringStore;

impl KeyringStore {
    fn entry(provider_id: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, provider_id).map_err(|e| SecretError::Keychain(e.to_string()))
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, provider_id: &str) -> Result<Option<String>, SecretError> {
        match Self::entry(provider_id)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Keychain(e.to_string())),
        }
    }

    fn set(&self, provider_id: &str, secret: &str) -> Result<(), SecretError> {
        Self::entry(provider_id)?
            .set_password(secret)
            .map_err(|e| SecretError::Keychain(e.to_string()))
    }

    fn delete(&self, provider_id: &str) -> Result<(), SecretError> {
        match Self::entry(provider_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Keychain(e.to_string())),
        }
    }
}

/// The test double. Also useful for a `--no-keychain` mode later.
#[derive(Default)]
pub struct InMemoryStore {
    entries: Mutex<HashMap<String, String>>,
}

impl InMemoryStore {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, String>>, SecretError> {
        self.entries
            .lock()
            .map_err(|e| SecretError::Poisoned(e.to_string()))
    }
}

impl SecretStore for InMemoryStore {
    fn get(&self, provider_id: &str) -> Result<Option<String>, SecretError> {
        Ok(self.lock()?.get(provider_id).cloned())
    }

    fn set(&self, provider_id: &str, secret: &str) -> Result<(), SecretError> {
        self.lock()?
            .insert(provider_id.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, provider_id: &str) -> Result<(), SecretError> {
        self.lock()?.remove(provider_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_reads_back_a_key() {
        let store = InMemoryStore::default();
        store.set("openai", "sk-test").expect("set must succeed");
        assert_eq!(store.get("openai").unwrap(), Some("sk-test".to_string()));
    }

    #[test]
    fn a_missing_key_is_none_rather_than_an_error() {
        // A provider with no key is the normal case — Ollama and LM Studio need
        // none. Returning Err here would make the happy path look like a failure
        // and push callers into treating real errors as routine.
        let store = InMemoryStore::default();
        assert_eq!(store.get("ollama").unwrap(), None);
    }

    #[test]
    fn setting_twice_replaces_rather_than_appends() {
        let store = InMemoryStore::default();
        store.set("openai", "old").unwrap();
        store.set("openai", "new").unwrap();
        assert_eq!(store.get("openai").unwrap(), Some("new".to_string()));
    }

    #[test]
    fn deleting_removes_the_key() {
        let store = InMemoryStore::default();
        store.set("openai", "sk-test").unwrap();
        store.delete("openai").unwrap();
        assert_eq!(store.get("openai").unwrap(), None);
    }

    #[test]
    fn deleting_a_key_that_is_not_there_succeeds() {
        // Removing a provider should not have to check whether it ever had a
        // key. Making this an error would force every caller to special-case it.
        let store = InMemoryStore::default();
        assert!(store.delete("never-existed").is_ok());
    }

    #[test]
    fn providers_do_not_share_secrets() {
        let store = InMemoryStore::default();
        store.set("a", "key-a").unwrap();
        store.set("b", "key-b").unwrap();
        assert_eq!(store.get("a").unwrap(), Some("key-a".to_string()));
        assert_eq!(store.get("b").unwrap(), Some("key-b".to_string()));
    }

    #[test]
    fn a_long_key_shows_both_ends_so_two_keys_can_be_told_apart() {
        let hint = fingerprint("sk-proj-abcdefghijklmnopqrstuvwxyz-4f2a");
        assert_eq!(hint, "sk-p…4f2a");
    }

    #[test]
    fn a_short_key_is_masked_entirely() {
        // Showing the ends of an eight-character secret reveals most of it. The
        // job is to distinguish two keys, not to display one.
        let hint = fingerprint("abc12345");
        assert!(!hint.contains('a'), "leaked the start: {hint}");
        assert!(!hint.contains('5'), "leaked the end: {hint}");
    }

    #[test]
    fn the_fingerprint_does_not_reveal_the_length_of_a_long_key() {
        // A fixed shape avoids turning the hint into a length oracle.
        let short_ish = fingerprint(&"x".repeat(30));
        let very_long = fingerprint(&"x".repeat(300));
        assert_eq!(short_ish.chars().count(), very_long.chars().count());
    }

    #[test]
    fn a_secret_is_not_printed_by_debug() {
        // Config and provider state get logged and pasted into bug reports. A
        // secret that renders in Debug output defeats keeping it out of the
        // config file in the first place.
        let secret = Secret::new("sk-do-not-print".to_string());
        let rendered = format!("{secret:?}");
        assert!(
            !rendered.contains("sk-do-not-print"),
            "Debug leaked the secret: {rendered}"
        );
        assert_eq!(secret.expose(), "sk-do-not-print");
    }
}
