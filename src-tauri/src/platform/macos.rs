//! macOS integration: paste-based text injection, Return key, permission
//! status and System Settings deep links.
//!
//! Text is inserted by putting it on the general pasteboard and synthesising
//! ⌘V. This is the only approach that is fast and exact for arbitrary
//! Unicode (Turkish characters, emoji) in every app. The user's previous
//! clipboard is snapshotted (all items, all types) and restored afterwards,
//! unless something else wrote to the clipboard in the meantime.

use std::thread;
use std::time::Duration;

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, ProtocolObject};
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};

use super::{MicrophoneStatus, PrivacyPane};

const KEY_V: CGKeyCode = 9;
const KEY_RETURN: CGKeyCode = 36;
/// Time for the target app to read the pasteboard before we restore it.
/// Too short and slow apps (Electron, JetBrains) paste the OLD clipboard.
const PASTE_SETTLE: Duration = Duration::from_millis(180);
/// Gap between paste and Return so the paste lands before the newline.
const ENTER_DELAY: Duration = Duration::from_millis(60);
/// CGEventKeyboardSetUnicodeString carries at most 20 UTF-16 units per event.
const UNICODE_CHUNK_UTF16: usize = 20;
/// Breathing room between chunks so slower event loops (Electron, web
/// views) keep characters in order. Not a spin: the thread sleeps.
const TYPE_CHUNK_DELAY: Duration = Duration::from_millis(4);
/// Do not snapshot clipboards larger than this (e.g. huge images); they are
/// left alone instead of being copied into memory.
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
/// nspasteboard.org markers: clipboard managers skip transient content.
const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

/// Plain-data copy of the pasteboard: per item, its (type, bytes) pairs.
struct Snapshot {
    items: Vec<Vec<(String, Vec<u8>)>>,
}

fn snapshot(pasteboard: &NSPasteboard) -> Option<Snapshot> {
    let items = pasteboard.pasteboardItems()?;
    let mut total = 0usize;
    let mut copied = Vec::with_capacity(items.count());
    for item in items.iter() {
        let mut entries = Vec::new();
        for ty in item.types().iter() {
            if let Some(data) = item.dataForType(&ty) {
                let bytes = data.to_vec();
                total += bytes.len();
                if total > MAX_SNAPSHOT_BYTES {
                    tracing::info!("clipboard too large to snapshot; it will not be restored");
                    return None;
                }
                entries.push((ty.to_string(), bytes));
            }
        }
        copied.push(entries);
    }
    Some(Snapshot { items: copied })
}

fn restore(pasteboard: &NSPasteboard, snapshot: Snapshot) {
    pasteboard.clearContents();
    if snapshot.items.is_empty() {
        return;
    }
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = snapshot
        .items
        .into_iter()
        .map(|entries| {
            let item = NSPasteboardItem::new();
            for (ty, bytes) in entries {
                item.setData_forType(&NSData::with_bytes(&bytes), &NSString::from_str(&ty));
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    if !pasteboard.writeObjects(&NSArray::from_retained_slice(&items)) {
        tracing::warn!("could not restore previous clipboard");
    }
}

fn post_key(code: CGKeyCode, flags: CGEventFlags) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|()| "olay kaynağı oluşturulamadı".to_owned())?;
    for key_down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), code, key_down)
            .map_err(|()| "klavye olayı oluşturulamadı".to_owned())?;
        // Explicit flags override any modifier the user is still holding
        // (e.g. ⌥ from the ⌥Space shortcut), so ⌘V stays a plain paste.
        event.set_flags(flags);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}

fn ensure_accessibility() -> Result<(), String> {
    if accessibility_trusted() {
        Ok(())
    } else {
        Err("Erişilebilirlik izni gerekli (Sistem Ayarları › Gizlilik › Erişilebilirlik).".into())
    }
}

/// Types `text` into the focused app with synthetic key events carrying the
/// Unicode string directly, so the clipboard is never touched and the
/// keyboard layout is irrelevant. Blocks while typing.
pub fn type_text(text: &str, press_enter: bool) -> Result<(), String> {
    ensure_accessibility()?;
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|()| "olay kaynağı oluşturulamadı".to_owned())?;
    for chunk in super::utf16_chunks(text, UNICODE_CHUNK_UTF16) {
        for key_down in [true, false] {
            // The key code is irrelevant: the attached string is what apps insert.
            let event = CGEvent::new_keyboard_event(source.clone(), 0, key_down)
                .map_err(|()| "klavye olayı oluşturulamadı".to_owned())?;
            event.set_flags(CGEventFlags::CGEventFlagNull);
            event.set_string_from_utf16_unchecked(&chunk);
            event.post(CGEventTapLocation::HID);
        }
        thread::sleep(TYPE_CHUNK_DELAY);
    }
    if press_enter {
        thread::sleep(ENTER_DELAY);
        post_key(KEY_RETURN, CGEventFlags::CGEventFlagNull)?;
    }
    Ok(())
}

/// Pastes `text` into the focused app, optionally presses Return, then puts
/// the previous clipboard back. Blocks for roughly `PASTE_SETTLE`.
pub fn paste_text(text: &str, press_enter: bool, restore_clipboard: bool) -> Result<(), String> {
    ensure_accessibility()?;
    let pasteboard = NSPasteboard::generalPasteboard();
    let previous = if restore_clipboard {
        snapshot(&pasteboard)
    } else {
        None
    };

    pasteboard.clearContents();
    // SAFETY: NSPasteboardTypeString is an immutable AppKit constant.
    let string_type = unsafe { NSPasteboardTypeString };
    if !pasteboard.setString_forType(&NSString::from_str(text), string_type) {
        return Err("Metin panoya yazılamadı.".into());
    }
    if restore_clipboard {
        for marker in [TRANSIENT_TYPE, CONCEALED_TYPE] {
            pasteboard.setString_forType(&NSString::from_str(""), &NSString::from_str(marker));
        }
    }
    let our_change = pasteboard.changeCount();

    post_key(KEY_V, CGEventFlags::CGEventFlagCommand)?;
    if press_enter {
        thread::sleep(ENTER_DELAY);
        post_key(KEY_RETURN, CGEventFlags::CGEventFlagNull)?;
    }

    if let Some(previous) = previous {
        thread::sleep(PASTE_SETTLE);
        // Someone copied something new in the meantime: theirs wins.
        if pasteboard.changeCount() == our_change {
            restore(&pasteboard, previous);
        }
    }
    Ok(())
}

/// Fallback when pasting is impossible: leave the text on the clipboard so
/// the dictation is never lost.
pub fn copy_text(text: &str) -> Result<(), String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: NSPasteboardTypeString is an immutable AppKit constant.
    let string_type = unsafe { NSPasteboardTypeString };
    if pasteboard.setString_forType(&NSString::from_str(text), string_type) {
        Ok(())
    } else {
        Err("Metin panoya yazılamadı.".into())
    }
}

pub fn accessibility_trusted() -> bool {
    // SAFETY: argument-less query of the TCC database.
    unsafe { AXIsProcessTrusted() }
}

pub fn microphone_status() -> MicrophoneStatus {
    let Some(class) = AnyClass::get(c"AVCaptureDevice") else {
        return MicrophoneStatus::Unknown;
    };
    // AVMediaTypeAudio's value is the FourCC string "soun".
    let media_type = NSString::from_str("soun");
    // SAFETY: documented class method taking an AVMediaType, returning NSInteger.
    let status: isize = unsafe { msg_send![class, authorizationStatusForMediaType: &*media_type] };
    match status {
        0 => MicrophoneStatus::NotDetermined,
        1 => MicrophoneStatus::Restricted,
        2 => MicrophoneStatus::Denied,
        3 => MicrophoneStatus::Granted,
        _ => MicrophoneStatus::Unknown,
    }
}

pub fn open_privacy_pane(pane: PrivacyPane) -> Result<(), String> {
    let url = match pane {
        PrivacyPane::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        PrivacyPane::Microphone => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
    };
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|err| err.to_string())
        .and_then(|s| {
            if s.success() {
                Ok(())
            } else {
                Err(format!("open başarısız: {s}"))
            }
        })
}
