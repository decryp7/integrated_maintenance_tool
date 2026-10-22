//! The state of the mouse outside any webview. A pop-out window dragged by the OS gives its host no "dropped" event,
//! so the host asks whether the primary button is still held to know when the window was released.

/// Whether the primary (usually left) mouse button is held down anywhere on screen; `None` when the platform cannot
/// tell (then the host falls back to docking when the pointer rests on a guide).
#[tauri::command]
pub async fn pointer_button_down() -> Option<bool> {
    primary_button_down()
}

#[cfg(target_os = "macos")]
pub(crate) fn primary_button_down() -> Option<bool> {
    // Bit 0 is the primary button, already mapped for left-handed setups by the system.
    Some(objc2_app_kit::NSEvent::pressedMouseButtons() & 1 == 1)
}

#[cfg(windows)]
pub(crate) fn primary_button_down() -> Option<bool> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
    // GetAsyncKeyState reports physical buttons: with swapped buttons the primary one is the right button.
    // SAFETY: plain Win32 queries without pointers or handles.
    let key = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        VK_RBUTTON
    } else {
        VK_LBUTTON
    };
    // SAFETY: as above.
    let state = unsafe { GetAsyncKeyState(i32::from(key.0)) };
    Some(state < 0)
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) fn primary_button_down() -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Now as _;

    #[test]
    fn reports_the_button_where_the_platform_can() {
        let state = pointer_button_down().now();
        if cfg!(any(target_os = "macos", windows)) {
            // Nobody holds the mouse while the tests run.
            assert_eq!(state, Some(false));
        } else {
            assert_eq!(state, None);
        }
    }
}
