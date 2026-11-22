# xHoly Win32 Window Manager

Native C++17 windowing infrastructure for xHoly, built **only** on the
Win32 API (`user32`, `gdi32`, `shell32`). No Qt/wx/GLFW — this is a
window *manager*, not a UI toolkit: it creates, tracks, and dispatches
windows; rendering is a separate concern.

## Architecture

```
WindowManager
├── owns Window instances (HWND wrappers)
├── shared WndProc → routes messages to the right Window
├── hwnd→Window registry for message dispatch
├── event queue + optional EventCallback
└── pumpMessages()/waitMessage() message pump

Window
├── geometry: moveTo / resize / setBounds / metrics
├── state: show/hide/minimize/maximize/restore/fullscreen/close
├── z-order: bringToFront / sendToBack / setTopmost
├── content: setTitle / setIcon (.ico via LoadImage)
└── DnD: setAcceptFileDrop (WM_DROPFILES) + caption drag events
```

## Build

```powershell
cd cpp/window_manager
cmake -B build -G Ninja
cmake --build build
build\window_manager_tests.exe
```

Requires MSVC Build Tools (any recent VS). CMake picks the compiler
automatically; on this machine VS 2019/18 BuildTools are installed.

## Usage

```cpp
#include "window_manager.h"
using namespace xholy::win32;

int WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR, int) {
    WindowManager mgr;
    mgr.setEventCallback([](const WindowEvent& ev) {
        // ev.type: Create/Destroy/Resize/Move/Minimize/.../FileDrop
    });

    WindowConfig cfg;
    cfg.title = L"xHoly";
    cfg.width = 1280; cfg.height = 720;
    cfg.icon_path = L"assets/icons/xholy.ico";
    cfg.accept_file_drop = true;

    Window* win = mgr.createWindow(cfg);

    while (mgr.pumpMessages()) {
        // render frame here (separate module)
    }
    return 0;
}
```

## Features

| Feature | API |
|---|---|
| Create/destroy | `createWindow`, `destroyWindow`, `destroyAll` |
| Geometry | `moveTo`, `resize`, `setBounds`, `metrics` |
| State | `minimize`, `maximize`, `restore`, `toggleFullscreen`, `close` |
| Z-order | `bringToFront`, `sendToBack`, `setTopmost` |
| Child windows | `WindowConfig::parent` |
| Owned popups | `WindowConfig::owner` + `WindowType` |
| Icons | `WindowConfig::icon_path`, `Window::setIcon` |
| Styles | `WindowStyle` (Default/Minimal/Titlebar/Bordered/Popup/Dialog), `WindowType` (Normal/Toolbar/Tooltip/ContextMenu/Dialog/Popup/Modal) |
| File drop | `accept_file_drop` → `WindowEventType::FileDrop` with paths |
| Translucency | `translucent` → `WS_EX_LAYERED` + alpha |
| Taskbar | `show_in_taskbar` (false → tool window) |
| Events | `WindowEventType` + `drainEvents()` / callback |
| Queries | `findById/Title/Hwnd`, `windowAtPoint/State/Type/Style` |

## Notes & limitations

- **Drag & drop** covers OS file drops and caption-drag events
  (`DragStart`/`DragEnd`). OLE `IDropTarget` (arbitrary in-app drag
  sources) is not implemented — add it if needed.
- **HiDPI**: the test binary doesn't set a manifest; for per-monitor
  DPI awareness add `SetProcessDpiAwarenessContext` or a manifest.
- **Multi-window**: all windows share one WndProc; closing one window
  does not quit the app (no `PostQuitMessage` per-window).
- The `Window*` returned by `createWindow` is owned by the manager —
  never `delete` it; call `destroyWindow`.
