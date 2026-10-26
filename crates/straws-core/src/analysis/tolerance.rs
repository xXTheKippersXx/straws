//! Tolerance analysis: how much a design's figures move when it's built
//! only as accurately as a workshop can.
//!
//! Every distinct wire end point (ends that meet move together, so joints
//! stay joined) is moved by a random amount up to the build accuracy along
//! each axis, independently, and the model is run again; many such builds
//! give the spread of the feed impedance, resonance, peak gain and F/B.
//! Ends standing on the ground stay on it. Straight wires (GW) only: arcs,
//! helices and patches are built as drawn.

use super::buried::Setup;
use super::convergence::{Figures, figures};
use super::plan::FrequencyPlan;
use crate::deck::NumericDeck;
use crate::engine::{CancelToken, Engine};

/// A model with every distinct GW end point moved by up to `accuracy`
/// metres along each axis, from the random stream `seed`.
pub fn perturbed(deck: &NumericDeck, accuracy: f64, seed: u64, over_ground: bool) -> NumericDeck {
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move || {
        // xorshift64*: a uniform number in -1..1.
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        let x = rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
        x as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    // Each distinct point (to a micrometre) gets one random move.
    let key = |p: &[f64]| {
        p.iter()
            .map(|v| (v * 1e6).round() as i64)
            .collect::<Vec<_>>()
    };
    let mut moves: std::collections::HashMap<Vec<i64>, [f64; 3]> = Default::default();
    let mut out = deck.clone();
    let end = out.control_start();
    for card in &mut out.cards[..end] {
        if !card.is("GW") || deck.helper_tags.contains(&card.ints[0]) {
            continue;
        }
        for e in 0..2 {
            let p = &card.floats[3 * e..3 * e + 3];
            let on_ground = over_ground && p[2].abs() < 1e-9;
            let d = *moves
                .entry(key(p))
                .or_insert_with(|| [next(), next(), next()].map(|r| r * accuracy));
            for (k, dk) in d.iter().enumerate() {
                if !(k == 2 && on_ground) {
                    card.floats[3 * e + k] += dk;
                }
            }
        }
    }
    out
}

/// The design as drawn and each of `builds` random builds.
#[derive(Debug, Clone, PartialEq)]
pub struct Spread {
    pub mhz: f64,
    pub accuracy: f64,
    pub design: Figures,
    pub builds: Vec<Figures>,
}

/// Runs `builds` random builds (on `threads` workers at once), calling
/// `progress(done, total)` as they finish.
#[allow(clippy::too_many_arguments)]
pub fn spread(
    engine: &Engine,
    deck: &NumericDeck,
    plan: &FrequencyPlan,
    mhz: f64,
    accuracy: f64,
    builds: usize,
    threads: usize,
    over_ground: bool,
    estimate: Option<&Setup>,
    cancel: &CancelToken,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Spread, String> {
    let design = figures(engine, deck, plan, mhz, estimate, cancel)?;
    let engine = engine.clone().with_solver_threads(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, builds.max(1)) {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if k >= builds || cancel.is_cancelled() {
                        break;
                    }
                    let build = perturbed(deck, accuracy, k as u64 + 1, over_ground);
                    let r = figures(&engine, &build, plan, mhz, estimate, cancel);
                    if let Ok(mut all) = results.lock() {
                        all.push(r);
                        progress(all.len(), builds);
                    }
                }
            });
        }
    });
    if cancel.is_cancelled() {
        return Err("stopped".into());
    }
    let all = results
        .into_inner()
        .map_err(|_| "a worker failed".to_string())?;
    let builds: Vec<Figures> = all.into_iter().filter_map(Result::ok).collect();
    if builds.is_empty() {
        return Err("none of the builds ran".into());
    }
    Ok(Spread {
        mhz,
        accuracy,
        design,
        builds,
    })
}

/// The lowest, 10th, 50th and 90th percentiles and highest of `values`.
pub fn percentiles(values: &mut Vec<f64>) -> Option<[f64; 5]> {
    values.retain(|v| v.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    Some([at(0.0), at(0.1), at(0.5), at(0.9), at(1.0)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::Deck;

    #[test]
    fn joined_ends_move_together() {
        let d = Deck::parse(
            "GW 1 5 0 0 0 0 0 1 .001\nGW 2 5 0 0 1 1 0 1 .001\nGE 1\nGN 1\nEX 0 1 1 0 1\nFR 0 1 0 0 14\nEN\n",
        )
        .expand()
        .unwrap();
        let p = perturbed(&d, 0.002, 7, true);
        let gw: Vec<&[f64]> = p
            .cards
            .iter()
            .filter(|c| c.is("GW"))
            .map(|c| &c.floats[..6])
            .collect();
        // The joint (0, 0, 1) moved the same way on both wires; the base
        // stays on the ground; every move within 2 mm.
        assert_eq!(&gw[0][3..6], &gw[1][0..3]);
        assert_eq!(gw[0][2], 0.0);
        assert!((gw[0][5] - 1.0).abs() <= 0.002 && (gw[0][5] - 1.0).abs() > 0.0);
        assert!(gw[1][3] != 1.0);
        let mut v = vec![3.0, 1.0, f64::NAN, 2.0];
        assert_eq!(percentiles(&mut v), Some([1.0, 1.0, 2.0, 3.0, 3.0]));
    }
}
