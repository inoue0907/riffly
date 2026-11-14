//! Where the window is, in Windows' own terms.
//!
//! gpui reports window bounds in logical pixels, converted with the scale
//! of whichever monitor the window is on. With monitors at different scales
//! (a 150% laptop screen beside 100% monitors) those numbers do not map back
//! to the same place. Windows' own window placement does: the restored
//! rectangle in physical pixels plus whether the window is maximized, which
//! `SetWindowPlacement` puts back on the right monitor at the right scale.

use serde::{Deserialize, Serialize};

/// A window's place as Windows keeps it: the rectangle it has when not
/// maximized, in physical pixels, and whether it is maximized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePlacement {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub maximized: bool,
}

#[cfg(windows)]
mod imp {
    use gpui_kit::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromRect};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_SHOWMAXIMIZED, SW_SHOWNORMAL, SetWindowPlacement, WINDOWPLACEMENT,
    };

    use super::NativePlacement;

    fn hwnd(window: &Window) -> Option<HWND> {
        // The trait's method, not gpui's own `window_handle` of the same name.
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
            _ => None,
        }
    }

    pub fn read(window: &Window) -> Option<NativePlacement> {
        let hwnd = hwnd(window)?;
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        // SAFETY: a valid window handle and a placement sized as required.
        unsafe { GetWindowPlacement(hwnd, &mut placement) }.ok()?;
        let rect = placement.rcNormalPosition;
        Some(NativePlacement {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
            maximized: placement.showCmd == SW_SHOWMAXIMIZED.0 as u32,
        })
    }

    /// Whether any monitor still shows part of the rectangle.
    pub fn is_on_a_monitor(placement: &NativePlacement) -> bool {
        let rect = RECT {
            left: placement.left,
            top: placement.top,
            right: placement.right,
            bottom: placement.bottom,
        };
        // SAFETY: a plain rectangle lookup.
        !unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_invalid()
    }

    /// Put the window back where it was, shown (and maximized if it was).
    pub fn apply(window: &Window, placement: &NativePlacement) -> bool {
        let Some(hwnd) = hwnd(window) else { return false };
        if placement.right - placement.left < 200 || placement.bottom - placement.top < 150 {
            return false;
        }
        if !is_on_a_monitor(placement) {
            return false;
        }
        let native = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            showCmd: if placement.maximized { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNORMAL.0 as u32 },
            rcNormalPosition: RECT {
                left: placement.left,
                top: placement.top,
                right: placement.right,
                bottom: placement.bottom,
            },
            ..Default::default()
        };
        // SAFETY: a valid window handle and a filled-in placement.
        unsafe { SetWindowPlacement(hwnd, &native) }.is_ok()
    }
}

#[cfg(not(windows))]
mod imp {
    use gpui_kit::Window;

    use super::NativePlacement;

    pub fn read(_: &Window) -> Option<NativePlacement> {
        None
    }

    pub fn is_on_a_monitor(_: &NativePlacement) -> bool {
        false
    }

    pub fn apply(_: &Window, _: &NativePlacement) -> bool {
        false
    }
}

pub use imp::{apply, is_on_a_monitor, read};
