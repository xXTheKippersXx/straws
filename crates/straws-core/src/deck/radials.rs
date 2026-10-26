//! Radials and counterpoises: a set of straight wires spreading out evenly
//! from one hub point (the base of a vertical, say), level or drooping.
//!
//! - Each radial is an ordinary GW card with its own tag, so it can be fed,
//!   loaded and given a material like any wire. Its ends are formulas of the
//!   set's variables, `NAME_x/y/z` (the hub), `NAME_len`, `NAME_droop`
//!   (degrees below level), `NAME_turn` (where the first radial points,
//!   degrees anticlockwise from +x) and `NAME_wire` (wire radius), so
//!   sliders and the optimiser can change them.
//! - Every card carries the comment `straws: radials NAME`, so the set
//!   can be found again, given a different number of radials, or removed.
//! - A hub placed on a wire end written as formulas (a vertical whose base
//!   is `h`) takes those formulas, so the radials follow that end.
//! - Where the radials are decides how they're worked out (see
//!   [`place_for`]): raised ones are NEC wires; ones on or under the ground
//!   can't be NEC-2 wires, so their cards are kept as comments
//!   (`' straws: buried GW ...`) and `analysis::buried` estimates their
//!   effect. [`Deck::sync_radials`] switches a set between the two as its
//!   height changes.

use super::edit::{plain_number, wires};
use super::expand::NumericDeck;
use super::vars::VarNotes;
use super::write::format_real;
use super::{Deck, LineKind};
use crate::units::wavelength;

/// Start of the comment on each of a set's cards.
pub const RADIAL_MARK: &str = "straws: radials";

/// Start of the comment on the cards of a ring joining a set's tips.
pub const RING_MARK: &str = "straws: ring";

/// Start of the comment that holds a buried radial's card.
pub const BURIED_MARK: &str = "straws: buried";

/// Where a radial set is, which decides how it's worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Clearly above the ground: NEC wires.
    Raised,
    /// On the ground, or so close that NEC-2 is unreliable: estimated.
    OnGround,
    /// Under the ground: estimated.
    Buried,
}

/// Where radials whose hub is `z` metres above the ground (negative:
/// below) are. Closer to the ground than ten wire radii or a thousandth of
/// a wavelength (at `mhz`, the top of the sweep), even NEC-2's
/// Sommerfeld-Norton ground gets unreliable, so they count as on it.
pub fn place_for(z: f64, wire_radius: f64, mhz: f64) -> Place {
    if z < -1e-6 {
        Place::Buried
    } else if z < (10.0 * wire_radius).max(1e-3 * wavelength(mhz.max(1e-3))) {
        Place::OnGround
    } else {
        Place::Raised
    }
}

/// A radial set's numbers, worked out (metres and degrees).
#[derive(Debug, Clone, PartialEq)]
pub struct RadialNumbers {
    pub name: String,
    pub count: usize,
    /// The hub, with the Ground view's height shift (z is above ground).
    pub hub: [f64; 3],
    pub length: f64,
    pub turn: f64,
    pub wire_radius: f64,
    /// Whether its cards are kept as comments (on or under the ground).
    pub buried: bool,
}

/// The radial sets' numbers in `numeric` (the deck expanded, with any
/// slider values).
pub fn radial_numbers(deck: &Deck, numeric: &NumericDeck) -> Vec<RadialNumbers> {
    let get = |name: &str| {
        numeric
            .symbols
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
    };
    let lift = super::ground::height_now(numeric, deck);
    radial_sets(deck)
        .into_iter()
        .filter_map(|s| {
            let v = |k: &str| get(&format!("{}_{k}", s.name));
            Some(RadialNumbers {
                count: s.tags.len(),
                hub: [v("x")?, v("y")?, v("z")? + lift],
                length: v("len")?,
                turn: v("turn").unwrap_or(0.0),
                wire_radius: v("wire")?,
                buried: s.buried,
                name: s.name,
            })
        })
        .collect()
}

/// A buried card line, from the comment that keeps it.
pub(crate) fn buried_card(line: &super::Line) -> Option<Deck> {
    let LineKind::Comment(text) = &line.kind else {
        return None;
    };
    let inner = text.strip_prefix(BURIED_MARK)?.trim_start();
    Some(Deck::parse(inner))
}

/// What a radial set should look like (lengths in metres, angles in
/// degrees).
#[derive(Debug, Clone, PartialEq)]
pub struct RadialSpec {
    pub name: String,
    pub count: usize,
    /// Where the radials meet, before the Ground view's height shift.
    pub hub: [f64; 3],
    pub length: f64,
    /// Degrees below level (negative slopes them upwards).
    pub droop: f64,
    /// Direction of the first radial, degrees anticlockwise from +x.
    pub turn: f64,
    /// Segments on each radial.
    pub segments: i64,
    pub wire_radius: f64,
}

/// A radial set found in a deck.
#[derive(Debug, Clone, PartialEq)]
pub struct RadialSet {
    pub name: String,
    /// The radials' tags (one each).
    pub tags: Vec<i64>,
    /// Its GW lines (indices into `Deck::lines`), in order: the radials,
    /// then any ring.
    pub lines: Vec<usize>,
    /// The ring joining the tips, if it has one: its tag and lines.
    pub ring: Option<(i64, Vec<usize>)>,
    /// Its cards are kept as comments (on or under the ground).
    pub buried: bool,
}

/// The radial sets in a deck (made by [`Deck::add_radials`]).
pub fn radial_sets(deck: &Deck) -> Vec<RadialSet> {
    let mut out: Vec<RadialSet> = Vec::new();
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
        let comment = line.comment.as_deref().unwrap_or("");
        let (name, ring) = match (
            comment.strip_prefix(RADIAL_MARK),
            comment.strip_prefix(RING_MARK),
        ) {
            (Some(r), _) => (r.split_whitespace().next(), false),
            (_, Some(r)) => (r.split_whitespace().next(), true),
            _ => continue,
        };
        let Some(name) = name else { continue };
        if mnemonic != "GW" {
            continue;
        }
        let tag = fields
            .first()
            .and_then(|t| t.parse::<f64>().ok())
            .map_or(0, |t| t as i64);
        let k = match out.iter().position(|s| s.name == name) {
            Some(k) => k,
            None => {
                out.push(RadialSet {
                    name: name.to_string(),
                    tags: Vec::new(),
                    lines: Vec::new(),
                    ring: None,
                    buried,
                });
                out.len() - 1
            }
        };
        let s = &mut out[k];
        s.lines.push(i);
        if ring {
            s.ring.get_or_insert((tag, Vec::new())).1.push(i);
        } else {
            s.tags.push(tag);
        }
    }
    out
}

/// What a radial-set variable is, in a few words.
pub fn radial_label(suffix: &str) -> &'static str {
    match suffix {
        "x" => "hub x",
        "y" => "hub y",
        "z" => "hub height (below 0: buried)",
        "len" => "length",
        "droop" => "droop below level",
        "turn" => "first radial's direction",
        "wire" => "wire radius",
        _ => "",
    }
}

impl Deck {
    /// Adds a radial set: its variables (grouped under its name) and one GW
    /// card per radial, tags `first_tag` onwards, at
    /// [`Deck::geometry_end`] (so the height card still lifts it).
    pub fn add_radials(&mut self, spec: &RadialSpec, first_tag: i64) -> Result<(), String> {
        let p = spec.name.trim();
        if p.is_empty() {
            return Err("the radials need a name".into());
        }
        if !(1..=360).contains(&spec.count) {
            return Err("there can be 1 to 360 radials".into());
        }
        if spec.segments < 1 {
            return Err("each radial needs at least one segment".into());
        }
        if spec.length <= 0.0 || spec.wire_radius <= 0.0 {
            return Err("the length and the wire size must be more than zero".into());
        }
        // The hub takes the formulas of a wire end it sits on.
        let hub = self.hub_fields(spec.hub);
        let vars: [(&str, String, &str, &str); 7] = [
            ("x", hub[0].clone(), "m", "hub x"),
            ("y", hub[1].clone(), "m", "hub y"),
            ("z", hub[2].clone(), "m", "hub height"),
            (
                "len",
                format_real(spec.length),
                "m",
                "length of each radial",
            ),
            ("droop", format_real(spec.droop), "deg", "droop below level"),
            (
                "turn",
                format_real(spec.turn),
                "deg",
                "first radial's direction",
            ),
            ("wire", format_real(spec.wire_radius), "m", "wire radius"),
        ];
        let names: Vec<String> = vars.iter().map(|v| format!("{p}_{}", v.0)).collect();
        for name in &names {
            self.check_new_name(name)?;
        }
        // Clear of every tag in use, buried radials' included.
        let first_tag = first_tag.max(self.highest_tag() + 1);
        let tags: Vec<i64> = (0..spec.count as i64).map(|k| first_tag + k).collect();
        let cards = radial_cards(p, &tags, spec.segments);
        for (name, (suffix, value, unit, description)) in names.iter().zip(&vars) {
            let notes = VarNotes {
                description: format!("{p} {description}"),
                unit: unit.to_string(),
                group: p.to_string(),
                // The hub and wire size stay out of the optimiser's list.
                fixed: matches!(*suffix, "x" | "y" | "z" | "turn" | "wire"),
                ..VarNotes::default()
            };
            self.add_variable(name, value, &notes)?;
        }
        let at = self.geometry_end();
        self.insert_lines(at, &cards);
        Ok(())
    }

    /// Gives radial set `name` a different number of radials, keeping its
    /// variables, segments and the tags it had (new radials get free tags).
    pub fn set_radial_count(&mut self, name: &str, count: usize) -> Result<(), String> {
        if !(1..=360).contains(&count) {
            return Err("there can be 1 to 360 radials".into());
        }
        let Some(set) = radial_sets(self).into_iter().find(|s| s.name == name) else {
            return Err(format!("there are no radials called {name}"));
        };
        let first = buried_card(&self.lines[set.lines[0]])
            .and_then(|d| d.lines.into_iter().next())
            .unwrap_or_else(|| self.lines[set.lines[0]].clone());
        let segments = match &first.kind {
            LineKind::Card { fields, .. } => fields
                .get(1)
                .and_then(|f| f.parse::<f64>().ok())
                .map_or(1, |s| s as i64),
            _ => 1,
        };
        let mut tags: Vec<i64> = set.tags.iter().copied().take(count).collect();
        let mut next = self.highest_tag().max(0) + 1;
        while tags.len() < count {
            tags.push(next);
            next += 1;
        }
        let ring = set
            .ring
            .as_ref()
            .map(|(tag, lines)| (*tag, self.segments_of(lines[0])));
        let at = set.lines[0];
        self.remove_lines(&set.lines);
        let mut cards = radial_cards(name, &tags, segments);
        if let Some((tag, segs)) = ring
            && count >= 3
        {
            cards.extend(ring_cards(name, count, tag, segs));
        }
        self.insert_lines(at, &cards);
        if set.buried {
            self.set_radials_buried(name, true);
        }
        Ok(())
    }

    /// Segments on the card at `line` (kept as a comment or not).
    fn segments_of(&self, line: usize) -> i64 {
        let card = buried_card(&self.lines[line])
            .and_then(|d| d.lines.into_iter().next())
            .unwrap_or_else(|| self.lines[line].clone());
        match &card.kind {
            LineKind::Card { fields, .. } => fields
                .get(1)
                .and_then(|f| f.parse::<f64>().ok())
                .map_or(1, |s| s as i64),
            _ => 1,
        }
    }

    /// Adds (`segments` per side) or removes the ring of wires joining
    /// radial set `name`'s tips: a counterpoise that's also a loop. It needs
    /// at least 3 radials.
    pub fn set_radial_ring(&mut self, name: &str, segments: Option<i64>) -> Result<(), String> {
        let Some(set) = radial_sets(self).into_iter().find(|s| s.name == name) else {
            return Err(format!("there are no radials called {name}"));
        };
        if let Some((_, lines)) = &set.ring {
            self.remove_lines(lines);
        }
        let Some(segments) = segments else {
            return Ok(());
        };
        if set.tags.len() < 3 {
            return Err("a ring needs at least 3 radials".into());
        }
        let tag = self.highest_tag().max(0) + 1;
        let set = radial_sets(self).into_iter().find(|s| s.name == name);
        let Some(set) = set else { return Ok(()) };
        let at = set.lines.last().map_or(self.geometry_end(), |l| l + 1);
        self.insert_lines(at, &ring_cards(name, set.tags.len(), tag, segments.max(1)));
        if set.buried {
            // The new cards go in as cards: keep the whole set alike.
            self.set_radials_buried(name, false);
            self.set_radials_buried(name, true);
        }
        Ok(())
    }

    /// Keeps radial set `name`'s cards as comments (`buried`), so NEC
    /// doesn't see them, or makes them cards again.
    pub fn set_radials_buried(&mut self, name: &str, buried: bool) {
        let Some(set) = radial_sets(self).into_iter().find(|s| s.name == name) else {
            return;
        };
        if set.buried == buried {
            return;
        }
        for &i in &set.lines {
            let raw = self.lines[i].raw.clone();
            let kept = buried_card(&self.lines[i]).is_some();
            if kept == buried {
                continue;
            }
            self.lines[i].raw = if buried {
                format!("'{BURIED_MARK} {raw}")
            } else {
                buried_card(&self.lines[i])
                    .and_then(|d| d.lines.into_iter().next())
                    .map_or(raw, |l| l.raw)
            };
        }
        self.reparse();
    }

    /// Raised radial sets lower than 5% of a wavelength (at `mhz`) over a
    /// reflection-coefficient ground need Sommerfeld-Norton: switches to it
    /// and says why.
    pub fn ground_for_low_radials(&mut self, numeric: &NumericDeck, mhz: f64) -> Option<String> {
        let limit = 0.05 * wavelength(mhz.max(1e-3));
        let low = radial_numbers(self, numeric)
            .into_iter()
            .find(|r| !r.buried && r.hub[2] < limit)
            .map(|r| format!("{} are", r.name))
            .or_else(|| {
                super::grids::grid_numbers(self, numeric)
                    .into_iter()
                    .find(|g| !g.buried && g.centre[2] < limit)
                    .map(|g| format!("{} is", g.name))
            })?;
        let quick = matches!(
            super::ground::ground(numeric),
            super::ground::Ground::Real {
                method: super::ground::Method::Reflection,
                ..
            }
        );
        (quick && self.use_sommerfeld()).then(|| {
            format!(
                "{low} close to the ground, where the quick reflection-coefficient method is wrong: \
                 the ground now uses Sommerfeld-Norton (Ctrl+Z undoes)."
            )
        })
    }

    /// Makes each radial set wires or estimated to suit where it is now
    /// (see [`place_for`]), in free space always wires. `numeric` is this
    /// deck expanded (with slider values); `mhz` the top of the sweep.
    /// Returns a message for each set that changed.
    pub fn sync_radials(&mut self, numeric: &NumericDeck, mhz: f64) -> Vec<String> {
        let free = super::ground::ground(numeric) == super::ground::Ground::FreeSpace;
        let mut messages = Vec::new();
        for r in radial_numbers(self, numeric) {
            let place = place_for(r.hub[2], r.wire_radius, mhz);
            let want = !free && place != Place::Raised;
            if want == r.buried {
                continue;
            }
            self.set_radials_buried(&r.name, want);
            messages.push(if want {
                format!(
                    "{} are {}: NEC-2 can't model wires there, so their effect is estimated (marked \"estimate\" in the results).",
                    r.name,
                    if place == Place::Buried { "under the ground" } else { "on the ground" }
                )
            } else {
                format!("{} are clear of the ground again, so they're modelled as wires.", r.name)
            });
        }
        messages
    }

    /// Removes a radial set's cards and the variables only it used.
    pub fn remove_radials(&mut self, name: &str) {
        let Some(set) = radial_sets(self).into_iter().find(|s| s.name == name) else {
            return;
        };
        self.remove_lines(&set.lines);
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

    /// A free name for a new radial set (`radials1`, `radials2`, ...).
    pub fn radials_name(&self) -> String {
        let taken: Vec<String> = self
            .variables()
            .into_iter()
            .map(|v| v.name.to_ascii_lowercase())
            .collect();
        (1..)
            .map(|i| format!("radials{i}"))
            .find(|name| {
                let prefix = format!("{name}_");
                !taken.iter().any(|v| v.starts_with(&prefix))
                    && !radial_sets(self).iter().any(|s| &s.name == name)
            })
            .unwrap_or_default()
    }

    /// The highest tag used by the deck's wires, arcs and helices, and by
    /// radials kept as comments (on or under the ground).
    pub(crate) fn highest_tag(&self) -> i64 {
        kept_tags(self)
            .into_iter()
            .max()
            .unwrap_or(0)
            .max(self.highest_card_tag())
    }

    fn highest_card_tag(&self) -> i64 {
        if let Ok(n) = self.expand() {
            n.cards[..n.control_start()]
                .iter()
                .filter(|c| ["GW", "GA", "GH"].iter().any(|m| c.is(m)))
                .map(|c| c.ints[0])
                .filter(|t| !n.helper_tags.contains(t))
                .max()
                .unwrap_or(0)
        } else {
            0
        }
    }

    /// The hub's x, y and z as deck text: a wire end's own formulas when the
    /// hub sits on one, otherwise plain numbers.
    pub(crate) fn hub_fields(&self, hub: [f64; 3]) -> [String; 3] {
        // To 0.1 micrometre: no floating-point dust like 0.6000000089.
        let plain = hub.map(|v| format_real((v * 1e7).round() / 1e7));
        let Ok(n) = self.expand() else {
            return plain;
        };
        let size = hub.iter().fold(1.0f64, |m, v| m.max(v.abs()));
        for w in wires(self, &n) {
            if w.moved || w.scale != 1.0 {
                continue;
            }
            for (end, first) in [(w.a, 2), (w.b, 5)] {
                // Back to the numbers as written (before any GM shift).
                let written = [0, 1, 2].map(|k| end[k] - w.offset[k]);
                if (0..3).all(|k| (written[k] - hub[k]).abs() <= 1e-6 * size) {
                    let mut out = plain.clone();
                    for (o, text) in out.iter_mut().zip(&w.fields[first..first + 3]) {
                        if plain_number(text).is_none() {
                            *o = text.clone();
                        }
                    }
                    return out;
                }
            }
        }
        plain
    }
}

/// Tags of radials, rings and grid counterpoises, including ones kept as
/// comments (on or under the ground), which no NEC card shows.
pub fn kept_tags(deck: &Deck) -> Vec<i64> {
    let mut tags: Vec<i64> = radial_sets(deck)
        .iter()
        .flat_map(|s| s.tags.iter().copied().chain(s.ring.as_ref().map(|r| r.0)))
        .collect();
    tags.extend(
        super::grids::grid_sets(deck)
            .iter()
            .flat_map(|g| g.tags.iter().copied()),
    );
    tags
}

/// The ring joining radial set `p`'s `count` tips: one GW card per side,
/// all with tag `tag`.
fn ring_cards(p: &str, count: usize, tag: i64, segments: i64) -> Vec<String> {
    let v = |s: &str| format!("{p}_{s}");
    let mark = format!("{RING_MARK} {p}");
    let level = format!("{}*cos({})", v("len"), v("droop"));
    let tip = |k: usize| -> [String; 3] {
        let angle = if k.is_multiple_of(count) {
            v("turn")
        } else {
            format!(
                "{}+{}",
                v("turn"),
                format_real(360.0 * (k % count) as f64 / count as f64)
            )
        };
        [
            format!("{}+{level}*cos({angle})", v("x")),
            format!("{}+{level}*sin({angle})", v("y")),
            format!("{}-{}*sin({})", v("z"), v("len"), v("droop")),
        ]
    };
    (0..count)
        .map(|k| {
            let mut fields = vec![tag.to_string(), segments.to_string()];
            fields.extend(tip(k));
            fields.extend(tip(k + 1));
            fields.push(v("wire"));
            super::edit::card_line("GW", &fields, Some(&mark))
        })
        .collect()
}

/// The GW cards of radial set `p`, one per tag, spread evenly round the hub.
fn radial_cards(p: &str, tags: &[i64], segments: i64) -> Vec<String> {
    let v = |s: &str| format!("{p}_{s}");
    let mark = format!("{RADIAL_MARK} {p}");
    let count = tags.len();
    tags.iter()
        .enumerate()
        .map(|(k, tag)| {
            let angle = if k == 0 {
                v("turn")
            } else {
                format!(
                    "{}+{}",
                    v("turn"),
                    format_real(360.0 * k as f64 / count as f64)
                )
            };
            let level = format!("{}*cos({})", v("len"), v("droop"));
            let fields = vec![
                tag.to_string(),
                segments.to_string(),
                v("x"),
                v("y"),
                v("z"),
                format!("{}+{level}*cos({angle})", v("x")),
                format!("{}+{level}*sin({angle})", v("y")),
                format!("{}-{}*sin({})", v("z"), v("len"), v("droop")),
                v("wire"),
            ];
            super::edit::card_line("GW", &fields, Some(&mark))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(hub: [f64; 3]) -> RadialSpec {
        RadialSpec {
            name: "radials1".into(),
            count: 4,
            hub,
            length: 2.5,
            droop: 30.0,
            turn: 0.0,
            segments: 9,
            wire_radius: 0.001,
        }
    }

    fn gw_ends(d: &Deck) -> Vec<[f64; 6]> {
        let n = d.expand().unwrap();
        n.cards
            .iter()
            .filter(|c| c.is("GW") && c.ints[0] >= 2)
            .map(|c| {
                let f = &c.floats;
                [f[0], f[1], f[2], f[3], f[4], f[5]]
            })
            .collect()
    }

    #[test]
    fn radials_are_spread_round_the_hub() {
        let mut d = Deck::parse("GW 1 11 0 0 5 0 0 7.5 .001\nGE 0\nEX 0 1 1 0 1\nEN\n");
        d.add_radials(&spec([0.0, 0.0, 5.0]), 2).unwrap();
        let ends = gw_ends(&d);
        assert_eq!(ends.len(), 4);
        let level = 2.5 * 30f64.to_radians().cos();
        for (k, e) in ends.iter().enumerate() {
            let a = (90.0 * k as f64).to_radians();
            let want = [0.0, 0.0, 5.0, level * a.cos(), level * a.sin(), 5.0 - 1.25];
            for i in 0..6 {
                assert!((e[i] - want[i]).abs() < 1e-9, "radial {k}: {e:?}");
            }
        }
        assert_eq!(radial_sets(&d)[0].tags, [2, 3, 4, 5]);
        // Eight radials: the same variables, four more tags.
        d.set_radial_count("radials1", 8).unwrap();
        assert_eq!(gw_ends(&d).len(), 8);
        assert_eq!(radial_sets(&d)[0].tags, [2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(d.variables().len(), 7);
        assert_eq!(d.radials_name(), "radials2");
        // Removed: wires and variables gone.
        d.remove_radials("radials1");
        assert!(gw_ends(&d).is_empty());
        assert!(d.variables().is_empty());
    }

    #[test]
    fn radials_on_or_under_the_ground_become_estimates() {
        let mut d = Deck::parse(
            "GW 1 11 0 0 0 0 0 10 .001
GE 1
GN 2 0 0 0 13 .005
EX 0 1 1 0 1
EN
",
        );
        let mut s = spec([0.0, 0.0, 0.0]);
        s.droop = 0.0;
        d.add_radials(&s, 2).unwrap();
        let n = d.expand().unwrap();
        // On the ground: kept as comments, found again, estimated.
        assert_eq!(d.sync_radials(&n, 7.0).len(), 1);
        let set = &radial_sets(&d)[0];
        assert!(set.buried);
        assert_eq!(set.tags, [2, 3, 4, 5]);
        let n = d.expand().unwrap();
        assert_eq!(n.cards.iter().filter(|c| c.is("GW")).count(), 1);
        let r = &radial_numbers(&d, &n)[0];
        assert_eq!((r.count, r.length, r.buried), (4, 2.5, true));
        // A count change keeps them buried, with tags of their own.
        d.set_radial_count("radials1", 6).unwrap();
        assert!(radial_sets(&d)[0].buried);
        assert_eq!(radial_sets(&d)[0].tags, [2, 3, 4, 5, 6, 7]);
        // Another set doesn't reuse the buried set's tags.
        let mut other = spec([1.0, 0.0, 2.0]);
        other.name = "radials2".into();
        d.add_radials(&other, 2).unwrap();
        assert_eq!(radial_sets(&d)[1].tags, [8, 9, 10, 11]);
        d.remove_radials("radials2");
        // Raised 1 m: wires again.
        let n = d.expand_with(&[("radials1_z".into(), 1.0)]).unwrap();
        assert_eq!(d.sync_radials(&n, 7.0).len(), 1);
        assert!(!radial_sets(&d)[0].buried);
        assert_eq!(
            d.expand()
                .unwrap()
                .cards
                .iter()
                .filter(|c| c.is("GW"))
                .count(),
            7
        );
        // Where things count as on the ground.
        assert_eq!(place_for(-0.15, 0.001, 7.0), Place::Buried);
        assert_eq!(place_for(0.02, 0.001, 7.0), Place::OnGround);
        assert_eq!(place_for(0.5, 0.001, 7.0), Place::Raised);
    }

    #[test]
    fn a_ring_joins_the_tips() {
        let mut d = Deck::parse(
            "GW 1 11 0 0 5 0 0 7.5 .001
GE 0
EX 0 1 1 0 1
EN
",
        );
        d.add_radials(&spec([0.0, 0.0, 5.0]), 2).unwrap();
        d.set_radial_ring("radials1", Some(3)).unwrap();
        let set = &radial_sets(&d)[0];
        assert_eq!(set.tags, [2, 3, 4, 5]);
        assert_eq!(set.ring.as_ref().map(|r| (r.0, r.1.len())), Some((6, 4)));
        // Each side runs from one radial's tip to the next.
        let ends = gw_ends(&d);
        for k in 0..4 {
            let (radial, side) = (ends[k], ends[4 + k]);
            assert!(
                (0..3).all(|i| (side[i] - radial[3 + i]).abs() < 1e-9),
                "{k}"
            );
            let next = ends[(k + 1) % 4];
            assert!(
                (0..3).all(|i| (side[3 + i] - next[3 + i]).abs() < 1e-9),
                "{k}"
            );
        }
        // More radials: the ring follows; then off again.
        d.set_radial_count("radials1", 6).unwrap();
        assert_eq!(gw_ends(&d).len(), 12);
        d.set_radial_ring("radials1", None).unwrap();
        assert_eq!(gw_ends(&d).len(), 6);
        assert!(radial_sets(&d)[0].ring.is_none());
    }

    #[test]
    fn decks_saved_under_the_old_name_still_read() {
        let old = concat!("nec-", "modeller: radials radials1");
        let d = Deck::parse(&format!(
            "SY radials1_x=0, radials1_y=0, radials1_z=0, radials1_len=1, radials1_droop=0, radials1_turn=0, radials1_wire=.001
             GW 1 9 0 0 0 0 0 1 .001
GW 2 3 radials1_x radials1_y radials1_z 1 0 0 .001 ' {old}
GE 0
EN
"
        ));
        assert_eq!(radial_sets(&d).len(), 1);
        assert!(d.to_text().contains("' straws: radials radials1"));
    }

    #[test]
    fn the_hub_follows_a_wire_end_written_as_a_formula() {
        let mut d = Deck::parse("SY h=5\nGW 1 11 0 0 h 0 0 h+2.5 .001\nGE 0\nEX 0 1 1 0 1\nEN\n");
        d.add_radials(&spec([0.0, 0.0, 5.0]), 2).unwrap();
        let z = d.variables().into_iter().find(|v| v.name == "radials1_z");
        assert_eq!(z.map(|v| v.expr), Some("h".to_string()));
        // Raising the vertical raises the radials.
        let n = d.expand_with(&[("h".into(), 8.0)]).unwrap();
        let radial = n
            .cards
            .iter()
            .find(|c| c.is("GW") && c.ints[0] == 2)
            .unwrap();
        assert!((radial.floats[2] - 8.0).abs() < 1e-12);
    }
}
