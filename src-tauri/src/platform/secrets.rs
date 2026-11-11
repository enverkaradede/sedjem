//! API key storage in the macOS Keychain. Keys never touch settings.json,
//! logs or IPC responses (the UI only learns whether one is set).

const SERVICE: &str = "com.sedjem.app";
const OPENAI_ACCOUNT: &str = "openai-compatible-api-key";
const MAX_KEY_BYTES: usize = 4_096;

#[cfg(target_os = "macos")]
mod imp {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    /// errSecItemNotFound
    const ITEM_NOT_FOUND: i32 = -25_300;

    pub fn get(service: &str, account: &str) -> Result<Option<String>, String> {
        match get_generic_password(service, account) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| "anahtar UTF-8 değil".to_owned()),
            Err(err) if err.code() == ITEM_NOT_FOUND => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }

    pub fn set(service: &str, account: &str, value: &str) -> Result<(), String> {
        set_generic_password(service, account, value.as_bytes()).map_err(|err| err.to_string())
    }

    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        match delete_generic_password(service, account) {
            Ok(()) => Ok(()),
            Err(err) if err.code() == ITEM_NOT_FOUND => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn get(_: &str, _: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    pub fn set(_: &str, _: &str, _: &str) -> Result<(), String> {
        Err("Anahtar zinciri yalnızca macOS'ta destekleniyor.".into())
    }
    pub fn delete(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

pub fn openai_api_key() -> Option<String> {
    match imp::get(SERVICE, OPENAI_ACCOUNT) {
        Ok(key) => key,
        Err(err) => {
            tracing::warn!(%err, "could not read API key from keychain");
            None
        }
    }
}

pub fn has_openai_api_key() -> bool {
    openai_api_key().is_some()
}

/// Stores the key, or deletes it when `None`/blank.
pub fn set_openai_api_key(key: Option<&str>) -> Result<(), String> {
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) if k.len() > MAX_KEY_BYTES => Err("Anahtar çok uzun.".into()),
        Some(k) if k.chars().any(char::is_control) => {
            Err("Anahtar geçersiz karakter içeriyor.".into())
        }
        Some(k) => imp::set(SERVICE, OPENAI_ACCOUNT, k),
        None => imp::delete(SERVICE, OPENAI_ACCOUNT),
    }
}
