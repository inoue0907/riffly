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
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO,
        MonitorFromRect,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, GetWindowRect, SW_SHOWMAXIMIZED, SW_SHOWNORMAL, SetWindowPlacement,
        WINDOWPLACEMENT,
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
        let maximized = placement.showCmd == SW_SHOWMAXIMIZED.0 as u32;
        if maximized {
            // The size it goes back to when restored, which a maximized
            // window keeps in its placement.
            let rect = placement.rcNormalPosition;
            return Some(NativePlacement {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
                maximized,
            });
        }
        // Not maximized: where it actually is. A window snapped to half the
        // screen (or arranged by FancyZones) keeps its pre-snap size in its
        // placement, so that would put it back unsnapped. The real rectangle
        // is in screen coordinates; a placement wants workspace coordinates
        // (relative to the monitor's work area), so convert.
        let mut rect = RECT::default();
        // SAFETY: a valid window handle and an out-rectangle.
        unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
        let (dx, dy) = work_area_offset(&rect);
        Some(NativePlacement {
            left: rect.left - dx,
            top: rect.top - dy,
            right: rect.right - dx,
            bottom: rect.bottom - dy,
            maximized,
        })
    }

    /// How far the work area of the monitor showing `rect` is from that
    /// monitor's corner (a taskbar on the top or left edge moves it).
    fn work_area_offset(rect: &RECT) -> (i32, i32) {
        // SAFETY: monitor lookup and info query into a sized struct.
        unsafe {
            let monitor = MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                (
                    info.rcWork.left - info.rcMonitor.left,
                    info.rcWork.top - info.rcMonitor.top,
                )
            } else {
                (0, 0)
            }
        }
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
