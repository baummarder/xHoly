//! Native x86-64 guest execution (Phase 25 foundation).
//!
//! Allocates a small executable buffer via the host OS (VirtualAlloc on
//! Windows, mmap on Linux) so a *synthetic* guest stub can be called as a
//! normal function pointer. This is not a full PE64 execution engine — it
//! is the shortest honest path to "guest instructions really ran".

use std::ffi::c_void;

/// Page protection: read + write + execute.
pub struct ExecutableMemory {
    ptr: *mut u8,
    len: usize,
}

impl ExecutableMemory {
    /// Allocate `len` bytes of RWX memory.
    pub fn new(len: usize) -> Result<Self, String> {
        if len == 0 {
            return Err("zero-length executable buffer".into());
        }
        #[cfg(target_os = "windows")]
        let ptr = unsafe {
            const MEM_COMMIT: u32 = 0x1000;
            const MEM_RESERVE: u32 = 0x2000;
            const PAGE_EXECUTE_READWRITE: u32 = 0x40;
            extern "system" {
                fn VirtualAlloc(
                    lp_address: *mut c_void,
                    dw_size: usize,
                    fl_allocation_type: u32,
                    fl_protect: u32,
                ) -> *mut c_void;
            }
            VirtualAlloc(
                std::ptr::null_mut(),
                len,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            ) as *mut u8
        };
        #[cfg(target_os = "linux")]
        let ptr = unsafe {
            const PROT_READ: i32 = 1;
            const PROT_WRITE: i32 = 2;
            const PROT_EXEC: i32 = 4;
            const MAP_PRIVATE: i32 = 2;
            const MAP_ANONYMOUS: i32 = 0x20;
            extern "C" {
                fn mmap(
                    addr: *mut c_void,
                    len: usize,
                    prot: i32,
                    flags: i32,
                    fd: i32,
                    offset: i64,
                ) -> *mut c_void;
            }
            mmap(
                std::ptr::null_mut(),
                len,
                PROT_READ | PROT_WRITE | PROT_EXEC,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            ) as *mut u8
        };
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let ptr = std::ptr::null_mut();

        if ptr.is_null() {
            return Err("could not allocate executable memory".into());
        }
        Ok(Self { ptr, len })
    }

    /// Copy guest bytes into the executable buffer.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > self.len {
            return Err("guest stub exceeds executable buffer".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr, bytes.len());
        }
        Ok(())
    }

    /// Call the first byte as `extern "C" fn() -> u64`.
    ///
    /// # Safety
    /// The buffer must contain valid x86-64 machine code for the host ABI.
    pub unsafe fn call(&self) -> u64 {
        let function: extern "C" fn() -> u64 = std::mem::transmute(self.ptr);
        function()
    }

    /// Call the stub with one HLE callback pointer (host x86-64 ABI).
    ///
    /// # Safety
    /// The buffer must contain code that calls the first-argument register.
    pub unsafe fn call_with(&self, callback: extern "C" fn() -> u64) -> u64 {
        let function: extern "C" fn(extern "C" fn() -> u64) -> u64 = std::mem::transmute(self.ptr);
        function(callback)
    }
}

impl Drop for ExecutableMemory {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        unsafe {
            const MEM_RELEASE: u32 = 0x8000;
            extern "system" {
                fn VirtualFree(lp_address: *mut c_void, dw_size: usize, dw_free_type: u32) -> i32;
            }
            let _ = VirtualFree(self.ptr as *mut c_void, 0, MEM_RELEASE);
        }
        #[cfg(target_os = "linux")]
        unsafe {
            extern "C" {
                fn munmap(addr: *mut c_void, len: usize) -> i32;
            }
            let _ = munmap(self.ptr as *mut c_void, self.len);
        }
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    #[test]
    fn guest_code_runs_natively() {
        let mut memory = ExecutableMemory::new(64).expect("RWX allocation");
        // mov eax, 42 ; ret
        memory.write(&[0xB8, 42, 0, 0, 0, 0xC3]).unwrap();
        assert_eq!(unsafe { memory.call() }, 42);
    }

    #[test]
    fn oversized_stub_is_rejected_without_panic() {
        let mut memory = ExecutableMemory::new(4).unwrap();
        assert!(memory.write(&[0u8; 8]).is_err());
    }
}
