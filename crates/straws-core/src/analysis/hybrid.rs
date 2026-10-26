//! A 90° quadrature hybrid feeding an array: the radio drives port 1 of an
//! ideal branch-line coupler (reference impedance Z0, the lines' own);
//! port 2 (output A) and port 3 (output B, a quarter cycle behind A) each
//! feed their elements through their own lines in parallel, and port 4
//! (isolated) goes to a dump load of Z0. With the coupler's S-matrix
//!
//! ```text
//! S = -1/√2 [[0, j, 1, 0], [j, 0, 0, 1], [1, 0, 0, j], [0, 1, j, 0]]
//! ```
//!
//! and a matched source and dump, the waves out of A and B are fixed
//! (-j/√2 and -1/√2), whatever the loads; what the loads reflect goes back
//! to the radio as (Γ_B - Γ_A)/2 and into the dump as j(Γ_A + Γ_B)/2. So
//! equal loads give the radio a perfect match and send their reflection to
//! the dump, while the 90° split holds in the outputs' waves (the voltages
//! and currents still depend on the loads). It's ideal and the same at
//! every frequency: a real branch-line hybrid is narrowband too.

use num_complex::Complex64;

use crate::analysis::feedline::{Cable, line_input};
use crate::analysis::phasing::{BandPoint, delivered};
use crate::units::C;

/// The hybrid feed: which elements hang off output B (the rest off A), and
/// each element's line length (m).
#[derive(Debug, Clone, PartialEq)]
pub struct HybridFeed {
    pub on_b: Vec<bool>,
    pub lengths: Vec<f64>,
}

/// What the feed gives at one frequency, for 1 W available from the radio.
#[derive(Debug, Clone, PartialEq)]
pub struct HybridResult {
    pub currents: Vec<Complex64>,
    /// What the radio sees (referenced to Z0).
    pub input: Complex64,
    /// Share of the power the radio puts in that ends up in the dump load.
    pub dumped: f64,
}

fn gamma_of(z: Complex64, z0: f64) -> Complex64 {
    (z - z0) / (z + z0)
}

/// The feed at `mhz` with element impedances `z`.
pub fn evaluate(z: &[Complex64], feed: &HybridFeed, cable: &Cable, mhz: f64) -> HybridResult {
    let z0 = cable.z0;
    let s2 = std::f64::consts::SQRT_2;
    let j = Complex64::new(0.0, 1.0);
    // Each output's load: its elements' lines in parallel.
    let load = |b: bool| -> Option<Complex64> {
        let y: Complex64 = (0..z.len())
            .filter(|&k| feed.on_b[k] == b)
            .map(|k| 1.0 / line_input(cable, feed.lengths[k], mhz, z[k]))
            .sum();
        (y.norm() > 0.0).then(|| 1.0 / y)
    };
    let (za, zb) = (load(false), load(true));
    // An output with nothing on it is open (all reflected).
    let ga = za.map_or(Complex64::new(1.0, 0.0), |z| gamma_of(z, z0));
    let gb = zb.map_or(Complex64::new(1.0, 0.0), |z| gamma_of(z, z0));
    let (b2, b3) = (-j / s2, Complex64::new(-1.0 / s2, 0.0));
    // Port voltages (waves in √W): V = √Z0 · b(1 + Γ).
    let va = z0.sqrt() * b2 * (1.0 + ga);
    let vb = z0.sqrt() * b3 * (1.0 + gb);
    let currents = (0..z.len())
        .map(|k| {
            let v = if feed.on_b[k] { vb } else { va };
            v * delivered(cable, mhz, z[k], feed.lengths[k])
        })
        .collect();
    let g_in = (gb - ga) / 2.0;
    let dump = j * (ga + gb) / 2.0;
    let accepted = (1.0 - g_in.norm_sqr()).max(1e-12);
    HybridResult {
        currents,
        input: z0 * (1.0 + g_in) / (1.0 - g_in),
        dumped: (dump.norm_sqr() / accepted).clamp(0.0, 1.0),
    }
}

/// Worst errors of `feed` over `band` against `want` (each element's
/// current against the reference element's): amplitude (dB), phase
/// (degrees) and the vector error |got/wanted - 1|.
pub fn errors(
    band: &[BandPoint],
    want: &[Complex64],
    feed: &HybridFeed,
    cable: &Cable,
    reference: usize,
) -> (f64, f64, f64) {
    let (mut db, mut deg, mut vector) = (0.0f64, 0.0f64, 0.0f64);
    for (mhz, z) in band {
        let r = evaluate(z, feed, cable, *mhz);
        let i_ref = r.currents[reference];
        for e in 0..want.len() {
            if e == reference || want[e].norm() == 0.0 || i_ref.norm() == 0.0 {
                continue;
            }
            let x = r.currents[e] / i_ref / (want[e] / want[reference]);
            db = db.max((20.0 * x.norm().log10()).abs());
            deg = deg.max(x.arg().to_degrees().abs());
            vector = vector.max((x - 1.0).norm());
        }
    }
    (db, deg, vector)
}

/// Which output suits each element: B where the wanted phase against the
/// reference is nearer ±90° than 0° or 180° (a half wave more of line turns
/// 180°).
pub fn default_groups(want: &[Complex64], reference: usize) -> Vec<bool> {
    want.iter()
        .map(|w| {
            let a = (w / want[reference]).arg().to_degrees().rem_euclid(180.0);
            (45.0..135.0).contains(&a)
        })
        .collect()
}

/// Line lengths for a hybrid feed with outputs `on_b`, tuned (Nelder-Mead
/// from a few starts) for the smallest worst vector error over `band` (or
/// at `mhz` alone if `band` is empty), none shorter than `min_len`.
pub fn design(
    z: &[Complex64],
    want: &[Complex64],
    on_b: &[bool],
    min_len: &[f64],
    cable: &Cable,
    mhz: f64,
    band: &[BandPoint],
) -> HybridFeed {
    let n = z.len();
    let reference = (0..n)
        .max_by(|&a, &b| want[a].norm().total_cmp(&want[b].norm()))
        .unwrap_or(0);
    let here = [(mhz, z.to_vec())];
    let pts: &[BandPoint] = if band.is_empty() { &here } else { band };
    let lam = C * cable.velocity_factor / (mhz * 1e6);
    let cost = |l: &[f64]| {
        let feed = HybridFeed {
            on_b: on_b.to_vec(),
            lengths: l.to_vec(),
        };
        let short: f64 = (0..n).map(|e| (min_len[e] - l[e]).max(0.0).powi(2)).sum();
        errors(pts, want, &feed, cable, reference).2 + 1e3 * short
    };
    // Each line in turn swept over a wavelength (the cost repeats every
    // wavelength of line), a few rounds, then a local polish.
    let mut l: Vec<f64> = min_len.to_vec();
    let steps = 360;
    for _ in 0..6 {
        for e in 0..n {
            let mut best = (cost(&l), l[e]);
            for k in 0..steps {
                let mut t = l.clone();
                t[e] = min_len[e] + lam * k as f64 / steps as f64;
                let c = cost(&t);
                if c < best.0 {
                    best = (c, t[e]);
                }
            }
            l[e] = best.1;
        }
    }
    let polished = nelder_mead(&cost, l.clone(), lam / 200.0, 800);
    let best = Some(if cost(&polished) <= cost(&l) {
        (0.0, polished)
    } else {
        (0.0, l)
    });
    let lengths = best.map(|b| b.1).unwrap_or_default();
    HybridFeed {
        on_b: on_b.to_vec(),
        lengths: lengths
            .iter()
            .zip(min_len)
            .map(|(l, m)| l.max(*m))
            .collect(),
    }
}

fn nelder_mead(cost: &dyn Fn(&[f64]) -> f64, start: Vec<f64>, step: f64, iters: usize) -> Vec<f64> {
    let n = start.len();
    let mut s: Vec<(Vec<f64>, f64)> = (0..=n)
        .map(|k| {
            let mut x = start.clone();
            if k > 0 {
                x[k - 1] += step;
            }
            let c = cost(&x);
            (x, c)
        })
        .collect();
    for _ in 0..iters {
        s.sort_by(|a, b| a.1.total_cmp(&b.1));
        let c: Vec<f64> = (0..n)
            .map(|d| s[..n].iter().map(|p| p.0[d]).sum::<f64>() / n as f64)
            .collect();
        let at = |t: f64| -> Vec<f64> { (0..n).map(|d| c[d] + t * (s[n].0[d] - c[d])).collect() };
        let r = at(-1.0);
        let fr = cost(&r);
        if fr < s[0].1 {
            let e = at(-2.0);
            let fe = cost(&e);
            s[n] = if fe < fr { (e, fe) } else { (r, fr) };
        } else if fr < s[n - 1].1 {
            s[n] = (r, fr);
        } else {
            let k = at(0.5);
            let fk = cost(&k);
            if fk < s[n].1 {
                s[n] = (k, fk);
            } else {
                let b = s[0].0.clone();
                for p in s.iter_mut().skip(1) {
                    for (x, bb) in p.0.iter_mut().zip(&b) {
                        *x = bb + 0.5 * (*x - bb);
                    }
                    p.1 = cost(&p.0);
                }
            }
        }
        if (s[n].1 - s[0].1).abs() < 1e-12 {
            break;
        }
    }
    s.sort_by(|a, b| a.1.total_cmp(&b.1));
    s.swap_remove(0).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::feedline::CABLES;

    #[test]
    fn matched_loads_split_equally_a_quarter_cycle_apart() {
        let ideal = CABLES[7];
        let z = [Complex64::new(50.0, 0.0), Complex64::new(50.0, 0.0)];
        let feed = HybridFeed {
            on_b: vec![false, true],
            lengths: vec![0.0, 0.0],
        };
        let r = evaluate(&z, &feed, &ideal, 7.0);
        let ratio = r.currents[1] / r.currents[0];
        assert!((ratio.norm() - 1.0).abs() < 1e-9, "{ratio}");
        assert!((ratio.arg().to_degrees() + 90.0).abs() < 1e-9, "{ratio}");
        assert!((r.input - Complex64::new(50.0, 0.0)).norm() < 1e-9 && r.dumped < 1e-12);
        // Half the watt into each 50 Ω load: |I|² · 50 = 0.5.
        assert!((r.currents[0].norm_sqr() * 50.0 - 0.5).abs() < 1e-9);
    }

    #[test]
    fn equal_mismatches_go_to_the_dump() {
        let ideal = CABLES[7];
        let z = [Complex64::new(25.0, 15.0), Complex64::new(25.0, 15.0)];
        let feed = HybridFeed {
            on_b: vec![false, true],
            lengths: vec![0.0, 0.0],
        };
        let r = evaluate(&z, &feed, &ideal, 7.0);
        let g = ((z[0] - 50.0) / (z[0] + 50.0)).norm_sqr();
        assert!(
            (r.input - Complex64::new(50.0, 0.0)).norm() < 1e-9,
            "{}",
            r.input
        );
        assert!((r.dumped - g).abs() < 1e-9, "{} vs {g}", r.dumped);
        let ratio = r.currents[1] / r.currents[0];
        assert!((ratio.arg().to_degrees() + 90.0).abs() < 1e-6);
    }

    #[test]
    fn a_designed_hybrid_feed_finds_the_best_lines() {
        // Two coupled elements wanted 90° apart, with quite different
        // impedances. A hybrid fixes the waves it sends out, not the
        // currents, so plain lines can't get these exactly: a full grid of
        // both lengths (400 x 400 over a wavelength) finds 36.9% at best,
        // and the design must find that too.
        let rg213 = CABLES[2];
        let z = [Complex64::new(22.0, -10.0), Complex64::new(57.0, 44.0)];
        let want = [Complex64::new(1.0, 0.0), Complex64::new(0.0, -1.0)];
        let groups = default_groups(&want, 0);
        assert_eq!(groups, vec![false, true]);
        let feed = design(&z, &want, &groups, &[1.0, 1.0], &rg213, 7.1, &[]);
        let (_, _, v) = errors(&[(7.1, z.to_vec())], &want, &feed, &rg213, 0);
        println!("hybrid lines {:?}, vector error {v:.4}", feed.lengths);
        assert!(v < 0.3688 + 0.002, "{v}");
        assert!(feed.lengths.iter().all(|l| *l >= 1.0));
        // Loads alike (a symmetric pair) do come out exactly.
        let alike = [Complex64::new(40.0, 20.0), Complex64::new(40.0, 20.0)];
        let feed = design(&alike, &want, &groups, &[1.0, 1.0], &rg213, 7.1, &[]);
        let (_, _, v) = errors(&[(7.1, alike.to_vec())], &want, &feed, &rg213, 0);
        assert!(v < 0.01, "{v}");
    }
}
