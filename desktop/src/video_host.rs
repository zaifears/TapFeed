//! A plain Win32 child window that sits over the "video" area of the web page.
//! libmpv draws into it (mpv option `wid`). Must be created and moved on the UI thread.

use std::sync::OnceLock;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, LoadCursorW, RegisterClassW, SetWindowPos, ShowWindow,
    HWND_TOP, IDC_ARROW, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, WNDCLASSW,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

static CLASS: OnceLock<Vec<u16>> = OnceLock::new();

fn class_name() -> *const u16 {
    CLASS
        .get_or_init(|| {
            let name = wide("TapFeedVideoHost");
            unsafe {
                let wc = WNDCLASSW {
                    style: 0,
                    lpfnWndProc: Some(DefWindowProcW),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: GetModuleHandleW(std::ptr::null()),
                    hIcon: std::ptr::null_mut(),
                    hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                    hbrBackground: GetStockObject(BLACK_BRUSH),
                    lpszMenuName: std::ptr::null(),
                    lpszClassName: name.as_ptr(),
                };
                RegisterClassW(&wc);
            }
            name
        })
        .as_ptr()
}

/// Create the child window inside `parent`. Returns its HWND as an integer.
pub fn create(parent: isize, x: i32, y: i32, w: i32, h: i32) -> Result<isize, String> {
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name(),
            std::ptr::null(),
            WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
            x,
            y,
            w.max(1),
            h.max(1),
            parent as HWND,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("CreateWindowExW failed for the video area".into());
    }
    place(hwnd as isize, x, y, w, h);
    Ok(hwnd as isize)
}

/// Move/resize the child window and keep it above the WebView2 window (visibility unchanged).
pub fn place(hwnd: isize, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        SetWindowPos(
            hwnd as HWND,
            HWND_TOP,
            x,
            y,
            w.max(1),
            h.max(1),
            SWP_NOACTIVATE,
        );
    }
}

/// Hide or show the video window (hidden while another page or a dialog is on screen).
pub fn show(hwnd: isize, visible: bool) {
    unsafe {
        ShowWindow(hwnd as HWND, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
}
