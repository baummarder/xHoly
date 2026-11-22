// xHoly Win32 Window Manager — native Windows windowing infrastructure.
//
// Pure Win32 (user32/gdi32/shell32). No external GUI framework.
// This is window *management* only — rendering lives elsewhere.

#pragma once

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <vector>

namespace xholy::win32 {

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Lifecycle/visual state of a managed window.
enum class WindowState {
    Normal,
    Minimized,
    Maximized,
    Fullscreen,
    Closed,
};

/// Predefined window chrome styles.
enum class WindowStyle {
    Default,    // standard overlapped window
    Minimal,    // thin frame, no minimize/maximize buttons
    Titlebar,   // caption + close only
    Bordered,   // border only, no caption
    Popup,      // WS_POPUP, no chrome at all
    Dialog,     // dialog-ish: fixed, centered-ish
};

/// Semantic window type (tool windows, popups, modals, ...).
enum class WindowType {
    Normal,
    Toolbar,
    Tooltip,
    ContextMenu,
    Dialog,
    Popup,
    Modal,
};

/// Window events emitted to the manager's event queue/callback.
enum class WindowEventType {
    Create,
    Destroy,
    Resize,
    Move,
    Minimize,
    Maximize,
    Restore,
    FullscreenEnter,
    FullscreenExit,
    Close,
    DragStart,   // user began moving the window by its caption
    DragEnd,     // user finished moving
    FileDrop,    // OS file(s) dropped onto the window
    Focus,
    Blur,
};

/// A single window event.
struct WindowEvent {
    WindowEventType type;
    uint64_t window_id;   // Window::id()
    int32_t x = 0;
    int32_t y = 0;
    int32_t width = 0;
    int32_t height = 0;
    std::wstring title;
    std::vector<std::wstring> files;  // FileDrop payload
};

// ---------------------------------------------------------------------------
// Configuration & metrics
// ---------------------------------------------------------------------------

/// Configuration for creating a window.
struct WindowConfig {
    std::wstring title = L"xHoly";
    int32_t x = CW_USEDEFAULT;
    int32_t y = CW_USEDEFAULT;
    int32_t width = 1280;
    int32_t height = 720;
    WindowStyle style = WindowStyle::Default;
    WindowType type = WindowType::Normal;
    WindowState initial_state = WindowState::Normal;

    /// Parent window (0 = top-level). Child windows are clipped to it.
    HWND parent = nullptr;
    /// Owner window for owned popups/tool windows.
    HWND owner = nullptr;

    /// .ico path for the title-bar/taskbar icon (empty = default).
    std::wstring icon_path;

    /// Start fullscreen (borderless, covers the monitor).
    bool fullscreen = false;
    /// Layered/translucent window (requires an opaque-ish paint path).
    bool translucent = false;
    /// Accept OS file drops (DragAcceptFiles + WM_DROPFILES).
    bool accept_file_drop = false;
    /// Show in taskbar & alt-tab (false for tool windows).
    bool show_in_taskbar = true;
    /// Accept keyboard focus.
    bool focusable = true;
    /// Start hidden.
    bool visible = true;
};

/// Snapshot of a window's geometry & z-order.
struct WindowMetrics {
    int32_t x = 0;
    int32_t y = 0;
    int32_t width = 0;
    int32_t height = 0;
    int32_t client_width = 0;
    int32_t client_height = 0;
    int32_t z_order = 0;   // 0 = topmost
    bool visible = false;
    bool enabled = true;
    bool focused = false;
    bool iconic = false;   // minimized
    bool maximized = false;
    bool fullscreen = false;
};

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

/// A managed Win32 window. Not copyable; owned by WindowManager.
class Window {
public:
    ~Window();

    Window(const Window&) = delete;
    Window& operator=(const Window&) = delete;

    uint64_t id() const { return id_; }
    HWND hwnd() const { return hwnd_; }
    const WindowConfig& config() const { return config_; }
    WindowType type() const { return config_.type; }
    WindowStyle style() const { return config_.style; }
    WindowState state() const;

    // --- geometry -----------------------------------------------------------
    void moveTo(int32_t x, int32_t y);
    void resize(int32_t w, int32_t h);
    void setBounds(int32_t x, int32_t y, int32_t w, int32_t h);
    WindowMetrics metrics() const;

    // --- state --------------------------------------------------------------
    void show();
    void hide();
    bool isVisible() const;
    void enable(bool on = true);
    void minimize();
    void maximize();
    void restore();
    void toggleFullscreen();
    bool isFullscreen() const;
    void close();   // posts WM_CLOSE; 'closed' once the wndproc sees it

    // --- z-order ------------------------------------------------------------
    void bringToFront();
    void sendToBack();
    void setTopmost(bool on);

    // --- content ------------------------------------------------------------
    void setTitle(const std::wstring& title);
    std::wstring title() const;
    void setIcon(const std::wstring& ico_path);

    // --- drag & drop --------------------------------------------------------
    void setAcceptFileDrop(bool on);
    bool acceptsFileDrop() const { return accepts_file_drop_; }

private:
    friend class WindowManager;
    Window(uint64_t id, HWND hwnd, WindowConfig cfg);

    LRESULT handleMessage(UINT msg, WPARAM wp, LPARAM lp);
    void emitViaManager(WindowEvent ev);
    void emitState(WindowEventType t);

    uint64_t id_;
    HWND hwnd_ = nullptr;
    WindowConfig config_;
    WindowState state_ = WindowState::Normal;
    WindowState pre_fullscreen_ = WindowState::Normal;
    bool fullscreen_ = false;
    bool accepts_file_drop_ = false;
    RECT restore_rect_{};
};

// ---------------------------------------------------------------------------
// WindowManager
// ---------------------------------------------------------------------------

using EventCallback = std::function<void(const WindowEvent&)>;

/// Owns all managed windows, the shared WndProc, and the event queue.
class WindowManager {
public:
    WindowManager();
    ~WindowManager();

    WindowManager(const WindowManager&) = delete;
    WindowManager& operator=(const WindowManager&) = delete;

    /// Create + register a window. Returns nullptr on failure.
    /// The returned pointer is owned by the manager — do NOT delete it;
    /// use destroyWindow() to release it.
    Window* createWindow(const WindowConfig& cfg);

    /// Destroy a window immediately.
    void destroyWindow(Window* w);
    void destroyAll();

    /// Message pump — dispatch all pending messages. Call every frame.
    /// Returns false when WM_QUIT was seen (app should exit).
    bool pumpMessages();
    /// Blocking variant of pumpMessages.
    bool waitMessage();

    // --- queries ------------------------------------------------------------
    size_t windowCount() const { return windows_.size(); }
    std::vector<Window*> windows();
    Window* findById(uint64_t id);
    Window* findByTitle(const std::wstring& title);
    Window* findByHwnd(HWND hwnd);
    Window* windowAtPoint(int32_t x, int32_t y);   // topmost at coords
    Window* windowAtState(WindowState s);
    Window* windowAtType(WindowType t);
    Window* windowAtStyle(WindowStyle s);

    // --- events -------------------------------------------------------------
    void setEventCallback(EventCallback cb) { on_event_ = std::move(cb); }
    std::vector<WindowEvent> drainEvents();
    void pushEvent(WindowEvent ev);   // internal + test hook

    // --- Win32 style mapping (exposed for unit tests) -----------------------
    static DWORD toWin32Style(WindowStyle s, WindowType t);
    static DWORD toWin32ExStyle(const WindowConfig& cfg);

private:
    static LRESULT CALLBACK sharedWndProc(HWND, UINT, WPARAM, LPARAM);
    void emit(WindowEvent ev);
    void registerWindowClass();
    void unregisterWindowClass();

    std::vector<std::unique_ptr<Window>> windows_;
    std::vector<WindowEvent> events_;
    EventCallback on_event_;
    ATOM class_atom_ = 0;
    static const wchar_t* kClassName;
};

}  // namespace xholy::win32
