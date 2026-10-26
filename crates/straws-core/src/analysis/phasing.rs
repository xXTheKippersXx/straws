//! Phasing lines: the feed network for an array. Every element is fed
//! through its own length of line from one junction, and the lengths are
//! chosen so the elements get the currents wanted (as set in the Feeds
//! view), given their active impedances (coupling included, from a run with
//! those currents).
//!
//! An element of active impedance Z at the end of a line (Z0, propagation
//! γ = α + jβ, the cable's loss included) of length l from a junction at
//! voltage V gets the current I = V / (Z cosh γl + Z0 sinh γl). A line's
//! length sets the phase it delivers; the amplitude then follows, so for
//! each choice of the reference element's line the others are cut for the
//! right phase, and the shortest set of lines whose amplitudes come within
//! a chosen slack (dB) of the best possible is kept. Equal-length quarter-wave lines are the classic case
//! (Christman's "current forcing": I = -jV/Z0 whatever the load).

use std::f64::consts::PI;

use num_complex::Complex64;

use crate::analysis::feedline::{Cable, line_input};
use crate::analysis::matching::Part;
use crate::deck::Deck;
use crate::deck::edit::card_line;
use crate::deck::sources::Source;
use crate::deck::write::format_real;
use crate::units::C;

/// A feed network found by [`design`].
#[derive(Debug, Clone, PartialEq)]
pub struct Design {
    /// Physical length of each element's line (m), in the elements' order.
    pub lengths: Vec<f64>,
    /// The currents wanted, and the currents the lines give (scaled so the
    /// reference element's is the one wanted).
    pub wanted: Vec<Complex64>,
    pub currents: Vec<Complex64>,
    /// The junction voltage that gives them.
    pub junction_voltage: Complex64,
    /// Impedance at the junction (all the lines in parallel).
    pub junction: Complex64,
    /// The element whose line was chosen freely.
    pub reference: usize,
    /// Worst amplitude (dB) and phase (degrees) error against the wanted
    /// currents.
    pub worst_db: f64,
    pub worst_deg: f64,
    /// An L network at each element (all empty unless
    /// [`Design::with_networks`] added them).
    pub nets: Vec<ElementNet>,
}

impl Design {
    /// The same lines with an L network at each element that makes every
    /// current exactly what's wanted at `mhz` ([`exact_networks`]).
    pub fn with_networks(&self, z: &[Complex64], cable: &Cable, mhz: f64) -> Option<Design> {
        let (nets, v0, junction) = exact_networks(z, self, cable, mhz)?;
        Some(Design {
            currents: self.wanted.clone(),
            junction_voltage: v0,
            junction,
            worst_db: 0.0,
            worst_deg: 0.0,
            nets,
            ..self.clone()
        })
    }
}

/// Propagation constant (per metre) of `cable` at `mhz`.
fn gamma(cable: &Cable, mhz: f64) -> Complex64 {
    let alpha = cable.loss_db_per_m(mhz) / (20.0 / std::f64::consts::LN_10);
    let beta = 2.0 * PI * mhz * 1e6 / (C * cable.velocity_factor);
    Complex64::new(alpha, beta)
}

/// Current into a load `z` at the end of `l` metres of `cable` fed with 1 V.
pub fn delivered(cable: &Cable, mhz: f64, z: Complex64, l: f64) -> Complex64 {
    let gl = gamma(cable, mhz) * l;
    1.0 / (z * gl.cosh() + cable.z0 * gl.sinh())
}

fn wrap(a: f64) -> f64 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

/// The shortest length from `min` up that delivers phase `target`
/// (radians) into `z`: the first place the phase passes it (scanned over
/// one wavelength in the line, then bisected).
fn length_for_phase(cable: &Cable, mhz: f64, z: Complex64, target: f64, min: f64) -> Option<f64> {
    let lam = C * cable.velocity_factor / (mhz * 1e6);
    let err = |l: f64| wrap(delivered(cable, mhz, z, l).arg() - target);
    let steps = 720;
    let mut prev = (min, err(min));
    if prev.1.abs() < 1e-12 {
        return Some(min);
    }
    for k in 1..=steps {
        let l = min + lam * 1.02 * k as f64 / steps as f64;
        let e = err(l);
        // A real crossing (not the jump at ±180°): the error is small on
        // both sides.
        if prev.1.signum() != e.signum() && (prev.1 - e).abs() < PI {
            let (mut a, mut b) = (prev, (l, e));
            for _ in 0..60 {
                let m = (a.0 + b.0) / 2.0;
                let em = err(m);
                if em.signum() == a.1.signum() {
                    a = (m, em);
                } else {
                    b = (m, em);
                }
            }
            return Some((a.0 + b.0) / 2.0);
        }
        prev = (l, e);
    }
    None
}

/// Phasing lines of `cable` that give the currents `want` to elements of
/// active impedance `z` (at least `min_len` metres each, e.g. to reach them
/// from the junction), at `mhz`: the shortest set whose worst amplitude
/// error is within `slack_db` of the best possible. `None` if there's
/// nothing to design (fewer than two elements, or no current wanted).
pub fn design(
    z: &[Complex64],
    want: &[Complex64],
    min_len: &[f64],
    cable: &Cable,
    mhz: f64,
    slack_db: f64,
) -> Option<Design> {
    let n = z.len();
    if n < 2 || want.len() != n || min_len.len() != n {
        return None;
    }
    let reference = (0..n).max_by(|&a, &b| want[a].norm().total_cmp(&want[b].norm()))?;
    if want[reference].norm() == 0.0 {
        return None;
    }
    let lam = C * cable.velocity_factor / (mhz * 1e6);
    // (worst amplitude error in dB, lengths) for each reference length.
    let mut found: Vec<(f64, Vec<f64>)> = Vec::new();
    let steps = 240;
    for k in 0..steps {
        let l_ref = min_len[reference] + lam * k as f64 / steps as f64;
        let i_ref = delivered(cable, mhz, z[reference], l_ref);
        let mut lengths = vec![0.0; n];
        lengths[reference] = l_ref;
        let mut worst: f64 = 0.0;
        let mut ok = true;
        for e in 0..n {
            if e == reference || want[e].norm() == 0.0 {
                continue;
            }
            let target = i_ref.arg() + (want[e] / want[reference]).arg();
            match length_for_phase(cable, mhz, z[e], target, min_len[e]) {
                Some(l) => {
                    lengths[e] = l;
                    let got = delivered(cable, mhz, z[e], l) / i_ref;
                    let wanted = want[e] / want[reference];
                    worst = worst.max((20.0 * (got.norm() / wanted.norm()).log10()).abs());
                }
                None => ok = false,
            }
        }
        if !ok {
            continue;
        }
        found.push((worst, lengths));
    }
    // The shortest lines whose amplitudes come within the slack of the best.
    let best = found.iter().map(|f| f.0).fold(f64::INFINITY, f64::min);
    let (_, lengths) = found
        .into_iter()
        .filter(|f| f.0 <= best + slack_db.max(0.0) + 1e-9)
        .min_by(|a, b| a.1.iter().sum::<f64>().total_cmp(&b.1.iter().sum::<f64>()))?;
    Some(finish(z, want, lengths, cable, mhz, reference))
}

/// The design for these `lengths`: the currents they give at `mhz`
/// (scaled so `reference` gets what it wants), the junction, the errors.
fn finish(
    z: &[Complex64],
    want: &[Complex64],
    lengths: Vec<f64>,
    cable: &Cable,
    mhz: f64,
    reference: usize,
) -> Design {
    let n = z.len();
    let raw: Vec<Complex64> = (0..n)
        .map(|e| delivered(cable, mhz, z[e], lengths[e]))
        .collect();
    let v = want[reference] / raw[reference];
    let currents: Vec<Complex64> = raw.iter().map(|i| i * v).collect();
    let admittance: Complex64 = (0..n)
        .map(|e| 1.0 / line_input(cable, lengths[e], mhz, z[e]))
        .sum();
    let (mut worst_db, mut worst_deg) = (0.0f64, 0.0f64);
    for e in 0..n {
        if want[e].norm() == 0.0 {
            continue;
        }
        let r = currents[e] / want[e];
        worst_db = worst_db.max((20.0 * r.norm().log10()).abs());
        worst_deg = worst_deg.max(r.arg().to_degrees().abs());
    }
    Design {
        lengths,
        wanted: want.to_vec(),
        currents,
        junction_voltage: v,
        junction: 1.0 / admittance,
        reference,
        worst_db,
        worst_deg,
        nets: vec![ElementNet::default(); n],
    }
}

/// The elements' active impedances at one frequency of a band.
pub type BandPoint = (f64, Vec<Complex64>);

/// Worst errors of lines `lengths` over `band`, each element's current
/// against the reference element's as a share of what's wanted: amplitude
/// (dB), phase (degrees), and the vector error (|got/wanted - 1|, the
/// share of the wanted current that's wrong: what [`design_band`]
/// minimises).
pub fn band_errors(
    band: &[BandPoint],
    want: &[Complex64],
    lengths: &[f64],
    nets: &[ElementNet],
    cable: &Cable,
    reference: usize,
) -> (f64, f64, f64) {
    let (mut db, mut deg, mut vector) = (0.0f64, 0.0f64, 0.0f64);
    let net = |e: usize| nets.get(e).copied().unwrap_or_default();
    for (mhz, z) in band {
        let at = |e: usize| 1.0 / branch(cable, *mhz, z[e], lengths[e], &net(e)).0;
        let i_ref = at(reference);
        for e in 0..z.len().min(want.len()) {
            if e == reference || want[e].norm() == 0.0 {
                continue;
            }
            let r = at(e) / i_ref / (want[e] / want[reference]);
            db = db.max((20.0 * r.norm().log10()).abs());
            deg = deg.max(r.arg().to_degrees().abs());
            vector = vector.max((r - 1.0).norm());
        }
    }
    (db, deg, vector)
}

/// An L network at an element, to make its current exactly what's wanted:
/// a part across the end of its line, then a part in series with the
/// element (either may be absent).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ElementNet {
    pub shunt: Option<Part>,
    pub series: Option<Part>,
}

impl ElementNet {
    pub fn describe(&self) -> String {
        let part = |p: &Part| match p {
            Part::Inductor(_) => format!("{} inductor", p.describe()),
            Part::Capacitor(_) => format!("{} capacitor", p.describe()),
        };
        match (&self.shunt, &self.series) {
            (None, None) => "none".into(),
            (Some(a), None) => format!("{} across the line", part(a)),
            (None, Some(b)) => format!("{} in series", part(b)),
            (Some(a), Some(b)) => {
                format!("{} across the line, then {} in series", part(a), part(b))
            }
        }
    }

    /// The same network with standard part values (12 or 24 a decade).
    pub fn snapped(&self, per_decade: usize) -> ElementNet {
        ElementNet {
            shunt: self.shunt.map(|p| p.snapped(per_decade)),
            series: self.series.map(|p| p.snapped(per_decade)),
        }
    }
}

/// One branch from the junction (line, then `net`, then the element of
/// impedance `z`), per ampere into the element: the junction voltage it
/// takes, and the current it draws from the junction.
pub fn branch(
    cable: &Cable,
    mhz: f64,
    z: Complex64,
    l: f64,
    net: &ElementNet,
) -> (Complex64, Complex64) {
    let j = Complex64::new(0.0, 1.0);
    let x = net.series.map_or(0.0, |p| p.reactance(mhz));
    let y = net
        .shunt
        .map_or(Complex64::default(), |p| 1.0 / (j * p.reactance(mhz)));
    let z1 = z + j * x;
    // At the line's end: the node voltage and the current into the node.
    let (vn, i_n) = (z1, 1.0 + y * z1);
    let gl = gamma(cable, mhz) * l;
    let (c, s) = (gl.cosh(), gl.sinh());
    (c * vn + cable.z0 * s * i_n, s / cable.z0 * vn + c * i_n)
}

/// L networks that make every element's current exactly what `design`
/// wants at `mhz`, on its lines as they are: the reference element's branch
/// has none (its line sets the junction voltage), and each other gets the
/// network of smallest reactances that solves (Z + jX)(A + jb·B) = V/w - B,
/// with (A, B) its line's ABCD terms, X the series reactance and b the
/// shunt susceptance. `None` for an element where no network solves it
/// (e.g. a line a whole number of half waves long). Also returns the
/// junction voltage and the junction's impedance.
pub fn exact_networks(
    z: &[Complex64],
    design: &Design,
    cable: &Cable,
    mhz: f64,
) -> Option<(Vec<ElementNet>, Complex64, Complex64)> {
    let n = z.len();
    let want = &design.wanted;
    let r = design.reference;
    let none = ElementNet::default();
    let v0 = want[r] * branch(cable, mhz, z[r], design.lengths[r], &none).0;
    let j = Complex64::new(0.0, 1.0);
    let mut nets = vec![none; n];
    for e in 0..n {
        if e == r || want[e].norm() == 0.0 {
            continue;
        }
        let gl = gamma(cable, mhz) * design.lengths[e];
        let (a, b) = (gl.cosh(), cable.z0 * gl.sinh());
        if b.norm() < 1e-9 * cable.z0 {
            return None;
        }
        let p = v0 / want[e] - b;
        // For series reactance x, the shunt susceptance that's needed, and
        // how far it is from being real (zero: a solution).
        let need = |x: f64| ((p / (z[e] + j * x) - a) / (j * b), x);
        let off = |x: f64| need(x).0.im;
        let top = 20.0 * (z[e].norm() + cable.z0) + 2000.0;
        let steps = 4000;
        let mut found: Vec<(f64, f64)> = Vec::new();
        let mut prev = (-top, off(-top));
        for k in 1..=steps {
            let x = -top + 2.0 * top * k as f64 / steps as f64;
            let o = off(x);
            if prev.1.signum() != o.signum()
                && (prev.1 - o).abs() < 1e3 * (prev.1.abs() + o.abs() + 1e-12).min(1.0) + 1.0
            {
                let (mut lo, mut hi) = (prev, (x, o));
                for _ in 0..80 {
                    let m = (lo.0 + hi.0) / 2.0;
                    let om = off(m);
                    if om.signum() == lo.1.signum() {
                        lo = (m, om);
                    } else {
                        hi = (m, om);
                    }
                }
                let x = (lo.0 + hi.0) / 2.0;
                let (bv, _) = need(x);
                // A real root (not a jump through infinity).
                if bv.im.abs() < 1e-6 * (bv.re.abs() + 1e-6) {
                    found.push((x, bv.re));
                }
            }
            prev = (x, o);
        }
        let (x, bsh) = found.into_iter().min_by(|p, q| {
            let size = |(x, b): (f64, f64)| x.abs() / cable.z0 + b.abs() * cable.z0;
            size(*p).total_cmp(&size(*q))
        })?;
        let small = |v: f64| v.abs() < 1e-9;
        nets[e] = ElementNet {
            shunt: (!small(bsh * cable.z0)).then(|| Part::for_susceptance(bsh, mhz)),
            series: (!small(x / cable.z0)).then(|| Part::for_reactance(x, mhz)),
        };
    }
    let admittance: Complex64 = (0..n)
        .map(|e| {
            let (v, i) = branch(cable, mhz, z[e], design.lengths[e], &nets[e]);
            // Per volt at the junction: the element gets want·(v0/v)/v0...
            i / v
        })
        .sum();
    Some((nets, v0, 1.0 / admittance))
}

/// Lines tuned over a band: starting from `start` (a design for one
/// frequency, at `mhz`), the lengths that keep the elements' currents
/// closest to what's wanted across all of `band` (the worst error, as a
/// fraction of the wanted current, made as small as possible; Nelder-Mead),
/// none shorter than `min_len`. The result's figures are at `mhz`.
pub fn design_band(
    band: &[BandPoint],
    z: &[Complex64],
    min_len: &[f64],
    cable: &Cable,
    mhz: f64,
    start: &Design,
) -> Design {
    let n = start.lengths.len();
    let want = &start.wanted;
    let r = start.reference;
    let cost = |l: &[f64]| -> f64 {
        let mut worst: f64 = 0.0;
        for (f, zs) in band {
            let i_ref = delivered(cable, *f, zs[r], l[r]);
            for e in 0..n {
                if e == r || want[e].norm() == 0.0 {
                    continue;
                }
                let x = delivered(cable, *f, zs[e], l[e]) / i_ref / (want[e] / want[r]);
                worst = worst.max((x - 1.0).norm());
            }
        }
        let short: f64 = (0..n).map(|e| (min_len[e] - l[e]).max(0.0).powi(2)).sum();
        worst + 1e3 * short
    };
    let lam = C * cable.velocity_factor / (mhz * 1e6);
    let mut simplex: Vec<(Vec<f64>, f64)> = (0..=n)
        .map(|k| {
            let mut l = start.lengths.clone();
            if k > 0 {
                l[k - 1] += lam / 40.0;
            }
            let c = cost(&l);
            (l, c)
        })
        .collect();
    for _ in 0..600 {
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
        let fr = cost(&refl);
        if fr < simplex[0].1 {
            let exp = towards(-2.0);
            let fe = cost(&exp);
            simplex[n] = if fe < fr { (exp, fe) } else { (refl, fr) };
        } else if fr < simplex[n - 1].1 {
            simplex[n] = (refl, fr);
        } else {
            let con = towards(0.5);
            let fc = cost(&con);
            if fc < simplex[n].1 {
                simplex[n] = (con, fc);
            } else {
                let best = simplex[0].0.clone();
                for p in simplex.iter_mut().skip(1) {
                    for (x, b) in p.0.iter_mut().zip(&best) {
                        *x = b + 0.5 * (*x - b);
                    }
                    p.1 = cost(&p.0);
                }
            }
        }
        let spread = simplex[n].1 - simplex[0].1;
        if spread.abs() < 1e-12 {
            break;
        }
    }
    simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
    let lengths: Vec<f64> = simplex[0]
        .0
        .iter()
        .zip(min_len)
        .map(|(l, m)| l.max(*m))
        .collect();
    finish(z, want, lengths, cable, mhz, r)
}

/// The comment marking the cards a phasing network adds.
pub const PHASING_MARK: &str = "straws: phasing";

/// Writes `design` into `deck`: the `sources`' EX cards go, a tiny junction
/// wire (tag `tag`, a thousandth of a wavelength, `at` metres: keep it
/// clear of the antenna) is fed with the design's junction voltage, and a
/// TL card joins it to each element's feed segment. NEC's lines are
/// lossless and take electrical length, so each is its physical length
/// over the cable's velocity factor.
pub fn write_lines(
    deck: &mut Deck,
    sources: &[Source],
    design: &Design,
    cable: &Cable,
    mhz: f64,
    tag: i64,
    at: [f64; 3],
) {
    let lam = C / (mhz * 1e6);
    let lines: Vec<usize> = sources.iter().map(|s| s.line).collect();
    deck.remove_lines(&lines);
    let half = lam / 2000.0;
    let gw: Vec<String> = [
        tag as f64,
        1.0,
        at[0],
        at[1],
        at[2] - half,
        at[0],
        at[1],
        at[2] + half,
        lam / 40_000.0,
    ]
    .into_iter()
    .map(format_real)
    .collect();
    let comment = format!("{PHASING_MARK} junction");
    deck.insert_lines(deck.geometry_end(), &[card_line("GW", &gw, Some(&comment))]);
    let mut cards = Vec::new();
    for (k, s) in sources.iter().enumerate() {
        let net = design.nets.get(k).copied().unwrap_or_default();
        // The part across the line's end: the TL's admittance at its
        // element end (NEC holds it fixed: right at the design frequency).
        let y_shunt = net.shunt.map_or(0.0, |p| -1.0 / p.reactance(mhz));
        let f: Vec<String> = [
            tag.to_string(),
            "1".into(),
            s.tag.to_string(),
            s.segment.to_string(),
            format_real(cable.z0),
            format_real(design.lengths[k] / cable.velocity_factor),
            "0".into(),
            "0".into(),
            "0".into(),
            format_real(y_shunt),
        ]
        .into();
        let w = design.wanted.get(k).copied().unwrap_or_default();
        let part = |p: &Part| match *p {
            Part::Inductor(v) => format!("L {}", format_real(v)),
            Part::Capacitor(v) => format!("C {}", format_real(v)),
        };
        let mut note = format!(
            "{PHASING_MARK} line {}: {} {} m, wants {} A at {} deg",
            k + 1,
            cable.name,
            format_real(design.lengths[k]),
            format_real(w.norm()),
            format_real(w.arg().to_degrees())
        );
        if let Some(p) = &net.shunt {
            note += &format!("; shunt {}", part(p));
        }
        if let Some(p) = &net.series {
            note += &format!("; series {}", part(p));
        }
        cards.push(card_line("TL", &f, Some(&note)));
        // The series part: a load on the element's feed segment (NEC's
        // series RLC load: right at every frequency).
        if let Some(p) = &net.series {
            let (l, c) = match *p {
                Part::Inductor(v) => (v, 0.0),
                Part::Capacitor(v) => (0.0, v),
            };
            let ld: Vec<String> = [
                "0".into(),
                s.tag.to_string(),
                s.segment.to_string(),
                s.segment.to_string(),
                "0".into(),
                format_real(l),
                format_real(c),
            ]
            .into();
            cards.push(card_line(
                "LD",
                &ld,
                Some(&format!("{PHASING_MARK} series part {}", k + 1)),
            ));
        }
    }
    let v = design.junction_voltage;
    let ex: Vec<String> = [
        "0".into(),
        tag.to_string(),
        "1".into(),
        "0".into(),
        format_real(v.re),
        format_real(v.im),
    ]
    .into();
    cards.push(card_line("EX", &ex, Some(&format!("{PHASING_MARK} feed"))));
    let after_ge = deck.ge_line().map_or(deck.lines.len(), |g| g + 1);
    deck.insert_lines(after_ge, &cards);
}

/// A phasing line found in a deck (written by [`write_lines`]).
#[derive(Debug, Clone, PartialEq)]
pub struct NetLine {
    /// The TL card's line.
    pub line: usize,
    /// The element it feeds: wire and segment.
    pub tag: i64,
    pub segment: i64,
    /// Its cable and physical length (m), and the current it's meant to
    /// give, from the card's comment.
    pub cable: String,
    pub length_m: Option<f64>,
    pub wanted: Option<Complex64>,
    /// Its element's L network, if it has one.
    pub net: ElementNet,
}

/// The phasing lines in `deck`, in order.
pub fn network(deck: &Deck) -> Vec<NetLine> {
    let mut out = Vec::new();
    for (i, l) in deck.lines.iter().enumerate() {
        let crate::deck::LineKind::Card { mnemonic, fields } = &l.kind else {
            continue;
        };
        let Some(note) = l.comment.as_deref() else {
            continue;
        };
        if mnemonic != "TL" || !note.contains(PHASING_MARK) {
            continue;
        }
        let num = |k: usize| fields.get(k).and_then(|f| f.parse::<f64>().ok());
        let (Some(tag), Some(segment)) = (num(2), num(3)) else {
            continue;
        };
        // "... line N: CABLE LEN m, wants AMP A at PHASE deg"
        let after = note.split_once(PHASING_MARK).map_or("", |r| r.1);
        let rest = after.split_once(": ").map_or("", |r| r.1);
        let mut sections = rest.split("; ");
        let rest = sections.next().unwrap_or("");
        let mut net = ElementNet::default();
        for sec in sections {
            let w: Vec<&str> = sec.split_whitespace().collect();
            if let [place, kind, v] = w.as_slice()
                && let Ok(v) = v.parse::<f64>()
            {
                let part = if *kind == "L" {
                    Part::Inductor(v)
                } else {
                    Part::Capacitor(v)
                };
                match *place {
                    "shunt" => net.shunt = Some(part),
                    "series" => net.series = Some(part),
                    _ => {}
                }
            }
        }
        let (cable_len, wants) = rest.split_once(", wants ").unwrap_or((rest, ""));
        let mut words: Vec<&str> = cable_len.split_whitespace().collect();
        let length_m = (words.last() == Some(&"m"))
            .then(|| {
                words.pop();
                words.pop().and_then(|w| w.parse().ok())
            })
            .flatten();
        let w: Vec<f64> = wants
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        out.push(NetLine {
            line: i,
            tag: tag as i64,
            segment: segment as i64,
            cable: words.join(" "),
            length_m,
            wanted: (w.len() == 2).then(|| Complex64::from_polar(w[0], w[1].to_radians())),
            net,
        });
    }
    out
}

/// Takes a phasing network out of `deck` and feeds each element again with
/// a current source of the current its line was meant to give.
pub fn remove_network(deck: &mut Deck) {
    let lines = network(deck);
    let marked: Vec<usize> = deck
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            l.comment
                .as_deref()
                .is_some_and(|c| c.contains(PHASING_MARK))
        })
        .map(|(i, _)| i)
        .collect();
    deck.remove_lines(&marked);
    for l in lines {
        deck.add_source(
            l.tag,
            l.segment,
            crate::deck::sources::SourceKind::Current,
            l.wanted.unwrap_or(Complex64::new(1.0, 0.0)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::feedline::CABLES;

    #[test]
    fn lines_tuned_over_a_band_hold_up_better() {
        // Impedances that change across 7.0-7.3 MHz: the band-tuned lines'
        // worst error over the band is no worse than the lines cut for the
        // middle, and usually better.
        let rg213 = CABLES[2];
        let band: Vec<BandPoint> = (0..=6)
            .map(|k| {
                let f = 7.0 + 0.05 * k as f64;
                let d = (f - 7.15) * 100.0;
                (
                    f,
                    vec![
                        Complex64::new(22.0 + 0.5 * d, -10.0 + 2.0 * d),
                        Complex64::new(57.0 - d, 44.0 + 3.0 * d),
                    ],
                )
            })
            .collect();
        let z = band[3].1.clone();
        let want = [
            Complex64::new(1.0, 0.0),
            Complex64::from_polar(1.0, -PI / 2.0),
        ];
        let mid = design(&z, &want, &[1.0, 1.0], &rg213, 7.15, 0.25).unwrap();
        let tuned = design_band(&band, &z, &[1.0, 1.0], &rg213, 7.15, &mid);
        let vector = |d: &Design| band_errors(&band, &want, &d.lengths, &[], &rg213, d.reference).2;
        println!("middle {:.4}, tuned {:.4}", vector(&mid), vector(&tuned));
        assert_eq!(mid.reference, tuned.reference);
        assert!(vector(&tuned) <= vector(&mid) + 1e-9);
        assert!(vector(&tuned) < 0.9 * vector(&mid));
        assert!(tuned.lengths.iter().all(|l| *l >= 1.0));
    }

    #[test]
    fn element_networks_make_the_currents_exact() {
        // Four elements, the lines' amplitudes off by up to a dB or so:
        // with an L network at each, every current is what's wanted, and
        // the junction is the branches in parallel.
        let rg213 = CABLES[2];
        let z = [
            Complex64::new(67.6, -60.6),
            Complex64::new(107.3, -72.1),
            Complex64::new(107.3, -72.1),
            Complex64::new(150.4, -4.5),
        ];
        let want = [
            Complex64::new(1.0, 0.0),
            Complex64::new(0.0, -1.0),
            Complex64::new(0.0, -1.0),
            Complex64::new(-1.0, 0.0),
        ];
        let d = design(&z, &want, &[7.5; 4], &rg213, 7.1, 0.25).unwrap();
        let (nets, v0, junction) = exact_networks(&z, &d, &rg213, 7.1).unwrap();
        let mut y = Complex64::default();
        for e in 0..4 {
            let (v, i) = branch(&rg213, 7.1, z[e], d.lengths[e], &nets[e]);
            let got = v0 / v;
            assert!(
                (got - want[e]).norm() < 1e-6,
                "element {e}: {got} {:?}",
                nets[e]
            );
            y += i / v;
        }
        assert!((junction - 1.0 / y).norm() < 1e-9);
        assert_eq!(nets[d.reference], ElementNet::default());
        println!(
            "{:?}",
            nets.iter().map(ElementNet::describe).collect::<Vec<_>>()
        );
        // Across a band the parts' reactances change with frequency.
        let band = vec![(7.0, z.to_vec()), (7.1, z.to_vec()), (7.2, z.to_vec())];
        let (db, _, _) = band_errors(&band, &want, &d.lengths, &nets, &rg213, d.reference);
        assert!(db.is_finite());
        let (db0, deg0, _) =
            band_errors(&band[1..2], &want, &d.lengths, &nets, &rg213, d.reference);
        assert!(db0 < 1e-6 && deg0 < 1e-4, "{db0} {deg0}");
    }

    #[test]
    fn quarter_wave_lines_force_the_current() {
        // Christman's current forcing: through a lossless quarter wave the
        // current is -j/Z0 whatever the load.
        let ideal = CABLES[7];
        let q = C / 7e6 / 4.0;
        for z in [Complex64::new(20.0, 30.0), Complex64::new(150.0, -80.0)] {
            let i = delivered(&ideal, 7.0, z, q);
            assert!((i - Complex64::new(0.0, -1.0 / 50.0)).norm() < 1e-9, "{i}");
        }
    }

    #[test]
    fn lines_for_a_two_element_end_fire_pair() {
        // Two elements with different (coupled) impedances, wanted equal
        // currents 90° apart: the design gets the phase exactly and the
        // amplitude close, and each line delivers what it says.
        let rg213 = CABLES[2];
        let z = [Complex64::new(22.0, -10.0), Complex64::new(57.0, 44.0)];
        let want = [
            Complex64::new(1.0, 0.0),
            Complex64::from_polar(1.0, -PI / 2.0),
        ];
        let d = design(&z, &want, &[1.0, 1.0], &rg213, 7.1, 0.25).unwrap();
        assert!(d.worst_deg < 0.01, "{d:?}");
        assert!(d.worst_db < 1.5, "{d:?}");
        for (e, &ze) in z.iter().enumerate() {
            assert!(d.lengths[e] >= 1.0);
            let i = delivered(&rg213, 7.1, ze, d.lengths[e]) * d.junction_voltage;
            assert!((i - d.currents[e]).norm() < 1e-9);
        }
        // The junction is the lines' inputs in parallel.
        let y: Complex64 = (0..2)
            .map(|e| 1.0 / line_input(&rg213, d.lengths[e], 7.1, z[e]))
            .sum();
        assert!((d.junction - 1.0 / y).norm() < 1e-9);
    }
}
