// extern "C" FFI implementation for the xHoly Win32 Window Manager.

#include "window_manager_ffi.h"
#include "window_manager.h"

#include <cstring>
#include <memory>
#include <vector>

using namespace xholy::win32;

namespace {

struct FfiState {
    std::unique_ptr<WindowManager> mgr;
    std::vector<WindowEvent> pending;  // events drained from mgr->events_
};

// Map opaque XH_WM → FfiState. We keep a small registry so the C API
// can stash per-manager state (pending events) alongside the manager.
std::vector<std::pair<XH_WM, FfiState*>>& registry() {
    static std::vector<std::pair<XH_WM, FfiState*>> r;
    return r;
}

FfiState* find(XH_WM h) {
    for (auto& [k, v] : registry()) {
        if (k == h) return v;
    }
    return nullptr;
}

Window* win_of(XH_WM h, XH_WINDOW w) {
    auto* s = find(h);
    if (!s || !s->mgr) return nullptr;
    // XH_WINDOW is the raw Window* (manager-owned).
    return static_cast<Window*>(w);
}

}  // namespace

extern "C" {

XH_WM xh_wm_create(void) {
    auto* s = new FfiState();
    try {
        s->mgr = std::make_unique<WindowManager>();
    } catch (...) {
        delete s;
        return nullptr;
    }
    XH_WM h = static_cast<XH_WM>(s);
    registry().push_back({h, s});
    return h;
}

void xh_wm_destroy(XH_WM wm) {
    auto* s = find(wm);
    if (!s) return;
    delete s->mgr.release();
    delete s;
    auto& r = registry();
    r.erase(std::remove_if(r.begin(), r.end(),
                           [wm](auto& p) { return p.first == wm; }),
            r.end());
}

XH_WINDOW xh_wm_create_window(XH_WM wm,
                              const wchar_t* title,
                              int32_t x, int32_t y,
                              int32_t w, int32_t h,
                              int32_t type, int32_t style,
                              int32_t flags) {
    auto* s = find(wm);
    if (!s || !s->mgr) return nullptr;
    WindowConfig cfg;
    cfg.title = title ? title : L"xHoly";
    cfg.x = x; cfg.y = y; cfg.width = w; cfg.height = h;
    cfg.type = static_cast<WindowType>(type);
    cfg.style = static_cast<WindowStyle>(style);
    // flags bit0 = accept_file_drop, bit1 = show_in_taskbar, bit2 = visible
    cfg.accept_file_drop = (flags & 1) != 0;
    cfg.show_in_taskbar = (flags & 2) != 0;
    cfg.visible = (flags & 4) != 0;
    Window* win = s->mgr->createWindow(cfg);
    return static_cast<XH_WINDOW>(win);
}

void xh_wm_destroy_window(XH_WM wm, XH_WINDOW win) {
    auto* s = find(wm);
    if (!s || !s->mgr) return;
    s->mgr->destroyWindow(win_of(wm, win));
}

int32_t xh_wm_pump(XH_WM wm) {
    auto* s = find(wm);
    if (!s || !s->mgr) return 0;
    // Move queued events into pending for xh_wm_poll_events.
    auto evs = s->mgr->drainEvents();
    s->pending.insert(s->pending.end(),
                      std::make_move_iterator(evs.begin()),
                      std::make_move_iterator(evs.end()));
    return s->mgr->pumpMessages() ? 1 : 0;
}

int32_t xh_wm_window_count(XH_WM wm) {
    auto* s = find(wm);
    return s && s->mgr ? static_cast<int32_t>(s->mgr->windowCount()) : 0;
}

int32_t xh_wm_window_title(XH_WM wm, XH_WINDOW win, wchar_t* buf, int32_t buf_len) {
    auto* w = win_of(wm, win);
    if (!w || !buf || buf_len <= 0) return 0;
    std::wstring t = w->title();
    int32_t n = static_cast<int32_t>(t.size());
    int32_t copy = n < buf_len - 1 ? n : buf_len - 1;
    std::wcsncpy(buf, t.c_str(), copy);
    buf[copy] = L'\0';
    return n;
}

void xh_wm_resize(XH_WM wm, XH_WINDOW win, int32_t w, int32_t h) {
    auto* w_ = win_of(wm, win);
    if (w_) w_->resize(w, h);
}

void xh_wm_move(XH_WM wm, XH_WINDOW win, int32_t x, int32_t y) {
    auto* w_ = win_of(wm, win);
    if (w_) w_->moveTo(x, y);
}

void xh_wm_close(XH_WM wm, XH_WINDOW win) {
    auto* w_ = win_of(wm, win);
    if (w_) w_->close();
}

void xh_wm_bring_to_front(XH_WM wm, XH_WINDOW win) {
    auto* w_ = win_of(wm, win);
    if (w_) w_->bringToFront();
}

void xh_wm_send_to_back(XH_WM wm, XH_WINDOW win) {
    auto* w_ = win_of(wm, win);
    if (w_) w_->sendToBack();
}

void xh_wm_toggle_fullscreen(XH_WM wm, XH_WINDOW win) {
    auto* w_ = win_of(wm, win);
    if (w_) w_->toggleFullscreen();
}

int32_t xh_wm_poll_events(XH_WM wm) {
    auto* s = find(wm);
    if (!s) return 0;
    // Move pending into a drained snapshot is complex across calls;
    // simplest contract: each call returns count of *new* events and
  // leaves them accessible via xh_wm_event_* until next pump.
    return static_cast<int32_t>(s->pending.size());
}

int32_t xh_wm_event_type(XH_WM wm, int32_t idx) {
    auto* s = find(wm);
    if (!s || idx < 0 || idx >= static_cast<int32_t>(s->pending.size())) return -1;
    return static_cast<int32_t>(s->pending[idx].type);
}

uint64_t xh_wm_event_window_id(XH_WM wm, int32_t idx) {
    auto* s = find(wm);
    if (!s || idx < 0 || idx >= static_cast<int32_t>(s->pending.size())) return 0;
    return s->pending[idx].window_id;
}

}  // extern "C"
