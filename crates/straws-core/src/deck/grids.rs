//! Grid counterpoises: a flat rectangular wire mesh round a centre point
//! (under a vertical, say), raised, on the ground or buried.
//!
//! - Every mesh side is its own GW card ending on the mesh's nodes, so NEC
//!   joins the wires at every crossing (wires crossing mid-segment aren't
//!   joined). Each line of the mesh is one tag (NEC numbers segments along a
//!   tag across cards).
//! - The sizes are variables: `NAME_x/y/z` (the centre), `NAME_w` and
//!   `NAME_l` (width along x and length along y before turning), `NAME_turn`
//!   (degrees about z) and `NAME_wire` (wire radius). The cell counts are in
//!   the cards' comment, `straws: grid NAME NXxNY` (even, so there's a node
//!   in the middle).
//! - Like radials (see `radials`), a grid on or under the ground is kept as
//!   commented-out cards and estimated (`analysis::buried`).

use super::expand::NumericDeck;
use super::radials::{BURIED_MARK, Place, buried_card, place_for};
use super::vars::VarNotes;
use super::write::format_real;
use super::{Deck, LineKind};

/// Start of the comment on a grid's cards.
pub const GRID_MARK: &str = "straws: grid";

/// What a new grid should look like (metres and degrees).
#[derive(Debug, Clone, PartialEq)]
pub struct GridSpec {
    pub name: String,
    /// The middle of the mesh, before the Ground view's height shift.
    pub centre: [f64; 3],
    pub width: f64,
    pub length: f64,
    pub turn: f64,
    /// Cells across (along x) and along (along y): even numbers.
    pub cells: [usize; 2],
    /// Segments on each side of a cell.
    pub segments: i64,
    pub wire_radius: f64,
}

/// A grid found in a deck.
#[derive(Debug, Clone, PartialEq)]
pub struct GridSet {
    pub name: String,
    pub cells: [usize; 2],
    pub tags: Vec<i64>,
    pub lines: Vec<usize>,
    /// Kept as comments (on or under the ground).
    pub buried: bool,
}

/// The grids in a deck (made by [`Deck::add_grid`]).
pub fn grid_sets(deck: &Deck) -> Vec<GridSet> {
    let mut out: Vec<GridSet> = Vec::new();
    for (i, line) in deck.lines.iter().enumerate() {
        let inner = buried_card(line);
        let buried = inner.is_some();
        let line = match &inner {
            Some(d) if !d.lines.is_empty() => &d.lines[0],
            _ => line,
        };
        let LineKind::Card { mnemonic, fields } = &line.kind else {
            continue;
        };
        let Some(rest) = line
            .comment
            .as_deref()
            .and_then(|c| c.strip_prefix(GRID_MARK))
        else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let (Some(name), Some(size)) = (words.next(), words.next()) else {
            continue;
        };
        let Some((nx, ny)) = size.split_once('x') else {
            continue;
        };
        let (Ok(nx), Ok(ny)) = (nx.parse::<usize>(), ny.parse::<usize>()) else {
            continue;
        };
        if mnemonic != "GW" {
            continue;
        }
        let tag = fields
            .first()
            .and_then(|t| t.parse::<f64>().ok())
            .map_or(0, |t| t as i64);
        let k = match out.iter().position(|g| g.name == name) {
            Some(k) => k,
            None => {
                out.push(GridSet {
                    name: name.to_string(),
                    cells: [nx, ny],
                    tags: Vec::new(),
                    lines: Vec::new(),
                    buried,
                });
                out.len() - 1
            }
        };
        let g = &mut out[k];
        g.lines.push(i);
        if !g.tags.contains(&tag) {
            g.tags.push(tag);
        }
    }
    out
}

/// What a grid variable is, in a few words.
pub fn grid_label(suffix: &str) -> &'static str {
    match suffix {
        "x" => "centre x",
        "y" => "centre y",
        "z" => "height (below 0: buried)",
        "w" => "width (x)",
        "l" => "length (y)",
        "turn" => "turn",
        "wire" => "wire radius",
        _ => "",
    }
}

/// A grid's numbers, worked out (metres and degrees).
#[derive(Debug, Clone, PartialEq)]
pub struct GridNumbers {
    pub name: String,
    /// The centre, with the Ground view's height shift (z above ground).
    pub centre: [f64; 3],
    pub width: f64,
    pub length: f64,
    pub turn: f64,
    pub cells: [usize; 2],
    pub wire_radius: f64,
    pub buried: bool,
}

impl GridNumbers {
    /// The mesh's sides, (from, to), in metres as shown.
    pub fn sides(&self) -> Vec<([f64; 3], [f64; 3])> {
        let [nx, ny] = self.cells;
        let node = |i: usize, j: usize| node_at(self, i, j);
        let mut out = Vec::new();
        for j in 0..=ny {
            for i in 0..nx {
                out.push((node(i, j), node(i + 1, j)));
            }
        }
        for i in 0..=nx {
            for j in 0..ny {
                out.push((node(i, j), node(i, j + 1)));
            }
        }
        out
    }

    /// Whether point (x, y) is over the mesh, and its position across and
    /// along it.
    pub fn local(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (dx, dy) = (x - self.centre[0], y - self.centre[1]);
        let (s, c) = self.turn.to_radians().sin_cos();
        let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
        (u.abs() <= self.width / 2.0 && v.abs() <= self.length / 2.0).then_some((u, v))
    }
}

fn node_at(g: &GridNumbers, i: usize, j: usize) -> [f64; 3] {
    let u = (i as f64 / g.cells[0] as f64 - 0.5) * g.width;
    let v = (j as f64 / g.cells[1] as f64 - 0.5) * g.length;
    let (s, c) = g.turn.to_radians().sin_cos();
    [
        g.centre[0] + u * c - v * s,
        g.centre[1] + u * s + v * c,
        g.centre[2],
    ]
}

/// The grids' numbers in `numeric` (the deck expanded, with slider values).
pub fn grid_numbers(deck: &Deck, numeric: &NumericDeck) -> Vec<GridNumbers> {
    let get = |name: &str| {
        numeric
            .symbols
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
    };
    let lift = super::ground::height_now(numeric, deck);
    grid_sets(deck)
        .into_iter()
        .filter_map(|g| {
            let v = |k: &str| get(&format!("{}_{k}", g.name));
            Some(GridNumbers {
                centre: [v("x")?, v("y")?, v("z")? + lift],
                width: v("w")?,
                length: v("l")?,
                turn: v("turn").unwrap_or(0.0),
                cells: g.cells,
                wire_radius: v("wire")?,
                buried: g.buried,
                name: g.name,
            })
        })
        .collect()
}

/// A number in a formula, with the formula's other part `term`:
/// `+0.25*term`, `-0.5*term`, or nothing for 0.
fn times(f: f64, term: &str) -> String {
    if f.abs() < 1e-12 {
        String::new()
    } else if f < 0.0 {
        format!("-{}*{term}", format_real(-f))
    } else {
        format!("+{}*{term}", format_real(f))
    }
}

/// The grid's cards: rows (along x) then columns (along y), one tag each.
fn grid_cards(p: &str, cells: [usize; 2], first_tag: i64, segments: i64) -> Vec<String> {
    let v = |s: &str| format!("{p}_{s}");
    let mark = format!("{GRID_MARK} {p} {}x{}", cells[0], cells[1]);
    let [nx, ny] = cells;
    let node = |i: usize, j: usize| -> [String; 3] {
        let a = i as f64 / nx as f64 - 0.5;
        let b = j as f64 / ny as f64 - 0.5;
        let (cos, sin) = (format!("cos({})", v("turn")), format!("sin({})", v("turn")));
        let x = format!(
            "{}{}{}",
            v("x"),
            times(a, &format!("{}*{cos}", v("w"))),
            times(-b, &format!("{}*{sin}", v("l")))
        );
        let y = format!(
            "{}{}{}",
            v("y"),
            times(a, &format!("{}*{sin}", v("w"))),
            times(b, &format!("{}*{cos}", v("l")))
        );
        [x, y, v("z")]
    };
    let card = |tag: i64, a: [String; 3], b: [String; 3]| {
        let mut f = vec![tag.to_string(), segments.to_string()];
        f.extend(a);
        f.extend(b);
        f.push(v("wire"));
        super::edit::card_line("GW", &f, Some(&mark))
    };
    let mut out = Vec::new();
    let mut tag = first_tag;
    for j in 0..=ny {
        for i in 0..nx {
            out.push(card(tag, node(i, j), node(i + 1, j)));
        }
        tag += 1;
    }
    for i in 0..=nx {
        for j in 0..ny {
            out.push(card(tag, node(i, j), node(i, j + 1)));
        }
        tag += 1;
    }
    out
}

fn check_cells(cells: [usize; 2]) -> Result<(), String> {
    if cells.iter().any(|&n| !(2..=40).contains(&n) || n % 2 != 0) {
        return Err(
            "a grid has 2 to 40 cells each way, an even number (so there's a node in the middle)"
                .into(),
        );
    }
    Ok(())
}

impl Deck {
    /// Adds a grid counterpoise: its variables (grouped under its name) and
    /// cards, at [`Deck::geometry_end`], with tags clear of any in use.
    pub fn add_grid(&mut self, spec: &GridSpec, first_tag: i64) -> Result<(), String> {
        let p = spec.name.trim();
        if p.is_empty() {
            return Err("the grid needs a name".into());
        }
        check_cells(spec.cells)?;
        if spec.width <= 0.0 || spec.length <= 0.0 || spec.wire_radius <= 0.0 {
            return Err("the size and the wire must be more than zero".into());
        }
        let centre = self.hub_fields(spec.centre);
        let vars: [(&str, String, &str, &str); 7] = [
            ("x", centre[0].clone(), "m", "centre x"),
            ("y", centre[1].clone(), "m", "centre y"),
            ("z", centre[2].clone(), "m", "height"),
            ("w", format_real(spec.width), "m", "width (x)"),
            ("l", format_real(spec.length), "m", "length (y)"),
            ("turn", format_real(spec.turn), "deg", "turn about z"),
            ("wire", format_real(spec.wire_radius), "m", "wire radius"),
        ];
        let names: Vec<String> = vars.iter().map(|v| format!("{p}_{}", v.0)).collect();
        for name in &names {
            self.check_new_name(name)?;
        }
        let first_tag = first_tag.max(self.highest_tag() + 1);
        let cards = grid_cards(p, spec.cells, first_tag, spec.segments.max(1));
        for (name, (suffix, value, unit, description)) in names.iter().zip(&vars) {
            let notes = VarNotes {
                description: format!("{p} {description}"),
                unit: unit.to_string(),
                group: p.to_string(),
                fixed: matches!(*suffix, "x" | "y" | "z" | "turn" | "wire"),
                ..VarNotes::default()
            };
            self.add_variable(name, value, &notes)?;
        }
        let at = self.geometry_end();
        self.insert_lines(at, &cards);
        Ok(())
    }

    /// Gives grid `name` a different number of cells, keeping its variables,
    /// segments, first tag and whether it's buried.
    pub fn set_grid_cells(&mut self, name: &str, cells: [usize; 2]) -> Result<(), String> {
        check_cells(cells)?;
        let Some(g) = grid_sets(self).into_iter().find(|g| g.name == name) else {
            return Err(format!("there's no grid called {name}"));
        };
        let card = buried_card(&self.lines[g.lines[0]])
            .and_then(|d| d.lines.into_iter().next())
            .unwrap_or_else(|| self.lines[g.lines[0]].clone());
        let segments = match &card.kind {
            LineKind::Card { fields, .. } => fields
                .get(1)
                .and_then(|f| f.parse::<f64>().ok())
                .map_or(1, |s| s as i64),
            _ => 1,
        };
        let at = g.lines[0];
        self.remove_lines(&g.lines);
        // Its own tags first, then free ones.
        let first = g.tags.iter().copied().min().unwrap_or(1);
        let needed = (cells[0] + cells[1] + 2) as i64;
        let first = if needed as usize <= g.tags.len() {
            first
        } else {
            first.max(self.highest_tag() + 1)
        };
        self.insert_lines(at, &grid_cards(name, cells, first, segments));
        if g.buried {
            self.set_grid_buried(name, true);
        }
        Ok(())
    }

    /// Keeps grid `name`'s cards as comments (`buried`), or makes them cards.
    pub fn set_grid_buried(&mut self, name: &str, buried: bool) {
        let Some(g) = grid_sets(self).into_iter().find(|g| g.name == name) else {
            return;
        };
        for &i in &g.lines {
            let kept = buried_card(&self.lines[i]);
            if kept.is_some() == buried {
                continue;
            }
            let raw = self.lines[i].raw.clone();
            self.lines[i].raw = match kept {
                None => format!("'{BURIED_MARK} {raw}"),
                Some(d) => d.lines.into_iter().next().map_or(raw, |l| l.raw),
            };
        }
        self.reparse();
    }

    /// Makes each grid wires or estimated to suit where it is now (see
    /// [`place_for`]); in free space always wires. Returns what changed.
    pub fn sync_grids(&mut self, numeric: &NumericDeck, mhz: f64) -> Vec<String> {
        let free = super::ground::ground(numeric) == super::ground::Ground::FreeSpace;
        let mut messages = Vec::new();
        for g in grid_numbers(self, numeric) {
            let place = place_for(g.centre[2], g.wire_radius, mhz);
            let want = !free && place != Place::Raised;
            if want == g.buried {
                continue;
            }
            self.set_grid_buried(&g.name, want);
            messages.push(if want {
                format!(
                    "{} is {}: NEC-2 can't model wires there, so its effect is estimated (marked \"estimate\" in the results).",
                    g.name,
                    if place == Place::Buried { "under the ground" } else { "on the ground" }
                )
            } else {
                format!("{} is clear of the ground again, so it's modelled as wires.", g.name)
            });
        }
        messages
    }

    /// Removes a grid's cards and the variables only it used.
    pub fn remove_grid(&mut self, name: &str) {
        let Some(g) = grid_sets(self).into_iter().find(|g| g.name == name) else {
            return;
        };
        self.remove_lines(&g.lines);
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

    /// A free name for a new grid (`grid1`, `grid2`, ...).
    pub fn grid_name(&self) -> String {
        let taken: Vec<String> = self
            .variables()
            .into_iter()
            .map(|v| v.name.to_ascii_lowercase())
            .collect();
        (1..)
            .map(|i| format!("grid{i}"))
            .find(|name| {
                let prefix = format!("{name}_");
                !taken.iter().any(|v| v.starts_with(&prefix))
                    && !grid_sets(self).iter().any(|g| &g.name == name)
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> GridSpec {
        GridSpec {
            name: "grid1".into(),
            centre: [0.0, 0.0, 1.0],
            width: 4.0,
            length: 2.0,
            turn: 0.0,
            cells: [4, 2],
            segments: 3,
            wire_radius: 0.001,
        }
    }

    #[test]
    fn grids_are_meshes_joined_at_every_node() {
        let mut d = Deck::parse("GW 1 11 0 0 1 0 0 6 .001\nGE 0\nEX 0 1 1 0 1\nEN\n");
        d.add_grid(&spec(), 2).unwrap();
        let g = &grid_sets(&d)[0];
        // 3 rows of 4 sides and 5 columns of 2: 22 sides, 8 tags.
        assert_eq!((g.cells, g.lines.len(), g.tags.len()), ([4, 2], 22, 8));
        let n = d.expand().unwrap();
        let numbers = &grid_numbers(&d, &n)[0];
        let sides = numbers.sides();
        // The worked-out cards are the sides, in order.
        let cards: Vec<_> = n
            .cards
            .iter()
            .filter(|c| c.is("GW") && c.ints[0] >= 2)
            .collect();
        for (c, (a, b)) in cards.iter().zip(&sides) {
            for k in 0..3 {
                assert!((c.floats[k] - a[k]).abs() < 1e-9 && (c.floats[3 + k] - b[k]).abs() < 1e-9);
            }
        }
        // The middle node is on the vertical's base.
        assert!(
            sides
                .iter()
                .any(|(a, _)| a[0].abs() < 1e-12 && a[1].abs() < 1e-12)
        );
        // More cells, buried, and back.
        d.set_grid_buried("grid1", true);
        d.set_grid_cells("grid1", [6, 6]).unwrap();
        let g = &grid_sets(&d)[0];
        assert!(g.buried && g.lines.len() == 84);
        assert_eq!(
            d.expand()
                .unwrap()
                .cards
                .iter()
                .filter(|c| c.is("GW"))
                .count(),
            1
        );
        assert!(numbers.local(1.9, 0.9).is_some() && numbers.local(2.1, 0.0).is_none());
        d.remove_grid("grid1");
        assert!(grid_sets(&d).is_empty() && d.variables().is_empty());
        assert!(d.set_grid_cells("grid1", [3, 2]).is_err());
    }
}
