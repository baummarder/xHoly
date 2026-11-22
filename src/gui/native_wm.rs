//! Native window manager bindings — Phase 8.
//!
//! Rust-side wrapper around the C++ Win32 Window Manager
//! (`cpp/window_manager/`), linked via `build.rs` + the `cc` crate.
//! Windows-only; on other targets every item here is compiled out.
//!
//! This is *infrastructure*: it creates and pumps native HWNDs. The
//! rendering path stays in Rust (wgpu) — this module exists so the GUI
//! can offload window lifecycle to the native WM when desired.

#![cfg(target_os = "windows")]

use std::ffi::c_void;

// ---------------------------------------------------------------------------
// extern "C" declarations (must match window_manager_ffi.h)
// ---------------------------------------------------------------------------

#[allow(non_camel_case_types)]
type XH_WM = *mut c_void;
#[allow(non_camel_case_types)]
type XH_WINDOW = *mut c_void;

unsafe extern "C" {
    fn xh_wm_create() -> XH_WM;
    fn xh_wm_destroy(wm: XH_WM);
    fn xh_wm_create_window(
        wm: XH_WM,
        title: *const u16,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        type_: i32,
        style: i32,
        flags: i32,
    ) -> XH_WINDOW;
    fn xh_wm_destroy_window(wm: XH_WM, win: XH_WINDOW);
    fn xh_wm_pump(wm: XH_WM) -> i32;
    fn xh_wm_window_count(wm: XH_WM) -> i32;
    fn xh_wm_window_title(wm: XH_WM, win: XH_WINDOW, buf: *mut u16, len: i32) -> i32;
    fn xh_wm_resize(wm: XH_WM, win: XH_WINDOW, w: i32, h: i32);
    fn xh_wm_move(wm: XH_WM, win: XH_WINDOW, x: i32, y: i32);
    fn xh_wm_close(wm: XH_WM, win: XH_WINDOW);
    fn xh_wm_bring_to_front(wm: XH_WM, win: XH_WINDOW);
    fn xh_wm_send_to_back(wm: XH_WM, win: XH_WINDOW);
    fn xh_wm_toggle_fullscreen(wm: XH_WM, win: XH_WINDOW);
    fn xh_wm_poll_events(wm: XH_WM) -> i32;
    fn xh_wm_event_type(wm: XH_WM, idx: i32) -> i32;
    fn xh_wm_event_window_id(wm: XH_WM, idx: i32) -> u64;
}

// ---------------------------------------------------------------------------
// Safe wrappers
// ---------------------------------------------------------------------------

/// Window semantic type (mirrors C++ `WindowType` enum order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum NativeWindowType {
    Normal = 0,
    Toolbar = 1,
    Tooltip = 2,
    ContextMenu = 3,
    Dialog = 4,
    Popup = 5,
    Modal = 6,
}

/// Window chrome style (mirrors C++ `WindowStyle` enum order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum NativeWindowStyle {
    Default = 0,
    Minimal = 1,
    Titlebar = 2,
    Bordered = 3,
    Popup = 4,
    Dialog = 5,
}

/// Creation flags bitfield.
pub mod flags {
    pub const ACCEPT_FILE_DROP: i32 = 1;
    pub const SHOW_IN_TASKBAR: i32 = 2;
    pub const VISIBLE: i32 = 4;
}

/// A managed native window (opaque handle).
#[derive(Debug)]
pub struct NativeWindow {
    ptr: XH_WINDOW,
}

impl NativeWindow {
    /// Raw handle (HWND-equivalent managed pointer).
    pub fn as_ptr(&self) -> *mut c_void {
        self.ptr
    }
}

/// Handle to the C++ WindowManager instance.
pub struct NativeWindowManager {
    ptr: XH_WM,
}

impl NativeWindowManager {
    /// Create the underlying C++ WindowManager. Returns `None` on failure.
    pub fn new() -> Option<Self> {
        let ptr = unsafe { xh_wm_create() };
        if ptr.is_null() {
            None
        } else {
            Some(Self { ptr })
        }
    }

    /// Create a managed window.
    #[allow(clippy::too_many_arguments)]
    pub fn create_window(
        &self,
        title: &str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        ty: NativeWindowType,
        style: NativeWindowStyle,
        flags: i32,
    ) -> Option<NativeWindow> {
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let ptr = unsafe {
            xh_wm_create_window(
                self.ptr,
                wide.as_ptr(),
                x,
                y,
                w,
                h,
                ty as i32,
                style as i32,
                flags,
            )
        };
        if ptr.is_null() {
            None
        } else {
            Some(NativeWindow { ptr })
        }
    }

    /// Destroy a managed window.
    pub fn destroy_window(&self, win: &NativeWindow) {
        unsafe { xh_wm_destroy_window(self.ptr, win.ptr) }
    }

    /// Pump messages; returns `false` on WM_QUIT.
    pub fn pump(&self) -> bool {
        unsafe { xh_wm_pump(self.ptr) != 0 }
    }

    /// Number of managed windows.
    pub fn window_count(&self) -> i32 {
        unsafe { xh_wm_window_count(self.ptr) }
    }

    /// Resize / move / close / z-order / fullscreen helpers.
    pub fn resize(&self, win: &NativeWindow, w: i32, h: i32) {
        unsafe { xh_wm_resize(self.ptr, win.ptr, w, h) }
    }
    pub fn move_window(&self, win: &NativeWindow, x: i32, y: i32) {
        unsafe { xh_wm_move(self.ptr, win.ptr, x, y) }
    }
    pub fn close(&self, win: &NativeWindow) {
        unsafe { xh_wm_close(self.ptr, win.ptr) }
    }
    pub fn bring_to_front(&self, win: &NativeWindow) {
        unsafe { xh_wm_bring_to_front(self.ptr, win.ptr) }
    }
    pub fn send_to_back(&self, win: &NativeWindow) {
        unsafe { xh_wm_send_to_back(self.ptr, win.ptr) }
    }
    pub fn toggle_fullscreen(&self, win: &NativeWindow) {
        unsafe { xh_wm_toggle_fullscreen(self.ptr, win.ptr) }
    }

    /// Window title (UTF-16 → String, lossy).
    pub fn window_title(&self, win: &NativeWindow) -> String {
        let mut buf = vec![0u16; 512];
        let n =
            unsafe { xh_wm_window_title(self.ptr, win.ptr, buf.as_mut_ptr(), buf.len() as i32) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }

    /// Number of pending events.
    pub fn pending_events(&self) -> i32 {
        unsafe { xh_wm_poll_events(self.ptr) }
    }

    /// Event type of pending event `idx` (see C++ `WindowEventType`).
    pub fn event_type(&self, idx: i32) -> i32 {
        unsafe { xh_wm_event_type(self.ptr, idx) }
    }

    /// Window id of pending event `idx`.
    pub fn event_window_id(&self, idx: i32) -> u64 {
        unsafe { xh_wm_event_window_id(self.ptr, idx) }
    }
}

impl Drop for NativeWindowManager {
    fn drop(&mut self) {
        unsafe { xh_wm_destroy(self.ptr) }
    }
}

// ---------------------------------------------------------------------------
// Demo entry point (used by `--native-wm`)
// ---------------------------------------------------------------------------

/// Run a minimal native-WM demo: one window, pumped until closed or
/// `max_frames` elapses. Prints event counts to stdout.
///
/// This proves the FFI path end-to-end on a real desktop session.
pub fn run_native_demo(max_frames: u64) -> Result<(), String> {
    let wm = NativeWindowManager::new().ok_or("xh_wm_create failed")?;
    let win = wm
        .create_window(
            "xHoly — Native WM (C++/Win32)",
            100,
            100,
            960,
            540,
            NativeWindowType::Normal,
            NativeWindowStyle::Default,
            flags::VISIBLE | flags::SHOW_IN_TASKBAR | flags::ACCEPT_FILE_DROP,
        )
        .ok_or("xh_wm_create_window failed")?;

    eprintln!(
        "xHoly: native WM window created (title='{}')",
        wm.window_title(&win)
    );

    let mut frames = 0u64;
    let mut events = 0i32;
    while frames < max_frames {
        if !wm.pump() {
            break; // WM_QUIT
        }
        let n = wm.pending_events();
        if n > events {
            events = n;
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
        frames += 1;
    }

    eprintln!(
        "xHoly: native WM demo done — {} windows, {} events, {} frames",
        wm.window_count(),
        events,
        frames
    );
    Ok(())
}
