//! Keys pressed while the page has the keyboard (ADR 0024). The page webview gets keys before IMT does, so a few are
//! taken from it: Ctrl/⌘+L focuses the panel's address bar, Esc gives the keyboard back to IMT, and IMT's own
//! shortcuts (Ctrl+Shift+…, Alt+…, the function keys except reload and full screen) are handed to the IMT window.
//! Everything else (typing, Ctrl+C/V/A/Z, Ctrl+F, Enter, arrows) stays with the page.
//!
//! Only WebView2 tells IMT about these keys (`AcceleratorKeyPressed`); WKWebView and WebKitGTK do not, so on macOS
//! and Linux the user clicks into IMT instead.

// Only the WebView2 hook uses these at run time; elsewhere they are compiled for the shared event type and tests.
#![cfg_attr(not(windows), allow(dead_code))]

use serde::Serialize;

/// Windows virtual-key codes used here.
pub mod vk {
    pub const ESCAPE: u32 = 0x1B;
    pub const KEY_0: u32 = 0x30;
    pub const KEY_9: u32 = 0x39;
    pub const KEY_A: u32 = 0x41;
    pub const KEY_L: u32 = 0x4C;
    pub const KEY_Z: u32 = 0x5A;
    pub const F1: u32 = 0x70;
    pub const F5: u32 = 0x74;
    pub const F11: u32 = 0x7A;
    pub const F12: u32 = 0x7B;
}

/// The modifier keys held with a key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

/// A key IMT takes from the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PageKey {
    /// Ctrl/⌘+L: focus the Browser panel's address bar.
    FocusAddress,
    /// Esc: the keyboard goes back to IMT (the Browser panel).
    ReturnFocus,
    /// One of IMT's shortcuts: the IMT window gets it as a key press (`key` as `KeyboardEvent.key`).
    Shortcut {
        key: String,
        #[serde(flatten)]
        mods: Mods,
    },
}

/// What a key press in the page does (`vk`: the Windows virtual-key code); `None` leaves it to the page.
pub fn page_key(vk: u32, mods: Mods) -> Option<PageKey> {
    let command = mods.ctrl || mods.meta;
    match vk {
        vk::KEY_L if command && !mods.alt && !mods.shift => Some(PageKey::FocusAddress),
        vk::ESCAPE if !command && !mods.alt && !mods.shift => Some(PageKey::ReturnFocus),
        // Reload, full screen and the developer tools stay the page's.
        vk::F5 | vk::F11 | vk::F12 => None,
        vk::F1..=vk::F12 => Some(shortcut(format!("F{}", vk - vk::F1 + 1), mods)),
        vk::KEY_A..=vk::KEY_Z | vk::KEY_0..=vk::KEY_9 if (command && mods.shift) || (mods.alt && !command) => {
            let c = char::from_u32(vk).map(|c| c.to_ascii_lowercase()).unwrap_or('?');
            Some(shortcut(c.to_string(), mods))
        }
        _ => None,
    }
}

fn shortcut(key: String, mods: Mods) -> PageKey {
    PageKey::Shortcut { key, mods }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: Mods = Mods {
        ctrl: true,
        shift: false,
        alt: false,
        meta: false,
    };
    const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
    };

    #[test]
    fn ctrl_l_focuses_the_address_bar_and_esc_returns_to_imt() {
        assert_eq!(page_key(vk::KEY_L, CTRL), Some(PageKey::FocusAddress));
        assert_eq!(page_key(vk::KEY_L, Mods { meta: true, ..NONE }), Some(PageKey::FocusAddress), "⌘+L");
        assert_eq!(page_key(vk::ESCAPE, NONE), Some(PageKey::ReturnFocus));
        assert_eq!(page_key(vk::ESCAPE, Mods { shift: true, ..NONE }), None);
        assert_eq!(page_key(vk::KEY_L, NONE), None, "typing an l");
    }

    #[test]
    fn imt_shortcuts_go_to_imt_and_the_rest_stays_with_the_page() {
        assert_eq!(
            page_key(0x50, Mods { shift: true, ..CTRL }),
            Some(PageKey::Shortcut {
                key: "p".into(),
                mods: Mods { shift: true, ..CTRL }
            }),
            "Ctrl+Shift+P"
        );
        assert_eq!(
            page_key(0x46, Mods { alt: true, ..NONE }),
            Some(PageKey::Shortcut {
                key: "f".into(),
                mods: Mods { alt: true, ..NONE }
            }),
            "Alt+F (the menu)"
        );
        assert_eq!(
            page_key(vk::F1, NONE),
            Some(PageKey::Shortcut {
                key: "F1".into(),
                mods: NONE
            })
        );
        assert_eq!(
            page_key(vk::F1 + 9, NONE),
            Some(PageKey::Shortcut {
                key: "F10".into(),
                mods: NONE
            })
        );
        for (k, m) in [
            (0x43, CTRL),                       // Ctrl+C
            (0x56, CTRL),                       // Ctrl+V
            (0x46, CTRL),                       // Ctrl+F (find in page)
            (vk::F5, NONE),                     // reload
            (vk::F11, NONE),                    // full screen
            (vk::F12, NONE),                    // developer tools
            (0x0D, NONE),                       // Enter
            (0x25, CTRL),                       // Ctrl+Left
            (0x41, Mods { alt: true, ..CTRL }), // AltGr (Ctrl+Alt) letters type characters
        ] {
            assert_eq!(page_key(k, m), None, "{k:#x} {m:?}");
        }
    }

    #[test]
    fn the_ui_gets_the_action_tagged() {
        assert_eq!(
            serde_json::to_value(PageKey::FocusAddress).unwrap(),
            serde_json::json!({"action": "focusAddress"})
        );
        assert_eq!(
            serde_json::to_value(page_key(0x50, Mods { shift: true, ..CTRL }).unwrap()).unwrap(),
            serde_json::json!({"action": "shortcut", "key": "p", "ctrl": true, "shift": true, "alt": false, "meta": false})
        );
    }
}
