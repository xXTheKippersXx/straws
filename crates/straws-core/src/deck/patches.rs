//! Closed boxes of surface patches (a vehicle body, an equipment box, a
//! mast-head enclosure).
//!
//! - NEC-2's patches model closed metal surfaces only (the magnetic-field
//!   integral equation), so the generator makes a whole box: six SM cards,
//!   each with its SC card, ordered so every patch's normal points out.
//! - Box `NAME`: variables `NAME_x/y/z` (the middle of its bottom face),
//!   `NAME_w` (length along `NAME_az`), `NAME_d` (depth across), `NAME_h`
//!   (height) and `NAME_az` (degrees anticlockwise from +x). Its cards carry
//!   the comment `straws: patch box NAME NUxNVxNZ` (patches along, across
//!   and up).
//! - A wire joins a patch surface only at a patch's centre, at right angles
//!   (NEC then splits that patch into four). Odd counts put a patch centre
//!   in the middle of every face.

use super::vars::VarNotes;
use super::write::format_real;
use super::{Deck, LineKind};

/// Start of the comment on a patch box's cards.
pub const BOX_MARK: &str = "straws: patch box";

/// A new box (metres and degrees).
#[derive(Debug, Clone, PartialEq)]
pub struct BoxSpec {
    pub name: String,
    /// The middle of the bottom face, before the Ground view's height shift.
    pub base: [f64; 3],
    /// Length along `facing`, depth across and height.
    pub size: [f64; 3],
    pub facing: f64,
    /// Patches along, across and up.
    pub cells: [usize; 3],
}

/// A box found in a deck.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxSet {
    pub name: String,
    pub cells: [usize; 3],
    pub lines: Vec<usize>,
}

/// The patch boxes in a deck (made by [`Deck::add_patch_box`]).
pub fn box_sets(deck: &Deck) -> Vec<BoxSet> {
    let mut out: Vec<BoxSet> = Vec::new();
    for (i, line) in deck.lines.iter().enumerate() {
        if !matches!(&line.kind, LineKind::Card { mnemonic, .. } if mnemonic == "SM" || mnemonic == "SC")
        {
            continue;
        }
        let Some(rest) = line
            .comment
            .as_deref()
            .and_then(|c| c.strip_prefix(BOX_MARK))
        else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let (Some(name), Some(size)) = (words.next(), words.next()) else {
            continue;
        };
        let cells: Vec<usize> = size.split('x').filter_map(|n| n.parse().ok()).collect();
        let Ok(cells) = <[usize; 3]>::try_from(cells) else {
            continue;
        };
        match out.iter_mut().find(|b| b.name == name) {
            Some(b) => b.lines.push(i),
            None => out.push(BoxSet {
                name: name.to_string(),
                cells,
                lines: vec![i],
            }),
        }
    }
    out
}

/// What a box variable is, in a few words.
pub fn box_label(suffix: &str) -> &'static str {
    match suffix {
        "x" => "bottom centre x",
        "y" => "bottom centre y",
        "z" => "bottom height",
        "w" => "length",
        "d" => "depth",
        "h" => "height",
        "az" => "length's direction",
        _ => "",
    }
}

/// Patches in a box of `cells`.
pub fn patch_count(cells: [usize; 3]) -> usize {
    let [u, v, z] = cells;
    2 * (u * v + v * z + u * z)
}

/// Corner (a, b, c) of box `p` as deck formulas: a, b = ±1 for the ends of
/// its length and depth, c = 0 (bottom) or 1 (top).
fn corner(p: &str, a: f64, b: f64, c: f64) -> [String; 3] {
    let v = |s: &str| format!("{p}_{s}");
    let half = |sign: f64, len: &str, trig: &str| {
        let op = if sign < 0.0 { "-" } else { "+" };
        format!("{op}{}/2*{trig}({})", v(len), v("az"))
    };
    [
        format!("{}{}{}", v("x"), half(a, "w", "cos"), half(-b, "d", "sin")),
        format!("{}{}{}", v("y"), half(a, "w", "sin"), half(b, "d", "cos")),
        if c > 0.0 {
            format!("{}+{}", v("z"), v("h"))
        } else {
            v("z")
        },
    ]
}

/// The box's cards: per face an SM (corners 1 and 2, patch counts along
/// 1-2 and 2-3) and its SC (corner 3), with (2 - 1) x (3 - 2) pointing out.
fn box_cards(p: &str, cells: [usize; 3]) -> Vec<String> {
    let mark = format!("{BOX_MARK} {p} {}x{}x{}", cells[0], cells[1], cells[2]);
    let [nu, nv, nz] = cells;
    // (corner 1, corner 2, corner 3, patches 1-2, patches 2-3)
    type C = (f64, f64, f64);
    let faces: [(C, C, C, usize, usize); 6] = [
        // top, bottom
        ((-1., -1., 1.), (1., -1., 1.), (1., 1., 1.), nu, nv),
        ((-1., -1., 0.), (-1., 1., 0.), (1., 1., 0.), nv, nu),
        // front (+length), back
        ((1., -1., 0.), (1., 1., 0.), (1., 1., 1.), nv, nz),
        ((-1., -1., 0.), (-1., -1., 1.), (-1., 1., 1.), nz, nv),
        // left (+depth), right
        ((-1., 1., 0.), (-1., 1., 1.), (1., 1., 1.), nz, nu),
        ((-1., -1., 0.), (1., -1., 0.), (1., -1., 1.), nu, nz),
    ];
    let mut out = Vec::new();
    for (c1, c2, c3, n12, n23) in faces {
        let mut f = vec![n12.to_string(), n23.to_string()];
        f.extend(corner(p, c1.0, c1.1, c1.2));
        f.extend(corner(p, c2.0, c2.1, c2.2));
        out.push(super::edit::card_line("SM", &f, Some(&mark)));
        let mut f = vec!["0".to_string(), "0".to_string()];
        f.extend(corner(p, c3.0, c3.1, c3.2));
        out.push(super::edit::card_line("SC", &f, Some(&mark)));
    }
    out
}

fn check_cells(cells: [usize; 3]) -> Result<(), String> {
    if cells.iter().any(|&n| !(1..=30).contains(&n)) {
        return Err("a patch box has 1 to 30 patches each way".into());
    }
    Ok(())
}

impl Deck {
    /// Adds a patch box: its variables (grouped under its name) and cards,
    /// at [`Deck::geometry_end`].
    pub fn add_patch_box(&mut self, spec: &BoxSpec) -> Result<(), String> {
        let p = spec.name.trim();
        if p.is_empty() {
            return Err("the box needs a name".into());
        }
        check_cells(spec.cells)?;
        if spec.size.iter().any(|&s| s <= 0.0) {
            return Err("the box's sizes must be more than zero".into());
        }
        let c = spec.base.map(|v| format_real((v * 1e7).round() / 1e7));
        let n = |v: f64| format_real(v);
        let vars: [(&str, String, &str); 7] = [
            ("x", c[0].clone(), "m"),
            ("y", c[1].clone(), "m"),
            ("z", c[2].clone(), "m"),
            ("w", n(spec.size[0]), "m"),
            ("d", n(spec.size[1]), "m"),
            ("h", n(spec.size[2]), "m"),
            ("az", n(spec.facing), "deg"),
        ];
        let names: Vec<String> = vars.iter().map(|v| format!("{p}_{}", v.0)).collect();
        for name in &names {
            self.check_new_name(name)?;
        }
        for (name, (suffix, value, unit)) in names.iter().zip(&vars) {
            let notes = VarNotes {
                description: format!("{p} {}", box_label(suffix)),
                unit: unit.to_string(),
                group: p.to_string(),
                fixed: matches!(*suffix, "x" | "y" | "z" | "az"),
                ..VarNotes::default()
            };
            self.add_variable(name, value, &notes)?;
        }
        let at = self.geometry_end();
        self.insert_lines(at, &box_cards(p, spec.cells));
        Ok(())
    }

    /// Gives box `name` different patch counts, keeping its variables.
    pub fn set_box_cells(&mut self, name: &str, cells: [usize; 3]) -> Result<(), String> {
        let Some(b) = box_sets(self).into_iter().find(|b| b.name == name) else {
            return Err(format!("there's no patch box called {name}"));
        };
        check_cells(cells)?;
        let at = b.lines[0];
        self.remove_lines(&b.lines);
        self.insert_lines(at, &box_cards(name, cells));
        Ok(())
    }

    /// Removes a box's cards and the variables only it used.
    pub fn remove_patch_box(&mut self, name: &str) {
        let Some(b) = box_sets(self).into_iter().find(|b| b.name == name) else {
            return;
        };
        self.remove_lines(&b.lines);
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

    /// A free name for a new box (`box1`, `box2`, ...).
    pub fn patch_box_name(&self) -> String {
        let taken: Vec<String> = self
            .variables()
            .into_iter()
            .map(|v| v.name.to_ascii_lowercase())
            .collect();
        (1..)
            .map(|i| format!("box{i}"))
            .find(|name| {
                let prefix = format!("{name}_");
                !taken.iter().any(|v| v.starts_with(&prefix))
                    && !box_sets(self).iter().any(|b| &b.name == name)
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_is_found_resized_and_removed() {
        let mut d = Deck::parse("GW 1 11 0 0 1 0 0 2 .001\nGE 0\nEN\n");
        let spec = BoxSpec {
            name: "box1".into(),
            base: [0.0, 0.0, 0.0],
            size: [2.0, 1.0, 0.5],
            facing: 30.0,
            cells: [5, 3, 1],
        };
        d.add_patch_box(&spec).unwrap();
        let b = &box_sets(&d)[0];
        assert_eq!((b.cells, b.lines.len()), ([5, 3, 1], 12));
        assert_eq!(patch_count(b.cells), 2 * (15 + 3 + 5));
        // SM cards before GE, after the wire; every corner on the box.
        let n = d.expand().unwrap();
        let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
        for card in n.cards.iter().filter(|c| c.is("SM") || c.is("SC")) {
            let k = if card.is("SM") { 6 } else { 3 };
            for p in card.floats[..k].chunks(3) {
                let (a, b) = (p[0] * c + p[1] * s, -p[0] * s + p[1] * c);
                assert!((a.abs() - 1.0).abs() < 1e-9, "{p:?}");
                assert!((b.abs() - 0.5).abs() < 1e-9, "{p:?}");
                assert!(p[2].abs() < 1e-9 || (p[2] - 0.5).abs() < 1e-9);
            }
        }
        d.set_box_cells("box1", [3, 3, 3]).unwrap();
        assert_eq!(box_sets(&d)[0].cells, [3, 3, 3]);
        d.remove_patch_box("box1");
        assert!(box_sets(&d).is_empty() && d.variables().is_empty());
    }
}
