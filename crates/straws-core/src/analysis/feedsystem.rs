//! The feed system between the antenna and the radio, optimised over a
//! band: a chain of stages (balun, matching parts, lines, stubs, fixed
//! losses), each a two-port, evaluated against the antenna's feed impedance
//! at every frequency of a sweep. Only network maths: no engine runs, so
//! part values, stub and line lengths and the balun ratio can be tuned for
//! the lowest worst-case SWR at the radio (with a weighting against loss)
//! in a fraction of a second.
//!
//! Every stage is an ABCD matrix with its input on the radio side, so the
//! chain's matrix gives both what the radio sees and the power that reaches
//! the antenna (parts have a finite Q, so lossy, high-Q networks show
//! their cost).

use std::f64::consts::PI;

use num_complex::Complex64;

use crate::analysis::feedline::Cable;
use crate::analysis::matching::{self, Element, Part};
use crate::units::C;

type M2 = [[Complex64; 2]; 2];

/// One stage of the feed system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    /// Impedance ratio (antenna side / radio side) and loss (dB).
    Balun {
        ratio: f64,
        loss_db: f64,
    },
    /// A part in series, or across the line, with its Q.
    Series {
        part: Part,
        q: f64,
    },
    Shunt {
        part: Part,
        q: f64,
    },
    /// A length (m) of cable.
    Line {
        cable: Cable,
        length: f64,
    },
    /// A stub of cable across the line, shorted or open at its far end.
    Stub {
        cable: Cable,
        length: f64,
        shorted: bool,
    },
    /// A fixed loss (dB): connectors and the like.
    Loss {
        db: f64,
    },
}

fn mul(a: M2, b: M2) -> M2 {
    let mut m = [[Complex64::default(); 2]; 2];
    for i in 0..2 {
        for j in 0..2 {
            m[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j];
        }
    }
    m
}

fn gamma(cable: &Cable, mhz: f64) -> Complex64 {
    let alpha = cable.loss_db_per_m(mhz) / (20.0 / std::f64::consts::LN_10);
    let beta = 2.0 * PI * mhz * 1e6 / (C * cable.velocity_factor);
    Complex64::new(alpha, beta)
}

/// A part's impedance with its loss: R = |X| / Q in series.
fn part_z(part: &Part, q: f64, mhz: f64) -> Complex64 {
    let x = part.reactance(mhz);
    Complex64::new(x.abs() / q.max(1.0), x)
}

impl Stage {
    /// The stage's matrix at `mhz`, and the share of the power it passes
    /// beyond what the matrix accounts for (balun and fixed losses).
    fn abcd(&self, mhz: f64) -> (M2, f64) {
        let one = Complex64::new(1.0, 0.0);
        let zero = Complex64::default();
        match *self {
            Stage::Balun { ratio, loss_db } => {
                let n = ratio.max(1e-6).sqrt();
                (
                    [[one / n, zero], [zero, one * n]],
                    10f64.powf(-loss_db / 10.0),
                )
            }
            Stage::Series { part, q } => ([[one, part_z(&part, q, mhz)], [zero, one]], 1.0),
            Stage::Shunt { part, q } => ([[one, zero], [1.0 / part_z(&part, q, mhz), one]], 1.0),
            Stage::Line { cable, length } => {
                let gl = gamma(&cable, mhz) * length;
                let (c, s) = (gl.cosh(), gl.sinh());
                ([[c, cable.z0 * s], [s / cable.z0, c]], 1.0)
            }
            Stage::Stub {
                cable,
                length,
                shorted,
            } => {
                let gl = gamma(&cable, mhz) * length;
                let (c, s) = (gl.cosh(), gl.sinh());
                // Admittance of the stub: shorted Y = coth/Z0, open tanh/Z0.
                let y = if shorted {
                    c / (s * cable.z0)
                } else {
                    s / (c * cable.z0)
                };
                ([[one, zero], [y, one]], 1.0)
            }
            Stage::Loss { db } => ([[one, zero], [zero, one]], 10f64.powf(-db / 10.0)),
        }
    }

    /// What can be tuned: a part's value, a length, or a ratio.
    pub fn value(&self) -> Option<f64> {
        match *self {
            Stage::Series { part, .. } | Stage::Shunt { part, .. } => Some(match part {
                Part::Inductor(v) | Part::Capacitor(v) => v,
            }),
            Stage::Line { length, .. } | Stage::Stub { length, .. } => Some(length),
            Stage::Balun { ratio, .. } => Some(ratio),
            Stage::Loss { .. } => None,
        }
    }

    /// The stage with its tunable value set to `v`.
    pub fn with_value(&self, v: f64) -> Stage {
        let mut s = *self;
        match &mut s {
            Stage::Series { part, .. } | Stage::Shunt { part, .. } => {
                *part = match *part {
                    Part::Inductor(_) => Part::Inductor(v),
                    Part::Capacitor(_) => Part::Capacitor(v),
                }
            }
            Stage::Line { length, .. } | Stage::Stub { length, .. } => *length = v,
            Stage::Balun { ratio, .. } => *ratio = v,
            Stage::Loss { .. } => {}
        }
        s
    }

    /// Parts at the nearest standard value (12 or 24 a decade).
    pub fn snapped(&self, per_decade: usize) -> Stage {
        match *self {
            Stage::Series { part, q } => Stage::Series {
                part: part.snapped(per_decade),
                q,
            },
            Stage::Shunt { part, q } => Stage::Shunt {
                part: part.snapped(per_decade),
                q,
            },
            s => s,
        }
    }

    /// e.g. "shunt capacitor 47.0 pF".
    pub fn describe(&self) -> String {
        match *self {
            Stage::Balun { ratio, loss_db } => format!("balun {ratio:.2}:1, {loss_db:.2} dB loss"),
            Stage::Series { part, .. } => Element::Series(part).describe(),
            Stage::Shunt { part, .. } => Element::Shunt(part).describe(),
            Stage::Line { cable, length } => format!("{length:.3} m of {}", cable.name),
            Stage::Stub {
                cable,
                length,
                shorted,
            } => format!(
                "{} stub, {length:.3} m of {}",
                if shorted { "shorted" } else { "open" },
                cable.name
            ),
            Stage::Loss { db } => format!("{db:.2} dB loss (connectors)"),
        }
    }
}

/// The Feedline view's feed as a chain (from the antenna): balun, the
/// antenna-end connectors' loss and the line inside them, the cable, the
/// radio-end connectors' loss (connector losses at `mhz`). Also the index of
/// the cable's stage. It gives what [`crate::analysis::feedline::at_radio`]
/// gives, so every view and goal measures the same feed.
pub fn chain_from_feed(feed: &crate::analysis::feedline::Feed, mhz: f64) -> (Vec<Stage>, usize) {
    let mut stages = vec![Stage::Balun {
        ratio: feed.balun_ratio,
        loss_db: feed.balun_loss_db,
    }];
    if let Some(c) = feed.antenna_connector {
        stages.push(Stage::Loss { db: c.loss_db(mhz) });
        if feed.connector_length_m > 0.0 {
            stages.push(Stage::Line {
                cable: crate::analysis::feedline::CONNECTOR_LINE,
                length: feed.connector_length_m,
            });
        }
    }
    let line = stages.len();
    stages.push(Stage::Line {
        cable: feed.cable,
        length: feed.length_m,
    });
    if let Some(c) = feed.radio_connector {
        stages.push(Stage::Loss { db: c.loss_db(mhz) });
    }
    (stages, line)
}

/// What the radio sees through `stages` (listed from the antenna) with the
/// antenna's feed impedance `z` at `mhz`: impedance, SWR on `z0`, and the
/// share of the radio's power that reaches the antenna.
pub fn evaluate(stages: &[Stage], z: Complex64, mhz: f64, z0: f64) -> (Complex64, f64, f64) {
    let one = Complex64::new(1.0, 0.0);
    let (m, passed) = chain(stages, mhz);
    // Load current 1 A: V = z at the antenna.
    let (vl, il) = (z, one);
    let vin = m[0][0] * vl + m[0][1] * il;
    let iin = m[1][0] * vl + m[1][1] * il;
    let zin = vin / iin;
    let p_load = (vl * il.conj()).re;
    let p_in = (vin * iin.conj()).re;
    let eff = if p_in > 0.0 {
        passed * p_load / p_in
    } else {
        0.0
    };
    let g = ((zin - z0) / (zin + z0)).norm();
    let swr = if g < 1.0 {
        (1.0 + g) / (1.0 - g)
    } else {
        f64::INFINITY
    };
    (zin, swr, eff.clamp(0.0, 1.0))
}

/// The chain's ABCD matrix at `mhz` (radio side as the input) and the share
/// of the power its balun and fixed losses pass.
fn chain(stages: &[Stage], mhz: f64) -> (M2, f64) {
    let one = Complex64::new(1.0, 0.0);
    let zero = Complex64::default();
    let mut m: M2 = [[one, zero], [zero, one]];
    let mut passed = 1.0;
    for s in stages {
        let (a, p) = s.abcd(mhz);
        m = mul(a, m);
        passed *= p;
    }
    (m, passed)
}

/// The chain's ABCD matrix at `mhz`, radio side as the input:
/// V_radio = A·V + B·I and I_radio = C·V + D·I at the antenna.
pub fn chain_matrix(stages: &[Stage], mhz: f64) -> [[Complex64; 2]; 2] {
    chain(stages, mhz).0
}

/// The worst SWR and the worst loss (dB) over `points` (MHz, antenna Z).
pub fn worst(stages: &[Stage], points: &[(f64, Complex64)], z0: f64) -> (f64, f64) {
    points.iter().fold((0.0f64, 0.0f64), |(s, l), (f, z)| {
        let (_, swr, eff) = evaluate(stages, *z, *f, z0);
        (s.max(swr), l.max(-10.0 * eff.max(1e-12).log10()))
    })
}

/// What [`optimise`] may change: a stage's value, between limits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Free {
    pub stage: usize,
    pub min: f64,
    pub max: f64,
}

/// The cost of a chain: the worst SWR (capped at 50) plus `loss_weight` per
/// dB of the worst loss.
fn cost(stages: &[Stage], points: &[(f64, Complex64)], z0: f64, loss_weight: f64) -> f64 {
    let (s, l) = worst(stages, points, z0);
    s.min(50.0) + loss_weight * l
}

/// Tunes the `free` values of `stages` over `points` (Nelder-Mead in log
/// steps, so part values a decade apart are as easy to reach as lengths),
/// keeping them within their limits. Returns the tuned chain.
pub fn optimise(
    stages: &[Stage],
    free: &[Free],
    points: &[(f64, Complex64)],
    z0: f64,
    loss_weight: f64,
) -> Vec<Stage> {
    let n = free.len();
    if n == 0 || points.is_empty() {
        return stages.to_vec();
    }
    let build = |x: &[f64]| -> Vec<Stage> {
        let mut s = stages.to_vec();
        for (f, v) in free.iter().zip(x) {
            let value = v.exp().clamp(f.min, f.max);
            s[f.stage] = s[f.stage].with_value(value);
        }
        s
    };
    let score = |x: &[f64]| cost(&build(x), points, z0, loss_weight);
    let start: Vec<f64> = free
        .iter()
        .map(|f| {
            stages[f.stage]
                .value()
                .unwrap_or(f.min)
                .clamp(f.min, f.max)
                .max(1e-30)
                .ln()
        })
        .collect();
    let mut best = (start.clone(), score(&start));
    // A few restarts from the best so far, with smaller steps each time.
    for round in 0..4 {
        let step = 0.4 / f64::from(round + 1);
        let mut simplex: Vec<(Vec<f64>, f64)> = (0..=n)
            .map(|k| {
                let mut x = best.0.clone();
                if k > 0 {
                    x[k - 1] += step;
                }
                let c = score(&x);
                (x, c)
            })
            .collect();
        for _ in 0..400 * n {
            simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
            let centroid: Vec<f64> = (0..n)
                .map(|d| simplex[..n].iter().map(|p| p.0[d]).sum::<f64>() / n as f64)
                .collect();
            let towards = |t: f64| -> Vec<f64> {
                (0..n)
                    .map(|d| centroid[d] + t * (simplex[n].0[d] - centroid[d]))
                    .collect()
            };
            let refl = towards(-1.0);
            let fr = score(&refl);
            if fr < simplex[0].1 {
                let exp = towards(-2.0);
                let fe = score(&exp);
                simplex[n] = if fe < fr { (exp, fe) } else { (refl, fr) };
            } else if fr < simplex[n - 1].1 {
                simplex[n] = (refl, fr);
            } else {
                let con = towards(0.5);
                let fc = score(&con);
                if fc < simplex[n].1 {
                    simplex[n] = (con, fc);
                } else {
                    let first = simplex[0].0.clone();
                    for p in simplex.iter_mut().skip(1) {
                        for (x, b) in p.0.iter_mut().zip(&first) {
                            *x = b + 0.5 * (*x - b);
                        }
                        p.1 = score(&p.0);
                    }
                }
            }
            if (simplex[n].1 - simplex[0].1).abs() < 1e-10 {
                break;
            }
        }
        simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
        if simplex[0].1 < best.1 {
            best = simplex[0].clone();
        }
    }
    build(&best.0)
}

/// Where a matching network goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// At the antenna (after the balun, before the cable).
    Antenna,
    /// At the radio (a tuner).
    Radio,
}

/// Starting L networks for the chain `before` + network + `after` (stages
/// from the antenna), worked out at `mhz` for antenna impedance `z`: the
/// network matches what it sees there to the line beyond it (`z_line`, the
/// cable's impedance at the antenna, or the radio's `z0` at the radio).
/// Each comes with the indices of its two parts in the full chain.
pub fn l_starts(
    before: &[Stage],
    after: &[Stage],
    z: Complex64,
    mhz: f64,
    z_line: f64,
    q: f64,
) -> Vec<(Vec<Stage>, [usize; 2])> {
    let (seen, _, _) = evaluate(before, z, mhz, z_line);
    matching::l_networks(seen, z_line, mhz)
        .into_iter()
        .filter_map(|net| {
            let parts: Vec<Stage> = net
                .iter()
                .filter_map(|e| match *e {
                    Element::Series(part) => Some(Stage::Series { part, q }),
                    Element::Shunt(part) => Some(Stage::Shunt { part, q }),
                    _ => None,
                })
                .collect();
            (parts.len() == 2).then(|| {
                let k = before.len();
                let mut chain = before.to_vec();
                chain.extend(parts);
                chain.extend_from_slice(after);
                (chain, [k, k + 1])
            })
        })
        .collect()
}

/// Starting pi (or T, `pi` false) networks for the chain `before` +
/// network + `after`, worked out at `mhz` like [`l_starts`], at a low
/// loaded Q (3, or just above the least this match allows): three parts,
/// with their indices in the chain.
pub fn pi_t_starts(
    before: &[Stage],
    after: &[Stage],
    z: Complex64,
    mhz: f64,
    z_line: f64,
    q: f64,
    pi: bool,
) -> Vec<(Vec<Stage>, Vec<usize>)> {
    let (seen, _, _) = evaluate(before, z, mhz, z_line);
    let design = |loaded: f64| {
        if pi {
            matching::pi_networks(seen, z_line, mhz, loaded)
        } else {
            matching::t_networks(seen, z_line, mhz, loaded)
        }
    };
    let nets = match design(3.0) {
        Ok(n) => n,
        Err(least) => design(least * 1.2).unwrap_or_default(),
    };
    nets.into_iter()
        .filter_map(|net| {
            let parts: Vec<Stage> = net
                .iter()
                .filter_map(|e| match *e {
                    Element::Series(part) => Some(Stage::Series { part, q }),
                    Element::Shunt(part) => Some(Stage::Shunt { part, q }),
                    _ => None,
                })
                .collect();
            (parts.len() == net.len()).then(|| {
                let k = before.len();
                let idx: Vec<usize> = (k..k + parts.len()).collect();
                let mut chain = before.to_vec();
                chain.extend(parts);
                chain.extend_from_slice(after);
                (chain, idx)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::feedline::{CABLES, Feed, at_radio, line_input};

    fn rg213() -> Cable {
        CABLES[2]
    }

    #[test]
    fn the_chain_agrees_with_the_feedline_maths() {
        // Balun + cable: the same impedance and loss as at_radio.
        let z = Complex64::new(120.0, -60.0);
        let chain = [
            Stage::Balun {
                ratio: 4.0,
                loss_db: 0.2,
            },
            Stage::Line {
                cable: rg213(),
                length: 20.0,
            },
        ];
        let (zin, swr, eff) = evaluate(&chain, z, 14.2, 50.0);
        let feed = Feed {
            cable: rg213(),
            length_m: 20.0,
            balun_ratio: 4.0,
            balun_loss_db: 0.2,
            antenna_connector: None,
            connector_length_m: 0.0,
            radio_connector: None,
        };
        let r = at_radio(&feed, 14.2, z);
        assert!((zin - r.impedance).norm() < 1e-6, "{zin} {}", r.impedance);
        assert!((swr - r.swr_line).abs() < 1e-6);
        assert!(
            (-10.0 * eff.log10() - r.total_loss_db).abs() < 1e-6,
            "{eff} {r:?}"
        );
        assert!((zin - line_input(&rg213(), 20.0, 14.2, z / 4.0)).norm() < 1e-6);
    }

    #[test]
    fn the_feedline_views_feed_as_a_chain_agrees() {
        // Connectors at both ends and 40 mm of line inside the antenna's:
        // the chain gives at_radio's impedance, SWR and loss exactly.
        use crate::analysis::feedline::CONNECTORS;
        let feed = Feed {
            cable: CABLES[3],
            length_m: 2.5,
            balun_ratio: 1.0,
            balun_loss_db: 0.1,
            antenna_connector: Some(CONNECTORS[1]),
            connector_length_m: 0.04,
            radio_connector: Some(CONNECTORS[2]),
        };
        let z = Complex64::new(35.0, 40.0);
        let (chain, line) = chain_from_feed(&feed, 1296.0);
        assert!(matches!(chain[line], Stage::Line { length, .. } if length == 2.5));
        let (zin, swr, eff) = evaluate(&chain, z, 1296.0, 50.0);
        let r = at_radio(&feed, 1296.0, z);
        assert!((zin - r.impedance).norm() < 1e-6, "{zin} {}", r.impedance);
        assert!((swr - r.swr_line).abs() < 1e-6);
        assert!(
            (-10.0 * eff.log10() - r.total_loss_db).abs() < 1e-6,
            "{eff} {r:?}"
        );
    }

    #[test]
    fn an_l_network_matches_and_its_parts_cost_power() {
        let z = Complex64::new(12.0, -35.0);
        let starts = l_starts(&[], &[], z, 7.1, 50.0, 1e9);
        assert!(!starts.is_empty());
        for (chain, _) in &starts {
            let (_, swr, eff) = evaluate(chain, z, 7.1, 50.0);
            assert!(swr < 1.0001 && (eff - 1.0).abs() < 1e-6, "{swr} {eff}");
        }
        // With Q 100 the same network loses some power.
        let lossy: Vec<Stage> = starts[0]
            .0
            .iter()
            .map(|s| match *s {
                Stage::Series { part, .. } => Stage::Series { part, q: 100.0 },
                Stage::Shunt { part, .. } => Stage::Shunt { part, q: 100.0 },
                s => s,
            })
            .collect();
        let (_, _, eff) = evaluate(&lossy, z, 7.1, 50.0);
        assert!(eff < 0.99 && eff > 0.8, "{eff}");
    }

    #[test]
    fn tuning_over_a_band_beats_the_centre_match() {
        // A short dipole-like load across 7.0-7.3 MHz: R rising, X swinging
        // through zero. The centre-frequency L network is perfect at 7.15
        // only; tuned over the band its worst SWR drops.
        let points: Vec<(f64, Complex64)> = (0..=12)
            .map(|k| {
                let f = 7.0 + 0.025 * k as f64;
                let d = (f - 7.15) / 0.15;
                (f, Complex64::new(25.0 + 5.0 * d, 60.0 * d))
            })
            .collect();
        let starts = l_starts(&[], &[], points[6].1, 7.15, 50.0, 200.0);
        let (chain, idx) = starts[0].clone();
        let before = worst(&chain, &points, 50.0).0;
        let free: Vec<Free> = idx
            .iter()
            .map(|&k| {
                let v = chain[k].value().unwrap();
                Free {
                    stage: k,
                    min: v / 10.0,
                    max: v * 10.0,
                }
            })
            .collect();
        let tuned = optimise(&chain, &free, &points, 50.0, 0.0);
        let after = worst(&tuned, &points, 50.0).0;
        println!("worst SWR {before:.3} -> {after:.3}");
        assert!(after < before - 0.05, "{before} {after}");
        // Snapping to E12 changes it, but it still works.
        let snapped: Vec<Stage> = tuned.iter().map(|s| s.snapped(12)).collect();
        assert!(worst(&snapped, &points, 50.0).0 < 2.0 * after + 1.0);
    }

    #[test]
    fn pi_and_t_starts_match() {
        let z = Complex64::new(300.0, 120.0);
        for pi in [true, false] {
            let starts = pi_t_starts(&[], &[], z, 3.6, 50.0, 1e9, pi);
            assert!(!starts.is_empty(), "pi {pi}");
            for (chain, idx) in &starts {
                assert_eq!(idx.len(), 3);
                let (_, swr, _) = evaluate(chain, z, 3.6, 50.0);
                assert!(swr < 1.001, "pi {pi}: {swr}");
            }
        }
    }

    #[test]
    fn a_shorted_quarter_wave_stub_is_invisible() {
        // A shorted quarter-wave stub is an open circuit: no effect.
        let ideal = CABLES[7];
        let q = C / 14e6 / 4.0;
        let z = Complex64::new(30.0, 20.0);
        let (zin, _, _) = evaluate(
            &[Stage::Stub {
                cable: ideal,
                length: q,
                shorted: true,
            }],
            z,
            14.0,
            50.0,
        );
        assert!((zin - z).norm() < 1e-6, "{zin}");
    }
}
