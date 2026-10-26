//! Feed-point figures from an impedance and a reference impedance.

use num_complex::Complex64;

/// Reflection coefficient Γ = (Z − Z0) / (Z + Z0).
pub fn reflection(z: Complex64, z0: f64) -> Complex64 {
    (z - z0) / (z + z0)
}

/// Voltage standing wave ratio. Infinite for a total mismatch.
pub fn swr(z: Complex64, z0: f64) -> f64 {
    let g = reflection(z, z0).norm();
    if g >= 1.0 {
        f64::INFINITY
    } else {
        (1.0 + g) / (1.0 - g)
    }
}

/// Return loss in dB (positive; larger is a better match).
pub fn return_loss_db(z: Complex64, z0: f64) -> f64 {
    -20.0 * reflection(z, z0).norm().max(1e-12).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matched_and_mismatched_loads() {
        let z0 = 50.0;
        assert!((swr(Complex64::new(50.0, 0.0), z0) - 1.0).abs() < 1e-12);
        assert!((swr(Complex64::new(100.0, 0.0), z0) - 2.0).abs() < 1e-12);
        assert!((swr(Complex64::new(25.0, 0.0), z0) - 2.0).abs() < 1e-12);
        assert!(swr(Complex64::new(0.0, 50.0), z0).is_infinite());
        // SWR 2:1 is a return loss of 9.54 dB.
        assert!((return_loss_db(Complex64::new(100.0, 0.0), z0) - 9.542).abs() < 1e-3);
    }

    #[test]
    fn swr_return_loss_and_reflection_agree() {
        for (r, x) in [
            (10.0, -40.0),
            (75.0, 0.0),
            (300.0, 150.0),
            (50.0, 5.0),
            (3.0, 0.5),
        ] {
            let z = Complex64::new(r, x);
            let g = reflection(z, 50.0).norm();
            assert!((swr(z, 50.0) - (1.0 + g) / (1.0 - g)).abs() < 1e-9);
            assert!((return_loss_db(z, 50.0) + 20.0 * g.log10()).abs() < 1e-9);
        }
    }
}
