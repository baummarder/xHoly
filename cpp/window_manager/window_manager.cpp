// xHoly Win32 Window Manager — implementation (pure Win32 API).

#include "window_manager.h"

#include <shellapi.h>   // DragAcceptFiles / DragQueryFile / WM_DROPFILES
#include <algorithm>
#include <atomic>

namespace xholy::win32 {

namespace {
// hwnd → (Window*, owning WindowManager) registry used by the shared
// wndproc and by Window::emitViaManager.
struct HwndEntry {
    HWND hwnd;
    Window* win;
    WindowManager* mgr;
};
std::vector<HwndEntry>& hwndRegistry() {
    static std::vector<HwndEntry> reg;
    return reg;
}
void unregisterHwnd(HWND hwnd) {
    auto& reg = hwndRegistry();
    reg.erase(std::remove_if(reg.begin(), reg.end(),
                             [hwnd](const HwndEntry& e) { return e.hwnd == hwnd; }),
              reg.end());
}
}  // namespace

const wchar_t* WindowManager::kClassName = L"xHolyWindowClass";

// ---------------------------------------------------------------------------
// Style mapping
// ---------------------------------------------------------------------------

DWORD WindowManager::toWin32Style(WindowStyle s, WindowType t) {
    DWORD base = WS_OVERLAPPEDWINDOW;
    switch (s) {
        case WindowStyle::Minimal:
            base = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME;
            break;
        case WindowStyle::Titlebar:
            base = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
            break;
        case WindowStyle::Bordered:
            base = WS_POPUP | WS_BORDER;
            break;
        case WindowStyle::Popup:
            base = WS_POPUP;
            break;
        case WindowStyle::Dialog:
            base = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
            break;
        case WindowStyle::Default:
            break;
    }
    // Tool windows get a smaller caption; dialogs/popups lose min/max.
    switch (t) {
        case WindowType::Toolbar:
        case WindowType::Tooltip:
            base |= WS_EX_TOOLWINDOW;  // applied via ex-style too
            break;
        case WindowType::ContextMenu:
        case WindowType::Popup:
        case WindowType::Modal:
            base &= ~(WS_MINIMIZEBOX | WS_MAXIMIZEBOX);
            break;
        default:
            break;
    }
    return base;
}

DWORD WindowManager::toWin32ExStyle(const WindowConfig& cfg) {
    DWORD ex = 0;
    if (!cfg.show_in_taskbar ||
        cfg.type == WindowType::Toolbar ||
        cfg.type == WindowType::Tooltip) {
        ex |= WS_EX_TOOLWINDOW;
    }
    if (cfg.type == WindowType::Popup || cfg.type == WindowType::ContextMenu) {
        ex |= WS_EX_TOPMOST;
    }
    if (cfg.translucent) {
        ex |= WS_EX_LAYERED;
    }
    if (!cfg.focusable) {
        ex |= WS_EX_NOACTIVATE;
    }
    return ex;
}

// ---------------------------------------------------------------------------
// WindowManager
// ---------------------------------------------------------------------------

WindowManager::WindowManager() {
    registerWindowClass();
}

WindowManager::~WindowManager() {
    destroyAll();
    unregisterWindowClass();
}

void WindowManager::registerWindowClass() {
    WNDCLASSEXW wc{};
    wc.cbSize = sizeof(wc);
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = &WindowManager::sharedWndProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    wc.lpszClassName = kClassName;
    wc.hIcon = LoadIconW(nullptr, IDI_APPLICATION);
    wc.hIconSm = wc.hIcon;
    class_atom_ = RegisterClassExW(&wc);
}

void WindowManager::unregisterWindowClass() {
    if (class_atom_) {
        UnregisterClassW(kClassName, GetModuleHandleW(nullptr));
        class_atom_ = 0;
    }
}

Window* WindowManager::createWindow(const WindowConfig& cfg) {
    if (!class_atom_) {
        registerWindowClass();
    }

    static std::atomic<uint64_t> next_id{1};
    uint64_t id = next_id.fetch_add(1);

    HWND parent = cfg.parent ? cfg.parent : nullptr;
    HWND owner = cfg.owner ? cfg.owner : parent;

    DWORD style = toWin32Style(cfg.style, cfg.type);
    DWORD ex = toWin32ExStyle(cfg);

    if (cfg.initial_state == WindowState::Maximized) {
        style |= WS_MAXIMIZE;
    }
    if (!cfg.visible) {
        style |= WS_VISIBLE == 0 ? 0 : 0;  // visibility applied post-create
    }

    // Wide-char title.
    std::wstring title = cfg.title.empty() ? L"xHoly" : cfg.title;

    // Convert icon path.
    HICON icon = nullptr;
    if (!cfg.icon_path.empty()) {
        icon = static_cast<HICON>(LoadImageW(
            nullptr, cfg.icon_path.c_str(), IMAGE_ICON,
            0, 0, LR_LOADFROMFILE | LR_DEFAULTSIZE));
    }

    HWND hwnd = CreateWindowExW(
        ex,
        kClassName,
        title.c_str(),
        style,
        cfg.x, cfg.y, cfg.width, cfg.height,
        parent,      // hwndParent (child) — owner handled below
        nullptr,     // hMenu
        GetModuleHandleW(nullptr),
        reinterpret_cast<LPVOID>(id));  // lpParam → Window::id via GWLP_USERDATA later

    if (!hwnd) {
        return nullptr;
    }
    // For owned windows (owner != parent) re-parent ownership via GWLP_HWNDPARENT
    // is not allowed post-create for owned popups; instead we accept
    // owner only when creating with parent==owner for tool windows.
    // Set user data to our id.
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, static_cast<LONG_PTR>(id));

    if (icon) {
        SendMessageW(hwnd, WM_SETICON, ICON_BIG, reinterpret_cast<LPARAM>(icon));
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL, reinterpret_cast<LPARAM>(icon));
    }

    if (cfg.translucent) {
        SetLayeredWindowAttributes(hwnd, 0, 230, LWA_ALPHA);
    }

    auto win = std::unique_ptr<Window>(new Window(id, hwnd, cfg));
    win->accepts_file_drop_ = cfg.accept_file_drop;
    if (cfg.accept_file_drop) {
        DragAcceptFiles(hwnd, TRUE);
    }
    if (!cfg.visible) {
        ShowWindow(hwnd, SW_HIDE);
    }

    Window* raw = win.get();
    windows_.push_back(std::move(win));
    hwndRegistry().push_back({hwnd, raw, this});

    WindowEvent ev{WindowEventType::Create, id, cfg.x, cfg.y, cfg.width, cfg.height, title, {}};
    emit(ev);
    return raw;
}

void WindowManager::destroyWindow(Window* w) {
    if (!w) return;
    for (auto it = windows_.begin(); it != windows_.end(); ++it) {
        if (it->get() == w) {
            WindowEvent ev{WindowEventType::Destroy, w->id(), 0, 0, 0, 0, w->title(), {}};
            emit(ev);
            if (w->hwnd_ && IsWindow(w->hwnd_)) {
                DestroyWindow(w->hwnd_);
            }
            unregisterHwnd(w->hwnd_);
            windows_.erase(it);
            return;
        }
    }
}

void WindowManager::destroyAll() {
    while (!windows_.empty()) {
        Window* w = windows_.back().get();
        if (w->hwnd_ && IsWindow(w->hwnd_)) {
            DestroyWindow(w->hwnd_);
        }
        unregisterHwnd(w->hwnd_);
        windows_.pop_back();
    }
}

bool WindowManager::pumpMessages() {
    MSG msg{};
    bool quit = false;
    while (PeekMessageW(&msg, nullptr, 0, 0, PM_REMOVE)) {
        if (msg.message == WM_QUIT) {
            quit = true;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    return !quit;
}

bool WindowManager::waitMessage() {
    MSG msg{};
    BOOL r = GetMessageW(&msg, nullptr, 0, 0);
    if (r > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
        return msg.message != WM_QUIT;
    }
    return false;
}

// --- queries ----------------------------------------------------------------

std::vector<Window*> WindowManager::windows() {
    std::vector<Window*> out;
    out.reserve(windows_.size());
    for (auto& w : windows_) out.push_back(w.get());
    return out;
}

Window* WindowManager::findById(uint64_t id) {
    for (auto& w : windows_) if (w->id_ == id) return w.get();
    return nullptr;
}

Window* WindowManager::findByTitle(const std::wstring& title) {
    for (auto& w : windows_) {
        if (w->title() == title) return w.get();
    }
    return nullptr;
}

Window* WindowManager::findByHwnd(HWND hwnd) {
    for (auto& w : windows_) if (w->hwnd_ == hwnd) return w.get();
    return nullptr;
}

Window* WindowManager::windowAtPoint(int32_t x, int32_t y) {
    POINT pt{ x, y };
    HWND hwnd = WindowFromPoint(pt);
    return hwnd ? findByHwnd(hwnd) : nullptr;
}

Window* WindowManager::windowAtState(WindowState s) {
    for (auto& w : windows_) if (w->state() == s) return w.get();
    return nullptr;
}

Window* WindowManager::windowAtType(WindowType t) {
    for (auto& w : windows_) if (w->type() == t) return w.get();
    return nullptr;
}

Window* WindowManager::windowAtStyle(WindowStyle s) {
    for (auto& w : windows_) if (w->style() == s) return w.get();
    return nullptr;
}

// --- events -----------------------------------------------------------------

void WindowManager::pushEvent(WindowEvent ev) { events_.push_back(std::move(ev)); }

std::vector<WindowEvent> WindowManager::drainEvents() {
    std::vector<WindowEvent> out = std::move(events_);
    events_.clear();
    return out;
}

void WindowManager::emit(WindowEvent ev) {
    events_.push_back(ev);
    if (on_event_) on_event_(ev);
}

LRESULT CALLBACK WindowManager::sharedWndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    // WM_NCCREATE arrives before the hwnd→Window registry entry exists;
    // the window id was passed as lpCreateParams.
    if (msg == WM_NCCREATE) {
        auto* cs = reinterpret_cast<CREATESTRUCTW*>(lp);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA,
                          reinterpret_cast<LONG_PTR>(cs->lpCreateParams));
    }

    // Look up the managed Window for this hwnd.
    Window* self = nullptr;
    for (auto& e : hwndRegistry()) {
        if (e.hwnd == hwnd) { self = e.win; break; }
    }
    if (self) {
        return self->handleMessage(msg, wp, lp);
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

Window::Window(uint64_t id, HWND hwnd, WindowConfig cfg)
    : id_(id), hwnd_(hwnd), config_(std::move(cfg)) {}

Window::~Window() {
    if (hwnd_ && IsWindow(hwnd_)) {
        DestroyWindow(hwnd_);
    }
}

WindowState Window::state() const {
    if (!hwnd_) return WindowState::Closed;
    if (fullscreen_) return WindowState::Fullscreen;
    if (IsIconic(hwnd_)) return WindowState::Minimized;
    if (IsZoomed(hwnd_)) return WindowState::Maximized;
    return WindowState::Normal;
}

void Window::moveTo(int32_t x, int32_t y) {
    if (!hwnd_) return;
    RECT r{};
    GetWindowRect(hwnd_, &r);
    SetWindowPos(hwnd_, nullptr, x, y, r.right - r.left, r.bottom - r.top,
                 SWP_NOZORDER | SWP_NOACTIVATE);
}

void Window::resize(int32_t w, int32_t h) {
    if (!hwnd_) return;
    SetWindowPos(hwnd_, nullptr, 0, 0, w, h,
                 SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
}

void Window::setBounds(int32_t x, int32_t y, int32_t w, int32_t h) {
    if (!hwnd_) return;
    SetWindowPos(hwnd_, nullptr, x, y, w, h, SWP_NOZORDER);
}

WindowMetrics Window::metrics() const {
    WindowMetrics m{};
    if (!hwnd_) return m;
    RECT wr{}, cr{};
    GetWindowRect(hwnd_, &wr);
    GetClientRect(hwnd_, &cr);
    POINT tl{wr.left, wr.top};
    ScreenToClient(hwnd_, &tl);
    m.x = wr.left; m.y = wr.top;
    m.width = wr.right - wr.left;
    m.height = wr.bottom - wr.top;
    m.client_width = cr.right - cr.left;
    m.client_height = cr.bottom - cr.top;
    m.visible = IsWindowVisible(hwnd_) != FALSE;
    m.enabled = IsWindowEnabled(hwnd_) != FALSE;
    m.focused = GetForegroundWindow() == hwnd_;
    m.iconic = IsIconic(hwnd_) != FALSE;
    m.maximized = IsZoomed(hwnd_) != FALSE;
    m.fullscreen = fullscreen_;
    // z-order: enumerate top-level windows and find our index
    HWND cur = GetTopWindow(nullptr);
    int z = 0;
    while (cur) {
        if (cur == hwnd_) { m.z_order = z; break; }
        cur = GetWindow(cur, GW_HWNDNEXT);
        ++z;
    }
    return m;
}

void Window::show() {
    if (hwnd_) ShowWindow(hwnd_, SW_SHOW);
}

void Window::hide() {
    if (hwnd_) ShowWindow(hwnd_, SW_HIDE);
}

bool Window::isVisible() const {
    return hwnd_ && IsWindowVisible(hwnd_) != FALSE;
}

void Window::enable(bool on) {
    if (hwnd_) EnableWindow(hwnd_, on ? TRUE : FALSE);
}

void Window::minimize() {
    if (hwnd_) ShowWindow(hwnd_, SW_MINIMIZE);
}

void Window::maximize() {
    if (hwnd_) ShowWindow(hwnd_, SW_MAXIMIZE);
}

void Window::restore() {
    if (hwnd_) ShowWindow(hwnd_, SW_RESTORE);
}

void Window::toggleFullscreen() {
    if (!hwnd_) return;
    if (!fullscreen_) {
        pre_fullscreen_ = state();
        GetWindowRect(hwnd_, &restore_rect_);
        MONITORINFO mi{ sizeof(mi) };
        HMONITOR mon = MonitorFromWindow(hwnd_, MONITOR_DEFAULTTONEAREST);
        GetMonitorInfoW(mon, &mi);
        // borderless fullscreen
        LONG_PTR ex = GetWindowLongPtrW(hwnd_, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd_, GWL_STYLE, WS_POPUP | WS_VISIBLE);
        SetWindowPos(hwnd_, HWND_TOP, mi.rcMonitor.left, mi.rcMonitor.top,
                     mi.rcMonitor.right - mi.rcMonitor.left,
                     mi.rcMonitor.bottom - mi.rcMonitor.top,
                     SWP_FRAMECHANGED | SWP_SHOWWINDOW);
        fullscreen_ = true;
    } else {
        SetWindowLongPtrW(hwnd_, GWL_STYLE, WS_OVERLAPPEDWINDOW | WS_VISIBLE);
        SetWindowPos(hwnd_, nullptr, restore_rect_.left, restore_rect_.top,
                     restore_rect_.right - restore_rect_.left,
                     restore_rect_.bottom - restore_rect_.top,
                     SWP_FRAMECHANGED | SWP_SHOWWINDOW);
        fullscreen_ = false;
        if (pre_fullscreen_ == WindowState::Maximized) maximize();
        else if (pre_fullscreen_ == WindowState::Minimized) minimize();
    }
}

bool Window::isFullscreen() const { return fullscreen_; }

void Window::close() {
    if (hwnd_) PostMessageW(hwnd_, WM_CLOSE, 0, 0);
}

void Window::bringToFront() {
    if (hwnd_) {
        SetWindowPos(hwnd_, HWND_TOP, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        SetForegroundWindow(hwnd_);
    }
}

void Window::sendToBack() {
    if (hwnd_) {
        SetWindowPos(hwnd_, HWND_BOTTOM, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

void Window::setTopmost(bool on) {
    if (hwnd_) {
        SetWindowPos(hwnd_, on ? HWND_TOPMOST : HWND_NOTOPMOST, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

void Window::setTitle(const std::wstring& title) {
    config_.title = title;
    if (hwnd_) SetWindowTextW(hwnd_, title.c_str());
}

std::wstring Window::title() const {
    if (!hwnd_) return config_.title;
    int len = GetWindowTextLengthW(hwnd_);
    std::wstring buf(len + 1, L'\0');
    GetWindowTextW(hwnd_, buf.data(), len + 1);
    buf.resize(len);
    return buf;
}

void Window::setIcon(const std::wstring& ico_path) {
    config_.icon_path = ico_path;
    if (!hwnd_ || ico_path.empty()) return;
    HICON icon = static_cast<HICON>(LoadImageW(
        nullptr, ico_path.c_str(), IMAGE_ICON, 0, 0,
        LR_LOADFROMFILE | LR_DEFAULTSIZE));
    if (icon) {
        SendMessageW(hwnd_, WM_SETICON, ICON_BIG, reinterpret_cast<LPARAM>(icon));
        SendMessageW(hwnd_, WM_SETICON, ICON_SMALL, reinterpret_cast<LPARAM>(icon));
    }
}

void Window::setAcceptFileDrop(bool on) {
    accepts_file_drop_ = on;
    if (hwnd_) DragAcceptFiles(hwnd_, on ? TRUE : FALSE);
}

// --- wndproc ---------------------------------------------------------------

LRESULT Window::handleMessage(UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
        case WM_SIZE: {
            int w = LOWORD(lp), h = HIWORD(lp);
            WindowEvent ev{WindowEventType::Resize, id_, 0, 0, w, h, title(), {}};
            // events are emitted through the manager — we need a back-pointer.
            // Simplest: stash the manager in a static map (see sharedWndProc).
            emitViaManager(ev);
            if (wp == SIZE_MINIMIZED) {
                state_ = WindowState::Minimized;
                emitState(WindowEventType::Minimize);
            } else if (wp == SIZE_MAXIMIZED) {
                state_ = WindowState::Maximized;
                emitState(WindowEventType::Maximize);
            } else {
                state_ = WindowState::Normal;
                emitState(WindowEventType::Restore);
            }
            return 0;
        }
        case WM_MOVE: {
            int x = LOWORD(lp), y = HIWORD(lp);
            emitViaManager(WindowEvent{WindowEventType::Move, id_, x, y, 0, 0, title(), {}});
            return 0;
        }
        case WM_CLOSE:
            emitState(WindowEventType::Close);
            DestroyWindow(hwnd_);
            return 0;
        case WM_DESTROY:
            state_ = WindowState::Closed;
            emitState(WindowEventType::Destroy);
            return 0;
        case WM_DROPFILES: {
            HDROP hdrop = reinterpret_cast<HDROP>(wp);
            UINT count = DragQueryFileW(hdrop, 0xFFFFFFFF, nullptr, 0);
            std::vector<std::wstring> files;
            for (UINT i = 0; i < count; ++i) {
                UINT len = DragQueryFileW(hdrop, i, nullptr, 0);
                std::wstring f(len + 1, L'\0');
                DragQueryFileW(hdrop, i, f.data(), len + 1);
                f.resize(len);
                files.push_back(std::move(f));
            }
            DragFinish(hdrop);
            emitViaManager(WindowEvent{WindowEventType::FileDrop, id_, 0, 0, 0, 0,
                                       title(), std::move(files)});
            return 0;
        }
        case WM_ACTIVATE: {
            bool active = (LOWORD(wp) != WA_INACTIVE);
            emitState(active ? WindowEventType::Focus : WindowEventType::Blur);
            return 0;
        }
        case WM_NCLBUTTONDOWN:
            if (wp == HTCAPTION) {
                emitState(WindowEventType::DragStart);
                // Let DefWindowProc handle the actual drag.
                ReleaseCapture();
                SendMessageW(hwnd_, WM_NCLBUTTONDOWN, HTCAPTION, lp);
                emitState(WindowEventType::DragEnd);
                return 0;
            }
            break;
        case WM_SYSCOMMAND:
            if ((wp & 0xFFF0) == SC_RESTORE) emitState(WindowEventType::Restore);
            break;
        case WM_GETMINMAXINFO: {
            auto* mmi = reinterpret_cast<MINMAXINFO*>(lp);
            if (config_.style == WindowStyle::Dialog ||
                config_.type == WindowType::Modal) {
                mmi->ptMaxTrackSize.x = GetSystemMetrics(SM_CXSCREEN);
                mmi->ptMaxTrackSize.y = GetSystemMetrics(SM_CYSCREEN);
            }
            return 0;
        }
        default:
            break;
    }
    return DefWindowProcW(hwnd_, msg, wp, lp);
}

// Event emission helpers — these need the manager; we keep a global
// hwnd→manager registry populated by createWindow().

void Window::emitViaManager(WindowEvent ev) {
    for (auto& e : hwndRegistry()) {
        if (e.hwnd == hwnd_) {
            e.mgr->pushEvent(std::move(ev));
            return;
        }
    }
}

void Window::emitState(WindowEventType t) {
    WindowMetrics m = metrics();
    emitViaManager(WindowEvent{t, id_, m.x, m.y, m.width, m.height, title(), {}});
}

}  // namespace xholy::win32
