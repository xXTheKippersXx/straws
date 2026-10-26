//! Weight and wind load of the straight wires (the tube table): each
//! wire's mass from its material's density (solid, or a tube with its wall
//! thickness), and its area facing the wind (length x diameter, every wire
//! taken broadside: the worst case).

use crate::deck::NumericDeck;
use crate::deck::edit::Wire;
use crate::deck::material::material_of;

/// Density (kg/m³) of a listed material, by its name in `MATERIALS`.
pub fn density(name: &str) -> Option<f64> {
    Some(match name {
        "Copper" => 8960.0,
        "Silver" => 10490.0,
        "Gold" => 19300.0,
        "Aluminium (pure)" | "Aluminium 6061-T6" => 2700.0,
        "Brass" => 8500.0,
        // A galvanised conductor is steel under its zinc.
        "Zinc (galvanised surface)" => 7850.0,
        "Phosphor bronze" => 8800.0,
        "Stainless steel 304" => 8000.0,
        _ => return None,
    })
}

/// Weight and wind area of a model's straight wires.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mechanics {
    /// Mass of the wires whose material is known (kg).
    pub mass: f64,
    /// Area facing the wind, every wire broadside (m²).
    pub area: f64,
    /// Wires with no listed material (perfect conductor or a custom
    /// conductivity), left out of the mass.
    pub unknown: usize,
}

/// Air density (kg/m³) and the drag coefficient of a round tube across the
/// wind, for [`wind_force`].
const AIR: f64 = 1.225;
const DRAG: f64 = 1.2;

/// Force (N) of wind at `speed` m/s on `area` m² of round tube.
pub fn wind_force(area: f64, speed: f64) -> f64 {
    0.5 * AIR * speed * speed * DRAG * area
}

/// Weight and wind area of `wires` (helper and feedline-shield wires left
/// out).
pub fn mechanics(wires: &[Wire], deck: &NumericDeck, skip_tags: &[i64]) -> Mechanics {
    let mut m = Mechanics::default();
    for w in wires {
        if deck.helper_tags.contains(&w.tag) || skip_tags.contains(&w.tag) {
            continue;
        }
        let (len, r) = (w.length(), w.radius);
        m.area += len * 2.0 * r;
        let material = material_of(deck, w.tag);
        // A conductivity that isn't exactly a listed one: the listed
        // material closest to it (within a factor of 1.3), e.g. 3.5e7 S/m
        // is aluminium.
        let near = material.conductivity.and_then(|sigma| {
            crate::deck::material::MATERIALS
                .iter()
                .map(|(name, s)| (name, (sigma / s).ln().abs()))
                .filter(|(_, d)| *d < 1.3f64.ln())
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .and_then(|(name, _)| density(name))
        });
        match density(&material.name()).or(near) {
            Some(rho) => {
                let inner = material.wall.map_or(0.0, |t| (r - t).max(0.0));
                m.mass += std::f64::consts::PI * (r * r - inner * inner) * len * rho;
            }
            None => m.unknown += 1,
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::Deck;

    #[test]
    fn copper_wire_and_aluminium_tube() {
        // 10 m of 2 mm copper wire, and 2 m of 25 mm aluminium tube with a
        // 1.5 mm wall.
        let d = Deck::parse(
            "GW 1 9 0 0 0 0 0 10 .001\nGW 2 9 1 0 0 1 0 2 .0125\nGE 0\nLD 5 1 0 0 5.8e7\nLD 5 2 0 0 2.5e7 ' straws: tube wall 0.0015\nEX 0 1 5 0 1\nEN\n",
        );
        let n = d.expand().unwrap();
        let m = mechanics(&crate::deck::edit::wires(&d, &n), &n, &[]);
        let copper = std::f64::consts::PI * 0.001f64.powi(2) * 10.0 * 8960.0;
        let tube = std::f64::consts::PI * (0.0125f64.powi(2) - 0.011f64.powi(2)) * 2.0 * 2700.0;
        assert!((m.mass - copper - tube).abs() < 1e-9, "{m:?}");
        assert!((m.area - (10.0 * 0.002 + 2.0 * 0.025)).abs() < 1e-12);
        assert_eq!(m.unknown, 0);
        // 0.07 m² at 100 km/h: about 40 N.
        assert!(
            (wind_force(m.area, 100.0 / 3.6) - 39.7).abs() < 0.5,
            "{}",
            wind_force(m.area, 100.0 / 3.6)
        );
    }
}
