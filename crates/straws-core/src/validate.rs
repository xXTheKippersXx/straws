//! Comparing two sets of NEC results, e.g. nec2c against 4nec2's engine.
//!
//! Tolerances are set by how many digits the engines print: nec2c prints
//! five significant figures and NEC-2D six, and both print gains to
//! 0.01 dB. Deep pattern nulls are excluded from the gain check because a
//! null's depth is extremely sensitive to tiny numerical differences.

use std::fmt;

use crate::output::{FrequencyResult, NecOutput};

#[derive(Debug, Clone, Copy)]
pub struct Tolerances {
    /// Largest allowed |ΔZ| as a fraction of |Z|.
    pub impedance_rel: f64,
    /// Plus this many ohms (for impedances near zero).
    pub impedance_abs: f64,
    /// Largest allowed |ΔI| as a fraction of the largest current.
    pub current_rel: f64,
    /// Largest allowed gain difference in dB at the pattern peak...
    pub gain_db: f64,
    /// ...plus this many dB for every dB below the peak, because weaker
    /// directions are more sensitive to tiny numerical differences...
    pub gain_db_per_db_down: f64,
    /// ...checked only where the gain is within this many dB of the peak.
    pub gain_range_db: f64,
    /// Largest allowed average-gain difference as a fraction.
    pub average_gain_rel: f64,
    /// Largest allowed efficiency difference in percentage points.
    pub efficiency_pct: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            impedance_rel: 2e-4,
            impedance_abs: 1e-3,
            current_rel: 2e-4,
            gain_db: 0.02,
            gain_db_per_db_down: 0.003,
            gain_range_db: 40.0,
            average_gain_rel: 1e-3,
            efficiency_pct: 0.02,
        }
    }
}

/// The largest differences found, and anything that failed.
#[derive(Debug, Clone, Default)]
pub struct Comparison {
    pub frequencies: usize,
    pub max_impedance_rel: f64,
    pub max_current_rel: f64,
    pub max_gain_db: f64,
    pub max_average_gain_rel: f64,
    pub max_efficiency_pct: f64,
    pub pattern_points: usize,
    pub failures: Vec<String>,
}

impl Comparison {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    fn fail(&mut self, msg: String) {
        // Keep reports readable on badly broken runs.
        if self.failures.len() < 20 {
            self.failures.push(msg);
        }
    }
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} freq, max ΔZ {:.1e} (rel), max ΔI {:.1e} (rel), max Δgain {:.3} dB over {} points",
            self.frequencies,
            self.max_impedance_rel,
            self.max_current_rel,
            self.max_gain_db,
            self.pattern_points
        )
    }
}

/// Compares `ours` against `reference`.
pub fn compare(ours: &NecOutput, reference: &NecOutput, tol: &Tolerances) -> Comparison {
    let mut c = Comparison {
        frequencies: reference.frequencies.len(),
        ..Default::default()
    };
    if ours.frequencies.len() != reference.frequencies.len() {
        c.fail(format!(
            "{} frequencies computed, reference has {}",
            ours.frequencies.len(),
            reference.frequencies.len()
        ));
    }
    if reference.frequencies.is_empty() {
        c.fail("reference has no results".to_string());
    }
    for (a, b) in ours.frequencies.iter().zip(&reference.frequencies) {
        compare_frequency(a, b, tol, &mut c);
    }
    c
}

fn compare_frequency(
    a: &FrequencyResult,
    b: &FrequencyResult,
    tol: &Tolerances,
    c: &mut Comparison,
) {
    let mhz = b.mhz;
    if (a.mhz - b.mhz).abs() > 1e-4 * b.mhz.abs().max(1e-9) {
        c.fail(format!("frequency {} MHz vs reference {mhz} MHz", a.mhz));
        return;
    }

    if a.inputs.len() != b.inputs.len() {
        c.fail(format!(
            "{mhz} MHz: {} sources vs reference {}",
            a.inputs.len(),
            b.inputs.len()
        ));
    }
    for rb in &b.inputs {
        let Some(ra) = a
            .inputs
            .iter()
            .find(|r| r.tag == rb.tag && r.segment == rb.segment)
        else {
            c.fail(format!(
                "{mhz} MHz: no result for source at tag {} seg {}",
                rb.tag, rb.segment
            ));
            continue;
        };
        let dz = (ra.impedance - rb.impedance).norm();
        let rel = dz / rb.impedance.norm().max(1e-12);
        c.max_impedance_rel = c.max_impedance_rel.max(rel);
        if dz > tol.impedance_rel * rb.impedance.norm() + tol.impedance_abs {
            c.fail(format!(
                "{mhz} MHz tag {} seg {}: Z = {:.4} {:+.4}j, reference {:.4} {:+.4}j",
                rb.tag,
                rb.segment,
                ra.impedance.re,
                ra.impedance.im,
                rb.impedance.re,
                rb.impedance.im
            ));
        }
    }

    if !b.currents.is_empty() {
        if a.currents.len() != b.currents.len() {
            c.fail(format!(
                "{mhz} MHz: {} segment currents vs reference {}",
                a.currents.len(),
                b.currents.len()
            ));
        } else {
            let peak = b
                .currents
                .iter()
                .map(|s| s.current.norm())
                .fold(0.0, f64::max)
                .max(1e-30);
            for (sa, sb) in a.currents.iter().zip(&b.currents) {
                let rel = (sa.current - sb.current).norm() / peak;
                c.max_current_rel = c.max_current_rel.max(rel);
                if rel > tol.current_rel {
                    c.fail(format!(
                        "{mhz} MHz segment {}: current differs by {:.2e} of peak",
                        sb.segment, rel
                    ));
                }
            }
        }
    }

    if let (Some(pa), Some(pb)) = (a.power, b.power) {
        let d = (pa.efficiency_percent - pb.efficiency_percent).abs();
        c.max_efficiency_pct = c.max_efficiency_pct.max(d);
        if d > tol.efficiency_pct {
            c.fail(format!(
                "{mhz} MHz: efficiency {:.2}% vs reference {:.2}%",
                pa.efficiency_percent, pb.efficiency_percent
            ));
        }
    }

    if a.patterns.len() != b.patterns.len() {
        c.fail(format!(
            "{mhz} MHz: {} pattern tables vs reference {}",
            a.patterns.len(),
            b.patterns.len()
        ));
    }
    for (ta, tb) in a.patterns.iter().zip(&b.patterns) {
        if ta.points.len() != tb.points.len() {
            c.fail(format!(
                "{mhz} MHz: {} pattern points vs reference {}",
                ta.points.len(),
                tb.points.len()
            ));
            continue;
        }
        let peak = tb
            .points
            .iter()
            .map(|p| p.gain_total)
            .fold(f64::MIN, f64::max);
        for (pa, pb) in ta.points.iter().zip(&tb.points) {
            if (pa.theta - pb.theta).abs() > 0.01 || (pa.phi - pb.phi).abs() > 0.01 {
                c.fail(format!(
                    "{mhz} MHz: pattern angles ({}, {}) vs reference ({}, {})",
                    pa.theta, pa.phi, pb.theta, pb.phi
                ));
                break;
            }
            for (ga, gb) in [
                (pa.gain_total, pb.gain_total),
                (pa.gain_vert, pb.gain_vert),
                (pa.gain_hor, pb.gain_hor),
            ] {
                if gb < peak - tol.gain_range_db || ga < peak - tol.gain_range_db {
                    continue;
                }
                c.pattern_points += 1;
                let d = (ga - gb).abs();
                c.max_gain_db = c.max_gain_db.max(d);
                // The tiny epsilon stops print rounding (0.01 dB steps) from
                // failing an exact-limit difference.
                let allowed = tol.gain_db + tol.gain_db_per_db_down * (peak - gb) + 1e-9;
                if d > allowed {
                    c.fail(format!(
                        "{mhz} MHz θ={} φ={}: gain {ga:.2} dB vs reference {gb:.2} dB",
                        pb.theta, pb.phi
                    ));
                }
            }
        }
        if let (Some(ga), Some(gb)) = (ta.average_gain, tb.average_gain) {
            let rel = (ga - gb).abs() / gb.abs().max(1e-12);
            c.max_average_gain_rel = c.max_average_gain_rel.max(rel);
            if rel > tol.average_gain_rel {
                c.fail(format!(
                    "{mhz} MHz: average gain {ga:.5} vs reference {gb:.5}"
                ));
            }
        }
    }
}
