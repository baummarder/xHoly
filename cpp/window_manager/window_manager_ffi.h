// extern "C" FFI layer for the xHoly Win32 Window Manager.
// Consumed by the Rust side (src/gui/native_wm.rs) via the `cc` build.

#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Opaque handle to a WindowManager instance.
typedef void* XH_WM;
/// Opaque handle to a managed Window.
typedef void* XH_WINDOW;

/// Create a WindowManager. Returns nullptr on failure.
XH_WM xh_wm_create(void);

/// Destroy a WindowManager and all its windows.
void xh_wm_destroy(XH_WM wm);

/// Create a window. `type`/`style` map to WindowType/WindowStyle enums
/// (see window_manager.h). Returns nullptr on failure.
XH_WINDOW xh_wm_create_window(XH_WM wm,
                              const wchar_t* title,
                              int32_t x, int32_t y,
                              int32_t w, int32_t h,
                              int32_t type, int32_t style,
                              int32_t flags);

/// Destroy a window.
void xh_wm_destroy_window(XH_WM wm, XH_WINDOW win);

/// Pump pending messages. Returns 0 when WM_QUIT was seen.
int32_t xh_wm_pump(XH_WM wm);

/// Number of managed windows.
int32_t xh_wm_window_count(XH_WM wm);

/// Copy the window title into `buf` (UTF-16). Returns chars written.
int32_t xh_wm_window_title(XH_WM wm, XH_WINDOW win, wchar_t* buf, int32_t buf_len);

/// Resize a window.
void xh_wm_resize(XH_WM wm, XH_WINDOW win, int32_t w, int32_t h);

/// Move a window.
void xh_wm_move(XH_WM wm, XH_WINDOW win, int32_t x, int32_t y);

/// Request close (posts WM_CLOSE).
void xh_wm_close(XH_WM wm, XH_WINDOW win);

/// Bring to front / send to back.
void xh_wm_bring_to_front(XH_WM wm, XH_WINDOW win);
void xh_wm_send_to_back(XH_WM wm, XH_WINDOW win);

/// Toggle fullscreen.
void xh_wm_toggle_fullscreen(XH_WM wm, XH_WINDOW win);

/// Drain queued events into a caller buffer.
/// Each event is packed as: [type:i32][window_id:u64][x:i32][y:i32]
/// [w:i32][h:i32][title_len:i32][title_utf16...][nfiles:i32]...
/// Returns number of events written; `buf_size` is in bytes.
/// Simplified: returns count; details via xh_wm_event_* accessors below.
int32_t xh_wm_poll_events(XH_WM wm);

/// Get event type of event `idx` (0-based) in the drained queue.
int32_t xh_wm_event_type(XH_WM wm, int32_t idx);
/// Get window id of event `idx`.
uint64_t xh_wm_event_window_id(XH_WM wm, int32_t idx);

#ifdef __cplusplus
}
#endif
