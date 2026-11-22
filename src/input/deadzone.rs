//! Radial deadzone for analog sticks.
//!
//! Values inside `deadzone` collapse to 0; outside values are
//! rescaled so the usable range starts at 0 (standard "scaled"
//! deadzone, avoids a dead band at the edge of travel).

/// Apply a radial deadzone to `value` in [-1, 1].
///
/// `deadzone` should be in [0, 0.95]. Returns a value in [-1, 1].
pub fn apply_deadzone(value: f32, deadzone: f32) -> f32 {
    let dz = deadzone.clamp(0.0, 0.95);
    let v = value.clamp(-1.0, 1.0);
    let a = v.abs();
    if a <= dz {
        return 0.0;
    }
    // Rescale: (a - dz) / (1 - dz), keep sign.
    let scaled = (a - dz) / (1.0 - dz);
    scaled * v.signum()
}

/// Apply deadzone to a 2D stick vector; returns (x, y) rescaled
/// preserving direction (true radial deadzone on magnitude).
pub fn apply_deadzone_2d(x: f32, y: f32, deadzone: f32) -> (f32, f32) {
    let mag = (x * x + y * y).sqrt();
    if mag <= deadzone || mag == 0.0 {
        return (0.0, 0.0);
    }
    let scale = ((mag - deadzone) / (1.0 - deadzone).max(1e-6)) / mag;
    (x * scale, y * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_deadzone_is_zero() {
        assert_eq!(apply_deadzone(0.1, 0.2), 0.0);
        assert_eq!(apply_deadzone(-0.15, 0.2), 0.0);
    }

    #[test]
    fn outside_rescales_to_full_range() {
        // At full deflection output should stay ~1.
        let v = apply_deadzone(1.0, 0.2);
        assert!((v - 1.0).abs() < 1e-6);
        // Just outside deadzone → small positive.
        let v2 = apply_deadzone(0.25, 0.2);
        assert!(v2 > 0.0 && v2 < 0.2);
        // Symmetric.
        assert_eq!(apply_deadzone(-1.0, 0.2), -1.0);
    }

    #[test]
    fn deadzone_2d_preserves_direction() {
        let (x, y) = apply_deadzone_2d(0.5, 0.5, 0.2);
        assert!(x > 0.0 && y > 0.0);
        assert!((x - y).abs() < 1e-6); // 45° preserved
        assert_eq!(apply_deadzone_2d(0.1, 0.1, 0.3), (0.0, 0.0));
    }
}
