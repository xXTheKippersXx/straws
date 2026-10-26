//! Convergence check: run the model again with about twice the segments
//! and see how much the results move. If they hardly move, the model has
//! enough segments; if they move a lot, its answers aren't settled yet.
//!
//! Each wire, arc or helix of n segments gets 2n + 1 (so a centre feed stays
//! exactly in the centre), and sources, loads, transmission lines and
//! networks move to the segment at the same place on the finer wire.
//! Surface patches are left as they are.

use num_complex::Complex64;

use super::buried::{self, Setup};
use super::pattern::{Component, PatternGrid};
use super::plan::{FrequencyPlan, PatternRequest, run_deck};
use crate::deck::NumericDeck;
use crate::engine::{CancelToken, Engine};

/// The most segments the finer model may have.
pub const MAX_SEGMENTS: i64 = 20_000;

/// Old to new segment counts of each piece of a tag, in order.
type Pieces = Vec<(i64, i64)>;

/// The deck with every wire, arc and helix given 2n + 1 segments for n,
/// and the references to segments moved to match. Errors say why not.
pub fn refined(deck: &NumericDeck) -> Result<NumericDeck, String> {
    let mut out = deck.clone();
    let geometry = deck.control_start();
    // Pieces (old, new) of each tag, in the order NEC numbers them.
    let mut tags: Vec<(i64, Pieces)> = Vec::new();
    let mut total = 0i64;
    for c in &mut out.cards[..geometry] {
        if !(c.is("GW") || c.is("GA") || c.is("GH")) || deck.helper_tags.contains(&c.ints[0]) {
            continue;
        }
        let n = c.ints[1].max(1);
        let m = 2 * n + 1;
        c.ints[1] = m;
        total += m;
        match tags.iter_mut().find(|t| t.0 == c.ints[0]) {
            Some(t) => t.1.push((n, m)),
            None => tags.push((c.ints[0], vec![(n, m)])),
        }
    }
    if tags.is_empty() {
        return Err("the model has no wires to refine".into());
    }
    // Copies made by GR and GX number their tags on from the original's.
    let increments: Vec<(i64, i64)> = deck.cards[..geometry]
        .iter()
        .filter(|c| c.is("GR") || c.is("GX"))
        .map(|c| {
            let copies = if c.is("GR") { c.ints[1].max(1) } else { 3 };
            (c.ints[0], copies)
        })
        .collect();
    let pieces = |tag: i64| -> Option<&Pieces> {
        if let Some(t) = tags.iter().find(|t| t.0 == tag) {
            return Some(&t.1);
        }
        for &(inc, copies) in &increments {
            if inc <= 0 {
                continue;
            }
            for k in 1..=copies {
                if let Some(t) = tags.iter().find(|t| t.0 == tag - k * inc) {
                    return Some(&t.1);
                }
            }
        }
        None
    };
    if total > MAX_SEGMENTS {
        return Err(format!(
            "the finer model would have {total} segments, more than {MAX_SEGMENTS}"
        ));
    }
    let absolute = |what: &str| {
        format!(
            "{what} gives a segment by its number in the whole model (tag 0), \
             which can't be moved onto a finer model"
        )
    };
    for c in &mut out.cards[geometry..] {
        let helper = |tag: i64| deck.helper_tags.contains(&tag);
        match c.mnemonic.as_str() {
            // Voltage and current sources on a segment (not plane waves).
            "EX" if matches!(c.ints[0], 0 | 4 | 5) => {
                let tag = c.ints[1];
                if helper(tag) {
                    continue;
                }
                if tag == 0 {
                    return Err(absolute("a source (EX card)"));
                }
                let p =
                    pieces(tag).ok_or(format!("a source is on wire {tag}, which isn't there"))?;
                c.ints[2] = at(p, c.ints[2]);
            }
            "LD" => {
                let tag = c.ints[1];
                if helper(tag) || (c.ints[2] == 0 && c.ints[3] == 0) {
                    continue;
                }
                if tag == 0 {
                    return Err(absolute("a load (LD card)"));
                }
                let p = pieces(tag).ok_or(format!("a load is on wire {tag}, which isn't there"))?;
                let (from, to) = (
                    c.ints[2],
                    if c.ints[3] == 0 { c.ints[2] } else { c.ints[3] },
                );
                c.ints[2] = start_of(p, from.max(1));
                c.ints[3] = end_of(p, to.max(from).max(1));
            }
            "TL" | "NT" => {
                for k in [0, 2] {
                    let tag = c.ints[k];
                    if helper(tag) {
                        continue;
                    }
                    if tag == 0 {
                        return Err(absolute("a transmission line or network"));
                    }
                    let p = pieces(tag).ok_or(format!(
                        "a transmission line or network ends on wire {tag}, which isn't there"
                    ))?;
                    c.ints[k + 1] = at(p, c.ints[k + 1]);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Which piece of a tag old segment `s` (1-based along the tag) is on, and
/// where in it (1-based).
fn locate(pieces: &Pieces, s: i64) -> (usize, i64, i64) {
    let mut before_old = 0;
    let mut before_new = 0;
    for (k, &(n, m)) in pieces.iter().enumerate() {
        if s <= before_old + n || k == pieces.len() - 1 {
            return (k, (s - before_old).clamp(1, n), before_new);
        }
        before_old += n;
        before_new += m;
    }
    (0, 1, 0)
}

/// The new segment at the same place as old segment `s`: its centre's, or
/// the end segment for an end one (so an end feed stays at the end).
fn at(pieces: &Pieces, s: i64) -> i64 {
    let (k, l, base) = locate(pieces, s);
    let (n, m) = pieces[k];
    let local = if l == 1 {
        1
    } else if l == n {
        m
    } else {
        (((l as f64 - 0.5) / n as f64) * m as f64).floor() as i64 + 1
    };
    base + local.clamp(1, m)
}

/// The first new segment of old segment `s` (for the start of a load).
fn start_of(pieces: &Pieces, s: i64) -> i64 {
    let (k, l, base) = locate(pieces, s);
    let (n, m) = pieces[k];
    base + ((((l - 1) as f64 / n as f64) * m as f64).floor() as i64 + 1).clamp(1, m)
}

/// The last new segment of old segment `s` (for the end of a load).
fn end_of(pieces: &Pieces, s: i64) -> i64 {
    let (k, l, base) = locate(pieces, s);
    let (n, m) = pieces[k];
    base + (((l as f64 / n as f64) * m as f64).ceil() as i64).clamp(1, m)
}

/// The figures compared, for one model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Figures {
    pub segments: usize,
    /// Feed impedance at the check frequency.
    pub z: Complex64,
    /// Resonance (reactance through zero) nearest the check frequency.
    pub resonance: Option<f64>,
    /// Peak gain (dBi) at the check frequency.
    pub peak_dbi: f64,
    /// Front-to-back (dB) in the peak's direction, at its elevation.
    pub front_to_back: Option<f64>,
}

/// The model and its finer copy, side by side.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub mhz: f64,
    pub before: Figures,
    pub after: Figures,
}

/// How settled the results are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Moves are well inside what matters in practice.
    Settled,
    /// Small but noticeable moves.
    NearlySettled,
    /// Big moves: more segments needed.
    NotSettled,
}

impl Comparison {
    /// Feed impedance change, as a share of |Z|.
    pub fn z_change(&self) -> f64 {
        (self.after.z - self.before.z).norm() / self.before.z.norm().max(1e-9)
    }

    /// Resonance change, as a share of the frequency.
    pub fn resonance_change(&self) -> Option<f64> {
        Some((self.after.resonance? - self.before.resonance?).abs() / self.mhz)
    }

    pub fn gain_change(&self) -> f64 {
        (self.after.peak_dbi - self.before.peak_dbi).abs()
    }

    pub fn front_to_back_change(&self) -> Option<f64> {
        Some((self.after.front_to_back? - self.before.front_to_back?).abs())
    }

    /// Settled: |Z| within 2%, resonance within 0.3%, gain within 0.1 dB
    /// and F/B within 1 dB; not settled beyond 3 times those.
    pub fn verdict(&self) -> Verdict {
        // Each move as a multiple of its "settled" limit.
        let worst = [
            self.z_change() / 0.02,
            self.resonance_change().unwrap_or(0.0) / 0.003,
            self.gain_change() / 0.1,
            // F/B is very sensitive when the back is a deep null.
            self.front_to_back_change()
                .filter(|_| self.before.front_to_back.unwrap_or(0.0) < 30.0)
                .unwrap_or(0.0)
                / 1.0,
        ]
        .into_iter()
        .fold(0.0, f64::max);
        if worst <= 1.0 {
            Verdict::Settled
        } else if worst <= 3.0 {
            Verdict::NearlySettled
        } else {
            Verdict::NotSettled
        }
    }
}

/// Runs `deck` and its finer copy over `plan` (impedance) and at `mhz`
/// (a 5° pattern), and compares them.
pub fn compare(
    engine: &Engine,
    deck: &NumericDeck,
    plan: &FrequencyPlan,
    mhz: f64,
    estimate: Option<&Setup>,
    cancel: &CancelToken,
) -> Result<Comparison, String> {
    let finer = refined(deck)?;
    let before = figures(engine, deck, plan, mhz, estimate, cancel)?;
    let after = figures(engine, &finer, plan, mhz, estimate, cancel)?;
    Ok(Comparison { mhz, before, after })
}

/// The model's figures at `mhz`: feed impedance, the nearest resonance (in
/// `plan`'s sweep), peak gain and F/B, with the buried-radial estimate if
/// given.
pub fn figures(
    engine: &Engine,
    deck: &NumericDeck,
    plan: &FrequencyPlan,
    mhz: f64,
    estimate: Option<&Setup>,
    cancel: &CancelToken,
) -> Result<Figures, String> {
    let sweep = engine
        .run(&run_deck(deck, plan, None), cancel)
        .map_err(|e| e.to_string())?;
    let one = engine
        .run(
            &run_deck(
                deck,
                &FrequencyPlan::single(mhz),
                Some(PatternRequest::new(5.0)),
            ),
            cancel,
        )
        .map_err(|e| e.to_string())?;
    let f = one
        .frequencies
        .first()
        .ok_or("the engine gave no results")?;
    let mut z = f.inputs.first().ok_or("the model has no source")?.impedance;
    let grid = f
        .patterns
        .first()
        .and_then(PatternGrid::from_table)
        .ok_or("the engine gave no pattern")?;
    let (mut peak_dbi, theta, _) = grid.max(Component::Total);
    // Radials on or under the ground: their estimate, as the views show it
    // (the same current shape, so gains move with the feed resistance).
    let estimated = match estimate {
        Some(setup) => {
            let freqs: Vec<f64> = sweep.frequencies.iter().map(|f| f.mhz).collect();
            let e = buried::estimate(engine, deck, setup, plan, 5, cancel)?;
            let p = buried::point(engine, deck, setup, mhz, cancel)?;
            let est = p.z_perfect + p.dz;
            if est.re > 0.0 && z.re > 0.0 {
                peak_dbi += 10.0 * (z.re / est.re).log10();
            }
            z = est;
            Some((freqs, e))
        }
        None => None,
    };
    let front_to_back = grid
        .azimuth_cut(Component::Total, 90.0 - theta)
        .figures()
        .and_then(|f| f.front_to_back);
    // The resonance nearest the check frequency.
    let points: Vec<(f64, f64)> = sweep
        .frequencies
        .iter()
        .filter_map(|f| {
            let x = match &estimated {
                Some((_, e)) => e.z(f.mhz)?.im,
                None => f.inputs.first()?.impedance.im,
            };
            Some((f.mhz, x))
        })
        .collect();
    let resonance = points
        .windows(2)
        .filter(|w| (w[0].1 < 0.0) != (w[1].1 < 0.0))
        .map(|w| w[0].0 + (w[1].0 - w[0].0) * w[0].1 / (w[0].1 - w[1].1))
        .min_by(|a, b| (a - mhz).abs().total_cmp(&(b - mhz).abs()));
    Ok(Figures {
        segments: one.segments.len(),
        z,
        resonance,
        peak_dbi,
        front_to_back,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::Deck;

    #[test]
    fn sources_and_loads_move_to_the_same_place() {
        let d = Deck::parse(
            "GW 1 21 0 0 -5 0 0 5 .001\nGW 2 5 1 0 0 2 0 0 .001\nGE 0\n\
             EX 0 1 11 0 1\nLD 0 1 3 4 10 0 0\nTL 1 1 2 5 50 0\nFR 0 1 0 0 14\nEN\n",
        )
        .expand()
        .unwrap();
        let r = refined(&d).unwrap();
        let card = |m: &str| r.cards.iter().find(|c| c.is(m)).unwrap().ints.clone();
        assert_eq!(r.cards[0].ints[1], 43);
        // The centre feed stays in the centre; ends stay at the ends.
        assert_eq!(card("EX")[2], 22);
        assert_eq!(card("TL")[..4], [1, 1, 2, 11]);
        // Segments 3-4 of 21 cover 2/21..4/21 of the wire: 5..9 of 43.
        assert_eq!(card("LD")[2..4], [5, 9]);
    }

    #[test]
    fn absolute_segment_numbers_are_refused() {
        let d = Deck::parse("GW 1 21 0 0 -5 0 0 5 .001\nGE 0\nEX 0 0 11 0 1\nEN\n")
            .expand()
            .unwrap();
        assert!(refined(&d).unwrap_err().contains("tag 0"));
    }
}
