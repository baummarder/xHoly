//! Interrupt controller stub — Phase 10.
//!
//! Simulates a GICv3-style interrupt controller: IRQ lines can be
//! raised/lowered, a timer source generates periodic ticks, and the
//! CPU can query/ack pending interrupts. No actual GIC MMIO — the
//! guest sees interrupts only if it reads our stub registers (which
//! live in guest memory at a fixed MMIO-ish address).

/// Simulated GICv3 stub.
#[derive(Debug)]
pub struct Gic {
    /// IRQ line states (index = IRQ number).
    lines: Vec<bool>,
    /// Lines enabled for delivery.
    enabled: Vec<bool>,
    /// Timer ticks remaining before next timer IRQ.
    timer_countdown: u64,
    /// Timer period in ticks.
    pub timer_period: u64,
    /// Total timer IRQs delivered.
    pub timer_irqs: u64,
    /// Guest-visible distributor base (arbitrary, inside device space).
    pub distributor_base: u64,
}

/// SPI range start (we only model a few lines).
pub const IRQ_TIMER: usize = 30;
pub const IRQ_UART: usize = 33;
pub const IRQ_GPU: usize = 34;

impl Default for Gic {
    fn default() -> Self {
        Self::new()
    }
}

impl Gic {
    /// Controller with 64 IRQ lines, timer every 1024 ticks.
    pub fn new() -> Self {
        Self {
            lines: vec![false; 64],
            enabled: vec![false; 64],
            timer_countdown: 0,
            timer_period: 1024,
            timer_irqs: 0,
            distributor_base: super::mmio::DISPLAY_BASE + 0x100,
        }
    }

    /// Enable an IRQ line for delivery.
    pub fn enable(&mut self, irq: usize) {
        if irq < self.enabled.len() {
            self.enabled[irq] = true;
        }
    }

    /// Raise (assert) an IRQ line.
    pub fn raise(&mut self, irq: usize) {
        if irq < self.lines.len() {
            self.lines[irq] = true;
        }
    }

    /// Lower (deassert) an IRQ line.
    pub fn lower(&mut self, irq: usize) {
        if irq < self.lines.len() {
            self.lines[irq] = false;
        }
    }

    /// Advance the timer; fires IRQ_TIMER when the countdown expires.
    /// Call once per emulated tick.
    pub fn tick(&mut self) {
        if self.timer_period == 0 {
            return;
        }
        self.timer_countdown += 1;
        if self.timer_countdown >= self.timer_period {
            self.timer_countdown = 0;
            self.raise(IRQ_TIMER);
            self.timer_irqs += 1;
        }
    }

    /// Highest-priority pending+enabled IRQ, if any.
    pub fn pending(&self) -> Option<usize> {
        self.lines
            .iter()
            .enumerate()
            .rev() // higher IRQ numbers win (arbitrary priority)
            .find(|(i, &l)| l && self.enabled.get(*i).copied().unwrap_or(false))
            .map(|(i, _)| i)
    }

    /// Acknowledge (clear) the pending IRQ.
    pub fn ack(&mut self, irq: usize) {
        self.lower(irq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_fires_periodically() {
        let mut gic = Gic::new();
        gic.timer_period = 4;
        gic.enable(IRQ_TIMER);
        for _ in 0..8 {
            gic.tick();
        }
        assert_eq!(gic.timer_irqs, 2);
        assert!(gic.pending().is_some());
        let irq = gic.pending().unwrap();
        gic.ack(irq);
        assert!(gic.pending().is_none());
    }

    #[test]
    fn disabled_irq_not_pending() {
        let mut gic = Gic::new();
        gic.raise(IRQ_UART); // not enabled
        assert!(gic.pending().is_none());
        gic.enable(IRQ_UART);
        assert_eq!(gic.pending(), Some(IRQ_UART));
    }
}
