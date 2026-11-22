//! Linear-scan register allocator — Phase 13.
//!
//! Assigns virtual registers (live intervals) to a fixed set of
//! physical slots ("colors"). When all colors are busy inside an
//! interval, the virtual register spills to a dedicated spill slot.
//!
//! In the JIT this allocates WASM locals for IR temporaries: colors
//! 0..N map to cheap locals, spill slots map to locals allocated
//! after the color range.

/// A live interval [start, end) in instruction indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub vreg: u32,
    pub start: u32,
    pub end: u32,
}

/// Allocation result for one vreg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alloc {
    /// Assigned to physical slot `color`.
    Reg(u32),
    /// Spilled to spill slot `index`.
    Spill(u32),
}

/// Linear-scan allocator.
#[derive(Debug)]
pub struct Allocator {
    /// Number of physical colors (registers/locals).
    pub n_colors: usize,
    /// Current allocations being built.
    active: Vec<(u32, u32, u32)>, // (end, vreg, color)
    spill_count: u32,
}

impl Allocator {
    /// New allocator with `n_colors` physical slots.
    pub fn new(n_colors: usize) -> Self {
        Self {
            n_colors,
            active: Vec::new(),
            spill_count: 0,
        }
    }

    /// Total locals needed: colors + spill slots.
    pub fn total_slots(&self) -> u32 {
        self.n_colors as u32 + self.spill_count
    }

    /// Allocate all intervals (sorted by start). Returns vreg → Alloc.
    pub fn allocate(&mut self, intervals: &[Interval]) -> Vec<(u32, Alloc)> {
        let mut sorted: Vec<Interval> = intervals.to_vec();
        sorted.sort_by_key(|i| (i.start, i.end));
        let mut out = Vec::with_capacity(sorted.len());
        for iv in sorted {
            // Expire intervals that ended before this one starts.
            // Intervals are sorted by start, so any still-active
            // interval has start <= iv.start; it conflicts with color
            // c iff its end is still in the future (> iv.start).
            self.active.retain(|&(end, _, _)| end > iv.start);
            let mut chosen = None;
            'colors: for c in 0..self.n_colors as u32 {
                for &(e, _, ac) in &self.active {
                    if ac == c && e > iv.start {
                        continue 'colors;
                    }
                }
                chosen = Some(c);
                break;
            }
            match chosen {
                Some(c) => {
                    self.active.push((iv.end, iv.vreg, c));
                    out.push((iv.vreg, Alloc::Reg(c)));
                }
                None => {
                    let s = self.spill_count;
                    self.spill_count += 1;
                    out.push((iv.vreg, Alloc::Spill(s)));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_overlapping_share_color() {
        let mut a = Allocator::new(2);
        let ivs = [
            Interval {
                vreg: 0,
                start: 0,
                end: 5,
            },
            Interval {
                vreg: 1,
                start: 5,
                end: 9,
            },
        ];
        let res = a.allocate(&ivs);
        assert!(res.iter().all(|(_, r)| matches!(r, Alloc::Reg(_))));
    }

    #[test]
    fn overlapping_spills_when_colors_exhausted() {
        let mut a = Allocator::new(1);
        let ivs = [
            Interval {
                vreg: 0,
                start: 0,
                end: 10,
            },
            Interval {
                vreg: 1,
                start: 2,
                end: 8,
            },
            Interval {
                vreg: 2,
                start: 3,
                end: 7,
            },
        ];
        let res = a.allocate(&ivs);
        let spills = res
            .iter()
            .filter(|(_, r)| matches!(r, Alloc::Spill(_)))
            .count();
        assert_eq!(spills, 2, "1 color, 3 overlapping intervals");
        assert_eq!(a.total_slots(), 3);
    }
}
