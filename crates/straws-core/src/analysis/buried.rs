//! Radials on or under the ground, estimated. NEC-2 can't model wires in or
//! on lossy ground, so a set of radials there isn't sent to the engine;
//! instead its effect on the feed impedance (and so on efficiency and gain)
//! is worked out the way Wait and Pope did for broadcast ground systems:
//!
//! Z = Z_perfect + (1 / I0²) ∬ Zs H_t · H_t dS
//!
//! - `Z_perfect` and `H_t` (the magnetic field along the ground surface) come
//!   from running the antenna over perfect ground, with the feed current I0.
//! - `Zs` is the surface impedance: the soil's, η0 / √εc, and inside a set of
//!   N radials of radius a, for the field circling the hub, the soil's in
//!   parallel with the radial screen's, j η0 k (ρ/N) ln(ρ / (N a)) (the same
//!   screen formula NEC-2 uses for its GN radial screen; ρ is softened to
//!   √(ρ² + (N a)²) where the radials crowd together at the hub).
//! - H·H (not |H|²) carries the field's phase, so far from the antenna the
//!   integrand turns round and round and adds up to a finite total; past the
//!   last ring it's added up in closed form.
//!
//! Checked against Brown, Lewis and Epstein's 1937 measurements (3 MHz,
//! 15-113 buried radials of 0.137 and 0.411 λ): within about 1-3 Ω.
//! Rougher with only a few radials, where the field between them is far from
//! the even "mat" the screen formula assumes; burial depth is ignored (a
//! few centimetres is a tiny fraction of the soil's skin depth at HF).

use num_complex::Complex64;

use super::plan::{FrequencyPlan, run_deck};
use crate::deck::expand::NumericDeck;
use crate::deck::grids::{GridNumbers, grid_numbers};
use crate::deck::ground::{Ground, ground};
use crate::deck::radials::RadialNumbers;
use crate::deck::{Deck, NumCard};
use crate::engine::{CancelToken, Engine};
use crate::units::wavelength;

const ETA0: f64 = 376.730_313;
const EPS0: f64 = 8.854_187_817e-12;
/// Directions round the hub where the field is sampled.
const DIRECTIONS: i64 = 16;

/// Everything the estimate needs from a model.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    /// The radial sets on or under the ground (hubs above ground in z).
    pub sets: Vec<RadialNumbers>,
    /// The grid counterpoises on or under the ground.
    pub grids: Vec<GridNumbers>,
    pub permittivity: f64,
    pub conductivity: f64,
}

/// What a model's buried radials mean for the estimate.
#[derive(Debug, Clone, PartialEq)]
pub enum Need {
    /// No radials on or under the ground.
    None,
    /// They're there, but make no difference (perfect ground), or can't be
    /// estimated: says why.
    Note(String),
    Estimate(Setup),
}

/// What `deck`'s radial sets on or under the ground need.
pub fn need(deck: &Deck, numeric: &NumericDeck) -> Need {
    let sets: Vec<RadialNumbers> = crate::deck::radials::radial_numbers(deck, numeric)
        .into_iter()
        .filter(|s| s.buried)
        .collect();
    let grids: Vec<GridNumbers> = grid_numbers(deck, numeric)
        .into_iter()
        .filter(|g| g.buried)
        .collect();
    if sets.is_empty() && grids.is_empty() {
        return Need::None;
    }
    match ground(numeric) {
        Ground::Real {
            permittivity,
            conductivity,
            ..
        } => Need::Estimate(Setup {
            sets,
            grids,
            permittivity,
            conductivity,
        }),
        Ground::Perfect => {
            Need::Note("Over perfect ground, radials on or under it make no difference.".into())
        }
        Ground::FreeSpace => Need::None,
    }
}

/// The soil's surface impedance (ohms) at `mhz`.
pub fn soil_impedance(permittivity: f64, conductivity: f64, mhz: f64) -> Complex64 {
    let w = 2.0 * std::f64::consts::PI * mhz * 1e6;
    let eps = Complex64::new(permittivity, -conductivity / (w * EPS0));
    Complex64::new(ETA0, 0.0) / eps.sqrt()
}

/// A wire mesh's surface impedance (ohms) for current along wires
/// `spacing` metres apart (Wait's formula for parallel wires, the same as
/// the radial screen's with the radials' spacing).
pub fn mesh_impedance(spacing: f64, wire_radius: f64, mhz: f64) -> Complex64 {
    let k = 2.0 * std::f64::consts::PI / wavelength(mhz);
    let s = spacing.max(2.0 * std::f64::consts::PI * wire_radius * 1.0001);
    let ln = (s / (2.0 * std::f64::consts::PI * wire_radius)).ln();
    Complex64::new(0.0, ETA0 * k * s / (2.0 * std::f64::consts::PI) * ln)
}

impl Setup {
    /// Where the field is sampled round: the first radial hub or grid centre.
    fn centre(&self) -> [f64; 2] {
        self.sets
            .first()
            .map(|s| [s.hub[0], s.hub[1]])
            .or(self.grids.first().map(|g| [g.centre[0], g.centre[1]]))
            .unwrap_or([0.0, 0.0])
    }

    /// How far from the centre the radials and grids reach.
    fn reach(&self) -> f64 {
        let c = self.centre();
        let radials = self
            .sets
            .iter()
            .map(|s| s.length + (s.hub[0] - c[0]).hypot(s.hub[1] - c[1]));
        let grids = self.grids.iter().map(|g| {
            g.width.hypot(g.length) / 2.0 + (g.centre[0] - c[0]).hypot(g.centre[1] - c[1])
        });
        radials.chain(grids).fold(0.0, f64::max)
    }
}

/// A radial screen's surface impedance (ohms) `rho` metres from its hub.
pub fn screen_impedance(rho: f64, count: usize, wire_radius: f64, mhz: f64) -> Complex64 {
    let n = count.max(1) as f64;
    let crowd = n * wire_radius;
    let r = (rho * rho + crowd * crowd).sqrt();
    let k = 2.0 * std::f64::consts::PI / wavelength(mhz);
    Complex64::new(0.0, ETA0 * k * (r / n) * (r / crowd).ln())
}

/// The soil's skin depth (metres) at `mhz`: depths well under this barely
/// change what buried radials do.
pub fn skin_depth(permittivity: f64, conductivity: f64, mhz: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * mhz * 1e6;
    let mu0 = 4e-7 * std::f64::consts::PI;
    let eps = permittivity * EPS0;
    let q = conductivity / (w * eps);
    // Attenuation constant of a lossy medium.
    let alpha = w * (mu0 * eps / 2.0 * ((1.0 + q * q).sqrt() - 1.0)).sqrt();
    1.0 / alpha.max(1e-12)
}

/// `numeric` over perfect ground, moved so `hub` (x, y) is at the origin.
fn perfect(numeric: &NumericDeck, hub: [f64; 2]) -> NumericDeck {
    let mut d = numeric.clone();
    d.cards.retain(|c| !(c.is("GN") || c.is("GD")));
    if let Some(ge) = d.cards.iter().position(|c| c.is("GE")) {
        d.cards[ge].ints[0] = 1;
        d.cards.insert(ge + 1, NumCard::new("GN", &[1], &[]));
        if hub != [0.0, 0.0] {
            d.cards.insert(
                ge,
                NumCard::new("GM", &[0, 0], &[0.0, 0.0, 0.0, -hub[0], -hub[1], 0.0, 0.0]),
            );
        }
    }
    d
}

/// The NH cards sampling the ground surface round the origin at `mhz`:
/// finer near the hub, out past the radials and about two wavelengths.
fn rings(mhz: f64, reach: f64, inner: f64) -> Vec<NumCard> {
    let lam = wavelength(mhz);
    let far = (2.1 * lam).max(1.2 * reach);
    let dphi = 360.0 / DIRECTIONS as f64;
    let mut cards = Vec::new();
    let mut push = |from: f64, to: f64, step: f64| {
        if to <= from {
            return;
        }
        let n = ((to - from) / step).ceil() as i64 + 1;
        cards.push(NumCard::new(
            "NH",
            &[1, n, DIRECTIONS, 1],
            &[from, 0.0, 90.0, step, dphi, 0.0],
        ));
    };
    let a = inner.max(5e-4 * lam);
    push(a, 0.02 * lam, 5e-4 * lam);
    push(a.max(0.021 * lam), 0.2 * lam, 2e-3 * lam);
    push(a.max(0.21 * lam), far, 1e-2 * lam);
    cards
}

/// The estimate at one frequency.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub mhz: f64,
    /// Feed impedance over perfect ground.
    pub z_perfect: Complex64,
    /// What the ground and radials add to it.
    pub dz: Complex64,
}

/// The estimate across a sweep.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Estimate {
    /// Over perfect ground, at every frequency of the plan.
    perfect: Vec<(f64, Complex64)>,
    /// What the ground and radials add, at a few frequencies.
    added: Vec<(f64, Complex64)>,
}

fn interpolate(points: &[(f64, Complex64)], mhz: f64) -> Option<Complex64> {
    let first = points.first()?;
    if points.len() == 1 || mhz <= first.0 {
        return Some(first.1);
    }
    for w in points.windows(2) {
        if mhz <= w[1].0 {
            let t = (mhz - w[0].0) / (w[1].0 - w[0].0).max(1e-12);
            return Some(w[0].1 + (w[1].1 - w[0].1) * t);
        }
    }
    points.last().map(|p| p.1)
}

impl Estimate {
    /// The estimated feed impedance at `mhz`.
    pub fn z(&self, mhz: f64) -> Option<Complex64> {
        Some(interpolate(&self.perfect, mhz)? + interpolate(&self.added, mhz)?)
    }
}

/// Works out the estimate for `plan`: the antenna over perfect ground at
/// every frequency, and the ground and radials' share at up to `samples`
/// frequencies across it (it changes slowly with frequency).
pub fn estimate(
    engine: &Engine,
    numeric: &NumericDeck,
    setup: &Setup,
    plan: &FrequencyPlan,
    samples: usize,
    cancel: &CancelToken,
) -> Result<Estimate, String> {
    let hub = setup.centre();
    let ground_deck = perfect(numeric, hub);
    let first = |r: &crate::output::FrequencyResult| {
        r.inputs
            .iter()
            .find(|i| !numeric.helper_tags.contains(&i64::from(i.tag)))
            .copied()
    };
    // Over perfect ground, at every frequency.
    let out = engine
        .run(&run_deck(&ground_deck, plan, None), cancel)
        .map_err(|e| e.to_string())?;
    let perfect: Vec<(f64, Complex64)> = out
        .frequencies
        .iter()
        .zip(plan.frequencies())
        .filter_map(|(r, mhz)| Some((mhz, first(r)?.impedance)))
        .collect();
    if perfect.is_empty() {
        return Err("the model has no source".into());
    }
    // The ground and radials' share, at a few frequencies.
    let n = samples.clamp(1, plan.points.max(1));
    let samples_plan = FrequencyPlan { points: n, ..*plan };
    let mut added = Vec::new();
    for mhz in samples_plan.frequencies() {
        let p = point(engine, numeric, setup, mhz, cancel)?;
        added.push((p.mhz, p.dz));
    }
    Ok(Estimate { perfect, added })
}

/// Like [`estimate`], at chosen frequencies (`mhz`, any order): over
/// perfect ground at each, in one engine run, and the ground and radials'
/// share at `samples` frequencies across their range. For the optimiser,
/// whose goals each have their own frequencies.
pub fn estimate_at(
    engine: &Engine,
    numeric: &NumericDeck,
    setup: &Setup,
    mhz: &[f64],
    samples: usize,
    cancel: &CancelToken,
) -> Result<Estimate, String> {
    let (lo, hi) = mhz
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &f| (lo.min(f), hi.max(f)));
    if lo > hi {
        return Ok(Estimate::default());
    }
    let mut run = run_deck(
        &perfect(numeric, setup.centre()),
        &FrequencyPlan::single(lo),
        None,
    );
    // One FR and XQ per frequency, in place of run_deck's.
    run.cards.truncate(run.cards.len().saturating_sub(2));
    run.cards.push(NumCard::new("PT", &[-1], &[]));
    for &f in mhz {
        run.cards.push(NumCard::new("FR", &[0, 1], &[f, 0.0]));
        run.cards.push(NumCard::new("XQ", &[0], &[]));
    }
    let out = engine.run(&run, cancel).map_err(|e| e.to_string())?;
    let mut perfect: Vec<(f64, Complex64)> = out
        .frequencies
        .iter()
        .zip(mhz)
        .filter_map(|(r, &f)| {
            let feed = r
                .inputs
                .iter()
                .find(|i| !numeric.helper_tags.contains(&i64::from(i.tag)))?;
            Some((f, feed.impedance))
        })
        .collect();
    if perfect.len() != mhz.len() {
        return Err("the model has no source".into());
    }
    perfect.sort_by(|a, b| a.0.total_cmp(&b.0));
    let n = if hi > lo { samples.max(2) } else { 1 };
    let plan = FrequencyPlan {
        start_mhz: lo,
        stop_mhz: hi,
        points: n,
    };
    let mut added = Vec::new();
    for f in plan.frequencies() {
        let p = point(engine, numeric, setup, f, cancel)?;
        added.push((p.mhz, p.dz));
    }
    Ok(Estimate { perfect, added })
}

/// The estimate at one frequency.
pub fn point(
    engine: &Engine,
    numeric: &NumericDeck,
    setup: &Setup,
    mhz: f64,
    cancel: &CancelToken,
) -> Result<Point, String> {
    let hub = setup.centre();
    let mut run = run_deck(&perfect(numeric, hub), &FrequencyPlan::single(mhz), None);
    run.cards.pop(); // the XQ
    run.cards.push(NumCard::new("PT", &[-1], &[]));
    let reach = setup.reach();
    // Start clear of the wires standing at the hub.
    let thickest = numeric.cards[..numeric.control_start()]
        .iter()
        .filter(|c| c.is("GW"))
        .map(|c| c.floats[6])
        .fold(0.0, f64::max);
    run.cards.extend(rings(mhz, reach, 3.0 * thickest));
    let out = engine.run(&run, cancel).map_err(|e| e.to_string())?;
    let r = out
        .frequencies
        .first()
        .ok_or("the engine gave no results")?;
    let feed = r
        .inputs
        .iter()
        .find(|i| !numeric.helper_tags.contains(&i64::from(i.tag)))
        .ok_or("the model has no source")?;
    let samples: Vec<([f64; 2], [Complex64; 2])> = r
        .near_h
        .iter()
        .map(|p| ([p.position[0], p.position[1]], [p.e[0], p.e[1]]))
        .collect();
    let dz = surface_integral(&samples, setup, hub, mhz) / (feed.current * feed.current);
    Ok(Point {
        mhz,
        z_perfect: feed.impedance,
        dz,
    })
}

/// A field sample on a ring: distance from the hub, (x, y), (Hx, Hy).
type Sample = (f64, [f64; 2], [Complex64; 2]);

/// ∬ Zs H·H dS from samples of (x, y) relative to `hub` and (Hx, Hy) on
/// rings round it, plus the part beyond the last ring.
fn surface_integral(
    samples: &[([f64; 2], [Complex64; 2])],
    setup: &Setup,
    hub: [f64; 2],
    mhz: f64,
) -> Complex64 {
    let zg = soil_impedance(setup.permittivity, setup.conductivity, mhz);
    let k = 2.0 * std::f64::consts::PI / wavelength(mhz);
    let dphi = 2.0 * std::f64::consts::PI / DIRECTIONS as f64;
    // Zs H·H at one point: the soil everywhere, and inside a set the screen
    // in parallel for the field circling that set's hub.
    let integrand = |p: [f64; 2], h: [Complex64; 2]| -> Complex64 {
        let mut z = zg * (h[0] * h[0] + h[1] * h[1]);
        let (x, y) = (p[0] + hub[0], p[1] + hub[1]);
        // A grid: current across it runs along wires spaced by a cell's
        // length, and current along it along wires a cell's width apart.
        for g in &setup.grids {
            if g.local(x, y).is_some() {
                let (s, c) = g.turn.to_radians().sin_cos();
                let h_across = h[0] * c + h[1] * s;
                let h_along = -h[0] * s + h[1] * c;
                let cell = [g.width / g.cells[0] as f64, g.length / g.cells[1] as f64];
                // Current across (driven by H along) and along (by H across).
                let z_across = mesh_impedance(cell[1], g.wire_radius, mhz);
                let z_along = mesh_impedance(cell[0], g.wire_radius, mhz);
                let par = |zm: Complex64| zm * zg / (zm + zg);
                z += (par(z_across) - zg) * h_along * h_along
                    + (par(z_along) - zg) * h_across * h_across;
                return z;
            }
        }
        for s in &setup.sets {
            let (x, y) = (p[0] + hub[0] - s.hub[0], p[1] + hub[1] - s.hub[1]);
            let rho = x.hypot(y);
            if rho <= s.length && rho > 0.0 {
                let h_phi = (h[1] * x - h[0] * y) / rho;
                let zw = screen_impedance(rho, s.count, s.wire_radius, mhz);
                let zs = zw * zg / (zw + zg);
                z += (zs - zg) * h_phi * h_phi;
                break;
            }
        }
        z
    };
    // One line of samples per direction, outwards.
    let mut lines: Vec<Vec<Sample>> = vec![Vec::new(); DIRECTIONS as usize];
    for &(p, h) in samples {
        let rho = p[0].hypot(p[1]);
        let phi = p[1].atan2(p[0]).rem_euclid(2.0 * std::f64::consts::PI);
        let k = ((phi / dphi).round() as usize) % DIRECTIONS as usize;
        lines[k].push((rho, p, h));
    }
    let mut total = Complex64::new(0.0, 0.0);
    for line in &mut lines {
        line.sort_by(|a, b| a.0.total_cmp(&b.0));
        for w in line.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let fa = integrand(a.1, a.2) * a.0;
            let fb = integrand(b.1, b.2) * b.0;
            total += (fa + fb) * 0.5 * (b.0 - a.0) * dphi;
        }
        // Beyond the last ring, H falls as 1/ρ and turns with e^-jkρ.
        if let Some(&(r, _, h)) = line.last() {
            let t = Complex64::new(0.0, 2.0 * k * r);
            let series = (Complex64::new(1.0, 0.0) - t.inv() + t.inv() * t.inv() * 2.0) / t;
            total += zg * (h[0] * h[0] + h[1] * h[1]) * r * series * dphi;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soil_and_screen_impedances() {
        // Average ground at 7 MHz: about 88 ohm at 22 degrees.
        let zg = soil_impedance(13.0, 0.005, 7.0);
        assert!((zg.norm() - 88.1).abs() < 0.5, "{zg}");
        assert!((zg.arg().to_degrees() - 22.3).abs() < 0.5, "{zg}");
        // A screen is inductive and grows with distance from the hub.
        let near = screen_impedance(0.5, 32, 0.001, 7.0);
        let far = screen_impedance(5.0, 32, 0.001, 7.0);
        assert!(near.re == 0.0 && near.im > 0.0 && far.im > near.im);
        // Skin depth of average ground at 3 MHz: several metres.
        let d = skin_depth(13.0, 0.005, 3.0);
        assert!((2.0..10.0).contains(&d), "{d}");
    }
}
