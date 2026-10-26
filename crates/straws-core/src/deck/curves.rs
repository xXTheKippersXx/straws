//! Curved wires: arcs, circles, helices and flat spirals, each written as
//! deck cards whose sizes and placement are SY variables (so sliders and the
//! optimiser can change them).
//!
//! - Arcs and circles are NEC's GA card (an arc in the x-z plane about the
//!   origin), helices its GH card (along z from the origin), and spirals a
//!   chain of short GW wires (in the x-y plane about the origin) sharing one
//!   tag.
//! - One GM card after each curve turns it to face the chosen way and moves
//!   it to its centre. GM moves everything from the curve's tag to the end
//!   of the structure so far, so the curve is written last (just before GE)
//!   and only it moves.
//! - The first card carries a comment `straws: curve NAME KIND`, so
//!   the curve can be found again; its variables are named `NAME_...` and
//!   grouped under NAME.
//!
//! A curve's axis is the normal of its plane (the helix axis for a helix).
//! It points up (+z) before turning: `tilt` degrees from +z towards +x,
//! then `azimuth` degrees about z.

use super::expr;
use super::vars::VarNotes;
use super::write::format_real;
use super::{Deck, LineKind};

/// Start of the comment that marks a curve's first card.
pub const CURVE_MARK: &str = "straws: curve";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveKind {
    Circle,
    Arc,
    Helix,
    /// Archimedean: the radius grows evenly with angle.
    Spiral,
    /// Logarithmic (equiangular): the radius grows by the same factor
    /// each turn.
    LogSpiral,
}

impl CurveKind {
    pub const ALL: [CurveKind; 5] = [
        CurveKind::Circle,
        CurveKind::Arc,
        CurveKind::Helix,
        CurveKind::Spiral,
        CurveKind::LogSpiral,
    ];

    /// The word used in decks.
    pub fn key(self) -> &'static str {
        match self {
            CurveKind::Circle => "circle",
            CurveKind::Arc => "arc",
            CurveKind::Helix => "helix",
            CurveKind::Spiral => "spiral",
            CurveKind::LogSpiral => "logspiral",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CurveKind::Circle => "Circle (loop)",
            CurveKind::Arc => "Arc",
            CurveKind::Helix => "Helix",
            CurveKind::Spiral => "Flat spiral (Archimedean)",
            CurveKind::LogSpiral => "Flat spiral (logarithmic)",
        }
    }

    fn from_key(key: &str) -> Option<CurveKind> {
        CurveKind::ALL.into_iter().find(|k| k.key() == key)
    }
}

/// Everything needed to write a curve (lengths in metres, angles in degrees).
#[derive(Debug, Clone, PartialEq)]
pub struct CurveSpec {
    pub kind: CurveKind,
    /// Prefix of its variables and its group, e.g. `loop1`.
    pub name: String,
    pub centre: [f64; 3],
    /// The axis: degrees from +z towards +x, then about z.
    pub tilt: f64,
    pub azimuth: f64,
    /// Arc, circle and helix radius; a spiral's inner radius.
    pub radius: f64,
    /// A helix's radius at its far end; a spiral's outer radius.
    pub radius2: f64,
    /// Arc start and end angles.
    pub from: f64,
    pub to: f64,
    /// Helix and spiral turns.
    pub turns: f64,
    /// Helix length along its axis.
    pub length: f64,
    /// A left-handed helix (or a spiral winding clockwise).
    pub left: bool,
    /// Segments (arc, circle, helix) or straight pieces (spirals).
    pub segments: i64,
    pub wire_radius: f64,
}

impl CurveSpec {
    /// A sensible curve of this kind at `centre`.
    pub fn new(kind: CurveKind, name: &str, centre: [f64; 3], radius: f64) -> CurveSpec {
        CurveSpec {
            kind,
            name: name.to_string(),
            centre,
            tilt: 0.0,
            azimuth: 0.0,
            radius,
            radius2: match kind {
                CurveKind::Spiral | CurveKind::LogSpiral => radius * 5.0,
                _ => radius,
            },
            from: 0.0,
            to: 180.0,
            turns: match kind {
                CurveKind::Helix => 5.0,
                _ => 3.0,
            },
            length: radius * 4.0,
            left: false,
            segments: 21,
            wire_radius: 0.001,
        }
    }

    /// The wire length of the curve (metres), for choosing segments.
    pub fn wire_length(&self) -> f64 {
        let tau = std::f64::consts::TAU;
        match self.kind {
            CurveKind::Circle => tau * self.radius,
            CurveKind::Arc => tau * self.radius * (self.to - self.from).abs() / 360.0,
            CurveKind::Helix => {
                let r = (self.radius + self.radius2) / 2.0;
                (tau * r * self.turns).hypot(self.length)
            }
            CurveKind::Spiral | CurveKind::LogSpiral => {
                // Near enough: the mean circumference times the turns.
                tau * (self.radius + self.radius2) / 2.0 * self.turns
            }
        }
    }
}

/// A curve found in a deck.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub name: String,
    pub kind: CurveKind,
    pub tag: i64,
    /// Its card lines (indices into `Deck::lines`), the GM card last.
    pub lines: Vec<usize>,
}

/// The curves in a deck (made by [`Deck::add_curve`]).
pub fn curves(deck: &Deck) -> Vec<Curve> {
    let mut out: Vec<Curve> = Vec::new();
    for (i, line) in deck.lines.iter().enumerate() {
        let LineKind::Card { fields, .. } = &line.kind else {
            continue;
        };
        let tag = || {
            fields
                .first()
                .and_then(|t| t.parse::<f64>().ok())
                .map(|t| t as i64)
        };
        if let Some(rest) = line
            .comment
            .as_deref()
            .and_then(|c| c.strip_prefix(CURVE_MARK))
        {
            let mut words = rest.split_whitespace();
            let (Some(name), Some(kind)) = (words.next(), words.next()) else {
                continue;
            };
            let (Some(kind), Some(tag)) = (CurveKind::from_key(kind), tag()) else {
                continue;
            };
            out.push(Curve {
                name: name.to_string(),
                kind,
                tag,
                lines: vec![i],
            });
        } else if let Some(c) = out.last_mut()
            && c.lines.last() == Some(&(i - 1))
        {
            // The rest of a spiral's pieces, and the GM card that places
            // the curve, follow on the next lines.
            let is =
                |m: &str| matches!(&line.kind, LineKind::Card { mnemonic, .. } if mnemonic == m);
            let last_is_gm = matches!(&deck.lines[i - 1].kind, LineKind::Card { mnemonic, .. } if mnemonic == "GM");
            if !last_is_gm && ((is("GW") && tag() == Some(c.tag)) || is("GM")) {
                c.lines.push(i);
            }
        }
    }
    out
}

/// Rotation by `deg` degrees about axis `k` (0 x, 1 y, 2 z).
fn rotate(p: [f64; 3], k: usize, deg: f64) -> [f64; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    let (i, j) = ((k + 1) % 3, (k + 2) % 3);
    let mut q = p;
    q[i] = p[i] * c - p[j] * s;
    q[j] = p[i] * s + p[j] * c;
    q
}

impl CurveSpec {
    /// Where a point in the curve's own plane (x, y, before turning) ends
    /// up, as NEC's GM card turns it (about y by the tilt, then about z by
    /// the azimuth) and moves it.
    pub fn place(&self, local: [f64; 3]) -> [f64; 3] {
        let p = rotate(rotate(local, 1, self.tilt), 2, self.azimuth);
        [0, 1, 2].map(|k| p[k] + self.centre[k])
    }

    /// `n + 1` points along the curve (for drawing a preview).
    pub fn preview(&self, n: usize) -> Vec<[f64; 3]> {
        let n = n.max(2);
        (0..=n)
            .map(|k| {
                let f = k as f64 / n as f64;
                let sign = if self.left { -1.0 } else { 1.0 };
                let local = match self.kind {
                    CurveKind::Circle | CurveKind::Arc => {
                        let (a, b) = if self.kind == CurveKind::Circle {
                            (0.0, 360.0)
                        } else {
                            (self.from, self.to)
                        };
                        let t = (a + (b - a) * f).to_radians();
                        [self.radius * t.cos(), self.radius * t.sin(), 0.0]
                    }
                    CurveKind::Helix => {
                        let r = self.radius + (self.radius2 - self.radius) * f;
                        let t = sign * (360.0 * self.turns * f).to_radians();
                        [r * t.cos(), r * t.sin(), self.length * f]
                    }
                    CurveKind::Spiral | CurveKind::LogSpiral => {
                        let r = if self.kind == CurveKind::Spiral {
                            self.radius + (self.radius2 - self.radius) * f
                        } else {
                            self.radius * (self.radius2 / self.radius).powf(f)
                        };
                        let t = sign * (360.0 * self.turns * f).to_radians();
                        [r * t.cos(), r * t.sin(), 0.0]
                    }
                };
                self.place(local)
            })
            .collect()
    }

    /// The angle (degrees) of world point `p` around the curve's axis,
    /// measured in its own plane.
    pub fn angle_of(&self, p: [f64; 3]) -> f64 {
        let d = [0, 1, 2].map(|k| p[k] - self.centre[k]);
        let l = rotate(rotate(d, 2, -self.azimuth), 1, -self.tilt);
        l[1].atan2(l[0]).to_degrees()
    }
}

/// The tilt and azimuth that make a curve face along axis `axis` (0 x,
/// 1 y, 2 z): its plane is the one where that coordinate is constant.
pub fn facing(axis: usize) -> (f64, f64) {
    match axis {
        0 => (90.0, 0.0),
        1 => (90.0, 90.0),
        _ => (0.0, 0.0),
    }
}

/// The other two axes of the plane facing along `axis`.
fn plane_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// The sweep (degrees, 0-360) of the arc from `s` to `e` that passes
/// through `p`, all on the plane facing along `axis`.
pub fn sweep_through(s: [f64; 3], e: [f64; 3], p: [f64; 3], axis: usize) -> f64 {
    let (i, j) = plane_axes(axis);
    let a = [s[i] - p[i], s[j] - p[j]];
    let b = [e[i] - p[i], e[j] - p[j]];
    let (la, lb) = (a[0].hypot(a[1]), b[0].hypot(b[1]));
    if la < 1e-12 || lb < 1e-12 {
        return 180.0;
    }
    // Inscribed angle: the angle at p is half the arc that p isn't on.
    let at_p = ((a[0] * b[0] + a[1] * b[1]) / (la * lb))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees();
    (360.0 - 2.0 * at_p).clamp(0.5, 359.5)
}

/// An arc from `s` to `e` on the plane facing along `axis` (it goes
/// through `s`'s coordinate on that axis), bulging towards `towards`,
/// sweeping `sweep` degrees. `None` if the ends are the same point.
pub fn arc_between(
    s: [f64; 3],
    e: [f64; 3],
    towards: [f64; 3],
    axis: usize,
    sweep: f64,
) -> Option<CurveSpec> {
    let (i, j) = plane_axes(axis);
    let c = [e[i] - s[i], e[j] - s[j]];
    let len = c[0].hypot(c[1]);
    if len < 1e-9 {
        return None;
    }
    // Which side of the chord the arc bulges to.
    let side = if c[0] * (towards[j] - s[j]) - c[1] * (towards[i] - s[i]) < 0.0 {
        -1.0
    } else {
        1.0
    };
    let n = [-c[1] / len, c[0] / len];
    let half = (sweep.clamp(0.5, 359.5) / 2.0).to_radians();
    let r = len / (2.0 * half.sin());
    let d = r * half.cos();
    let m = [(s[i] + e[i]) / 2.0, (s[j] + e[j]) / 2.0];
    let centre2 = [m[0] - side * n[0] * d, m[1] - side * n[1] * d];
    let mut centre = s;
    centre[i] = centre2[0];
    centre[j] = centre2[1];
    let mut mid = s;
    mid[i] = centre2[0] + side * n[0] * r;
    mid[j] = centre2[1] + side * n[1] * r;
    let mut spec = CurveSpec::new(CurveKind::Arc, "", centre, r);
    (spec.tilt, spec.azimuth) = facing(axis);
    let clean = |a: f64| (a * 1e9).round() / 1e9;
    let (a_s, a_e, a_m) = (
        clean(spec.angle_of(s)),
        clean(spec.angle_of(e)),
        spec.angle_of(mid),
    );
    let ccw = |a: f64, b: f64| (b - a).rem_euclid(360.0);
    // GA arcs run anticlockwise (in their own plane) from `from` to `to`:
    // start from whichever end makes that pass through the bulge.
    (spec.from, spec.to) = if ccw(a_s, a_m) < ccw(a_s, a_e) {
        (a_s, a_s + ccw(a_s, a_e))
    } else {
        (a_e, a_e + ccw(a_e, a_s))
    };
    Some(spec)
}

/// The axis a curve faces along (0-2), if it faces along one.
fn facing_of(tilt: f64, azimuth: f64) -> Option<usize> {
    (0..3).find(|&a| {
        let (t, z) = facing(a);
        (t - tilt).abs() < 1e-9 && (z - azimuth).abs() < 1e-9
    })
}

/// `(text)`, unless it's a plain number or a name.
fn paren(text: &str) -> String {
    if text.chars().skip(1).any(|c| "+-*/\\^<>= ".contains(c)) {
        format!("({text})")
    } else {
        text.to_string()
    }
}

impl Deck {
    /// Curve `c` as its variables stand (values from an expansion's
    /// `symbols`), for working out where its ends are.
    pub fn curve_spec(&self, c: &Curve, symbols: &[(String, f64)]) -> Option<CurveSpec> {
        let get = |s: &str| {
            let n = format!("{}_{s}", c.name);
            symbols
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&n))
                .map(|(_, v)| *v)
        };
        let mut spec = CurveSpec::new(
            c.kind,
            &c.name,
            [get("x")?, get("y")?, get("z")?],
            get("r")?,
        );
        spec.tilt = get("tilt")?;
        spec.azimuth = get("az")?;
        spec.radius2 = get("r2").unwrap_or(spec.radius);
        (spec.from, spec.to) = match c.kind {
            CurveKind::Arc => (get("from")?, get("to")?),
            _ => (0.0, 360.0),
        };
        spec.turns = get("turns").unwrap_or(1.0);
        spec.length = get("len").unwrap_or(0.0);
        spec.wire_radius = get("wire").unwrap_or(0.001);
        Some(spec)
    }

    /// Ties curves to the wire ends they touch when those ends are written
    /// as formulas, so the curves follow when the wires change: an arc on
    /// two wire ends keeps both (its centre and radius become formulas of
    /// them, its angles stay); any other curve touching a wire end moves
    /// with it. Arcs whose chord turns as the wires change aren't followed
    /// exactly. Returns the curves tied.
    pub fn link_curves(&mut self) -> Vec<String> {
        let Ok(numeric) = self.expand() else {
            return Vec::new();
        };
        let lift = super::ground::height_now(&numeric, self);
        let ws = super::edit::wires(self, &numeric);
        let mut tied = Vec::new();
        for c in curves(self) {
            let Some(spec) = self.curve_spec(&c, &numeric.symbols) else {
                continue;
            };
            let at = |deg: f64| {
                let a = deg.to_radians();
                spec.place([spec.radius * a.cos(), spec.radius * a.sin(), 0.0])
            };
            let ends: Vec<[f64; 3]> = match c.kind {
                CurveKind::Arc => vec![at(spec.from), at(spec.to)],
                CurveKind::Circle => Vec::new(),
                _ => vec![spec.place([spec.radius, 0.0, 0.0])],
            };
            let lifted = |p: [f64; 3]| [p[0], p[1], p[2] + lift];
            // Each end's coordinates as formulas of the wire it's on.
            let texts: Vec<Option<[String; 3]>> = ends
                .iter()
                .map(|&p| {
                    let q = lifted(p);
                    let tol = 1e-6 * q.iter().fold(1.0_f64, |a, x| a.max(x.abs()));
                    ws.iter().filter(|w| !w.moved).find_map(|w| {
                        let e = (0..2).find(|&e| {
                            let r = w.end(e);
                            (0..3).all(|k| (r[k] - q[k]).abs() <= tol)
                        })?;
                        Some([0, 1, 2].map(|k| {
                            let field = &w.fields[2 + e * 3 + k];
                            let shift = w.offset[k] - if k == 2 { lift } else { 0.0 };
                            if w.scale == 1.0 && shift.abs() < 1e-12 {
                                field.clone()
                            } else {
                                format!(
                                    "{}*{}+{}",
                                    paren(field),
                                    format_real(w.scale),
                                    paren(&format_real(shift))
                                )
                            }
                        }))
                    })
                })
                .collect();
            let formula = |t: &[String; 3]| t.iter().any(|f| !expr::names_in(f).is_empty());
            if !texts.iter().flatten().any(formula) {
                continue;
            }
            let v = |s: &str| format!("{}_{s}", c.name);
            let mut set: Vec<(String, String)> = Vec::new();
            let axis = facing_of(spec.tilt, spec.azimuth);
            match (c.kind, &texts[..], axis) {
                (CurveKind::Arc, [Some(s), Some(e)], Some(a)) => {
                    // centre = M + q (dj, -di), radius = c2 |d|, from the
                    // ends; q and c2 are what they are now.
                    let (i, j) = plane_axes(a);
                    let (p, q_end) = (ends[0], ends[1]);
                    let (di, dj) = (q_end[i] - p[i], q_end[j] - p[j]);
                    let l2 = di * di + dj * dj;
                    if l2 < 1e-24 {
                        continue;
                    }
                    let (mi, mj) = ((p[i] + q_end[i]) / 2.0, (p[j] + q_end[j]) / 2.0);
                    let q = ((spec.centre[i] - mi) * dj - (spec.centre[j] - mj) * di) / l2;
                    let c2 = spec.radius / l2.sqrt();
                    // Formulas as plain as they can be: a difference of equal
                    // texts is 0, numbers are worked out, zero terms dropped.
                    let num = |t: &str| super::edit::plain_number(t);
                    let d = |k: usize| -> String {
                        match (num(&e[k]), num(&s[k])) {
                            _ if e[k] == s[k] => "0".into(),
                            (Some(x), Some(y)) => format_real(x - y),
                            _ => format!("({}-{})", paren(&e[k]), paren(&s[k])),
                        }
                    };
                    let m = |k: usize| -> String {
                        match (num(&s[k]), num(&e[k])) {
                            _ if e[k] == s[k] => s[k].clone(),
                            (Some(x), Some(y)) => format_real((x + y) / 2.0),
                            _ => format!("({}+{})/2", paren(&s[k]), paren(&e[k])),
                        }
                    };
                    let plus = |base: String, coef: f64, diff: String| -> String {
                        if coef.abs() < 1e-12 || diff == "0" {
                            base
                        } else {
                            format!("{base}+{}*{}", paren(&format_real(coef)), paren(&diff))
                        }
                    };
                    let names = ["x", "y", "z"];
                    set.push((v(names[i]), plus(m(i), q, d(j))));
                    set.push((v(names[j]), plus(m(j), -q, d(i))));
                    set.push((v(names[a]), s[a].clone()));
                    let (di_t, dj_t) = (d(i), d(j));
                    let length = match (num(&di_t), num(&dj_t)) {
                        (Some(x), Some(y)) => format_real(x.hypot(y)),
                        (Some(0.0), None) => format!("abs({dj_t})"),
                        (None, Some(0.0)) => format!("abs({di_t})"),
                        _ => format!("sqr({}^2+{}^2)", paren(&di_t), paren(&dj_t)),
                    };
                    let radius = match num(&length) {
                        Some(l) => format_real(c2 * l),
                        None => format!("{}*{length}", format_real(c2)),
                    };
                    set.push((v("r"), radius));
                }
                _ => {
                    // Move with the first end that's on a wire.
                    let Some((k_end, t)) = texts
                        .iter()
                        .enumerate()
                        .find_map(|(k, t)| t.as_ref().map(|t| (k, t)))
                    else {
                        continue;
                    };
                    for (k, n) in ["x", "y", "z"].iter().enumerate() {
                        let off = spec.centre[k] - ends[k_end][k];
                        let text = if off.abs() < 1e-12 {
                            t[k].clone()
                        } else {
                            format!("{}+{}", paren(&t[k]), paren(&format_real(off)))
                        };
                        set.push((v(n), text));
                    }
                }
            }
            for (name, text) in &set {
                self.set_expression(name, text);
                // Below the variables its formula uses.
                if let Some(line) = self.lines.iter().position(|l| match &l.kind {
                    LineKind::Symbols(a) => a.len() == 1 && a[0].name.eq_ignore_ascii_case(name),
                    _ => false,
                }) {
                    let raw = self.lines[line].raw.clone();
                    self.remove_lines(&[line]);
                    let at = self.variables_line();
                    self.insert_lines(at, &[raw]);
                }
            }
            tied.push(c.name.clone());
        }
        tied
    }
}

/// The part of variable `name` after `prefix_`.
fn name_suffix<'a>(name: &'a str, prefix: &str) -> &'a str {
    name.get(prefix.len() + 1..).unwrap_or("")
}

/// Text for a number in a formula.
fn n(v: f64) -> String {
    format_real(v)
}

impl Deck {
    /// Adds a curve: its variables (grouped under its name), its cards and
    /// the GM card that places it, at [`Deck::geometry_end`] (so it's the
    /// last structure its GM moves, and the height card still lifts it:
    /// the centre is before the height shift). `tag` is a free tag.
    pub fn add_curve(&mut self, spec: &CurveSpec, tag: i64) -> Result<(), String> {
        let p = spec.name.trim();
        if p.is_empty() {
            return Err("the curve needs a name".into());
        }
        if spec.segments < 1 {
            return Err("it needs at least one segment".into());
        }
        if spec.radius <= 0.0 || spec.wire_radius <= 0.0 {
            return Err("the radius and the wire size must be more than zero".into());
        }
        let kind = spec.kind;
        if kind == CurveKind::LogSpiral && spec.radius2 <= 0.0 {
            return Err("a logarithmic spiral's outer radius must be more than zero".into());
        }
        // The variables: (suffix, value, unit, description).
        let mut vars: Vec<(&str, f64, &str, &str)> = vec![
            ("x", spec.centre[0], "m", "centre x"),
            ("y", spec.centre[1], "m", "centre y"),
            ("z", spec.centre[2], "m", "centre z"),
            ("tilt", spec.tilt, "deg", "axis tilt from upright"),
            ("az", spec.azimuth, "deg", "axis direction (azimuth)"),
        ];
        match kind {
            CurveKind::Circle => vars.push(("r", spec.radius, "m", "radius")),
            CurveKind::Arc => vars.extend([
                ("r", spec.radius, "m", "radius"),
                ("from", spec.from, "deg", "start angle"),
                ("to", spec.to, "deg", "end angle"),
            ]),
            CurveKind::Helix => vars.extend([
                ("r", spec.radius, "m", "radius at the start"),
                ("r2", spec.radius2, "m", "radius at the end"),
                ("len", spec.length, "m", "length along the axis"),
                ("turns", spec.turns, "", "turns"),
            ]),
            CurveKind::Spiral | CurveKind::LogSpiral => vars.extend([
                ("r", spec.radius, "m", "inner radius"),
                ("r2", spec.radius2, "m", "outer radius"),
                ("turns", spec.turns, "", "turns"),
            ]),
        }
        vars.push(("wire", spec.wire_radius, "m", "wire radius"));
        let names: Vec<String> = vars.iter().map(|v| format!("{p}_{}", v.0)).collect();
        for name in &names {
            self.check_new_name(name)?;
        }
        let v = |s: &str| format!("{p}_{s}");
        let mark = format!("{CURVE_MARK} {p} {}", kind.key());
        let segs = spec.segments;
        let mut cards: Vec<String> = Vec::new();
        let card =
            |m: &str, f: Vec<String>, comment: Option<&str>| super::edit::card_line(m, &f, comment);
        match kind {
            CurveKind::Circle | CurveKind::Arc => {
                let (a, b) = if kind == CurveKind::Circle {
                    ("0".to_string(), "360".to_string())
                } else {
                    (v("from"), v("to"))
                };
                cards.push(card(
                    "GA",
                    vec![tag.to_string(), segs.to_string(), v("r"), a, b, v("wire")],
                    Some(&mark),
                ));
            }
            CurveKind::Helix => {
                let hl = if spec.left {
                    format!("-{}", v("len"))
                } else {
                    v("len")
                };
                cards.push(card(
                    "GH",
                    vec![
                        tag.to_string(),
                        segs.to_string(),
                        format!("{}/{}", v("len"), v("turns")),
                        hl,
                        v("r"),
                        v("r"),
                        v("r2"),
                        v("r2"),
                        v("wire"),
                    ],
                    Some(&mark),
                ));
            }
            CurveKind::Spiral | CurveKind::LogSpiral => {
                let sign = if spec.left { "-" } else { "" };
                let point = |k: i64| -> [String; 3] {
                    let f = k as f64 / segs as f64;
                    let radius = if kind == CurveKind::Spiral {
                        format!("({}+({}-{})*{})", v("r"), v("r2"), v("r"), n(f))
                    } else {
                        format!("{}*({}/{})^{}", v("r"), v("r2"), v("r"), n(f))
                    };
                    let angle = format!("{sign}{}*{}", v("turns"), n(360.0 * f));
                    if k == 0 {
                        [v("r"), "0".into(), "0".into()]
                    } else {
                        [
                            format!("{radius}*cos({angle})"),
                            format!("{radius}*sin({angle})"),
                            "0".into(),
                        ]
                    }
                };
                for k in 0..segs {
                    let (a, b) = (point(k), point(k + 1));
                    let mut f = vec![tag.to_string(), "1".to_string()];
                    f.extend(a);
                    f.extend(b);
                    f.push(v("wire"));
                    cards.push(card("GW", f, (k == 0).then_some(mark.as_str())));
                }
            }
        }
        // GA lies in the x-z plane: turn it into the x-y plane first.
        let rox = if matches!(kind, CurveKind::Circle | CurveKind::Arc) {
            "-90"
        } else {
            "0"
        };
        cards.push(card(
            "GM",
            vec![
                "0".into(),
                "0".into(),
                rox.into(),
                v("tilt"),
                v("az"),
                v("x"),
                v("y"),
                v("z"),
                tag.to_string(),
            ],
            None,
        ));
        // Variables first (they go above the first card), then the cards.
        for (name, (_, value, unit, description)) in names.iter().zip(&vars) {
            let notes = VarNotes {
                description: format!("{p} {description}"),
                unit: unit.to_string(),
                group: p.to_string(),
                // Its turn, angles and wire size are left out of the
                // optimiser's list (tick them back on in the Variables view).
                fixed: matches!(name_suffix(name, p), "tilt" | "az" | "from" | "to" | "wire"),
                ..VarNotes::default()
            };
            self.add_variable(name, &n(*value), &notes)?;
        }
        let at = self.geometry_end();
        self.insert_lines(at, &cards);
        Ok(())
    }

    /// Removes a curve's cards and the variables only it used.
    pub fn remove_curve(&mut self, name: &str) {
        let Some(c) = curves(self).into_iter().find(|c| c.name == name) else {
            return;
        };
        self.remove_lines(&c.lines);
        let prefix = format!("{name}_").to_ascii_lowercase();
        let mine: Vec<String> = self
            .variables()
            .into_iter()
            .map(|v| v.name)
            .filter(|v| v.to_ascii_lowercase().starts_with(&prefix))
            .collect();
        for v in mine {
            if self.uses(&v).is_empty() {
                let _ = self.delete_variable(&v, None);
            }
        }
    }

    /// A free name for a new curve of this kind (`circle1`, `helix2`, ...).
    pub fn curve_name(&self, kind: CurveKind) -> String {
        let base = match kind {
            CurveKind::LogSpiral => "spiral",
            k => k.key(),
        };
        (1..)
            .map(|i| format!("{base}{i}"))
            .find(|name| {
                !self
                    .variables()
                    .iter()
                    .any(|v| v.name.to_ascii_lowercase().starts_with(&format!("{name}_")))
            })
            .unwrap_or_else(|| base.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::edit::wires;

    const DECK: &str = "CE\nGW 1 11 0 -5 10 0 5 10 .001\nGE 0\nEX 0 1 6 0 1\nFR 0 1 0 0 14\nEN\n";

    #[test]
    fn every_kind_expands_and_is_found_again() {
        for kind in CurveKind::ALL {
            let mut deck = Deck::parse(DECK);
            let mut spec = CurveSpec::new(kind, "c1", [1.0, 2.0, 3.0], 0.5);
            spec.tilt = 90.0;
            spec.azimuth = 30.0;
            deck.add_curve(&spec, 2).unwrap();
            let n = deck.expand().unwrap();
            assert!(
                n.cards.iter().any(|c| c.is("GM") && c.ints[0] == 0),
                "{kind:?}"
            );
            let found = curves(&deck);
            assert_eq!(found.len(), 1, "{kind:?}");
            assert_eq!((found[0].kind, found[0].tag), (kind, 2));
            let gm = *found[0].lines.last().unwrap();
            assert!(
                matches!(&deck.lines[gm].kind, LineKind::Card { mnemonic, .. } if mnemonic == "GM")
            );
            // The straight wire is still listed and not moved by the curve's GM.
            let w = wires(&deck, &n);
            assert!(!w[0].moved);
            deck.remove_curve("c1");
            assert!(curves(&deck).is_empty());
            assert_eq!(deck.variables().len(), 0, "{kind:?}");
            assert_eq!(deck.lines.len(), Deck::parse(DECK).lines.len());
        }
    }

    #[test]
    fn arcs_follow_resized_wires() {
        use crate::deck::vars::Anchor;
        // A folded dipole standing up: rods along y, 5 cm apart in z, with
        // a semicircle at each end.
        let mut deck = Deck::parse(
            "CE\nGW 1 15 0.3 -0.35 0 0.3 0.35 0 .005\nGW 2 15 0.3 -0.35 0.05 0.3 0.35 0.05 .005\nGE 0\nEX 0 1 8 0 1\nFR 0 1 0 0 200\nEN\n",
        );
        for (tag, y, out) in [(3, 0.35, 0.4), (4, -0.35, -0.4)] {
            let mut spec =
                arc_between([0.3, y, 0.0], [0.3, y, 0.05], [0.3, out, 0.025], 0, 180.0).unwrap();
            spec.name = format!("end{tag}");
            spec.segments = 3;
            spec.wire_radius = 0.005;
            deck.add_curve(&spec, tag).unwrap();
        }
        let n = deck.expand().unwrap();
        let ws = wires(&deck, &n);
        deck.drive_length(
            &ws[..2],
            &ws,
            Anchor::Centre,
            "de_len",
            &VarNotes::default(),
        )
        .unwrap();
        // As plain as they can be.
        let expr_of = |n: &str| {
            deck.variables()
                .into_iter()
                .find(|v| v.name == n)
                .map(|v| v.expr)
                .unwrap()
        };
        assert_eq!(expr_of("end3_y"), "de_len/2");
        assert_eq!(expr_of("end3_z"), "0.025");
        assert_eq!(expr_of("end4_y"), "-de_len/2");
        for len in [0.7, 0.9, 0.5] {
            let n = deck.expand_with(&[("de_len".into(), len)]).unwrap();
            let ws = wires(&deck, &n);
            for c in curves(&deck) {
                let spec = deck.curve_spec(&c, &n.symbols).unwrap();
                for deg in [spec.from, spec.to] {
                    let a: f64 = deg.to_radians();
                    let p = spec.place([spec.radius * a.cos(), spec.radius * a.sin(), 0.0]);
                    let on_a_rod = ws
                        .iter()
                        .any(|w| (0..2).any(|e| (0..3).all(|k| (w.end(e)[k] - p[k]).abs() < 1e-9)));
                    assert!(on_a_rod, "{} at {len}: {p:?}", c.name);
                }
                assert!((spec.radius - 0.025).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn arcs_between_two_points() {
        // Every plane, both sides, several sweeps: the arc's ends land on
        // the points, and its middle bulges towards the mouse.
        for axis in 0..3 {
            let (i, j) = plane_axes(axis);
            let s = [0.3, -0.2, 0.5];
            let mut e = s;
            e[i] += 0.4;
            e[j] += 0.1;
            for side in [1.0, -1.0] {
                let mut towards = [
                    (s[0] + e[0]) / 2.0,
                    (s[1] + e[1]) / 2.0,
                    (s[2] + e[2]) / 2.0,
                ];
                // Off the chord, perpendicular to it.
                towards[i] += -0.1 * 0.3 * side;
                towards[j] += 0.4 * 0.3 * side;
                for sweep in [30.0, 90.0, 180.0, 270.0] {
                    let spec = arc_between(s, e, towards, axis, sweep).unwrap();
                    assert!(
                        (spec.to - spec.from - sweep).abs() < 1e-6,
                        "{axis} {side} {sweep}"
                    );
                    let at = |deg: f64| {
                        let a = deg.to_radians();
                        spec.place([spec.radius * a.cos(), spec.radius * a.sin(), 0.0])
                    };
                    let ends = [at(spec.from), at(spec.to)];
                    let near =
                        |p: [f64; 3], q: [f64; 3]| (0..3).all(|k| (p[k] - q[k]).abs() < 1e-9);
                    assert!(
                        (near(ends[0], s) && near(ends[1], e))
                            || (near(ends[0], e) && near(ends[1], s)),
                        "{axis} {side} {sweep}: {ends:?}"
                    );
                    let mid = at((spec.from + spec.to) / 2.0);
                    let dot = (0..3)
                        .map(|k| {
                            (mid[k] - (s[k] + e[k]) / 2.0) * (towards[k] - (s[k] + e[k]) / 2.0)
                        })
                        .sum::<f64>();
                    assert!(dot > 0.0, "bulges the right way: {axis} {side} {sweep}");
                    assert!((mid[axis] - s[axis]).abs() < 1e-9, "stays in the plane");
                }
            }
        }
        // The sweep through a point on the perpendicular bisector at half
        // the chord is a semicircle.
        let sweep = sweep_through([0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 0);
        assert!((sweep - 180.0).abs() < 1e-9);
    }

    #[test]
    fn placement_matches_angles() {
        let mut spec = CurveSpec::new(CurveKind::Arc, "a", [1.0, 0.0, 0.0], 2.0);
        spec.tilt = 90.0;
        spec.azimuth = 90.0;
        // Local (x, y) at 30 degrees, placed, then measured back.
        let local = [
            2.0 * 30f64.to_radians().cos(),
            2.0 * 30f64.to_radians().sin(),
            0.0,
        ];
        let p = spec.place(local);
        assert!((spec.angle_of(p) - 30.0).abs() < 1e-9);
        // The axis now points along +y.
        let axis = spec.place([0.0, 0.0, 1.0]);
        assert!((axis[1] - 1.0).abs() < 1e-9 && (axis[0] - 1.0).abs() < 1e-9);
    }
}
