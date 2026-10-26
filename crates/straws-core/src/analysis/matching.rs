//! Matching networks: L, pi and T networks of ideal inductors and
//! capacitors, and single shunt stubs, that match a load impedance to a
//! real source impedance Z₀ at one frequency.
//!
//! Networks are listed from the load (antenna) towards the source (radio).

use std::f64::consts::PI;

use num_complex::Complex64;

/// Speed of light, m/s.
const C: f64 = 299.792458e6;

/// An ideal part, by value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Part {
    /// Henries.
    Inductor(f64),
    /// Farads.
    Capacitor(f64),
}

impl Part {
    /// Reactance at `mhz` (ohms).
    pub fn reactance(&self, mhz: f64) -> f64 {
        let w = 2.0 * PI * mhz * 1e6;
        match *self {
            Part::Inductor(l) => w * l,
            Part::Capacitor(c) => -1.0 / (w * c),
        }
    }

    /// The part with reactance `x` at `mhz` (positive: an inductor).
    pub fn for_reactance(x: f64, mhz: f64) -> Part {
        let w = 2.0 * PI * mhz * 1e6;
        if x >= 0.0 {
            Part::Inductor(x / w)
        } else {
            Part::Capacitor(-1.0 / (w * x))
        }
    }

    /// The part with susceptance `b` at `mhz` (positive: a capacitor).
    pub fn for_susceptance(b: f64, mhz: f64) -> Part {
        let w = 2.0 * PI * mhz * 1e6;
        if b >= 0.0 {
            Part::Capacitor(b / w)
        } else {
            Part::Inductor(-1.0 / (w * b))
        }
    }

    /// The nearest standard value from a series (12 or 24 values a decade).
    pub fn snapped(&self, per_decade: usize) -> Part {
        let snap = |v: f64| {
            let table: &[f64] = if per_decade >= 24 { &E24 } else { &E12 };
            let exp = v.log10().floor();
            let m = v / 10f64.powf(exp);
            let best = table
                .iter()
                .chain([10.0].iter())
                .min_by(|a, b| ((*a / m).ln().abs()).total_cmp(&(*b / m).ln().abs()))
                .copied()
                .unwrap_or(1.0);
            best * 10f64.powf(exp)
        };
        match *self {
            Part::Inductor(l) => Part::Inductor(snap(l)),
            Part::Capacitor(c) => Part::Capacitor(snap(c)),
        }
    }

    /// e.g. "1.23 µH" or "47.0 pF".
    pub fn describe(&self) -> String {
        let (v, units): (f64, [&str; 4]) = match *self {
            Part::Inductor(l) => (l, ["H", "mH", "µH", "nH"]),
            Part::Capacitor(c) => (c, ["F", "mF", "µF", "nF"]),
        };
        let scaled = [(1.0, 0), (1e-3, 1), (1e-6, 2), (1e-9, 3)]
            .into_iter()
            .find(|(s, _)| v >= *s * 0.9999)
            .map(|(s, k)| (v / s, units[k]));
        match scaled {
            Some((x, u)) => format!("{} {u}", sig3(x)),
            None if matches!(self, Part::Capacitor(_)) => format!("{} pF", sig3(v * 1e12)),
            None => format!("{} nH", sig3(v * 1e9)),
        }
    }
}

fn sig3(x: f64) -> String {
    if x >= 100.0 {
        format!("{x:.0}")
    } else if x >= 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.2}")
    }
}

/// Standard preferred values.
const E12: [f64; 12] = [1.0, 1.2, 1.5, 1.8, 2.2, 2.7, 3.3, 3.9, 4.7, 5.6, 6.8, 8.2];
const E24: [f64; 24] = [
    1.0, 1.1, 1.2, 1.3, 1.5, 1.6, 1.8, 2.0, 2.2, 2.4, 2.7, 3.0, 3.3, 3.6, 3.9, 4.3, 4.7, 5.1, 5.6,
    6.2, 6.8, 7.5, 8.2, 9.1,
];

/// One step of a network, from the load side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Element {
    Series(Part),
    Shunt(Part),
    /// A length of line (metres) of impedance `z0` and velocity factor `vf`.
    Line {
        length: f64,
        z0: f64,
        vf: f64,
    },
    /// A stub across the line: open or shorted at its far end.
    Stub {
        length: f64,
        z0: f64,
        vf: f64,
        shorted: bool,
    },
}

impl Element {
    /// Impedance looking into this element with `z` beyond it.
    pub fn apply(&self, z: Complex64, mhz: f64) -> Complex64 {
        let j = Complex64::new(0.0, 1.0);
        match *self {
            Element::Series(p) => z + j * p.reactance(mhz),
            Element::Shunt(p) => {
                let x = p.reactance(mhz);
                1.0 / (1.0 / z + 1.0 / (j * x))
            }
            Element::Line { length, z0, vf } => {
                let t = (2.0 * PI * length * mhz * 1e6 / (C * vf)).tan();
                z0 * (z + j * z0 * t) / (z0 + j * z * t)
            }
            Element::Stub {
                length,
                z0,
                vf,
                shorted,
            } => {
                let t = (2.0 * PI * length * mhz * 1e6 / (C * vf)).tan();
                let zs = if shorted { j * z0 * t } else { -j * z0 / t };
                1.0 / (1.0 / z + 1.0 / zs)
            }
        }
    }

    pub fn describe(&self) -> String {
        match *self {
            Element::Series(p) => format!("series {} {}", kind(&p), p.describe()),
            Element::Shunt(p) => format!("shunt {} {}", kind(&p), p.describe()),
            Element::Line { length, z0, .. } => format!("{length:.3} m of {z0:.0} Ω line"),
            Element::Stub {
                length,
                z0,
                shorted,
                ..
            } => format!(
                "{} stub, {length:.3} m of {z0:.0} Ω line",
                if shorted { "shorted" } else { "open" }
            ),
        }
    }

    /// The same element with standard part values.
    pub fn snapped(&self, per_decade: usize) -> Element {
        match *self {
            Element::Series(p) => Element::Series(p.snapped(per_decade)),
            Element::Shunt(p) => Element::Shunt(p.snapped(per_decade)),
            e => e,
        }
    }
}

fn kind(p: &Part) -> &'static str {
    match p {
        Part::Inductor(_) => "inductor",
        Part::Capacitor(_) => "capacitor",
    }
}

/// Impedance seen by the source through `network` with load `z`.
pub fn input_impedance(network: &[Element], z: Complex64, mhz: f64) -> Complex64 {
    network.iter().fold(z, |z, e| e.apply(z, mhz))
}

/// Reactances for an L network from `z` to the real `r`: shunt at the
/// load then series (`shunt_first`), or series then shunt. Up to two.
fn l_sections(z: Complex64, r: f64, shunt_first: bool) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    if shunt_first {
        // Shunt susceptance b, then series reactance x.
        let y = 1.0 / z;
        let (g, b0) = (y.re, y.im);
        let d = g / r - g * g;
        if d < -1e-15 {
            return out;
        }
        for s in [1.0, -1.0] {
            let b = -b0 + s * d.max(0.0).sqrt();
            let z1 = 1.0 / Complex64::new(g, b0 + b);
            out.push((b, -z1.im));
            if d <= 1e-15 {
                break;
            }
        }
    } else {
        let (rl, x0) = (z.re, z.im);
        let d = rl * r - rl * rl;
        if d < -1e-15 {
            return out;
        }
        for s in [1.0, -1.0] {
            let x = -x0 + s * d.max(0.0).sqrt();
            let y1 = 1.0 / Complex64::new(rl, x0 + x);
            out.push((x, -y1.im));
            if d <= 1e-15 {
                break;
            }
        }
    }
    out
}

/// Every L network (up to four) matching `z` to `z0` at `mhz`.
pub fn l_networks(z: Complex64, z0: f64, mhz: f64) -> Vec<Vec<Element>> {
    let mut out = Vec::new();
    for (b, x) in l_sections(z, z0, true) {
        out.push(vec![
            Element::Shunt(Part::for_susceptance(b, mhz)),
            Element::Series(Part::for_reactance(x, mhz)),
        ]);
    }
    for (x, b) in l_sections(z, z0, false) {
        out.push(vec![
            Element::Series(Part::for_reactance(x, mhz)),
            Element::Shunt(Part::for_susceptance(b, mhz)),
        ]);
    }
    out.retain(|n| n.iter().all(part_ok));
    out
}

fn part_ok(e: &Element) -> bool {
    match e {
        Element::Series(p) | Element::Shunt(p) => match p {
            Part::Inductor(v) | Part::Capacitor(v) => v.is_finite() && *v > 0.0,
        },
        _ => true,
    }
}

/// Pi networks (shunt, series, shunt) with loaded Q `q`, through a virtual
/// resistance below both the load's and the source's. Err says the
/// lowest Q that works.
pub fn pi_networks(z: Complex64, z0: f64, mhz: f64, q: f64) -> Result<Vec<Vec<Element>>, f64> {
    let (lo, hi) = (z.re.min(z0), z.re.max(z0));
    let rv = hi / (q * q + 1.0);
    if rv >= lo {
        return Err((hi / lo - 1.0).max(0.0).sqrt());
    }
    let mut out = Vec::new();
    for (b1, x1) in l_sections(z, rv, true) {
        for (x2, b2) in l_sections(Complex64::new(rv, 0.0), z0, false) {
            out.push(vec![
                Element::Shunt(Part::for_susceptance(b1, mhz)),
                Element::Series(Part::for_reactance(x1 + x2, mhz)),
                Element::Shunt(Part::for_susceptance(b2, mhz)),
            ]);
        }
    }
    out.retain(|n| n.iter().all(part_ok));
    Ok(out)
}

/// T networks (series, shunt, series) with loaded Q `q`, through a virtual
/// resistance above both. Err says the lowest Q that works.
pub fn t_networks(z: Complex64, z0: f64, mhz: f64, q: f64) -> Result<Vec<Vec<Element>>, f64> {
    let (lo, hi) = (z.re.min(z0), z.re.max(z0));
    let rv = lo * (q * q + 1.0);
    if rv <= hi {
        return Err((hi / lo - 1.0).max(0.0).sqrt());
    }
    let mut out = Vec::new();
    for (x1, b1) in l_sections(z, rv, false) {
        for (b2, x2) in l_sections(Complex64::new(rv, 0.0), z0, true) {
            out.push(vec![
                Element::Series(Part::for_reactance(x1, mhz)),
                Element::Shunt(Part::for_susceptance(b1 + b2, mhz)),
                Element::Series(Part::for_reactance(x2, mhz)),
            ]);
        }
    }
    out.retain(|n| n.iter().all(part_ok));
    Ok(out)
}

/// Single shunt-stub matches on a line of `z0` (the feedline) with
/// velocity factor `vf`: a length of line from the load, then a stub.
pub fn stub_matches(z: Complex64, z0: f64, vf: f64, mhz: f64, shorted: bool) -> Vec<Vec<Element>> {
    let wavelength = C * vf / (mhz * 1e6);
    let (r, x) = (z.re, z.im);
    let ts: Vec<f64> = if (r - z0).abs() < 1e-9 {
        vec![-x / (2.0 * z0)]
    } else {
        let root = (r * ((z0 - r).powi(2) + x * x) / z0).sqrt();
        vec![(x + root) / (r - z0), (x - root) / (r - z0)]
    };
    let mut out = Vec::new();
    for t in ts {
        let d = if t >= 0.0 {
            t.atan() / (2.0 * PI)
        } else {
            (PI + t.atan()) / (2.0 * PI)
        } * wavelength;
        // Susceptance at the stub point; the stub cancels it.
        let b = (r * r * t - (z0 - x * t) * (x + z0 * t)) / (z0 * (r * r + (x + z0 * t).powi(2)));
        let mut l = if shorted {
            (1.0 / (b * z0)).atan() / (2.0 * PI)
        } else {
            -(b * z0).atan() / (2.0 * PI)
        };
        if l < 0.0 {
            l += 0.5;
        }
        out.push(vec![
            Element::Line { length: d, z0, vf },
            Element::Stub {
                length: l * wavelength,
                z0,
                vf,
                shorted,
            },
        ]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::feed::swr;

    fn check(networks: &[Vec<Element>], z: Complex64, z0: f64, mhz: f64) {
        assert!(!networks.is_empty());
        for n in networks {
            let zin = input_impedance(n, z, mhz);
            assert!(swr(zin, z0) < 1.0001, "{n:?} gives {zin}");
        }
    }

    #[test]
    fn every_network_matches() {
        let mhz = 14.2;
        for z in [
            Complex64::new(12.0, -35.0),
            Complex64::new(150.0, 80.0),
            Complex64::new(73.0, 42.0),
            Complex64::new(20.0, 0.0),
        ] {
            check(&l_networks(z, 50.0, mhz), z, 50.0, mhz);
            check(&pi_networks(z, 50.0, mhz, 5.0).unwrap(), z, 50.0, mhz);
            check(&t_networks(z, 50.0, mhz, 5.0).unwrap(), z, 50.0, mhz);
            for shorted in [true, false] {
                check(&stub_matches(z, 50.0, 0.66, mhz, shorted), z, 50.0, mhz);
            }
        }
        // Too low a Q can't make a pi network for 150 Ω to 50 Ω.
        assert!(pi_networks(Complex64::new(150.0, 0.0), 50.0, mhz, 1.0).is_err());
    }

    #[test]
    fn standard_values() {
        assert_eq!(
            Part::Capacitor(108e-12).snapped(12),
            Part::Capacitor(100e-12)
        );
        let l = Part::Inductor(1.27e-6).snapped(24);
        assert!(matches!(l, Part::Inductor(v) if (v - 1.3e-6).abs() < 1e-12));
        assert_eq!(Part::Capacitor(47e-12).describe(), "47.0 pF");
        assert_eq!(Part::Inductor(1.5e-6).describe(), "1.50 µH");
    }
}
