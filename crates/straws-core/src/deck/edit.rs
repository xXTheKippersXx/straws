//! Editing a deck's straight wires (GW cards) in place.
//!
//! Edits work on the deck's text, line by line, so everything else in the
//! file (comments, symbols, the user's layout) stays as it was. Fields
//! written as formulas are kept as formulas wherever possible: moving a
//! wire adds an offset to them rather than replacing them with a number.

use super::expand::NumericDeck;
use super::write::format_real;
use super::{Deck, LineKind};

/// A straight wire (GW card), as written and as worked out.
#[derive(Debug, Clone, PartialEq)]
pub struct Wire {
    /// Index of the GW line in `Deck::lines`.
    pub line: usize,
    /// The nine fields as written: tag, segments, x1 y1 z1 x2 y2 z2, radius.
    pub fields: Vec<String>,
    pub tag: i64,
    pub segments: i64,
    /// End points and radius in metres (GS scaling applied).
    pub a: [f64; 3],
    pub b: [f64; 3],
    pub radius: f64,
    /// Metres per deck unit for this wire (from GS cards after it).
    pub scale: f64,
    /// Metres added after scaling (from GM cards that only shift it).
    pub offset: [f64; 3],
    /// A GM card later rotates this wire, so where it's drawn isn't simply
    /// where its fields say. It can still be edited in the wire table.
    pub moved: bool,
}

impl Wire {
    pub fn length(&self) -> f64 {
        (0..3)
            .map(|k| (self.b[k] - self.a[k]).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    /// Whether field `k` (0-8) is a plain number, not a formula.
    pub fn is_number(&self, k: usize) -> bool {
        self.fields
            .get(k)
            .is_some_and(|f| plain_number(f).is_some())
    }

    /// End point `end` (0 or 1), in metres.
    pub fn end(&self, end: usize) -> [f64; 3] {
        if end == 0 { self.a } else { self.b }
    }
}

/// A plain number written in a field, if it is one.
pub fn plain_number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The deck's straight wires, worked out from the expanded deck.
pub fn wires(deck: &Deck, numeric: &NumericDeck) -> Vec<Wire> {
    let geometry = &numeric.cards[..numeric.control_start()];
    let mut out = Vec::new();
    for (k, card) in geometry.iter().enumerate() {
        if !card.is("GW") || numeric.helper_tags.contains(&card.ints[0]) {
            continue;
        }
        let Some(number) = card.line else { continue };
        let index = number - 1;
        let Some(line) = deck.lines.get(index) else {
            continue;
        };
        let LineKind::Card { mnemonic, fields } = &line.kind else {
            continue;
        };
        if mnemonic != "GW" {
            continue;
        }
        // Where the wire ends up: deck units times `scale`, plus `offset`.
        let mut scale = 1.0;
        let mut offset = [0.0; 3];
        let mut moved = false;
        for later in &geometry[k + 1..] {
            match later.mnemonic.as_str() {
                "GS" => {
                    scale *= later.floats[0];
                    offset = offset.map(|o| o * later.floats[0]);
                }
                // GM with no copies moves the structure from tag ITS on.
                "GM" if later.ints[1] == 0 => {
                    let its = later.floats[6] as i64;
                    if its == 0 || card.ints[0] >= its {
                        let g = &later.floats;
                        if g[..3].iter().all(|r| *r == 0.0) {
                            // A plain shift (e.g. the height above ground).
                            for (o, t) in offset.iter_mut().zip(&g[3..6]) {
                                *o += t;
                            }
                        } else {
                            moved = true;
                        }
                    }
                }
                _ => {}
            }
        }
        let f = &card.floats;
        let mut fields = fields.clone();
        fields.resize(9, "0".to_string());
        out.push(Wire {
            line: index,
            fields,
            tag: card.ints[0],
            segments: card.ints[1],
            a: [0, 1, 2].map(|i| f[i] * scale + offset[i]),
            b: [0, 1, 2].map(|i| f[3 + i] * scale + offset[i]),
            radius: f[6] * scale,
            scale,
            offset,
            moved,
        });
    }
    out
}

/// A card line in 4nec2's tab-separated style, keeping a trailing comment.
pub fn card_line(mnemonic: &str, fields: &[String], comment: Option<&str>) -> String {
    let mut raw = mnemonic.to_string();
    for f in fields {
        raw.push('\t');
        raw.push_str(f);
    }
    if let Some(c) = comment {
        raw.push_str("\t' ");
        raw.push_str(c);
    }
    raw
}

/// Tidies a field the user typed: no spaces (fields are space-separated).
pub fn clean_field(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `text` plus `delta`, keeping a formula a formula.
pub fn offset_field(text: &str, delta: f64) -> String {
    if delta == 0.0 {
        return text.to_string();
    }
    match plain_number(text) {
        Some(v) => format_real(v + delta),
        None if delta > 0.0 => format!("{text}+{}", format_real(delta)),
        None => format!("{text}-{}", format_real(-delta)),
    }
}

/// `-text`, keeping a formula a formula.
pub fn negate_field(text: &str) -> String {
    match plain_number(text) {
        Some(v) => format_real(-v),
        None => format!("-({text})"),
    }
}

/// Line-level edits.
impl Deck {
    /// Re-reads the deck from its own text, so line numbers and kinds are
    /// right again after lines were changed, added or removed.
    pub fn reparse(&mut self) {
        *self = Deck::parse(&self.to_text());
    }

    /// Replaces one field of a card line (0-based), keeping its comment.
    pub fn set_field(&mut self, line: usize, field: usize, text: &str) {
        let Some(l) = self.lines.get(line) else {
            return;
        };
        let LineKind::Card { mnemonic, fields } = &l.kind else {
            return;
        };
        let mut fields = fields.clone();
        if fields.len() <= field {
            fields.resize(field + 1, "0".to_string());
        }
        fields[field] = clean_field(text);
        let raw = card_line(mnemonic, &fields, l.comment.as_deref());
        self.lines[line].raw = raw;
        self.reparse();
    }

    /// Replaces a wire's fields with new ones (all nine).
    pub fn set_wire_fields(&mut self, line: usize, fields: &[String]) {
        let comment = self.lines.get(line).and_then(|l| l.comment.clone());
        if let Some(l) = self.lines.get_mut(line) {
            l.raw = card_line("GW", fields, comment.as_deref());
        }
        self.reparse();
    }

    /// Index of the deck's GE line, if it has one.
    pub fn ge_line(&self) -> Option<usize> {
        self.lines
            .iter()
            .position(|l| matches!(&l.kind, LineKind::Card { mnemonic, .. } if mnemonic == "GE"))
    }

    /// Inserts raw lines before line `at` and re-reads the deck.
    pub fn insert_lines(&mut self, at: usize, raws: &[String]) {
        let at = at.min(self.lines.len());
        let mut text = String::new();
        for (i, l) in self.lines.iter().enumerate() {
            if i == at {
                for r in raws {
                    text.push_str(r);
                    text.push('\n');
                }
            }
            text.push_str(&l.raw);
            text.push('\n');
        }
        if at == self.lines.len() {
            for r in raws {
                text.push_str(r);
                text.push('\n');
            }
        }
        *self = Deck::parse(&text);
    }

    /// Removes lines (indices into `lines`) and re-reads the deck.
    pub fn remove_lines(&mut self, lines: &[usize]) {
        let mut text = String::new();
        for (i, l) in self.lines.iter().enumerate() {
            if !lines.contains(&i) {
                text.push_str(&l.raw);
                text.push('\n');
            }
        }
        *self = Deck::parse(&text);
    }

    /// Where new geometry goes: just before the GM card that sets the
    /// antenna's height (so it's lifted with everything else), or before GE.
    pub fn geometry_end(&self) -> usize {
        self.height_line()
            .unwrap_or_else(|| self.ge_line().unwrap_or(self.lines.len()))
    }

    /// Adds a new straight wire (in metres, before the height shift: take
    /// [`super::ground::height_now`] off the z of a point on screen) at
    /// [`Deck::geometry_end`], where no GS card affects it. Returns the new
    /// line's index.
    pub fn add_wire(
        &mut self,
        tag: i64,
        segments: i64,
        a: [f64; 3],
        b: [f64; 3],
        radius: f64,
    ) -> usize {
        let fields: Vec<String> = [tag as f64, segments as f64]
            .into_iter()
            .chain(a)
            .chain(b)
            .chain([radius])
            // No floating-point dust like 1e-19 for a zero coordinate.
            .map(|v| format_real(if v.abs() < 1e-12 { 0.0 } else { v }))
            .collect();
        let at = self.geometry_end();
        self.insert_lines(at, &[card_line("GW", &fields, None)]);
        at
    }

    /// Feeds the centre of wire `tag`: moves the first EX card there, or
    /// adds a 1 V source just after GE if the deck has none.
    pub fn feed_wire(&mut self, tag: i64, segments: i64) {
        let centre = (segments + 1) / 2;
        let ex = self
            .lines
            .iter()
            .position(|l| matches!(&l.kind, LineKind::Card { mnemonic, .. } if mnemonic == "EX"));
        match ex {
            Some(line) => {
                self.set_field(line, 1, &tag.to_string());
                self.set_field(line, 2, &centre.to_string());
            }
            None => {
                let at = self.ge_line().map_or(self.lines.len(), |g| g + 1);
                let fields: Vec<String> =
                    ["0", &tag.to_string(), &centre.to_string(), "0", "1", "0"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                self.insert_lines(at, &[card_line("EX", &fields, None)]);
            }
        }
    }
}

/// `a + (b - a) × f` for two fields, as a number if both are numbers or
/// as a formula that keeps them.
fn between(a: &str, b: &str, f: f64) -> String {
    if f == 0.0 || a == b {
        return a.to_string();
    }
    if f == 1.0 {
        return b.to_string();
    }
    match (plain_number(a), plain_number(b)) {
        (Some(x), Some(y)) => format_real(x + (y - x) * f),
        _ => {
            let wrap = |t: &str| {
                if t.chars().skip(1).any(|c| "+-*/\\^<>= ".contains(c)) {
                    format!("({t})")
                } else {
                    t.to_string()
                }
            };
            format!("{}+({}-{})*{}", wrap(a), wrap(b), wrap(a), format_real(f))
        }
    }
}

/// Splitting a wire.
impl Deck {
    /// Splits `wire` into `pieces` wires end to end. Every segment stays
    /// where it was (the pieces share its segments out), so results don't
    /// change; sources, loads and transmission lines on it move to the
    /// piece that now holds their segment. The first piece keeps the tag;
    /// the others are numbered from `first_new_tag`.
    pub fn split_wire(
        &mut self,
        wire: &Wire,
        pieces: usize,
        first_new_tag: i64,
    ) -> Result<(), String> {
        let n = wire.segments.max(0) as usize;
        if pieces < 2 {
            return Err("split it into two pieces or more".into());
        }
        if n < pieces {
            return Err(format!(
                "wire {} has only {n} segment{}; give it more segments first",
                wire.tag,
                if n == 1 { "" } else { "s" }
            ));
        }
        let same_tag = self
            .lines
            .iter()
            .filter(|l| matches!(&l.kind, LineKind::Card { mnemonic, fields } if mnemonic == "GW" && fields.first().and_then(|t| plain_number(t)) == Some(wire.tag as f64)))
            .count();
        if same_tag > 1 {
            return Err(format!(
                "wire {} shares its tag with other wires (a curve?), so it can't be split",
                wire.tag
            ));
        }
        // Segments per piece, and where each piece starts.
        let counts: Vec<usize> = (0..pieces)
            .map(|i| n / pieces + usize::from(i < n % pieces))
            .collect();
        let starts: Vec<usize> = counts
            .iter()
            .scan(0, |acc, c| {
                let s = *acc;
                *acc += c;
                Some(s)
            })
            .collect();
        let tag_of = |i: usize| {
            if i == 0 {
                wire.tag
            } else {
                first_new_tag + i as i64 - 1
            }
        };
        // Which piece holds segment `seg` of the wire, and its number there.
        let locate = |seg: usize| -> Option<(i64, usize)> {
            (0..pieces)
                .find(|&i| seg > starts[i] && seg <= starts[i] + counts[i])
                .map(|i| (tag_of(i), seg - starts[i]))
        };
        let f = &wire.fields;
        let mut replace: Vec<(usize, Vec<String>)> = Vec::new();
        let gw: Vec<String> = (0..pieces)
            .map(|i| {
                let (fa, fb) = (
                    starts[i] as f64 / n as f64,
                    (starts[i] + counts[i]) as f64 / n as f64,
                );
                let mut fields = vec![tag_of(i).to_string(), counts[i].to_string()];
                fields.extend((0..3).map(|k| between(&f[2 + k], &f[5 + k], fa)));
                fields.extend((0..3).map(|k| between(&f[2 + k], &f[5 + k], fb)));
                fields.push(f[8].clone());
                let comment = self.lines[wire.line].comment.as_deref();
                card_line("GW", &fields, if i == 0 { comment } else { None })
            })
            .collect();
        replace.push((wire.line, gw));

        let int = |t: &str| plain_number(t).map(|v| v as i64);
        for (i, line) in self.lines.iter().enumerate() {
            let LineKind::Card { mnemonic, fields } = &line.kind else {
                continue;
            };
            let mut fields = fields.clone();
            fields.resize(fields.len().max(4), "0".into());
            let comment = line.comment.as_deref();
            match mnemonic.as_str() {
                "EX" | "TL" | "NT" => {
                    let pairs: &[(usize, usize)] = if mnemonic == "EX" {
                        &[(1, 2)]
                    } else {
                        &[(0, 1), (2, 3)]
                    };
                    let mut changed = false;
                    for &(t, s) in pairs {
                        if int(&fields[t]) == Some(wire.tag)
                            && let Some(seg) = int(&fields[s]).filter(|s| *s > 0)
                        {
                            let (tag, local) = locate(seg as usize).ok_or_else(|| {
                                format!("line {}: segment {seg} isn't on wire {}", i + 1, wire.tag)
                            })?;
                            fields[t] = tag.to_string();
                            fields[s] = local.to_string();
                            changed = true;
                        }
                    }
                    if changed {
                        replace.push((i, vec![card_line(mnemonic, &fields, comment)]));
                    }
                }
                "LD" if int(&fields[1]) == Some(wire.tag) => {
                    let (from, to) = (int(&fields[2]).unwrap_or(0), int(&fields[3]).unwrap_or(0));
                    let mut out = Vec::new();
                    if from == 0 && to == 0 {
                        // The whole wire: the same load on every piece.
                        for p in 0..pieces {
                            let mut g = fields.clone();
                            g[1] = tag_of(p).to_string();
                            out.push(card_line("LD", &g, comment));
                        }
                    } else {
                        let (lo, hi) = (
                            from.max(1) as usize,
                            (if to == 0 { from } else { to }).max(1) as usize,
                        );
                        for p in 0..pieces {
                            let (a, b) = (lo.max(starts[p] + 1), hi.min(starts[p] + counts[p]));
                            if a <= b {
                                let mut g = fields.clone();
                                g[1] = tag_of(p).to_string();
                                g[2] = (a - starts[p]).to_string();
                                g[3] = (b - starts[p]).to_string();
                                out.push(card_line("LD", &g, comment));
                            }
                        }
                    }
                    replace.push((i, out));
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for (i, l) in self.lines.iter().enumerate() {
            match replace.iter().find(|(at, _)| *at == i) {
                Some((_, raws)) => {
                    for r in raws {
                        text.push_str(r);
                        text.push('\n');
                    }
                }
                None => {
                    text.push_str(&l.raw);
                    text.push('\n');
                }
            }
        }
        *self = Deck::parse(&text);
        Ok(())
    }
}

/// The fields of a copy of `wire` moved by `delta` metres (formulas kept),
/// with a new tag.
pub fn moved_copy(wire: &Wire, delta: [f64; 3], tag: i64) -> Vec<String> {
    let mut f = wire.fields.clone();
    f[0] = tag.to_string();
    for k in 0..3 {
        let d = delta[k] / wire.scale;
        f[2 + k] = offset_field(&f[2 + k], d);
        f[5 + k] = offset_field(&f[5 + k], d);
    }
    f
}

/// The fields of `wire` mirrored in the plane where coordinate `axis`
/// (0 = x, 1 = y, 2 = z) is zero, with a new tag.
pub fn mirrored_copy(wire: &Wire, axis: usize, tag: i64) -> Vec<String> {
    let mut f = wire.fields.clone();
    f[0] = tag.to_string();
    f[2 + axis] = negate_field(&f[2 + axis]);
    f[5 + axis] = negate_field(&f[5 + axis]);
    f
}

/// The fields of `wire` with end `end` moved to `to` (metres). Coordinates
/// written as formulas are left alone, so a dragged end can't break the
/// deck's variables; edit those in the wire table instead.
pub fn moved_end(wire: &Wire, end: usize, to: [f64; 3]) -> Vec<String> {
    let mut f = wire.fields.clone();
    for (k, v) in to.iter().enumerate() {
        let field = 2 + end * 3 + k;
        if plain_number(&f[field]).is_some() {
            // Ten significant digits also drops floating-point dust.
            f[field] = format_real((v - wire.offset[k]) / wire.scale);
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECK: &str = "CM test\nCE\nSY h=10, len=5\nGW 1 11 0 -len h 0 len h #14 ' driven\nGW 2 11 1 -5 10 1 5 10 .001\nGE 0\nFR 0 1 0 0 14.2\nEN\n";

    fn wires_of(deck: &Deck) -> Vec<Wire> {
        wires(deck, &deck.expand().unwrap())
    }

    #[test]
    fn lists_wires_with_values() {
        let deck = Deck::parse(DECK);
        let w = wires_of(&deck);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].line, 3);
        assert_eq!(w[0].a, [0.0, -5.0, 10.0]);
        assert!(!w[0].is_number(3) && w[0].is_number(2));
        assert!((w[0].radius - 0.8143e-3).abs() < 1e-6);
        assert_eq!(w[1].length(), 10.0);
    }

    #[test]
    fn edits_keep_formulas_and_comments() {
        let mut deck = Deck::parse(DECK);
        let w = wires_of(&deck);
        // Dragging an end only changes the plain-number coordinates.
        deck.set_wire_fields(w[0].line, &moved_end(&w[0], 1, [0.5, 7.0, 12.0]));
        let w = wires_of(&deck);
        assert_eq!(w[0].b, [0.5, 5.0, 10.0]);
        assert!(
            deck.lines[3].raw.ends_with("' driven"),
            "{}",
            deck.lines[3].raw
        );
        // Copies offset formulas instead of replacing them.
        let copy = moved_copy(&w[0], [0.0, 0.0, 2.0], 3);
        assert_eq!(copy[4], "h+2");
        let mirror = mirrored_copy(&w[0], 1, 4);
        assert_eq!(mirror[3], "-(-len)");
        let at = deck.ge_line().unwrap();
        deck.insert_lines(
            at,
            &[card_line("GW", &copy, None), card_line("GW", &mirror, None)],
        );
        let w = wires_of(&deck);
        assert_eq!(w.len(), 4);
        assert_eq!(w[2].a[2], 12.0);
        assert_eq!(w[3].a[1], 5.0);
        deck.remove_lines(&[w[1].line]);
        assert_eq!(wires_of(&deck).len(), 3);
    }

    #[test]
    fn splitting_keeps_segments_and_moves_sources() {
        let text = "CE\nSY h=10\nGW 1 9 0 -4.5 h 0 4.5 h .001 ' dipole\nGE 0\nLD 5 1 0 0 3.7e7\nLD 4 1 2 4 50 0\nEX 0 1 5 0 1\nFR 0 1 0 0 14\nEN\n";
        let mut deck = Deck::parse(text);
        let before = deck.expand().unwrap();
        let w = wires_of(&deck);
        deck.split_wire(&w[0], 2, 2).unwrap();
        let w = wires_of(&deck);
        assert_eq!((w.len(), w[0].segments, w[1].segments), (2, 5, 4));
        assert_eq!(w[0].fields[7], "h", "formulas are kept");
        assert!((w[0].b[1] - 0.5).abs() < 1e-12 && w[1].a == w[0].b);
        assert!(deck.lines[2].raw.ends_with("' dipole"));
        let n = deck.expand().unwrap();
        let ex = n.cards.iter().find(|c| c.is("EX")).unwrap();
        assert_eq!(
            &ex.ints[1..3],
            &[1, 5],
            "segment 5 is still on the first piece"
        );
        let lds: Vec<_> = n
            .cards
            .iter()
            .filter(|c| c.is("LD"))
            .map(|c| c.ints.clone())
            .collect();
        assert_eq!(
            lds,
            vec![vec![5, 1, 0, 0], vec![5, 2, 0, 0], vec![4, 1, 2, 4]]
        );
        assert_eq!(
            before
                .cards
                .iter()
                .filter(|c| c.is("GW"))
                .map(|c| c.ints[1])
                .sum::<i64>(),
            n.cards
                .iter()
                .filter(|c| c.is("GW"))
                .map(|c| c.ints[1])
                .sum::<i64>()
        );
        // A source on the second half moves to the new piece.
        let mut deck = Deck::parse(&text.replace("EX 0 1 5", "EX 0 1 7"));
        let w = wires_of(&deck);
        deck.split_wire(&w[0], 3, 2).unwrap();
        let n = deck.expand().unwrap();
        let ex = n.cards.iter().find(|c| c.is("EX")).unwrap();
        assert_eq!(&ex.ints[1..3], &[3, 1]);
        assert!(Deck::parse(text).split_wire(&w[0], 20, 2).is_err());
    }

    #[test]
    fn new_wires_and_feeds() {
        let mut deck =
            Deck::parse("CE\nGW 1 5 0 0 0 0 0 1 .001\nGS 0 0 .0254\nGE 0\nFR 0 1 0 0 14\nEN\n");
        let w = wires_of(&deck);
        assert!((w[0].b[2] - 0.0254).abs() < 1e-12, "GS scales the wire");
        deck.add_wire(2, 9, [0.0, 0.0, 1.0], [0.0, 0.0, 3.0], 0.002);
        let w = wires_of(&deck);
        assert_eq!(w[1].scale, 1.0, "new wires go after GS");
        assert_eq!(w[1].b, [0.0, 0.0, 3.0]);
        deck.feed_wire(2, 9);
        let n = deck.expand().unwrap();
        let ex = n.cards.iter().find(|c| c.is("EX")).unwrap();
        assert_eq!(&ex.ints[..3], &[0, 2, 5]);
        deck.feed_wire(1, 5);
        let n = deck.expand().unwrap();
        let ex = n.cards.iter().find(|c| c.is("EX")).unwrap();
        assert_eq!(&ex.ints[..3], &[0, 1, 3]);
    }
}
