//! Kernel console — Phase 10.
//!
//! Buffered debug output (`KPrint`/`KLog`) separate from the guest
//! console aperture: kernel messages go here, guest `SYS_WRITE`
//! output goes to `cpu.output`. `KernelPanic` latches a fatal flag
//! the runner can inspect.

/// Kernel console state.
#[derive(Debug, Default)]
pub struct KernelConsole {
    /// Buffered log lines.
    pub lines: Vec<String>,
    /// Set on `panic()` — execution should halt.
    pub panic: Option<String>,
    /// Total bytes written (diagnostics).
    pub bytes_written: u64,
}

impl KernelConsole {
    /// Append a formatted line.
    pub fn kprint(&mut self, args: std::fmt::Arguments) {
        let line = args.to_string();
        self.bytes_written += line.len() as u64 + 1;
        self.lines.push(line);
    }

    /// `kprintf`-style: `console.kprintf(format_args!(...))`.
    pub fn klog(&mut self, msg: impl Into<String>) {
        let line = msg.into();
        self.bytes_written += line.len() as u64 + 1;
        self.lines.push(line);
    }

    /// Trigger a kernel panic (halts the guest).
    pub fn panic(&mut self, msg: impl Into<String>) {
        let line = format!("KERNEL PANIC: {}", msg.into());
        self.lines.push(line.clone());
        self.panic = Some(line);
    }

    /// Whether a panic has been raised.
    pub fn panicked(&self) -> bool {
        self.panic.is_some()
    }
}

/// Convenience macro mirroring `kprintf!`.
#[macro_export]
macro_rules! kprintf {
    ($console:expr, $($arg:tt)*) => {
        $console.kprint(format_args!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_buffers_and_panics() {
        let mut c = KernelConsole::default();
        c.klog("boot ok");
        c.kprint(format_args!("ticks={}", 42));
        assert_eq!(c.lines.len(), 2);
        assert!(!c.panicked());
        c.panic("bad syscall");
        assert!(c.panicked());
        assert!(c.panic.unwrap().contains("bad syscall"));
    }
}
