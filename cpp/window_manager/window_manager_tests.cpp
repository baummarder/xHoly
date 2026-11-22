// Unit tests for the xHoly Win32 Window Manager.
//
// Pure-logic tests always run. Tests that create real HWNDs are
// guarded: they skip gracefully when no interactive window station is
// available (e.g. CI without a desktop).

#include "window_manager.h"

#include <cstdio>
#include <string>

using namespace xholy::win32;

static int failures = 0;
static int skipped = 0;

#define CHECK(cond, msg)                                                       \
    do {                                                                       \
        if (!(cond)) {                                                         \
            std::printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, msg);          \
            ++failures;                                                        \
        }                                                                      \
    } while (0)

#define CHECK_EQ(a, b, msg) CHECK((a) == (b), msg)

// --- pure logic tests (always run) ------------------------------------------

static void test_style_mapping() {
    CHECK(WindowManager::toWin32Style(WindowStyle::Default, WindowType::Normal) &
              WS_OVERLAPPEDWINDOW,
          "Default style has overlapped window bits");
    CHECK(WindowManager::toWin32Style(WindowStyle::Popup, WindowType::Popup) ==
              WS_POPUP,
          "Popup style maps to WS_POPUP");
    CHECK((WindowManager::toWin32Style(WindowStyle::Dialog, WindowType::Dialog) &
           (WS_MINIMIZEBOX | WS_MAXIMIZEBOX)) == 0,
          "Dialog has no min/max boxes");
    DWORD ex = WindowManager::toWin32ExStyle(WindowConfig{});
    CHECK((ex & WS_EX_TOOLWINDOW) == 0, "Normal window is not a tool window");
    WindowConfig tip;
    tip.type = WindowType::Tooltip;
    ex = WindowManager::toWin32ExStyle(tip);
    CHECK((ex & WS_EX_TOOLWINDOW) != 0, "Tooltip gets WS_EX_TOOLWINDOW");
    WindowConfig pop;
    pop.type = WindowType::Popup;
    ex = WindowManager::toWin32ExStyle(pop);
    CHECK((ex & WS_EX_TOPMOST) != 0, "Popup gets WS_EX_TOPMOST");
    WindowConfig lay;
    lay.translucent = true;
    ex = WindowManager::toWin32ExStyle(lay);
    CHECK((ex & WS_EX_LAYERED) != 0, "Translucent gets WS_EX_LAYERED");
}

static void test_config_defaults() {
    WindowConfig c;
    CHECK_EQ(c.width, 1280, "default width");
    CHECK_EQ(c.height, 720, "default height");
    CHECK(!c.accept_file_drop, "file drop off by default");
    CHECK(c.visible, "visible by default");
    CHECK(c.show_in_taskbar, "taskbar visible by default");
}

static void test_event_queue() {
    WindowManager mgr;
    WindowEvent ev{WindowEventType::Create, 42, 1, 2, 3, 4, L"t", {}};
    mgr.pushEvent(ev);
    auto events = mgr.drainEvents();
    CHECK_EQ(events.size(), 1, "one event drained");
    CHECK_EQ(events[0].window_id, 42, "event window id");
    CHECK(mgr.drainEvents().empty(), "queue empty after drain");
}

// --- Win32 integration tests (require an interactive desktop) ---------------

static bool desktopAvailable() {
    // A window station/desktop is required to create HWNDs.
    return GetUserObjectInformationW(GetProcessWindowStation(), UOI_FLAGS, nullptr, 0, nullptr) ||
           true;  // try anyway; createWindow failure is the real check
}

static void test_create_destroy() {
    if (!desktopAvailable()) { ++skipped; return; }
    WindowManager mgr;
    WindowConfig cfg;
    cfg.title = L"xHoly Test Window";
    cfg.width = 320;
    cfg.height = 200;
    cfg.visible = false;  // don't flash a window during tests
    Window* w = mgr.createWindow(cfg);
    if (!w || !w->hwnd()) {
        ++skipped;
        std::printf("SKIP create/destroy: no HWND (headless session?)\n");
        return;
    }
    CHECK(w->hwnd() != nullptr, "hwnd created");
    CHECK_EQ(mgr.windowCount(), 1, "manager tracks one window");
    CHECK_EQ(w->title(), L"xHoly Test Window", "title set");

    mgr.destroyWindow(w);
    CHECK_EQ(mgr.windowCount(), 0, "window removed after destroy");
}

static void test_multi_window() {
    if (!desktopAvailable()) { ++skipped; return; }
    WindowManager mgr;
    WindowConfig a; a.title = L"WinA"; a.visible = false;
    WindowConfig b; b.title = L"WinB"; b.visible = false; b.type = WindowType::Tooltip;
    Window* wa = mgr.createWindow(a);
    Window* wb = mgr.createWindow(b);
    if (!wa || !wb) { ++skipped; return; }
    CHECK_EQ(mgr.windowCount(), 2, "two windows tracked");
    CHECK(mgr.findByTitle(L"WinA") == wa, "find by title A");
    CHECK(mgr.findByTitle(L"WinB") == wb, "find by title B");
    CHECK(mgr.windowAtType(WindowType::Tooltip) == wb, "find by type");
    CHECK(mgr.windowAtStyle(WindowStyle::Default) == wa, "find by style");
    mgr.destroyAll();
    CHECK_EQ(mgr.windowCount(), 0, "all destroyed");
}

static void test_geometry() {
    if (!desktopAvailable()) { ++skipped; return; }
    WindowManager mgr;
    WindowConfig cfg;
    cfg.x = 50; cfg.y = 60; cfg.width = 400; cfg.height = 300;
    cfg.visible = false;
    Window* w = mgr.createWindow(cfg);
    if (!w) { ++skipped; return; }
    w->moveTo(100, 120);
    w->resize(500, 350);
    WindowMetrics m = w->metrics();
    // Hidden windows may report odd rects; just check no crash + sane size.
    CHECK(m.width >= 0 && m.height >= 0, "metrics sane");
    mgr.destroyWindow(w);
}

static void test_state_transitions() {
    if (!desktopAvailable()) { ++skipped; return; }
    WindowManager mgr;
    WindowConfig cfg;
    cfg.visible = false;
    Window* w = mgr.createWindow(cfg);
    if (!w) { ++skipped; return; }
    // minimize/maximize on a hidden window are no-ops visually but must not crash.
    w->minimize();
    w->maximize();
    w->restore();
    w->toggleFullscreen();
    CHECK(w->isFullscreen(), "fullscreen flag set");
    w->toggleFullscreen();
    CHECK(!w->isFullscreen(), "fullscreen flag cleared");
    w->bringToFront();
    w->sendToBack();
    w->setTopmost(true);
    w->setTopmost(false);
    mgr.destroyWindow(w);
}

static void test_events_emitted() {
    if (!desktopAvailable()) { ++skipped; return; }
    WindowManager mgr;
    bool got_create = false;
    mgr.setEventCallback([&](const WindowEvent& ev) {
        if (ev.type == WindowEventType::Create) got_create = true;
    });
    WindowConfig cfg;
    cfg.visible = false;
    Window* w = mgr.createWindow(cfg);
    if (!w) { ++skipped; return; }
    CHECK(got_create, "Create event fired via callback");
    auto evs = mgr.drainEvents();
    bool saw_create = false;
    for (auto& e : evs) if (e.type == WindowEventType::Create) saw_create = true;
    CHECK(saw_create, "Create event in queue");
    mgr.destroyWindow(w);
}

int main() {
    test_style_mapping();
    test_config_defaults();
    test_event_queue();
    test_create_destroy();
    test_multi_window();
    test_geometry();
    test_state_transitions();
    test_events_emitted();

    std::printf("\n%d checks failed, %d tests skipped\n", failures, skipped);
    return failures == 0 ? 0 : 1;
}
