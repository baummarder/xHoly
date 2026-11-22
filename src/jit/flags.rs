//! x86-64 EFLAGS model for the JIT — Phase 14/15.
//!
//! The compiler keeps flags in mutable `i32` WASM globals
//! (indices 16–21); [`JitFlags`] is the CPU-side mirror used by the
//! interpreter fallback, tests, and debugging. Bit positions follow
//! the architectural EFLAGS layout for the tracked subset:
//!
//! | Flag | Bit | Set by |
//! |------|-----|--------|
//! | CF   | 0   | add/sub/adc/sbb/neg/shifts |
//! | PF   | 2   | all flag-writing ALU ops (parity of low byte) |
//! | AF   | 4   | add/sub/adc/sbb/inc/dec/neg |
//! | ZF   | 6   | all flag-writing ALU ops |
//! | SF   | 7   | all flag-writing ALU ops |
//! | OF   | 11  | add/sub/adc/sbb/inc/dec/neg |
//!
//! PF is the parity flag: set when the low byte of the result has
//! *even* parity (even number of set bits).

use super::JitResult;

/// EFLAGS bits tracked by the JIT.
pub mod bits {
    pub const CF: u16 = 1 << 0;
    pub const PF: u16 = 1 << 2;
    pub const AF: u16 = 1 << 4;
    pub const ZF: u16 = 1 << 6;
    pub const SF: u16 = 1 << 7;
    pub const OF: u16 = 1 << 11;
}

/// Tracked flag state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JitFlags {
    /// Raw flag bits (only the tracked positions are meaningful).
    pub bits: u16,
}

impl JitFlags {
    /// All flags clear.
    pub fn new() -> Self {
        Self { bits: 0 }
    }

    /// Flags with ZF set (result was zero).
    pub fn zero_flag_set() -> Self {
        Self { bits: bits::ZF }
    }

    /// Set PF (even parity of low result byte).
    pub fn set_pf(&mut self) {
        self.bits |= bits::PF;
    }

    /// Clear PF.
    pub fn clear_pf(&mut self) {
        self.bits &= !bits::PF;
    }

    /// Set CF.
    pub fn set_cf(&mut self) {
        self.bits |= bits::CF;
    }

    /// Clear CF.
    pub fn clear_cf(&mut self) {
        self.bits &= !bits::CF;
    }

    /// Set OF.
    pub fn set_of(&mut self) {
        self.bits |= bits::OF;
    }

    /// Clear OF.
    pub fn clear_of(&mut self) {
        self.bits &= !bits::OF;
    }

    /// Set AF.
    pub fn set_af(&mut self) {
        self.bits |= bits::AF;
    }

    /// Clear AF.
    pub fn clear_af(&mut self) {
        self.bits &= !bits::AF;
    }

    /// Set ZF.
    pub fn set_zf(&mut self) {
        self.bits |= bits::ZF;
    }

    /// Clear ZF.
    pub fn clear_zf(&mut self) {
        self.bits &= !bits::ZF;
    }

    /// Set SF.
    pub fn set_sf(&mut self) {
        self.bits |= bits::SF;
    }

    /// Clear SF.
    pub fn clear_sf(&mut self) {
        self.bits &= !bits::SF;
    }

    /// Test a single flag bit.
    pub fn contains(&self, bit: u16) -> bool {
        self.bits & bit != 0
    }

    /// Parity of `v`'s low byte is even → PF should be set.
    pub fn parity_even(v: u64) -> bool {
        (v as u8).count_ones().is_multiple_of(2)
    }

    /// Update ZF/SF/PF from an ALU result `res`.
    /// Preserves CF/OF/AF (caller decides whether they change).
    pub fn update_szp(&mut self, res: u64) {
        if res == 0 {
            self.set_zf();
        } else {
            self.clear_zf();
        }
        if (res as i64) < 0 {
            self.set_sf();
        } else {
            self.clear_sf();
        }
        if Self::parity_even(res) {
            self.set_pf();
        } else {
            self.clear_pf();
        }
    }

    /// AF for add/sub-family ops: bit 4 carry/borrow between nibbles.
    pub fn aux_carry_add(a: u64, b: u64, res: u64) -> bool {
        ((a ^ b ^ res) >> 4) & 1 == 1
    }

    /// Convert to the 6-`i32`-global encoding used by the WASM
    /// runtime: returns `[zf, sf, cf, of, pf, af]` as 0/1.
    pub fn to_globals(&self) -> [i32; 6] {
        [
            self.contains(bits::ZF) as i32,
            self.contains(bits::SF) as i32,
            self.contains(bits::CF) as i32,
            self.contains(bits::OF) as i32,
            self.contains(bits::PF) as i32,
            self.contains(bits::AF) as i32,
        ]
    }

    /// Build from the 6-global encoding (inverse of [`Self::to_globals`]).
    pub fn from_globals(g: &[i32]) -> JitResult<Self> {
        if g.len() < 6 {
            return Err(super::JitError::Runtime(
                "flags global slice too short".into(),
            ));
        }
        let mut f = Self::new();
        for (i, bit) in [bits::ZF, bits::SF, bits::CF, bits::OF, bits::PF, bits::AF]
            .iter()
            .enumerate()
        {
            if g[i] != 0 {
                f.bits |= *bit;
            }
        }
        Ok(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_flag_set_helper() {
        let f = JitFlags::zero_flag_set();
        assert!(f.contains(bits::ZF));
        assert!(!f.contains(bits::CF));
    }

    #[test]
    fn parity_even_detection() {
        assert!(JitFlags::parity_even(0x00)); // 0 bits → even
        assert!(JitFlags::parity_even(0x03)); // 2 bits → even
        assert!(!JitFlags::parity_even(0x01)); // 1 bit → odd
        assert!(JitFlags::parity_even(0xFF & 0x0F)); // low byte 0x0F → 4 bits even
    }

    #[test]
    fn update_szp_sets_zf_sf_pf() {
        let mut f = JitFlags::new();
        f.update_szp(0);
        assert!(f.contains(bits::ZF) && f.contains(bits::PF));
        assert!(!f.contains(bits::SF));
        f.update_szp(u64::MAX);
        assert!(f.contains(bits::SF));
        assert!(!f.contains(bits::ZF));
    }

    #[test]
    fn aux_carry_detects_nibble_carry() {
        // 0x0F + 0x01 = 0x10 → carry out of bit 3? AF = bit4 of (a^b^res)
        assert!(JitFlags::aux_carry_add(0x0F, 0x01, 0x10));
        assert!(!JitFlags::aux_carry_add(0x01, 0x02, 0x03));
    }

    #[test]
    fn globals_roundtrip() {
        let mut f = JitFlags::new();
        f.set_cf();
        f.set_pf();
        f.set_of();
        let g = f.to_globals();
        let back = JitFlags::from_globals(&g).unwrap();
        assert_eq!(back.bits, f.bits);
    }
}
