//! Sweep results as they arrive, for a live frequency slider.

use num_complex::Complex64;

use super::pattern::PatternGrid;
use super::plan::FrequencyPlan;
use crate::deck::NumericDeck;
use crate::output::{FrequencyResult, InputParams, PowerBudget, SegmentCurrent};

/// What the views need at one frequency.
#[derive(Debug, Clone, PartialEq)]
pub struct FreqPoint {
    pub mhz: f64,
    /// Feed-point results, one per source, in the deck's order (4nec2
    /// current sources: their real feed points, see `without_helpers`).
    pub inputs: Vec<InputParams>,
    /// Results at network and transmission-line connection points.
    pub network: Vec<InputParams>,
    pub power: Option<PowerBudget>,
    pub pattern: Option<PatternGrid>,
    /// Pattern grid step in degrees (finer patterns replace coarser ones).
    pub pattern_step: Option<f64>,
    /// Current on every segment, for the sources as given (1 V unless the
    /// deck says otherwise).
    pub currents: Vec<SegmentCurrent>,
    /// The feed impedance (and gains) include an estimate for radials on or
    /// under the ground (`analysis::buried`).
    pub estimated: bool,
}

impl FreqPoint {
    /// Feed point `k`'s results (in the deck's order of sources), or the
    /// first's if there aren't that many (the model changed since `k` was
    /// chosen).
    pub fn feed(&self, k: usize) -> Option<&InputParams> {
        self.inputs.get(k).or(self.inputs.first())
    }

    /// Drops the feed results of helper wires (the dummy sources behind
    /// 4nec2 current sources), which aren't real feed points, and puts in
    /// the real feed points behind them (from the network table): `deck`'s
    /// sources then all show, in its order.
    pub fn without_helpers(mut self, deck: &NumericDeck) -> FreqPoint {
        if deck.helper_tags.is_empty() {
            return self;
        }
        let helper = |t: u32| deck.helper_tags.contains(&i64::from(t));
        let voltage: Vec<InputParams> = self
            .inputs
            .iter()
            .filter(|i| !helper(i.tag))
            .copied()
            .collect();
        let mut current = deck.current_fed().into_iter().filter_map(|tag| {
            self.network
                .iter()
                .find(|n| i64::from(n.tag) == tag && !helper(n.tag))
                .copied()
        });
        let mut voltage = voltage.into_iter();
        // The deck's sources in order: voltage ones from the input table,
        // current ones from the network table.
        self.inputs = deck
            .cards
            .iter()
            .filter(|c| c.is("EX"))
            .filter_map(|c| {
                if deck.helper_tags.contains(&c.ints[1]) {
                    current.next()
                } else if matches!(c.ints[0], 0 | 5) {
                    voltage.next()
                } else {
                    None
                }
            })
            .collect();
        self
    }

    /// Scale that turns the run's currents into those for `watts` of
    /// input power (1 if the input power isn't known).
    pub fn current_scale(&self, watts: f64) -> f64 {
        // The power budget counts every source (current sources too).
        let p = self
            .power
            .map(|b| b.input)
            .filter(|p| *p > 0.0)
            .unwrap_or_else(|| self.inputs.iter().map(|i| i.power).sum());
        if p > 0.0 { (watts / p).sqrt() } else { 1.0 }
    }
}

impl FreqPoint {
    pub fn from_result(r: &FrequencyResult, pattern_step: Option<f64>) -> FreqPoint {
        let pattern = r.patterns.first().and_then(PatternGrid::from_table);
        FreqPoint {
            mhz: r.mhz,
            inputs: r.inputs.clone(),
            network: r.network.clone(),
            power: r.power,
            pattern_step: pattern.as_ref().and(pattern_step),
            pattern,
            currents: r.currents.clone(),
            estimated: false,
        }
    }

    /// Puts in the estimated feed impedance `z` (radials on or under the
    /// ground) for the first source. The pattern keeps its shape; gains
    /// change with the input power the same feed current now takes.
    pub fn with_estimate(mut self, z: Complex64) -> FreqPoint {
        let Some(feed) = self.inputs.first_mut() else {
            return self;
        };
        let (old_r, old_i) = (feed.impedance.re, feed.current);
        if z.re <= 0.0 || old_r <= 0.0 {
            return self;
        }
        feed.impedance = z;
        feed.admittance = z.inv();
        feed.current = feed.voltage / z;
        feed.power = 0.5 * (feed.voltage * feed.current.conj()).re;
        let db = 10.0 * (old_r / z.re).log10();
        if let Some(p) = &mut self.pattern {
            p.shift_db(db);
        }
        // Same current shape, scaled to the new feed current.
        let scale = feed.current / old_i;
        for c in &mut self.currents {
            c.current *= scale;
        }
        if let Some(b) = &mut self.power {
            let s = scale.norm_sqr();
            b.radiated *= s;
            b.structure_loss *= s;
            b.network_loss *= s;
            b.input = feed.power;
            b.efficiency_percent = 100.0 * b.radiated / b.input.max(1e-30);
        }
        self.estimated = true;
        self
    }
}

/// Results of one sweep plan, filled in as they arrive (in any order).
#[derive(Debug, Clone)]
pub struct SweepResults {
    pub plan: FrequencyPlan,
    points: Vec<Option<FreqPoint>>,
}

impl SweepResults {
    pub fn new(plan: FrequencyPlan) -> Self {
        Self {
            plan,
            points: vec![None; plan.points.max(1)],
        }
    }

    pub fn insert(&mut self, index: usize, point: FreqPoint) {
        if let Some(slot) = self.points.get_mut(index) {
            *slot = Some(point);
        }
    }

    /// How many frequencies have arrived.
    pub fn done(&self) -> usize {
        self.points.iter().filter(|p| p.is_some()).count()
    }

    pub fn is_complete(&self) -> bool {
        self.done() == self.points.len()
    }

    /// Computed points in frequency order.
    pub fn points(&self) -> impl Iterator<Item = &FreqPoint> {
        self.points.iter().flatten()
    }

    /// The computed point nearest to `mhz`.
    pub fn nearest(&self, mhz: f64) -> Option<&FreqPoint> {
        self.points()
            .min_by(|a, b| (a.mhz - mhz).abs().total_cmp(&(b.mhz - mhz).abs()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(mhz: f64) -> FreqPoint {
        FreqPoint {
            mhz,
            inputs: Vec::new(),
            network: Vec::new(),
            power: None,
            pattern: None,
            pattern_step: None,
            currents: Vec::new(),
            estimated: false,
        }
    }

    #[test]
    fn fills_in_out_of_order_and_finds_nearest() {
        let plan = FrequencyPlan {
            start_mhz: 14.0,
            stop_mhz: 14.4,
            points: 5,
        };
        let mut r = SweepResults::new(plan);
        assert_eq!(r.nearest(14.2), None);
        r.insert(4, point(14.4));
        r.insert(0, point(14.0));
        assert_eq!(r.done(), 2);
        assert_eq!(r.nearest(14.15).unwrap().mhz, 14.0);
        assert_eq!(r.nearest(14.35).unwrap().mhz, 14.4);
        let order: Vec<f64> = r.points().map(|p| p.mhz).collect();
        assert_eq!(order, vec![14.0, 14.4]);
        assert!(!r.is_complete());
    }
}
