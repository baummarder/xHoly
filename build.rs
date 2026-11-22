//! Build script: compile the C++ Win32 window manager for FFI.
//!
//! On Windows we compile `cpp/window_manager/*.cpp` into a static lib
//! via the `cc` crate and link it into the Rust binary. On other
//! targets the module is a no-op (the FFI bindings are cfg-gated).

fn main() {
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=cpp/window_manager/window_manager.cpp");
        println!("cargo:rerun-if-changed=cpp/window_manager/window_manager_ffi.cpp");
        println!("cargo:rerun-if-changed=cpp/window_manager/window_manager.h");
        println!("cargo:rerun-if-changed=cpp/window_manager/window_manager_ffi.h");

        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .file("cpp/window_manager/window_manager.cpp")
            .file("cpp/window_manager/window_manager_ffi.cpp")
            .include("cpp/window_manager")
            .define("UNICODE", None)
            .define("_UNICODE", None)
            .define("WIN32_LEAN_AND_MEAN", None)
            .define("NOMINMAX", None)
            .compile("xholy_wm");

        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=gdi32");
        println!("cargo:rustc-link-lib=shell32");
    }
}
