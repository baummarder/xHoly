// Minimal demo: creates two managed windows and pumps messages until
// both are closed. Demonstrates the WindowManager API end-to-end.

#include "window_manager.h"

#include <cstdio>

using namespace xholy::win32;

int WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR, int) {
    // Per-monitor DPI awareness (v2) so coordinates aren't scaled.
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

    WindowManager mgr;

    int events_seen = 0;
    mgr.setEventCallback([&](const WindowEvent& ev) {
        ++events_seen;
        if (ev.type == WindowEventType::Close) {
            std::wprintf(L"[event] Close id=%llu\n",
                         static_cast<unsigned long long>(ev.window_id));
        }
    });

    WindowConfig main_cfg;
    main_cfg.title = L"xHoly Window Manager";
    main_cfg.width = 960;
    main_cfg.height = 540;
    main_cfg.x = 100;
    main_cfg.y = 100;
    main_cfg.accept_file_drop = true;

    WindowConfig child_cfg;
    child_cfg.title = L"xHoly Child (tooltip-style)";
    child_cfg.type = WindowType::Tooltip;
    child_cfg.width = 320;
    child_cfg.height = 120;
    child_cfg.x = 150;
    child_cfg.y = 150;

    Window* main_win = mgr.createWindow(main_cfg);
    Window* child_win = nullptr;
    if (main_win) {
        child_cfg.parent = main_win->hwnd();
        child_win = mgr.createWindow(child_cfg);
    }

    if (!main_win) {
        std::printf("Failed to create main window.\n");
        return 1;
    }

    std::printf("xHoly WM: %zu window(s) managed, %d events so far.\n",
                mgr.windowCount(), events_seen);

    while (mgr.pumpMessages()) {
        // A real app would render here; demo just idles on the pump.
    }

    mgr.destroyAll();
    std::printf("xHoly WM: shutdown clean, %d events total.\n", events_seen);
    return 0;
}
