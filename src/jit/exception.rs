//! Exception handling for the JIT runtime — Phase 13.
//!
//! WASM has no native exceptions in MVP; we model guest exceptions as
//! frames on an explicit stack. A `syscall`/`trap` unwinds to the
//! nearest handler registered via [`ExceptionHandler::push`]; with no
//! handler the runtime returns [`JitError::Exception`].

use super::{JitError, JitResult};

/// One exception frame: where to resume + saved context pointer.
#[derive(Debug, Clone, Copy)]
pub struct ExceptionFrame {
    /// Guest PC (byte offset) to resume at, or `u64::MAX` for exit.
    pub resume_pc: u64,
    /// Saved guest stack pointer.
    pub saved_sp: u64,
}

/// Exception handler stack.
#[derive(Debug, Default)]
pub struct ExceptionHandler {
    frames: Vec<ExceptionFrame>,
}

impl ExceptionHandler {
    /// New empty handler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Install a handler frame.
    pub fn push(&mut self, frame: ExceptionFrame) {
        self.frames.push(frame);
    }

    /// Remove and return the top frame (None = no handler → fatal).
    pub fn pop(&mut self) -> Option<ExceptionFrame> {
        self.frames.pop()
    }

    /// Depth of the handler stack.
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Raise an exception: pops the nearest handler.
    /// Returns `Err(JitError::Exception)` when nothing is registered.
    pub fn raise(&mut self, code: u64, msg: &str) -> JitResult<ExceptionFrame> {
        self.pop()
            .ok_or_else(|| JitError::Exception(code, msg.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwind_to_handler() {
        let mut h = ExceptionHandler::new();
        h.push(ExceptionFrame {
            resume_pc: 0x1000,
            saved_sp: 0x7FFF_FFF0,
        });
        let f = h.raise(1, "test").unwrap();
        assert_eq!(f.resume_pc, 0x1000);
        assert!(h.raise(2, "again").is_err());
    }
}
