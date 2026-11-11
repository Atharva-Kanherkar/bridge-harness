//! API keys for accounts on agents whose own login cannot be kept apart per
//! folder (Cursor keeps its CLI login in one global Keychain item). T3 Code
//! holds such keys as instance secrets; Bridge keeps them in the macOS
//! Keychain under its own service, one item per account, and never in the
//! settings record. Reads go through the same `bridged` helper as Bridge's
//! other Keychain reads, so an unsigned rebuild does not prompt. Saved API
//! keys (`api_keys`) use the same [`keychain`] store under their own service.

use crate::BridgeError;
use zeroize::Zeroizing;

const SERVICE: &str = "dev.bridge.deck.provider-accounts";
pub(crate) const MAX_KEY_BYTES: usize = 8 * 1024;

fn item(harness: &str, account: &str) -> String {
    format!("{harness}/{account}")
}

/// Save (or replace) an account's key.
pub fn save(harness: &str, account: &str, key: &str) -> Result<(), BridgeError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(BridgeError::Invalid("The API key cannot be empty".into()));
    }
    if key.len() > MAX_KEY_BYTES || key.chars().any(char::is_control) {
        return Err(BridgeError::Invalid("That does not look like an API key".into()));
    }
    keychain::save(SERVICE, &item(harness, account), key)
}

/// An account's key, or `None` when none was saved.
pub fn read(harness: &str, account: &str) -> Result<Option<Zeroizing<String>>, BridgeError> {
    keychain::read(SERVICE, &item(harness, account))
}

/// Forget an account's key. Missing is not an error.
pub fn delete(harness: &str, account: &str) -> Result<(), BridgeError> {
    keychain::delete(SERVICE, &item(harness, account))
}

#[cfg(all(target_os = "macos", not(test)))]
pub(crate) mod keychain {
    use super::{BridgeError, Zeroizing};

    pub fn save(service: &str, item: &str, key: &str) -> Result<(), BridgeError> {
        security_framework::passwords::set_generic_password(service, item, key.as_bytes())
            .map_err(|_| BridgeError::Invalid("Bridge could not save the key in the macOS Keychain".into()))
    }

    pub fn read(service: &str, item: &str) -> Result<Option<Zeroizing<String>>, BridgeError> {
        // The helper reports a missing item and a refused read alike; both
        // mean there is no key this launch can use.
        Ok(crate::provider_usage::credentials::keychain(service, Some(item))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|value| Zeroizing::new(value.trim().to_owned()))
            .filter(|value| !value.is_empty()))
    }

    pub fn delete(service: &str, item: &str) -> Result<(), BridgeError> {
        match security_framework::passwords::delete_generic_password(service, item) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => Ok(()),
            Err(_) => Err(BridgeError::Invalid("Bridge could not remove the key from the macOS Keychain".into())),
        }
    }
}

#[cfg(all(not(target_os = "macos"), not(test)))]
pub(crate) mod keychain {
    use super::{BridgeError, Zeroizing};

    const UNSUPPORTED: &str = "Saving a key needs the macOS Keychain, which this build does not have";

    pub fn save(_: &str, _: &str, _: &str) -> Result<(), BridgeError> {
        Err(BridgeError::Invalid(UNSUPPORTED.into()))
    }

    pub fn read(_: &str, _: &str) -> Result<Option<Zeroizing<String>>, BridgeError> {
        Ok(None)
    }

    pub fn delete(_: &str, _: &str) -> Result<(), BridgeError> {
        Ok(())
    }
}

/// Tests never touch the real Keychain.
#[cfg(test)]
pub(crate) mod keychain {
    use super::{BridgeError, Zeroizing};
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    fn items() -> &'static Mutex<HashMap<(String, String), String>> {
        static ITEMS: OnceLock<Mutex<HashMap<(String, String), String>>> = OnceLock::new();
        ITEMS.get_or_init(Default::default)
    }

    pub fn save(service: &str, item: &str, key: &str) -> Result<(), BridgeError> {
        items().lock().unwrap().insert((service.to_owned(), item.to_owned()), key.to_owned());
        Ok(())
    }

    pub fn read(service: &str, item: &str) -> Result<Option<Zeroizing<String>>, BridgeError> {
        Ok(items().lock().unwrap().get(&(service.to_owned(), item.to_owned())).cloned().map(Zeroizing::new))
    }

    pub fn delete(service: &str, item: &str) -> Result<(), BridgeError> {
        items().lock().unwrap().remove(&(service.to_owned(), item.to_owned()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_keys_round_trip_and_removal_deletes_them() {
        save("cursor", "work-roundtrip", "  key_live_123  ").unwrap();
        assert_eq!(read("cursor", "work-roundtrip").unwrap().as_deref().map(String::as_str), Some("key_live_123"));
        assert_eq!(read("cursor", "personal-roundtrip").unwrap(), None, "keys are per account");
        delete("cursor", "work-roundtrip").unwrap();
        assert_eq!(read("cursor", "work-roundtrip").unwrap(), None);
        delete("cursor", "work-roundtrip").unwrap();
        assert!(save("cursor", "x", "   ").unwrap_err().to_string().contains("cannot be empty"));
        assert!(save("cursor", "x", "a\nb").is_err());
    }
}
