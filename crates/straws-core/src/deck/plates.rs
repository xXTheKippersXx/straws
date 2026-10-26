//! Reflector plates as wire meshes: a flat plate, and a corner reflector
//! (two plates hinged along a vertical line).
//!
//! - NEC models a solid sheet well as a mesh of wires spaced well under a
//!   tenth of a wavelength. Every side of every cell is its own GW card ending
//!   on the mesh's nodes, so NEC joins the wires at every crossing; each mesh
//!   line is one tag.
//! - Flat plate `NAME`: variables `NAME_x/y/z` (centre), `NAME_w` (width),
//!   `NAME_h` (height), `NAME_az` (the direction its front faces, degrees
//!   anticlockwise from +x), `NAME_tilt` (leaning back from upright) and
//!   `NAME_wire`. Comment `straws: plate NAME NXxNY`.
//! - Corner reflector `NAME`: `NAME_x/y/z` (middle of the apex line),
//!   `NAME_side` (each side's width from the apex), `NAME_h`, `NAME_angle`
//!   (between the sides), `NAME_az` (the direction it opens towards) and
//!   `NAME_wire`. Comment `straws: corner NAME NXxNY` (NX cells across both
//!   sides, even: half on each).

use super::vars::VarNotes;
use super::write::format_real;
use super::{Deck, LineKind};

/// Start of the comment on a reflector's cards.
pub const PLATE_MARK: &str = "straws: plate";
pub const CORNER_MARK: &str = "straws: corner";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlateKind {
    Flat,
    Corner,
}

impl PlateKind {
    fn mark(self) -> &'static str {
        match self {
            PlateKind::Flat => PLATE_MARK,
            PlateKind::Corner => CORNER_MARK,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PlateKind::Flat => "flat plate",
            PlateKind::Corner => "corner reflector",
        }
    }
}

/// A new reflector (metres and degrees).
#[derive(Debug, Clone, PartialEq)]
pub struct PlateSpec {
    pub name: String,
    pub kind: PlateKind,
    /// The centre (flat) or the middle of the apex line (corner), before the
    /// Ground view's height shift.
    pub centre: [f64; 3],
    /// Flat: the whole width. Corner: each side's width from the apex.
    pub width: f64,
    pub height: f64,
    /// Flat: the direction its front faces. Corner: the direction it opens.
    pub facing: f64,
    /// Flat: leaning back from upright. Corner: the angle between the sides.
    pub angle: f64,
    /// Cells across (corner: across both sides, even) and up.
    pub cells: [usize; 2],
    /// Segments on each side of a cell.
    pub segments: i64,
    pub wire_radius: f64,
}

/// A reflector found in a deck.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateSet {
    pub name: String,
    pub kind: PlateKind,
    pub cells: [usize; 2],
    pub tags: Vec<i64>,
    pub lines: Vec<usize>,
}

/// The reflectors in a deck (made by [`Deck::add_plate`]).
pub fn plate_sets(deck: &Deck) -> Vec<PlateSet> {
    let mut out: Vec<PlateSet> = Vec::new();
    for (i, line) in deck.lines.iter().enumerate() {
        let LineKind::Card { mnemonic, fields } = &line.kind else {
            continue;
        };
        let comment = line.comment.as_deref().unwrap_or("");
        let (kind, rest) = match (
            comment.strip_prefix(PLATE_MARK),
            comment.strip_prefix(CORNER_MARK),
        ) {
            (Some(r), _) => (PlateKind::Flat, r),
            (_, Some(r)) => (PlateKind::Corner, r),
            _ => continue,
        };
        let mut words = rest.split_whitespace();
        let (Some(name), Some(size)) = (words.next(), words.next()) else {
            continue;
        };
        let Some((Ok(nx), Ok(ny))) = size
            .split_once('x')
            .map(|(a, b)| (a.parse::<usize>(), b.parse::<usize>()))
        else {
            continue;
        };
        if mnemonic != "GW" {
            continue;
        }
        let tag = fields
            .first()
            .and_then(|t| t.parse::<f64>().ok())
            .map_or(0, |t| t as i64);
        let k = match out.iter().position(|p| p.name == name) {
            Some(k) => k,
            None => {
                out.push(PlateSet {
                    name: name.to_string(),
                    kind,
                    cells: [nx, ny],
                    tags: Vec::new(),
                    lines: Vec::new(),
                });
                out.len() - 1
            }
        };
        let p = &mut out[k];
        p.lines.push(i);
        if !p.tags.contains(&tag) {
            p.tags.push(tag);
        }
    }
    out
}

/// What a reflector variable is, in a few words.
pub fn plate_label(suffix: &str) -> &'static str {
    match suffix {
        "x" => "centre x",
        "y" => "centre y",
        "z" => "centre height",
        "w" => "width",
        "h" => "height",
        "side" => "each side's width",
        "az" => "faces (azimuth)",
        "tilt" => "leans back",
        "angle" => "angle between sides",
        "wire" => "wire radius",
        _ => "",
    }
}

/// `+f*term`, `-f*term`, or nothing for 0.
fn times(f: f64, term: &str) -> String {
    if f.abs() < 1e-12 {
        String::new()
    } else if f < 0.0 {
        format!("-{}*{term}", format_real(-f))
    } else {
        format!("+{}*{term}", format_real(f))
    }
}

/// Node (i, j) of a reflector's mesh as deck formulas.
fn node(p: &str, kind: PlateKind, cells: [usize; 2], i: usize, j: usize) -> [String; 3] {
    let v = |s: &str| format!("{p}_{s}");
    let b = j as f64 / cells[1] as f64 - 0.5;
    match kind {
        PlateKind::Flat => {
            let a = i as f64 / cells[0] as f64 - 0.5;
            let (az, tilt) = (v("az"), v("tilt"));
            [
                format!(
                    "{}{}{}",
                    v("x"),
                    times(-a, &format!("{}*sin({az})", v("w"))),
                    times(-b, &format!("{}*sin({tilt})*cos({az})", v("h")))
                ),
                format!(
                    "{}{}{}",
                    v("y"),
                    times(a, &format!("{}*cos({az})", v("w"))),
                    times(-b, &format!("{}*sin({tilt})*sin({az})", v("h")))
                ),
                format!("{}{}", v("z"), times(b, &format!("{}*cos({tilt})", v("h")))),
            ]
        }
        PlateKind::Corner => {
            // Across both sides: i = half is the apex line.
            let half = cells[0] / 2;
            let f = (i as f64 - half as f64).abs() / half as f64;
            let sign = if i < half { "-" } else { "+" };
            let dir = format!("{}{sign}{}/2", v("az"), v("angle"));
            [
                format!(
                    "{}{}",
                    v("x"),
                    times(f, &format!("{}*cos({dir})", v("side")))
                ),
                format!(
                    "{}{}",
                    v("y"),
                    times(f, &format!("{}*sin({dir})", v("side")))
                ),
                format!("{}{}", v("z"), times(b, &v("h"))),
            ]
        }
    }
}

/// The reflector's cards: rows then columns of the mesh, one tag each.
fn plate_cards(
    p: &str,
    kind: PlateKind,
    cells: [usize; 2],
    first_tag: i64,
    segments: i64,
) -> Vec<String> {
    let mark = format!("{} {p} {}x{}", kind.mark(), cells[0], cells[1]);
    let wire = format!("{p}_wire");
    let card = |tag: i64, a: [String; 3], b: [String; 3]| {
        let mut f = vec![tag.to_string(), segments.to_string()];
        f.extend(a);
        f.extend(b);
        f.push(wire.clone());
        super::edit::card_line("GW", &f, Some(&mark))
    };
    let [nx, ny] = cells;
    let at = |i, j| node(p, kind, cells, i, j);
    let mut out = Vec::new();
    let mut tag = first_tag;
    for j in 0..=ny {
        for i in 0..nx {
            out.push(card(tag, at(i, j), at(i + 1, j)));
        }
        tag += 1;
    }
    for i in 0..=nx {
        for j in 0..ny {
            out.push(card(tag, at(i, j), at(i, j + 1)));
        }
        tag += 1;
    }
    out
}

/// Segments in a reflector of `cells` with `segments` per side.
pub fn segment_count(cells: [usize; 2], segments: i64) -> i64 {
    let [nx, ny] = cells.map(|c| c as i64);
    (nx * (ny + 1) + ny * (nx + 1)) * segments.max(1)
}

fn check_cells(kind: PlateKind, cells: [usize; 2]) -> Result<(), String> {
    if cells.iter().any(|&n| !(1..=60).contains(&n)) {
        return Err("a reflector has 1 to 60 cells each way".into());
    }
    if kind == PlateKind::Corner && (cells[0] < 2 || !cells[0].is_multiple_of(2)) {
        return Err(
            "a corner reflector has an even number of cells across (half on each side)".into(),
        );
    }
    Ok(())
}

impl Deck {
    /// Adds a reflector: its variables (grouped under its name) and cards,
    /// at [`Deck::geometry_end`], with tags clear of any in use.
    pub fn add_plate(&mut self, spec: &PlateSpec, first_tag: i64) -> Result<(), String> {
        let p = spec.name.trim();
        if p.is_empty() {
            return Err("the reflector needs a name".into());
        }
        check_cells(spec.kind, spec.cells)?;
        if spec.width <= 0.0 || spec.height <= 0.0 || spec.wire_radius <= 0.0 {
            return Err("the size and the wire must be more than zero".into());
        }
        let c = spec.centre.map(|v| format_real((v * 1e7).round() / 1e7));
        let n = |v: f64| format_real(v);
        let vars: Vec<(&str, String, &str, &str)> = match spec.kind {
            PlateKind::Flat => vec![
                ("x", c[0].clone(), "m", "centre x"),
                ("y", c[1].clone(), "m", "centre y"),
                ("z", c[2].clone(), "m", "centre height"),
                ("w", n(spec.width), "m", "width"),
                ("h", n(spec.height), "m", "height"),
                ("az", n(spec.facing), "deg", "direction it faces"),
                ("tilt", n(spec.angle), "deg", "leaning back from upright"),
                ("wire", n(spec.wire_radius), "m", "wire radius"),
            ],
            PlateKind::Corner => vec![
                ("x", c[0].clone(), "m", "apex x"),
                ("y", c[1].clone(), "m", "apex y"),
                ("z", c[2].clone(), "m", "apex middle height"),
                ("side", n(spec.width), "m", "each side's width"),
                ("h", n(spec.height), "m", "height"),
                ("angle", n(spec.angle), "deg", "angle between the sides"),
                ("az", n(spec.facing), "deg", "direction it opens"),
                ("wire", n(spec.wire_radius), "m", "wire radius"),
            ],
        };
        let names: Vec<String> = vars.iter().map(|v| format!("{p}_{}", v.0)).collect();
        for name in &names {
            self.check_new_name(name)?;
        }
        let first_tag = first_tag.max(self.highest_tag() + 1);
        let cards = plate_cards(p, spec.kind, spec.cells, first_tag, spec.segments.max(1));
        for (name, (suffix, value, unit, description)) in names.iter().zip(&vars) {
            let notes = VarNotes {
                description: format!("{p} {description}"),
                unit: unit.to_string(),
                group: p.to_string(),
                fixed: matches!(*suffix, "x" | "y" | "z" | "az" | "wire"),
                ..VarNotes::default()
            };
            self.add_variable(name, value, &notes)?;
        }
        let at = self.geometry_end();
        self.insert_lines(at, &cards);
        Ok(())
    }

    /// Gives reflector `name` a different number of cells, keeping its
    /// variables, segments and first tag.
    pub fn set_plate_cells(&mut self, name: &str, cells: [usize; 2]) -> Result<(), String> {
        let Some(p) = plate_sets(self).into_iter().find(|p| p.name == name) else {
            return Err(format!("there's no reflector called {name}"));
        };
        check_cells(p.kind, cells)?;
        let segments = match &self.lines[p.lines[0]].kind {
            LineKind::Card { fields, .. } => fields
                .get(1)
                .and_then(|f| f.parse::<f64>().ok())
                .map_or(1, |s| s as i64),
            _ => 1,
        };
        let at = p.lines[0];
        self.remove_lines(&p.lines);
        let first = p.tags.iter().copied().min().unwrap_or(1);
        let needed = cells[0] + cells[1] + 2;
        let first = if needed <= p.tags.len() {
            first
        } else {
            first.max(self.highest_tag() + 1)
        };
        self.insert_lines(at, &plate_cards(name, p.kind, cells, first, segments));
        Ok(())
    }

    /// Removes a reflector's cards and the variables only it used.
    pub fn remove_plate(&mut self, name: &str) {
        let Some(p) = plate_sets(self).into_iter().find(|p| p.name == name) else {
            return;
        };
        self.remove_lines(&p.lines);
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

    /// A free name for a new reflector (`plate1`, `corner1`, ...).
    pub fn plate_name(&self, kind: PlateKind) -> String {
        let base = match kind {
            PlateKind::Flat => "plate",
            PlateKind::Corner => "corner",
        };
        let taken: Vec<String> = self
            .variables()
            .into_iter()
            .map(|v| v.name.to_ascii_lowercase())
            .collect();
        (1..)
            .map(|i| format!("{base}{i}"))
            .find(|name| {
                let prefix = format!("{name}_");
                !taken.iter().any(|v| v.starts_with(&prefix))
                    && !plate_sets(self).iter().any(|p| &p.name == name)
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: PlateKind) -> PlateSpec {
        PlateSpec {
            name: "r1".into(),
            kind,
            centre: [-1.0, 0.0, 5.0],
            width: 2.0,
            height: 1.0,
            facing: 0.0,
            angle: if kind == PlateKind::Corner { 90.0 } else { 0.0 },
            cells: [4, 2],
            segments: 2,
            wire_radius: 0.001,
        }
    }

    fn ends(d: &Deck) -> Vec<[f64; 6]> {
        d.expand()
            .unwrap()
            .cards
            .iter()
            .filter(|c| c.is("GW") && c.ints[0] >= 2)
            .map(|c| [0, 1, 2, 3, 4, 5].map(|k| c.floats[k]))
            .collect()
    }

    #[test]
    fn a_flat_plate_faces_its_way() {
        let mut d = Deck::parse("GW 1 11 0 0 4.5 0 0 5.5 .001\nGE 0\nEN\n");
        d.add_plate(&spec(PlateKind::Flat), 2).unwrap();
        let p = &plate_sets(&d)[0];
        // 3 rows of 4 sides and 5 columns of 2: 22 sides, 8 tags.
        assert_eq!((p.lines.len(), p.tags.len()), (22, 8));
        assert_eq!(segment_count(p.cells, 2), 44);
        // Facing +x: the plate lies in the plane x = -1, 2 m wide in y, 1 m high.
        for e in ends(&d) {
            assert!(
                (e[0] + 1.0).abs() < 1e-9 && (e[3] + 1.0).abs() < 1e-9,
                "{e:?}"
            );
            assert!(e[1].abs() <= 1.0 + 1e-9 && (e[2] - 5.0).abs() <= 0.5 + 1e-9);
        }
        d.set_plate_cells("r1", [6, 4]).unwrap();
        assert_eq!(plate_sets(&d)[0].lines.len(), 6 * 5 + 4 * 7);
        d.remove_plate("r1");
        assert!(plate_sets(&d).is_empty() && d.variables().is_empty());
    }

    #[test]
    fn a_corner_reflector_opens_its_way() {
        let mut d = Deck::parse("GW 1 11 0 0 4.5 0 0 5.5 .001\nGE 0\nEN\n");
        d.add_plate(&spec(PlateKind::Corner), 2).unwrap();
        // Every node is on one of the two sides, at ±45° from +x, from the
        // apex at (-1, 0); the apex line nodes are shared.
        for e in ends(&d) {
            for p in [[e[0], e[1]], [e[3], e[4]]] {
                let (dx, dy) = (p[0] + 1.0, p[1]);
                assert!(dx >= -1e-9 && (dx - dy.abs()).abs() < 1e-9, "{p:?}");
                assert!(dx.hypot(dy) <= 2.0 + 1e-9);
            }
        }
        assert!(d.set_plate_cells("r1", [3, 2]).is_err());
    }
}
