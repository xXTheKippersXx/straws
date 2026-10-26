//! Sensitivity: which wire to trim, and which way.
//!
//! Each chosen wire (tag) is made a little longer at its free tips, as it
//! would be trimmed for real (a tip that meets nothing), and the model is run
//! again; how far the feed impedance, resonance, peak gain and F/B move shows
//! which wires matter most. A wire with no free tip (part of a loop, or
//! joined at both ends) can't be trimmed alone and is left out.

use super::buried::Setup;
use super::convergence::{Figures, figures};
use super::plan::FrequencyPlan;
use crate::deck::NumericDeck;
use crate::engine::{CancelToken, Engine};

/// Ends of the GW cards (card index, end 0/1) that meet no other end and
/// don't stand on the ground.
fn free_ends(deck: &NumericDeck, over_ground: bool) -> Vec<(usize, usize)> {
    let geometry = &deck.cards[..deck.control_start()];
    let ends: Vec<(usize, usize, [f64; 3])> = geometry
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is("GW"))
        .flat_map(|(i, c)| {
            (0..2).map(move |e| {
                (
                    i,
                    e,
                    [c.floats[3 * e], c.floats[3 * e + 1], c.floats[3 * e + 2]],
                )
            })
        })
        .collect();
    let same = |a: [f64; 3], b: [f64; 3]| {
        let d = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt();
        d <= 1e-6 * (1.0 + a.iter().map(|v| v.abs()).fold(0.0, f64::max))
    };
    ends.iter()
        .filter(|(i, e, p)| {
            !(over_ground && p[2].abs() < 1e-9)
                && !ends
                    .iter()
                    .any(|(j, f, q)| (j, f) != (i, e) && same(*p, *q))
        })
        .map(|(i, e, _)| (*i, *e))
        .collect()
}

/// `deck` with wire `tag` longer by `delta` metres in all, shared between
/// its free tips (each moved outwards along its wire). `None` if the wire
/// has no free tip.
pub fn lengthened(
    deck: &NumericDeck,
    tag: i64,
    delta: f64,
    over_ground: bool,
) -> Option<NumericDeck> {
    let tips: Vec<(usize, usize)> = free_ends(deck, over_ground)
        .into_iter()
        .filter(|(i, _)| deck.cards[*i].ints[0] == tag)
        .collect();
    if tips.is_empty() {
        return None;
    }
    let each = delta / tips.len() as f64;
    let mut out = deck.clone();
    for (i, e) in tips {
        let f = &mut out.cards[i].floats;
        let (a, b) = ([f[0], f[1], f[2]], [f[3], f[4], f[5]]);
        let len = (0..3).map(|k| (b[k] - a[k]).powi(2)).sum::<f64>().sqrt();
        if len <= 0.0 {
            return None;
        }
        // Outwards: from the other end through this one.
        let sign = if e == 0 { -1.0 } else { 1.0 };
        for k in 0..3 {
            f[3 * e + k] += sign * each * (b[k] - a[k]) / len;
        }
    }
    Some(out)
}

/// The design and each wire made `delta` longer.
#[derive(Debug, Clone, PartialEq)]
pub struct Trims {
    pub mhz: f64,
    pub delta: f64,
    pub design: Figures,
    /// (tag, figures with that wire longer), for the wires that could be.
    pub wires: Vec<(i64, Figures)>,
}

/// Runs each wire in `tags` made `delta` longer (on `threads` workers at
/// once), calling `progress(done, total)` as they finish.
#[allow(clippy::too_many_arguments)]
pub fn trims(
    engine: &Engine,
    deck: &NumericDeck,
    tags: &[i64],
    plan: &FrequencyPlan,
    mhz: f64,
    delta: f64,
    threads: usize,
    over_ground: bool,
    estimate: Option<&Setup>,
    cancel: &CancelToken,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Trims, String> {
    let design = figures(engine, deck, plan, mhz, estimate, cancel)?;
    let jobs: Vec<(i64, NumericDeck)> = tags
        .iter()
        .filter_map(|&t| Some((t, lengthened(deck, t, delta, over_ground)?)))
        .collect();
    if jobs.is_empty() {
        return Err("none of the wires has a free tip to trim".into());
    }
    let engine = engine.clone().with_solver_threads(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, jobs.len()) {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if k >= jobs.len() || cancel.is_cancelled() {
                        break;
                    }
                    let r = figures(&engine, &jobs[k].1, plan, mhz, estimate, cancel);
                    if let Ok(mut all) = results.lock() {
                        all.push((k, r));
                        progress(all.len(), jobs.len());
                    }
                }
            });
        }
    });
    if cancel.is_cancelled() {
        return Err("stopped".into());
    }
    let mut all = results
        .into_inner()
        .map_err(|_| "a worker failed".to_string())?;
    all.sort_by_key(|(k, _)| *k);
    let wires = all
        .into_iter()
        .filter_map(|(k, r)| Some((jobs[k].0, r.ok()?)))
        .collect();
    Ok(Trims {
        mhz,
        delta,
        design,
        wires,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::Deck;

    #[test]
    fn only_free_tips_grow() {
        // An inverted L: a vertical standing on the ground, joined to a
        // horizontal top wire. Only the top wire's far end is free.
        let d = Deck::parse(
            "GW 1 5 0 0 0 0 0 5 .001\nGW 2 5 0 0 5 4 0 5 .001\nGE 1\nGN 1\nEX 0 1 1 0 1\nFR 0 1 0 0 14\nEN\n",
        )
        .expand()
        .unwrap();
        assert!(lengthened(&d, 1, 0.1, true).is_none());
        let l = lengthened(&d, 2, 0.1, true).unwrap();
        let gw: Vec<&[f64]> = l
            .cards
            .iter()
            .filter(|c| c.is("GW"))
            .map(|c| &c.floats[..6])
            .collect();
        assert_eq!(gw[0], &d.cards[0].floats[..6]);
        assert!((gw[1][3] - 4.1).abs() < 1e-12 && gw[1][0] == 0.0);
        // In free space the vertical's foot is free too.
        let l = lengthened(&d, 1, 0.1, false).unwrap();
        assert!((l.cards[0].floats[2] + 0.1).abs() < 1e-12);
    }
}
