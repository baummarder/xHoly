//! Input press-state machine — Hold / Click / Drag / Release edges.

/// Edge events derived from raw button transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressEvent {
    /// Button went down this frame.
    Pressed,
    /// Button went up this frame.
    Released,
    /// Button held and pointer moved beyond threshold.
    Dragged,
    /// Held without drag (still down).
    Held,
}

/// State of one tracked button/pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressState {
    /// Not pressed.
    Up,
    /// Pressed this frame.
    JustPressed,
    /// Held (pressed previously).
    Down,
    /// Released this frame.
    JustReleased,
}

impl PressState {
    /// Advance the machine given current physical down-state.
    /// Returns the edge event for this frame.
    pub fn update(&mut self, physically_down: bool, dragged: bool) -> Option<PressEvent> {
        use PressEvent::*;
        use PressState::*;
        let ev = match (*self, physically_down, dragged) {
            (Up, true, _) => Some(Pressed),
            (JustPressed | Down, true, true) => Some(Dragged),
            (JustPressed | Down, true, false) => Some(Held),
            (JustPressed | Down, false, _) => Some(Released),
            (Up, false, _) | (JustReleased, false, _) => None,
            (JustReleased, true, _) => Some(Pressed),
        };
        *self = match physically_down {
            true => Down,
            false => Up,
        };
        // Distinguish JustPressed/JustReleased for one frame via event;
        // state itself collapses to Down/Up (edges are in the event).
        ev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_hold_release_cycle() {
        let mut s = PressState::Up;
        assert_eq!(s.update(true, false), Some(PressEvent::Pressed));
        assert_eq!(s.update(true, false), Some(PressEvent::Held));
        assert_eq!(s.update(true, true), Some(PressEvent::Dragged));
        assert_eq!(s.update(false, false), Some(PressEvent::Released));
        assert_eq!(s.update(false, false), None);
    }

    #[test]
    fn re_press_after_release() {
        let mut s = PressState::Up;
        s.update(true, false);
        s.update(false, false);
        assert_eq!(s.update(true, false), Some(PressEvent::Pressed));
    }
}
