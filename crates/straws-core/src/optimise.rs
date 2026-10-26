//! The optimiser: adjusts a deck's variables, within limits, to meet goals
//! such as low SWR across one or more bands, gain (in a direction or at the
//! peak), front-to-back and front-to-rear ratio.
//!
//! Each goal gives a penalty: zero once its target is met, growing as the
//! design falls short, plus a tiny reward for doing better still so a
//! design that meets everything keeps improving. The cost to minimise is
//! the weighted sum.
//!
//! Speed: every goal of a design is worked out in one engine run (one deck
//! with an FR card per goal), designs are scored many at a time on every
//! core, and designs already tried are remembered.
//!
//! Searches: Nelder-Mead (local; each step tries all its candidate moves at
//! once, and restarts when it settles) and a global search (adaptive
//! differential evolution, JADE-style "current-to-pbest"), which finishes
//! with a Nelder-Mead polish of its best design.
//!
//! Designs that can't be built (wires crossing or lying on top of each
//! other, wires below the ground, or bigger than the build limits) are
//! skipped: they get the cost of a design that failed to run.

use num_complex::Complex64;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::analysis::feedsystem::{Stage, evaluate};
use crate::analysis::plan::{FrequencyPlan, has_ground, run_deck};
use crate::analysis::{buried, checks, feed};
use crate::deck::{Deck, NumCard, NumericDeck};
use crate::engine::{CancelToken, Engine};
use crate::output::FrequencyResult;

/// A variable the optimiser may change, between `min` and `max`.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub min: f64,
    pub max: f64,
    /// Where to start (the current value).
    pub start: f64,
}

/// The feed chain an SWR goal can be measured through, so it's the SWR at
/// the radio rather than at the feed point: its stages from the antenna,
/// and the stages whose value a variable sets (the feedline's length, the
/// balun's ratio), by the variable's name, so the optimiser can tune the
/// feed with the antenna.
#[derive(Debug, Clone, PartialEq)]
pub struct RadioChain {
    pub stages: Vec<Stage>,
    pub vars: Vec<(usize, String)>,
}

impl RadioChain {
    /// The stages with the variables' values from `symbols`.
    pub fn with_symbols(&self, symbols: &[(String, f64)]) -> Vec<Stage> {
        let mut stages = self.stages.clone();
        for (k, name) in &self.vars {
            if let (Some(s), Some((_, v))) = (
                stages.get_mut(*k),
                symbols.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)),
            ) {
                *s = s.with_value(*v);
            }
        }
        stages
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Goal {
    /// SWR at or below `target` across `start_mhz`..`stop_mhz`: at the feed
    /// point, or at the radio `through` a feed chain.
    Swr {
        start_mhz: f64,
        stop_mhz: f64,
        points: usize,
        z0: f64,
        target: f64,
        weight: f64,
        through: Option<RadioChain>,
    },
    /// Gain (dBi) at `mhz`, at least `target`: towards (elevation, azimuth),
    /// or the pattern's peak wherever it is (`peak`).
    Gain {
        mhz: f64,
        elevation: f64,
        azimuth: f64,
        peak: bool,
        target: f64,
        weight: f64,
    },
    /// Common-mode current: the current on the outside of the coax (the
    /// top of the shield the Feedline view models) at most `target` per
    /// cent of the feed current, at `mhz`. Needs the shield in the model.
    CommonMode { mhz: f64, target: f64, weight: f64 },
    /// The feed impedance matches a measurement (`name`): `measured` holds
    /// the measured impedances at evenly spaced frequencies from `start_mhz`
    /// to `stop_mhz`. Scored by the RMS difference between the two
    /// reflection coefficients on `z0` (what a VNA measures), at most
    /// `target`.
    Match {
        name: String,
        start_mhz: f64,
        stop_mhz: f64,
        measured: Vec<Complex64>,
        z0: f64,
        target: f64,
        weight: f64,
    },
    /// Front-to-back ratio (dB) towards (elevation, azimuth), at least
    /// `target`. With `rear`, front-to-rear: against the strongest
    /// direction anywhere in the back half at that elevation.
    FrontToBack {
        mhz: f64,
        elevation: f64,
        azimuth: f64,
        rear: bool,
        target: f64,
        weight: f64,
    },
}

impl Goal {
    pub fn weight(&self) -> f64 {
        match self {
            Goal::Swr { weight, .. }
            | Goal::Match { weight, .. }
            | Goal::Gain { weight, .. }
            | Goal::CommonMode { weight, .. }
            | Goal::FrontToBack { weight, .. } => *weight,
        }
    }

    /// A goal to match a measured sweep with `n` frequencies evenly spread
    /// over `start_mhz`..`stop_mhz` (kept within the measurement), the
    /// measured impedance at each interpolated. `None` when the range misses
    /// the measurement.
    pub fn fit_to(
        name: &str,
        points: &[crate::analysis::measured::MeasuredPoint],
        start_mhz: f64,
        stop_mhz: f64,
        n: usize,
        z0: f64,
    ) -> Option<Goal> {
        let (first, last) = (points.first()?.mhz, points.last()?.mhz);
        let (a, b) = (start_mhz.max(first), stop_mhz.min(last));
        if a.is_nan() || b.is_nan() || a > b {
            return None;
        }
        let n = if b > a { n.max(2) } else { 1 };
        let plan = FrequencyPlan {
            start_mhz: a,
            stop_mhz: b,
            points: n,
        };
        let measured = plan
            .frequencies()
            .iter()
            .map(|&f| crate::analysis::measured::interpolate(points, f))
            .collect::<Option<Vec<_>>>()?;
        Some(Goal::Match {
            name: name.to_string(),
            start_mhz: a,
            stop_mhz: b,
            measured,
            z0,
            target: 0.05,
            weight: 1.0,
        })
    }

    fn frequencies(&self) -> (f64, f64) {
        match self {
            Goal::Swr {
                start_mhz,
                stop_mhz,
                ..
            }
            | Goal::Match {
                start_mhz,
                stop_mhz,
                ..
            } => (*start_mhz, *stop_mhz),
            Goal::Gain { mhz, .. }
            | Goal::FrontToBack { mhz, .. }
            | Goal::CommonMode { mhz, .. } => (*mhz, *mhz),
        }
    }

    /// Every frequency the goal's runs use, in the order they come back.
    fn check_frequencies(&self) -> Vec<f64> {
        match self {
            Goal::Swr {
                start_mhz,
                stop_mhz,
                points,
                ..
            } => FrequencyPlan {
                start_mhz: *start_mhz,
                stop_mhz: stop_mhz.max(*start_mhz),
                points: (*points).max(1),
            }
            .frequencies(),
            Goal::Match {
                start_mhz,
                stop_mhz,
                measured,
                ..
            } => FrequencyPlan {
                start_mhz: *start_mhz,
                stop_mhz: stop_mhz.max(*start_mhz),
                points: measured.len().max(1),
            }
            .frequencies(),
            Goal::Gain { mhz, .. }
            | Goal::FrontToBack { mhz, .. }
            | Goal::CommonMode { mhz, .. } => vec![*mhz],
        }
    }

    /// How many of the run's frequency results belong to this goal.
    fn result_count(&self) -> usize {
        match self {
            Goal::Swr { points, .. } => (*points).max(1),
            Goal::Match { measured, .. } => measured.len().max(1),
            _ => 1,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Goal::Swr {
                start_mhz,
                stop_mhz,
                target,
                z0,
                through,
                ..
            } => format!(
                "SWR ≤ {target} ({z0} Ω) from {start_mhz} to {stop_mhz} MHz{}",
                if through.is_some() {
                    " at the radio"
                } else {
                    ""
                }
            ),
            Goal::Gain {
                mhz,
                elevation,
                azimuth,
                peak,
                target,
                ..
            } => {
                if *peak {
                    format!("Peak gain ≥ {target} dBi at {mhz} MHz")
                } else {
                    format!(
                        "Gain ≥ {target} dBi at {mhz} MHz, elevation {elevation}°, azimuth {azimuth}°"
                    )
                }
            }
            Goal::CommonMode { mhz, target, .. } => {
                format!("Common-mode current ≤ {target} % of the feed current at {mhz} MHz")
            }
            Goal::Match {
                name,
                start_mhz,
                stop_mhz,
                target,
                ..
            } => format!(
                "Match {name} from {start_mhz:.3} to {stop_mhz:.3} MHz (RMS reflection difference ≤ {target})"
            ),
            Goal::FrontToBack {
                mhz,
                elevation,
                azimuth,
                rear,
                target,
                ..
            } => format!(
                "{} ≥ {target} dB at {mhz} MHz, towards azimuth {azimuth}°, elevation {elevation}°",
                if *rear {
                    "Front-to-rear"
                } else {
                    "Front-to-back"
                }
            ),
        }
    }
}

/// How one design did against one goal.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalResult {
    /// The figure itself: worst SWR, gain in dBi, or F/B (F/R) in dB.
    pub value: f64,
    pub penalty: f64,
    /// The target is reached.
    pub met: bool,
}

/// A design the optimiser has tried.
#[derive(Debug, Clone, PartialEq)]
pub struct Trial {
    /// Variable values, in the order of the params.
    pub values: Vec<f64>,
    pub cost: f64,
    /// One per goal, or empty if the design couldn't be run.
    pub results: Vec<GoalResult>,
    /// Why the design was skipped without scoring, if it was.
    pub rejected: Option<String>,
}

impl Trial {
    /// Every goal is met.
    pub fn all_met(&self) -> bool {
        !self.results.is_empty() && self.results.iter().all(|r| r.met)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Nelder-Mead from the current design.
    NelderMead,
    /// Differential evolution over the whole range, then a local polish.
    DifferentialEvolution,
}

/// Cost given to designs the engine can't run.
pub const FAILED: f64 = 1e9;

/// The problem: a deck, the variables to change and the goals.
pub struct Problem {
    pub deck: Deck,
    /// Variable values that stay fixed (from the Variables view).
    pub fixed: Vec<(String, f64)>,
    pub params: Vec<Param>,
    pub goals: Vec<Goal>,
    pub limits: Limits,
}

/// What a design must keep to, or it's skipped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Limits {
    /// Most clashes ([`checks::clashes`]: wires crossing, overlapping or
    /// underground) allowed, normally the starting design's count; `None`
    /// doesn't check.
    pub clashes: Option<usize>,
    /// Largest overall size along x, y and z, in metres.
    pub size: [Option<f64>; 3],
}

impl Limits {
    /// Why the run's geometry breaks these limits, if it does.
    fn broken(
        &self,
        deck: &Deck,
        numeric: &NumericDeck,
        segments: &[crate::output::Segment],
    ) -> Option<String> {
        if let Some(allowed) = self.clashes {
            let found = checks::clashes(segments, numeric, has_ground(numeric));
            if found.len() > allowed {
                return Some(found[0].message.clone());
            }
        }
        if self.size.iter().any(Option::is_some) {
            let shield = crate::deck::shield::shield_tag(deck);
            let mut lo = [f64::MAX; 3];
            let mut hi = [f64::MIN; 3];
            for s in segments {
                let tag = i64::from(s.tag);
                if numeric.helper_tags.contains(&tag) || shield == Some(tag) {
                    continue;
                }
                let (a, b) = s.ends();
                for k in 0..3 {
                    lo[k] = lo[k].min(a[k]).min(b[k]);
                    hi[k] = hi[k].max(a[k]).max(b[k]);
                }
            }
            for (k, limit) in self.size.iter().enumerate() {
                if let Some(limit) = limit
                    && hi[k] - lo[k] > limit * (1.0 + 1e-9)
                {
                    return Some(format!(
                        "{:.3} m along {} (limit {limit:.3} m)",
                        hi[k] - lo[k],
                        ["x", "y", "z"][k]
                    ));
                }
            }
        }
        None
    }
}

/// Azimuth step (degrees) of the ring used for front-to-rear.
const RING_STEP: f64 = 5.0;
/// Grid step (degrees) used to find the peak gain.
const PEAK_STEP: f64 = 5.0;

/// One deck that answers every goal: the model, then for each goal its own
/// FR card and an XQ (SWR) or RP (gain) card. Results come back in order.
pub fn goal_deck(deck: &NumericDeck, goals: &[Goal]) -> NumericDeck {
    let (lo, hi) = goals
        .iter()
        .map(Goal::frequencies)
        .fold((f64::MAX, f64::MIN), |(lo, hi), (a, b)| {
            (lo.min(a), hi.max(b))
        });
    let centre = if lo <= hi { (lo + hi) / 2.0 } else { 14.0 };
    let mut run = run_deck(deck, &FrequencyPlan::single(centre), None);
    // Drop the FR and XQ run_deck added; ask for no current tables (less
    // text to write and read).
    run.cards.truncate(run.cards.len().saturating_sub(2));
    run.cards.push(NumCard::new("PT", &[-1], &[]));
    let over_ground = has_ground(deck);
    for g in goals {
        match g {
            Goal::Swr {
                start_mhz,
                stop_mhz,
                points,
                ..
            } => {
                let plan = FrequencyPlan {
                    start_mhz: *start_mhz,
                    stop_mhz: stop_mhz.max(*start_mhz),
                    points: (*points).max(1),
                };
                run.cards.push(NumCard::new(
                    "FR",
                    &[0, plan.points as i64],
                    &[plan.start_mhz, plan.step_mhz()],
                ));
                run.cards.push(NumCard::new("XQ", &[0], &[]));
            }
            Goal::Match {
                start_mhz,
                stop_mhz,
                measured,
                ..
            } => {
                let plan = FrequencyPlan {
                    start_mhz: *start_mhz,
                    stop_mhz: stop_mhz.max(*start_mhz),
                    points: measured.len().max(1),
                };
                run.cards.push(NumCard::new(
                    "FR",
                    &[0, plan.points as i64],
                    &[plan.start_mhz, plan.step_mhz()],
                ));
                run.cards.push(NumCard::new("XQ", &[0], &[]));
            }
            Goal::Gain {
                mhz,
                elevation,
                azimuth,
                peak,
                ..
            } => {
                run.cards.push(NumCard::new("FR", &[0, 1], &[*mhz, 0.0]));
                run.cards.push(if *peak {
                    let nth = if over_ground {
                        90.0 / PEAK_STEP + 1.0
                    } else {
                        180.0 / PEAK_STEP + 1.0
                    };
                    NumCard::new(
                        "RP",
                        &[0, nth as i64, (360.0 / PEAK_STEP) as i64, 1000],
                        &[0.0, 0.0, PEAK_STEP, PEAK_STEP],
                    )
                } else {
                    NumCard::new(
                        "RP",
                        &[0, 1, 1, 1000],
                        &[90.0 - elevation, *azimuth, 0.0, 0.0],
                    )
                });
            }
            Goal::CommonMode { mhz, .. } => {
                // Currents for this run only (PT -1 again after it).
                run.cards.push(NumCard::new("PT", &[0, 0, 0, 0], &[]));
                run.cards.push(NumCard::new("FR", &[0, 1], &[*mhz, 0.0]));
                run.cards.push(NumCard::new("XQ", &[0], &[]));
                run.cards.push(NumCard::new("PT", &[-1], &[]));
            }
            Goal::FrontToBack {
                mhz,
                elevation,
                azimuth,
                ..
            } => {
                run.cards.push(NumCard::new("FR", &[0, 1], &[*mhz, 0.0]));
                // A ring of azimuths at this elevation, starting at the front.
                run.cards.push(NumCard::new(
                    "RP",
                    &[0, 1, (360.0 / RING_STEP) as i64, 1000],
                    &[90.0 - elevation, *azimuth, 0.0, RING_STEP],
                ));
            }
        }
    }
    run
}

/// A run's results with the feed impedance `z` from the buried-radial
/// estimate: gains move by the change in feed resistance (the same current
/// shape, fed with the same voltage), as `FreqPoint::with_estimate` does.
fn with_estimate(r: &mut FrequencyResult, z: num_complex::Complex64) {
    let Some(feed) = r.inputs.first_mut() else {
        return;
    };
    let old = feed.impedance.re;
    if z.re <= 0.0 || old <= 0.0 {
        return;
    }
    feed.impedance = z;
    let db = 10.0 * (old / z.re).log10();
    for p in r.patterns.iter_mut().flat_map(|t| t.points.iter_mut()) {
        for g in [&mut p.gain_vert, &mut p.gain_hor, &mut p.gain_total] {
            if *g > -999.0 {
                *g += db;
            }
        }
    }
}

/// The chain with its cables' loss taken out (the parts and balun keep
/// theirs).
fn without_cable_loss(stages: &[Stage]) -> Vec<Stage> {
    stages
        .iter()
        .map(|s| match *s {
            Stage::Line { mut cable, length } => {
                cable.loss_10 = 0.0;
                cable.loss_100 = 0.0;
                Stage::Line { cable, length }
            }
            Stage::Stub {
                mut cable,
                length,
                shorted,
            } => {
                cable.loss_10 = 0.0;
                cable.loss_100 = 0.0;
                Stage::Stub {
                    cable,
                    length,
                    shorted,
                }
            }
            s => s,
        })
        .collect()
}

/// Scores one goal from its share of the run's results.
fn score_goal(
    goal: &Goal,
    results: &[FrequencyResult],
    symbols: &[(String, f64)],
    shield: Option<i64>,
) -> Option<GoalResult> {
    match goal {
        Goal::CommonMode { target, .. } => {
            let f = results.first()?;
            let tag = u32::try_from(shield?).ok()?;
            // The shield's first segment is its top, at the feed.
            let on_shield = f.currents.iter().find(|c| c.tag == tag)?.current.norm();
            let feed = f.inputs.first()?.current.norm();
            if feed <= 0.0 {
                return None;
            }
            let percent = 100.0 * on_shield / feed;
            Some(GoalResult {
                value: percent,
                penalty: 0.1 * (percent - target).max(0.0) + 0.0001 * percent,
                met: percent <= *target,
            })
        }
        Goal::Swr {
            z0,
            target,
            through,
            ..
        } => {
            // Through a feed chain: the SWR at the radio as the cables would
            // give it without their loss (loss only hides a mismatch, and a
            // longer, lossier cable mustn't pass for a better match), and the
            // real loss (dB), which counts against the design separately.
            let chain = through.as_ref().map(|c| c.with_symbols(symbols));
            let lossless = chain.as_ref().map(|c| without_cable_loss(c));
            let pairs: Vec<(f64, f64)> = results
                .iter()
                .map(|f| {
                    f.inputs.first().map(|i| match (&chain, &lossless) {
                        (Some(c), Some(l)) => {
                            let (_, swr, _) = evaluate(l, i.impedance, f.mhz, *z0);
                            let (_, _, eff) = evaluate(c, i.impedance, f.mhz, *z0);
                            (swr.min(100.0), -10.0 * eff.max(1e-12).log10())
                        }
                        _ => (feed::swr(i.impedance, *z0).min(100.0), 0.0),
                    })
                })
                .collect::<Option<_>>()?;
            let swrs: Vec<f64> = pairs.iter().map(|p| p.0).collect();
            let loss = pairs.iter().map(|p| p.1).fold(0.0, f64::max);
            if swrs.is_empty() {
                return None;
            }
            let worst = swrs.iter().copied().fold(0.0, f64::max);
            // Every frequency counts, the worst most.
            let short: f64 = swrs
                .iter()
                .map(|s| (s - target).max(0.0).powi(2))
                .sum::<f64>()
                / swrs.len() as f64;
            let mean = swrs.iter().sum::<f64>() / swrs.len() as f64;
            Some(GoalResult {
                value: worst,
                penalty: short + (worst - target).max(0.0) + 0.001 * mean + 0.5 * loss,
                met: worst <= *target,
            })
        }
        Goal::Match {
            measured,
            z0,
            target,
            ..
        } => {
            if results.len() != measured.len() || measured.is_empty() {
                return None;
            }
            let gamma = |z: Complex64| (z - *z0) / (z + *z0);
            let mut sum = 0.0f64;
            for (f, m) in results.iter().zip(measured) {
                sum += (gamma(f.inputs.first()?.impedance) - gamma(*m)).norm_sqr();
            }
            let rms = (sum / measured.len() as f64).sqrt();
            Some(GoalResult {
                value: rms,
                // Always worth closing further: a fit has no "good enough".
                penalty: 10.0 * rms + 10.0 * (rms - target).max(0.0),
                met: rms <= *target,
            })
        }
        Goal::Gain { target, .. } => {
            let points = &results.first()?.patterns.first()?.points;
            let g = points
                .iter()
                .map(|p| p.gain_total)
                .fold(f64::MIN, f64::max)
                .max(-999.0);
            Some(GoalResult {
                value: g,
                penalty: (target - g).max(0.0) - 0.001 * g,
                met: g >= *target,
            })
        }
        Goal::FrontToBack { rear, target, .. } => {
            let points = &results.first()?.patterns.first()?.points;
            let n = points.len();
            if n < 4 {
                return None;
            }
            let gain = |k: usize| points[k % n].gain_total.max(-999.0);
            let front = gain(0);
            let back = if *rear {
                // Everything from 90° to 270° off the front.
                (n / 4..=3 * n / 4).map(gain).fold(f64::MIN, f64::max)
            } else {
                gain(n / 2)
            };
            let fb = (front - back).min(60.0);
            Some(GoalResult {
                value: fb,
                penalty: (target - fb).max(0.0) / 3.0 - 0.001 * fb,
                met: fb >= *target,
            })
        }
    }
}

impl Problem {
    /// Runs the design with `values` and scores it, in one engine run.
    pub fn evaluate(&self, engine: &Engine, values: &[f64], cancel: &CancelToken) -> Trial {
        let failed = || Trial {
            values: values.to_vec(),
            cost: FAILED,
            results: Vec::new(),
            rejected: None,
        };
        let mut overrides = self.fixed.clone();
        for (p, v) in self.params.iter().zip(values) {
            overrides.retain(|(n, _)| !n.eq_ignore_ascii_case(&p.name));
            overrides.push((p.name.clone(), *v));
        }
        let Ok(deck) = self.deck.expand_with(&overrides) else {
            return failed();
        };
        let run = goal_deck(&deck, &self.goals);
        let Ok(mut out) = engine.run(&run, cancel) else {
            return failed();
        };
        if let Some(why) = self.limits.broken(&self.deck, &deck, &out.segments) {
            return Trial {
                rejected: Some(why),
                ..failed()
            };
        }
        // Radials on or under the ground: the feed impedance and gains as
        // their estimate has them (as in the sweep the views show).
        if let buried::Need::Estimate(setup) = buried::need(&self.deck, &deck) {
            let mhz: Vec<f64> = self
                .goals
                .iter()
                .flat_map(Goal::check_frequencies)
                .collect();
            let Ok(estimate) = buried::estimate_at(engine, &deck, &setup, &mhz, 3, cancel) else {
                return failed();
            };
            for (r, f) in out.frequencies.iter_mut().zip(&mhz) {
                if let Some(z) = estimate.z(*f) {
                    with_estimate(r, z);
                }
            }
        }
        let shield = crate::deck::shield::shield_tag(&self.deck);
        let mut rest = out.frequencies.as_slice();
        let mut results = Vec::new();
        let mut cost = 0.0;
        for goal in &self.goals {
            let take = goal.result_count();
            if rest.len() < take {
                return failed();
            }
            let (mine, more) = rest.split_at(take);
            rest = more;
            let Some(r) = score_goal(goal, mine, &deck.symbols, shield) else {
                return failed();
            };
            cost += goal.weight() * r.penalty;
            results.push(r);
        }
        Trial {
            values: values.to_vec(),
            cost,
            results,
            rejected: None,
        }
    }

    fn values_at(&self, unit: &[f64]) -> Vec<f64> {
        self.params
            .iter()
            .zip(unit)
            .map(|(p, u)| p.min + (p.max - p.min) * u.clamp(0.0, 1.0))
            .collect()
    }

    fn unit_of(&self, values: &[f64]) -> Vec<f64> {
        self.params
            .iter()
            .zip(values)
            .map(|(p, v)| {
                if p.max > p.min {
                    ((v - p.min) / (p.max - p.min)).clamp(0.0, 1.0)
                } else {
                    0.5
                }
            })
            .collect()
    }
}

/// Progress reported while optimising.
pub struct Progress<'a> {
    /// Designs run so far (repeats aren't run again and don't count).
    pub evaluations: usize,
    /// The best design so far.
    pub best: &'a Trial,
    /// True when the best design just changed.
    pub improved: bool,
    /// What the search is doing, in a few words.
    pub stage: &'static str,
    /// Designs skipped as impossible to build, and the last reason.
    pub rejected: usize,
    pub last_rejection: Option<&'a str>,
}

/// Options for a run.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub method: Method,
    /// Most designs to run.
    pub max_evaluations: usize,
    /// Stop as soon as a design meets every goal.
    pub stop_when_met: bool,
    /// Designs run at once (normally the number of cores).
    pub threads: usize,
}

impl Options {
    pub fn new(method: Method, max_evaluations: usize) -> Options {
        Options {
            method,
            max_evaluations,
            stop_when_met: false,
            threads: std::thread::available_parallelism().map_or(4, |n| n.get()),
        }
    }
}

/// The shared state of a run: best design, counts and the memory of
/// designs already tried.
struct Search<'a, P: FnMut(Progress<'_>) + Send> {
    problem: &'a Problem,
    engine: &'a Engine,
    cancel: &'a CancelToken,
    options: Options,
    state: Mutex<State>,
    progress: Mutex<P>,
}

struct State {
    best: Trial,
    evaluations: usize,
    rejected: usize,
    last_rejection: Option<String>,
    seen: HashMap<Vec<u64>, f64>,
    stage: &'static str,
}

impl<P: FnMut(Progress<'_>) + Send> Search<'_, P> {
    fn finished(&self) -> bool {
        if self.cancel.is_cancelled() {
            return true;
        }
        let s = self.state.lock().unwrap();
        s.evaluations >= self.options.max_evaluations
            || (self.options.stop_when_met && s.best.all_met())
    }

    fn set_stage(&self, stage: &'static str) {
        self.state.lock().unwrap().stage = stage;
    }

    /// Scores designs (unit coordinates), several at once.
    fn score(&self, batch: &[Vec<f64>]) -> Vec<f64> {
        let costs = Mutex::new(vec![FAILED; batch.len()]);
        let next = Mutex::new(0usize);
        let threads = self.options.threads.max(1).min(batch.len());
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| {
                    loop {
                        let k = {
                            let mut n = next.lock().unwrap();
                            let k = *n;
                            *n += 1;
                            k
                        };
                        if k >= batch.len() || self.finished() {
                            break;
                        }
                        let values = self.problem.values_at(&batch[k]);
                        // Designs already tried aren't run again.
                        let key: Vec<u64> = values.iter().map(|v| v.to_bits()).collect();
                        if let Some(c) = self.state.lock().unwrap().seen.get(&key) {
                            costs.lock().unwrap()[k] = *c;
                            continue;
                        }
                        let trial = self.problem.evaluate(self.engine, &values, self.cancel);
                        if self.cancel.is_cancelled() {
                            break;
                        }
                        costs.lock().unwrap()[k] = trial.cost;
                        let mut st = self.state.lock().unwrap();
                        st.seen.insert(key, trial.cost);
                        st.evaluations += 1;
                        if let Some(why) = &trial.rejected {
                            st.rejected += 1;
                            st.last_rejection = Some(why.clone());
                        }
                        let improved = trial.cost < st.best.cost;
                        if improved {
                            st.best = trial;
                        }
                        (self.progress.lock().unwrap())(Progress {
                            evaluations: st.evaluations,
                            rejected: st.rejected,
                            last_rejection: st.last_rejection.as_deref(),
                            best: &st.best,
                            improved,
                            stage: st.stage,
                        });
                    }
                });
            }
        });
        costs.into_inner().unwrap()
    }
}

/// Optimises until cancelled, out of designs to try, settled, or (if asked)
/// every goal is met. Returns the best design found (at worst, the start).
pub fn optimise(
    problem: &Problem,
    engine: &Engine,
    options: Options,
    cancel: &CancelToken,
    progress: impl FnMut(Progress<'_>) + Send,
) -> Trial {
    let start: Vec<f64> = problem
        .params
        .iter()
        .map(|p| p.start.clamp(p.min, p.max))
        .collect();
    let search = Search {
        problem,
        engine,
        cancel,
        options,
        state: Mutex::new(State {
            best: Trial {
                values: start.clone(),
                cost: f64::INFINITY,
                results: Vec::new(),
                rejected: None,
            },
            evaluations: 0,
            rejected: 0,
            last_rejection: None,
            seen: HashMap::new(),
            stage: "Trying the starting design",
        }),
        progress: Mutex::new(progress),
    };
    let x0 = problem.unit_of(&start);
    search.score(std::slice::from_ref(&x0));
    match options.method {
        Method::NelderMead => {
            search.set_stage("Fine-tuning (Nelder-Mead)");
            nelder_mead(&search, &x0, 0.1);
        }
        Method::DifferentialEvolution => {
            search.set_stage("Searching the whole range (differential evolution)");
            let polish_budget = options.max_evaluations / 5;
            differential_evolution(&search, &x0, options.max_evaluations - polish_budget);
            let best = search.state.lock().unwrap().best.values.clone();
            search.set_stage("Polishing the best design (Nelder-Mead)");
            nelder_mead(&search, &problem.unit_of(&best), 0.03);
        }
    }
    search.state.into_inner().unwrap().best
}

/// Nelder-Mead in the unit cube from `x0`, starting with a simplex `size`
/// of each range across. Each step tries reflection, expansion and both
/// contractions at once, so a step takes one batch of parallel runs. When
/// the simplex shrinks to nothing it restarts around the best design, up to
/// three times, as long as restarts keep finding something better.
fn nelder_mead<P: FnMut(Progress<'_>) + Send>(search: &Search<'_, P>, x0: &[f64], size: f64) {
    let n = x0.len();
    if n == 0 {
        return;
    }
    let mut centre = x0.to_vec();
    let mut step = size;
    for _restart in 0..4 {
        let before = search.state.lock().unwrap().best.cost;
        let mut simplex: Vec<Vec<f64>> = vec![centre.clone()];
        for k in 0..n {
            let mut v = centre.clone();
            v[k] = if v[k] + step <= 1.0 {
                v[k] + step
            } else {
                v[k] - step
            };
            simplex.push(v);
        }
        let mut costs = search.score(&simplex);
        while !search.finished() {
            let mut order: Vec<usize> = (0..=n).collect();
            order.sort_by(|&a, &b| costs[a].total_cmp(&costs[b]));
            simplex = order.iter().map(|&k| simplex[k].clone()).collect();
            costs = order.iter().map(|&k| costs[k]).collect();
            let spread = simplex[1..]
                .iter()
                .map(|v| {
                    v.iter()
                        .zip(&simplex[0])
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0, f64::max)
                })
                .fold(0.0, f64::max);
            if spread < 1e-6 {
                break;
            }
            let centroid: Vec<f64> = (0..n)
                .map(|k| simplex[..n].iter().map(|v| v[k]).sum::<f64>() / n as f64)
                .collect();
            let towards = |t: f64| -> Vec<f64> {
                (0..n)
                    .map(|k| (centroid[k] + t * (simplex[n][k] - centroid[k])).clamp(0.0, 1.0))
                    .collect()
            };
            // Reflection, expansion, outside and inside contraction.
            let tries = [towards(-1.0), towards(-2.0), towards(-0.5), towards(0.5)];
            let c = search.score(&tries);
            let (fr, fe, fo, fi) = (c[0], c[1], c[2], c[3]);
            let replace = if fr < costs[0] {
                Some(if fe < fr { (1, fe) } else { (0, fr) })
            } else if fr < costs[n - 1] {
                Some((0, fr))
            } else if fr < costs[n] && fo <= fr {
                Some((2, fo))
            } else if fi < costs[n] {
                Some((3, fi))
            } else {
                None
            };
            match replace {
                Some((k, f)) => {
                    simplex[n] = tries[k].clone();
                    costs[n] = f;
                }
                None => {
                    // Shrink towards the best.
                    let best = simplex[0].clone();
                    let shrunk: Vec<Vec<f64>> = simplex[1..]
                        .iter()
                        .map(|v| {
                            v.iter()
                                .zip(&best)
                                .map(|(a, b)| b + 0.5 * (a - b))
                                .collect()
                        })
                        .collect();
                    let c = search.score(&shrunk);
                    for (k, (v, f)) in shrunk.into_iter().zip(c).enumerate() {
                        simplex[k + 1] = v;
                        costs[k + 1] = f;
                    }
                }
            }
        }
        let best = search.state.lock().unwrap().best.clone();
        if search.finished() || best.cost >= before - 1e-9 {
            break;
        }
        centre = search.problem.unit_of(&best.values);
        step = (step * 0.5).max(0.005);
    }
}

/// Adaptive differential evolution (JADE-style DE/current-to-pbest/1/bin
/// with bounce-back limits), using at most `budget` designs. The start
/// design is in the first population; the rest are spread evenly over the
/// limits (Latin hypercube). Each generation is scored at once.
fn differential_evolution<P: FnMut(Progress<'_>) + Send>(
    search: &Search<'_, P>,
    x0: &[f64],
    budget: usize,
) {
    let n = x0.len();
    if n == 0 {
        return;
    }
    // A multiple of the core count keeps every core busy.
    let threads = search.options.threads.max(1);
    let size = ((10 * n).clamp(16, 64)).div_ceil(threads) * threads;
    let size = size.clamp(8, 96);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut pop: Vec<Vec<f64>> = vec![x0.to_vec()];
    let strata: Vec<Vec<usize>> = (0..n)
        .map(|_| {
            let mut s: Vec<usize> = (0..size - 1).collect();
            for i in (1..s.len()).rev() {
                s.swap(i, rng.below(i + 1));
            }
            s
        })
        .collect();
    for i in 0..size - 1 {
        let design = strata
            .iter()
            .map(|s| (s[i] as f64 + rng.unit()) / (size - 1) as f64)
            .collect();
        pop.push(design);
    }
    let mut costs = search.score(&pop);
    let (mut mu_f, mut mu_cr) = (0.5, 0.5);
    let start = search.state.lock().unwrap().evaluations;
    while !search.finished() && search.state.lock().unwrap().evaluations - start < budget {
        let mut order: Vec<usize> = (0..size).collect();
        order.sort_by(|&a, &b| costs[a].total_cmp(&costs[b]));
        let top = ((size as f64 * 0.2).ceil() as usize).max(2);
        let mut fs = Vec::with_capacity(size);
        let mut crs = Vec::with_capacity(size);
        let trials: Vec<Vec<f64>> = (0..size)
            .map(|i| {
                // Each design gets its own F and CR around the running means.
                let f = loop {
                    let f = mu_f + 0.1 * (std::f64::consts::PI * (rng.unit() - 0.5)).tan();
                    if f > 0.0 {
                        break f.min(1.0);
                    }
                };
                let cr = (mu_cr + 0.1 * rng.normal()).clamp(0.0, 1.0);
                fs.push(f);
                crs.push(cr);
                let pbest = order[rng.below(top)];
                let pick = |rng: &mut Rng, not: &[usize]| loop {
                    let k = rng.below(size);
                    if !not.contains(&k) {
                        break k;
                    }
                };
                let a = pick(&mut rng, &[i]);
                let b = pick(&mut rng, &[i, a]);
                let forced = rng.below(n);
                (0..n)
                    .map(|k| {
                        if k == forced || rng.unit() < cr {
                            let v = pop[i][k]
                                + f * (pop[pbest][k] - pop[i][k])
                                + f * (pop[a][k] - pop[b][k]);
                            // Bounce back inside the limits.
                            if v < 0.0 {
                                pop[i][k] * rng.unit()
                            } else if v > 1.0 {
                                pop[i][k] + (1.0 - pop[i][k]) * rng.unit()
                            } else {
                                v
                            }
                        } else {
                            pop[i][k]
                        }
                    })
                    .collect()
            })
            .collect();
        let c = search.score(&trials);
        let (mut good_f, mut good_cr) = (Vec::new(), Vec::new());
        for (i, (t, ct)) in trials.into_iter().zip(c).enumerate() {
            if ct <= costs[i] {
                if ct < costs[i] {
                    good_f.push(fs[i]);
                    good_cr.push(crs[i]);
                }
                pop[i] = t;
                costs[i] = ct;
            }
        }
        // Adapt the means towards what worked (Lehmer mean for F).
        if !good_f.is_empty() {
            let lehmer = good_f.iter().map(|f| f * f).sum::<f64>() / good_f.iter().sum::<f64>();
            mu_f = 0.9 * mu_f + 0.1 * lehmer;
            mu_cr = 0.9 * mu_cr + 0.1 * good_cr.iter().sum::<f64>() / good_cr.len() as f64;
        }
        // Settled: every design costs about the same.
        let (lo, hi) = costs
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), &c| (lo.min(c), hi.max(c)));
        if hi - lo < 1e-7 * lo.abs().max(1e-3) {
            break;
        }
    }
}

/// A small, fast pseudo-random generator (xorshift), so runs repeat.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// A standard normal sample (Box-Muller).
    fn normal(&mut self) -> f64 {
        let u = self.unit().max(1e-12);
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}
