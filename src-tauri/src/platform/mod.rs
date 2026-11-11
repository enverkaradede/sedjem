//! OS integration behind a small portable surface. macOS is the supported
//! platform; other targets compile with explicit "unsupported" errors.

pub mod secrets;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MicrophoneStatus {
    Granted,
    Denied,
    Restricted,
    NotDetermined,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrivacyPane {
    Accessibility,
    Microphone,
}

/// Splits text into UTF-16 chunks of at most `max_units`, never separating a
/// surrogate pair (a lone surrogate would be typed as a replacement glyph).
pub fn utf16_chunks(text: &str, max_units: usize) -> Vec<Vec<u16>> {
    let max_units = max_units.max(2);
    let mut chunks = Vec::new();
    let mut current: Vec<u16> = Vec::with_capacity(max_units);
    let mut buf = [0u16; 2];
    for ch in text.chars() {
        let units = ch.encode_utf16(&mut buf);
        if current.len() + units.len() > max_units {
            chunks.push(std::mem::take(&mut current));
        }
        current.extend_from_slice(units);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{
    accessibility_trusted, copy_text, microphone_status, open_privacy_pane, paste_text, type_text,
};

#[cfg(not(target_os = "macos"))]
mod unsupported {
    use super::{MicrophoneStatus, PrivacyPane};

    pub fn paste_text(
        _text: &str,
        _press_enter: bool,
        _restore_clipboard: bool,
    ) -> Result<(), String> {
        Err("Metin ekleme yalnızca macOS'ta destekleniyor.".into())
    }
    pub fn type_text(_text: &str, _press_enter: bool) -> Result<(), String> {
        Err("Metin ekleme yalnızca macOS'ta destekleniyor.".into())
    }
    pub fn copy_text(_text: &str) -> Result<(), String> {
        Err("Pano yalnızca macOS'ta destekleniyor.".into())
    }
    pub fn accessibility_trusted() -> bool {
        false
    }
    pub fn microphone_status() -> MicrophoneStatus {
        MicrophoneStatus::Unknown
    }
    pub fn open_privacy_pane(_pane: PrivacyPane) -> Result<(), String> {
        Err("Yalnızca macOS'ta destekleniyor.".into())
    }
}
#[cfg(not(target_os = "macos"))]
pub use unsupported::{
    accessibility_trusted, copy_text, microphone_status, open_privacy_pane, paste_text, type_text,
};

#[cfg(test)]
mod tests {
    use super::utf16_chunks;

    #[test]
    fn chunks_respect_the_unit_limit() {
        let chunks = utf16_chunks("çok güzel bir Türkçe cümle yazıyorum", 20);
        assert!(chunks.iter().all(|c| c.len() <= 20));
        let joined: Vec<u16> = chunks.concat();
        assert_eq!(
            String::from_utf16(&joined).unwrap(),
            "çok güzel bir Türkçe cümle yazıyorum"
        );
    }

    #[test]
    fn surrogate_pairs_are_never_split() {
        // 19 ASCII units then an emoji (2 units): the emoji must move to chunk 2.
        let text = format!("{}😀", "a".repeat(19));
        let chunks = utf16_chunks(&text, 20);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 19);
        assert_eq!(String::from_utf16(&chunks[1]).unwrap(), "😀");
    }

    #[test]
    fn empty_text_yields_no_chunks() {
        assert!(utf16_chunks("", 20).is_empty());
    }
}
