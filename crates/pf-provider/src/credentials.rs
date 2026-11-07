//! The credential store (FR-PRV-06, KQ-05): the OS credential store through `keyring` (#38),
//! target `pocket-forge/<provider>`. There is no file fallback: if the store is unavailable,
//! the provider is "not configured".

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::secret::Credentials;

/// The credential target for a provider (`pocket-forge/scenario`).
pub fn target_for(provider: &str) -> String {
    format!("pocket-forge/{provider}")
}

/// Credential store failure. The message never contains the stored values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the OS credential store is unavailable: {0}")]
pub struct StoreUnavailable(pub String);

/// Where credentials live. One entry per provider.
pub trait CredentialStore: Send + Sync {
    /// The stored credentials, or `None` if nothing is stored.
    fn load(&self) -> Result<Option<Credentials>, StoreUnavailable>;
    fn save(&self, credentials: &Credentials) -> Result<(), StoreUnavailable>;
    /// Removes the entry. Idempotent.
    fn clear(&self) -> Result<(), StoreUnavailable>;
}

/// What one entry holds (the password field of the OS entry).
#[derive(Serialize, Deserialize)]
struct Stored {
    key: String,
    secret: String,
}

fn encode(c: &Credentials) -> String {
    serde_json::json!({"key": c.key.expose(), "secret": c.secret.expose()}).to_string()
}

fn decode(raw: &str) -> Option<Credentials> {
    let s: Stored = serde_json::from_str(raw).ok()?;
    (!s.key.is_empty() && !s.secret.is_empty()).then(|| Credentials::new(s.key, s.secret))
}

/// The OS credential store (Windows Credential Manager, macOS Keychain, Linux keyutils).
#[derive(Debug, Clone)]
pub struct KeyringStore {
    target: String,
    service: String,
    user: String,
}

impl KeyringStore {
    /// The store for a provider (`scenario` → target `pocket-forge/scenario`).
    pub fn for_provider(provider: &str) -> Self {
        Self::with_target(&target_for(provider), provider)
    }

    /// A store with an explicit target (integration tests use a throwaway one).
    pub fn with_target(target: &str, user: &str) -> Self {
        KeyringStore {
            target: target.to_owned(),
            service: "pocket-forge".to_owned(),
            user: user.to_owned(),
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    fn entry(&self) -> Result<keyring::Entry, StoreUnavailable> {
        keyring::Entry::new_with_target(&self.target, &self.service, &self.user)
            .map_err(|e| unavailable(&e))
    }
}

/// keyring errors can carry the attempted value in a few variants; only the kind is kept.
fn unavailable(e: &keyring::Error) -> StoreUnavailable {
    let kind = match e {
        keyring::Error::PlatformFailure(_) => "platform failure",
        keyring::Error::NoStorageAccess(_) => "no storage access",
        keyring::Error::NoEntry => "no entry",
        keyring::Error::BadEncoding(_) => "bad encoding",
        keyring::Error::TooLong(_, _) => "value too long",
        keyring::Error::Invalid(_, _) => "invalid attribute",
        keyring::Error::Ambiguous(_) => "ambiguous entry",
        _ => "unknown error",
    };
    StoreUnavailable(kind.to_owned())
}

impl CredentialStore for KeyringStore {
    fn load(&self) -> Result<Option<Credentials>, StoreUnavailable> {
        match self.entry()?.get_password() {
            Ok(raw) => {
                let c = decode(&raw);
                if c.is_none() {
                    tracing::warn!(target_name = %self.target, "credential entry is unreadable; treating it as empty");
                }
                Ok(c)
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(unavailable(&e)),
        }
    }

    fn save(&self, credentials: &Credentials) -> Result<(), StoreUnavailable> {
        self.entry()?
            .set_password(&encode(credentials))
            .map_err(|e| unavailable(&e))
    }

    fn clear(&self) -> Result<(), StoreUnavailable> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(unavailable(&e)),
        }
    }
}

/// An in-memory store for tests and fixtures. `unavailable` simulates a broken OS store.
#[derive(Debug, Default)]
pub struct MemoryStore {
    raw: Mutex<Option<String>>,
    pub unavailable: AtomicBool,
}

impl MemoryStore {
    pub fn with(credentials: &Credentials) -> Self {
        let s = MemoryStore::default();
        *s.lock() = Some(encode(credentials));
        s
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.raw.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn check(&self) -> Result<(), StoreUnavailable> {
        if self.unavailable.load(Ordering::Relaxed) {
            Err(StoreUnavailable("simulated".into()))
        } else {
            Ok(())
        }
    }
}

impl CredentialStore for MemoryStore {
    fn load(&self) -> Result<Option<Credentials>, StoreUnavailable> {
        self.check()?;
        Ok(self.lock().as_deref().and_then(decode))
    }

    fn save(&self, credentials: &Credentials) -> Result<(), StoreUnavailable> {
        self.check()?;
        *self.lock() = Some(encode(credentials));
        Ok(())
    }

    fn clear(&self) -> Result<(), StoreUnavailable> {
        self.check()?;
        *self.lock() = None;
        Ok(())
    }
}

impl<T: CredentialStore + ?Sized> CredentialStore for std::sync::Arc<T> {
    fn load(&self) -> Result<Option<Credentials>, StoreUnavailable> {
        (**self).load()
    }
    fn save(&self, credentials: &Credentials) -> Result<(), StoreUnavailable> {
        (**self).save(credentials)
    }
    fn clear(&self) -> Result<(), StoreUnavailable> {
        (**self).clear()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prv_06_target_is_pocket_forge_slash_provider() {
        assert_eq!(target_for("scenario"), "pocket-forge/scenario");
        assert_eq!(
            KeyringStore::for_provider("scenario").target(),
            "pocket-forge/scenario"
        );
    }

    #[test]
    fn fr_prv_06_memory_store_round_trip_clear_and_unavailable() {
        let s = MemoryStore::default();
        assert_eq!(s.load().unwrap(), None);
        let c = Credentials::new("k1234", "s");
        s.save(&c).unwrap();
        assert_eq!(s.load().unwrap(), Some(c));
        s.clear().unwrap();
        s.clear().unwrap();
        assert_eq!(s.load().unwrap(), None);
        s.unavailable.store(true, Ordering::Relaxed);
        assert!(s.load().is_err());
    }

    /// The real OS store with a throwaway target (spec §13 integration; CI runs it on Windows).
    #[cfg(windows)]
    #[test]
    fn fr_prv_06_os_credential_store_round_trip_with_a_throwaway_target() {
        let target = format!("pocket-forge-test/{}", std::process::id());
        let store = KeyringStore::with_target(&target, "test");
        let c = Credentials::new("throwaway-key-0042", "throwaway-secret");
        store.save(&c).unwrap();
        assert_eq!(store.load().unwrap(), Some(c));
        store.clear().unwrap();
        assert_eq!(store.load().unwrap(), None);
        store.clear().unwrap();
    }
}
