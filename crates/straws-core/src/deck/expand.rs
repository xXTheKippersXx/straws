//! Turning a [`Deck`] into a plain numeric deck for the engine.

use super::expr::{self, Symbols};
use super::{Deck, DeckError, LineKind, layout};

/// A deck with every symbol and expression evaluated: what the engine runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NumericDeck {
    /// Comment text from `CM`/`CE` cards.
    pub comments: Vec<String>,
    /// Geometry and control cards in order, ending before `EN`.
    pub cards: Vec<NumCard>,
    /// Final symbol values, in the order first defined.
    pub symbols: Vec<(String, f64)>,
    /// Things that were accepted but are worth mentioning.
    pub warnings: Vec<DeckError>,
    /// Tags of wires the reader added itself (the dummy wires behind 4nec2
    /// current sources). Views should hide them.
    pub helper_tags: Vec<i64>,
    /// Tube walls (deck line of the LD 5 card, wall in metres), from the
    /// cards' comments; see `deck::material`.
    pub tubes: Vec<(usize, f64)>,
}

/// One numeric card. `ints` and `floats` always have the card's full field
/// count (missing fields are zero, as in NEC).
#[derive(Debug, Clone, PartialEq)]
pub struct NumCard {
    pub mnemonic: String,
    pub ints: Vec<i64>,
    pub floats: Vec<f64>,
    /// Line in the original deck, if the card came from one.
    pub line: Option<usize>,
}

impl NumCard {
    /// Builds a card, padding missing fields with zeros.
    pub fn new(mnemonic: &str, ints: &[i64], floats: &[f64]) -> NumCard {
        let lay = layout(mnemonic).expect("NEC-2 card");
        let mut i = ints.to_vec();
        let mut f = floats.to_vec();
        i.resize(lay.ints, 0);
        f.resize(lay.floats, 0.0);
        NumCard {
            mnemonic: mnemonic.to_string(),
            ints: i,
            floats: f,
            line: None,
        }
    }

    pub fn is(&self, mnemonic: &str) -> bool {
        self.mnemonic == mnemonic
    }
}

/// Cards that make NEC run a calculation.
pub const EXECUTE_CARDS: &[&str] = &["XQ", "RP", "NE", "NH"];

/// Engine features of 4nec2's own engines that nec2c does not have.
const UNSUPPORTED: &[(&str, &str)] = &[
    (
        "GF",
        "uses a Numerical Green's Function file (GF card), which nec2c does not support",
    ),
    (
        "WG",
        "writes a Numerical Green's Function file (WG card), which nec2c does not support",
    ),
];

/// Half-length of 4nec2's dummy source wire is this many metres-MHz divided
/// by the frequency, in deck units. Found from 4nec2's own expanded decks.
const CURRENT_SOURCE_HALF_LENGTH: f64 = 0.152898;

/// Segment-number fields that 4nec2 lets you give as a percentage along a
/// wire (`EX 0 2 50% ...`): (card, segment field, tag field), 0-based.
const SEGMENT_FIELDS: &[(&str, usize, usize)] = &[
    ("EX", 2, 1),
    ("LD", 2, 1),
    ("LD", 3, 1),
    ("NT", 1, 0),
    ("NT", 3, 2),
    ("TL", 1, 0),
    ("TL", 3, 2),
    ("PT", 2, 1),
    ("PT", 3, 1),
    ("CP", 1, 0),
    ("CP", 3, 2),
];

/// The segment containing the point `percent` of the way along a wire of
/// `segments` segments.
fn segment_at_percent(percent: f64, segments: i64) -> i64 {
    ((segments as f64 * percent / 100.0).ceil() as i64).clamp(1, segments.max(1))
}

pub(super) fn expand(deck: &Deck, overrides: &[(String, f64)]) -> Result<NumericDeck, DeckError> {
    let mut out = NumericDeck::default();
    let mut symbols = Symbols::default();
    // Overrides replace the first definition of a symbol; later
    // redefinitions (A=A*2) still build on it.
    let mut pending: Vec<(String, f64)> = overrides
        .iter()
        .map(|(n, v)| (n.to_ascii_lowercase(), *v))
        .collect();
    // Segment count of each tag, from cards that create a wire directly.
    let mut tag_segments: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();

    for line in &deck.lines {
        match &line.kind {
            LineKind::Blank | LineKind::Comment(_) => {}
            LineKind::Text { mnemonic, text } => match mnemonic.as_str() {
                "CM" | "CE" => {
                    if !text.is_empty() || mnemonic == "CM" {
                        out.comments.push(text.clone());
                    }
                }
                other => return Err(unsupported(line.number, other)),
            },
            LineKind::Symbols(assignments) => {
                for a in assignments {
                    if a.name.is_empty() || a.expr.is_empty() {
                        return Err(DeckError::at(
                            line.number,
                            format!("SY needs name=value, found \"{}{}\"", a.name, a.expr),
                        ));
                    }
                    let key = a.name.to_ascii_lowercase();
                    let v = if let Some(i) = pending.iter().position(|(n, _)| *n == key) {
                        pending.swap_remove(i).1
                    } else {
                        expr::eval(&a.expr, &symbols).map_err(|e| {
                            DeckError::at(line.number, format!("in SY {}: {e}", a.name))
                        })?
                    };
                    symbols.set(&a.name, v);
                    let key = a.name.to_ascii_lowercase();
                    match out
                        .symbols
                        .iter_mut()
                        .find(|(n, _)| n.eq_ignore_ascii_case(&key))
                    {
                        Some(entry) => entry.1 = v,
                        None => out.symbols.push((a.name.clone(), v)),
                    }
                }
            }
            LineKind::Card { mnemonic, fields } => {
                if mnemonic == "EN" {
                    break;
                }
                if UNSUPPORTED.iter().any(|(m, _)| m == mnemonic) {
                    return Err(unsupported(line.number, mnemonic));
                }
                let Some(lay) = layout(mnemonic) else {
                    return Err(DeckError::at(
                        line.number,
                        format!("\"{mnemonic}\" is not a NEC-2 card"),
                    ));
                };
                let mut card = NumCard::new(mnemonic, &[], &[]);
                card.line = Some(line.number);
                let wanted = lay.ints + lay.floats;
                for (k, field) in fields.iter().enumerate() {
                    if let Some(pct) = field.strip_suffix('%') {
                        let Some(&(_, _, tag_field)) = SEGMENT_FIELDS
                            .iter()
                            .find(|(m, f, _)| m == mnemonic && *f == k)
                        else {
                            return Err(DeckError::at(
                                line.number,
                                format!("{mnemonic} field {} can't be a percentage", k + 1),
                            ));
                        };
                        let p = expr::eval(pct, &symbols).map_err(|e| {
                            DeckError::at(
                                line.number,
                                format!("{mnemonic} {}: {e}", field_name(mnemonic, k)),
                            )
                        })?;
                        let tag = card.ints[tag_field];
                        let Some(&n) = tag_segments.get(&tag) else {
                            return Err(DeckError::at(
                                line.number,
                                format!(
                                    "{mnemonic} field {}: no wire with tag {tag} to take {field} of",
                                    k + 1
                                ),
                            ));
                        };
                        card.ints[k] = segment_at_percent(p, n);
                        continue;
                    }
                    let value = match expr::eval(field, &symbols) {
                        Ok(v) => v,
                        Err(e) if k >= wanted => {
                            // Trailing text after the last field: NEC ignores it.
                            out.warnings.push(DeckError::at(
                                line.number,
                                format!("ignored extra text \"{field}\" ({e})"),
                            ));
                            break;
                        }
                        Err(e) => {
                            return Err(DeckError::at(
                                line.number,
                                format!("{mnemonic} {}: {e}", field_name(mnemonic, k)),
                            ));
                        }
                    };
                    if k < lay.ints {
                        let rounded = value.round();
                        if (rounded - value).abs() > 1e-9 {
                            out.warnings.push(DeckError::at(
                                line.number,
                                format!(
                                    "{mnemonic} field {} should be a whole number; {value} rounded to {rounded}",
                                    k + 1
                                ),
                            ));
                        }
                        card.ints[k] = rounded as i64;
                    } else if k < wanted {
                        card.floats[k - lay.ints] = value;
                    } else {
                        out.warnings.push(DeckError::at(
                            line.number,
                            format!("ignored extra field \"{field}\" on {mnemonic} card"),
                        ));
                        break;
                    }
                }
                if matches!(mnemonic.as_str(), "GW" | "GA" | "GH") {
                    check_wire(&card, line.number)?;
                    tag_segments.insert(card.ints[0], card.ints[1]);
                }
                if mnemonic == "LD"
                    && let Some(t) = super::material::tube_wall_text(line.comment.as_deref())
                {
                    // The wall may be a formula of the deck's variables.
                    let wall = expr::eval(t, &symbols).map_err(|e| {
                        DeckError::at(line.number, format!("tube wall thickness: {e}"))
                    })?;
                    if wall > 0.0 {
                        out.tubes.push((line.number, wall));
                    }
                }
                out.cards.push(card);
            }
        }
    }

    if !out.cards.iter().any(|c| c.is("GE")) {
        return Err(DeckError::general(
            "the deck has no GE card to end the geometry section",
        ));
    }
    tidy_ge(&mut out);
    drop_trailing_nx(&mut out)?;
    convert_mininec_ground(&mut out);
    ignore_stand_in_screen(deck, &mut out);
    convert_trap_loads(&mut out);
    convert_coated_wires(&mut out)?;
    convert_current_sources(&mut out);
    Ok(out)
}

/// NEC-2's GE card has one field. nec2c treats a second one as "write a
/// plot file", and crashes because no plot file is open; 4nec2's engine
/// ignores it. Keep only the first field.
fn tidy_ge(deck: &mut NumericDeck) {
    for card in deck.cards.iter_mut().filter(|c| c.is("GE")) {
        if card.ints[1] != 0 {
            deck.warnings.push(DeckError {
                line: card.line,
                message: format!(
                    "ignored GE field 2 ({}), which is not part of NEC-2",
                    card.ints[1]
                ),
            });
            card.ints[1] = 0;
        }
    }
}

/// `NX` starts a new structure. One at the very end (nothing follows it) is
/// harmless in 4nec2 but makes nec2c try to read another structure, so drop
/// it. Decks that really hold several structures aren't supported yet.
fn drop_trailing_nx(deck: &mut NumericDeck) -> Result<(), DeckError> {
    if let Some(i) = deck.cards.iter().position(|c| c.is("NX")) {
        if i + 1 < deck.cards.len() {
            return Err(DeckError {
                line: deck.cards[i].line,
                message: "decks holding several structures (cards after NX) aren't supported yet"
                    .into(),
            });
        }
        deck.cards.pop();
    }
    Ok(())
}

/// The first frequency the deck asks for, or NEC's default of 299.8 MHz.
fn design_frequency(deck: &NumericDeck) -> f64 {
    deck.cards
        .iter()
        .find(|c| c.is("FR"))
        .map_or(299.8, |c| c.floats[0])
}

/// 4nec2's `LD 6` LC trap: fields Q of the coil (0 means 100), L, C and an
/// optional frequency (default: the first FR frequency). 4nec2 converts it
/// to a parallel RLC load (`LD 1`); the coil loss is represented by the
/// parallel resistance R = Q·X_L at that frequency.
fn convert_trap_loads(deck: &mut NumericDeck) {
    let f_design = design_frequency(deck);
    for card in deck
        .cards
        .iter_mut()
        .filter(|c| c.is("LD") && c.ints[0] == 6)
    {
        let q = if card.floats[0] > 0.0 {
            card.floats[0]
        } else {
            100.0
        };
        let (l, c) = (card.floats[1], card.floats[2]);
        let f = if card.floats[3] > 0.0 {
            card.floats[3]
        } else {
            f_design
        };
        let x_l = 2.0 * std::f64::consts::PI * f * 1e6 * l;
        let line = card.line;
        *card = NumCard::new(
            "LD",
            &[1, card.ints[1], card.ints[2], card.ints[3]],
            &[q * x_l, l, c],
        );
        card.line = line;
    }
}

/// 4nec2's `LD 7` insulated wire: fields dielectric constant and the radius
/// over the insulation (metres). 4nec2 converts it to a distributed
/// inductance (`LD 2`) with L.B. Cebik's formula:
/// L = 2e-7 · (ε·R/r)^(1/12) · (1 − 1/ε) · ln(R/r) henries per metre.
fn convert_coated_wires(deck: &mut NumericDeck) -> Result<(), DeckError> {
    // Wire radius of each tag in metres (GS scales the geometry).
    let mut radius = std::collections::HashMap::new();
    let mut scale = 1.0;
    for c in &deck.cards[..deck.control_start()] {
        match c.mnemonic.as_str() {
            "GW" => {
                radius.entry(c.ints[0]).or_insert(c.floats[6]);
            }
            "GA" => {
                radius.entry(c.ints[0]).or_insert(c.floats[3]);
            }
            "GH" => {
                radius.entry(c.ints[0]).or_insert(c.floats[6]);
            }
            "GS" => scale *= c.floats[0],
            _ => {}
        }
    }
    let mut k = 0;
    while k < deck.cards.len() {
        let card = deck.cards[k].clone();
        if !(card.is("LD") && card.ints[0] == 7) {
            k += 1;
            continue;
        }
        let (eps, outer) = (card.floats[0], card.floats[1]);
        let tags: Vec<(i64, i64, i64)> = if card.ints[1] == 0 {
            let mut all: Vec<i64> = radius.keys().copied().collect();
            all.sort();
            all.into_iter().map(|t| (t, 0, 0)).collect()
        } else {
            vec![(card.ints[1], card.ints[2], card.ints[3])]
        };
        let mut replacement = Vec::new();
        for (tag, first, last) in tags {
            let r = radius.get(&tag).copied().unwrap_or(0.0) * scale;
            if r <= 0.0 {
                return Err(DeckError {
                    line: card.line,
                    message: format!(
                        "LD 7 (insulated wire) on tag {tag}: can't find that wire's radius"
                    ),
                });
            }
            if eps <= 1.0 || outer <= r {
                deck.warnings.push(DeckError {
                    line: card.line,
                    message: format!("LD 7 on tag {tag} has no effect (dielectric constant {eps}, insulation radius {outer} m, wire radius {r} m)"),
                });
                continue;
            }
            let ratio = outer / r;
            let l = 2e-7 * (eps * ratio).powf(1.0 / 12.0) * (1.0 - 1.0 / eps) * ratio.ln();
            let mut ld = NumCard::new("LD", &[2, tag, first, last], &[0.0, l, 0.0]);
            ld.line = card.line;
            replacement.push(ld);
        }
        let n = replacement.len();
        deck.cards.splice(k..=k, replacement);
        k += n;
    }
    Ok(())
}

/// A GN radial screen written only for 4nec2, standing in for radials on or
/// under the ground that this app estimates better itself, is left out.
fn ignore_stand_in_screen(deck: &super::Deck, out: &mut NumericDeck) {
    for c in out.cards.iter_mut().filter(|c| c.is("GN")) {
        let marked = c
            .line
            .and_then(|l| deck.lines.get(l - 1))
            .and_then(|l| l.comment.as_deref())
            == Some(super::ground::SCREEN_MARK);
        if marked {
            c.ints[1] = 0;
            for f in c.floats.iter_mut().skip(2) {
                *f = 0.0;
            }
        }
    }
}

/// 4nec2's `GN 3` ("MININEC ground") is not a NEC-2 ground type. 4nec2
/// turns it into a perfect ground for the currents and impedance (`GN 1`)
/// plus a `GD` card so the far field uses the real ground's constants.
fn convert_mininec_ground(deck: &mut NumericDeck) {
    let mut k = 0;
    while k < deck.cards.len() {
        let card = &deck.cards[k];
        if card.is("GN") && card.ints[0] == 3 {
            let (eps, sig, line) = (card.floats[0], card.floats[1], card.line);
            let mut gn = NumCard::new("GN", &[1], &[]);
            gn.line = line;
            let mut gd = NumCard::new("GD", &[], &[eps, sig]);
            gd.line = line;
            deck.cards[k] = gn;
            deck.cards.insert(k + 1, gd);
            k += 1;
        }
        k += 1;
    }
}

/// Problems that nec2c doesn't report clearly (a zero-length wire makes it
/// run forever), caught before the deck reaches the engine.
fn check_wire(card: &NumCard, line: usize) -> Result<(), DeckError> {
    let tag = card.ints[0];
    if card.ints[1] < 1 {
        return Err(DeckError::at(
            line,
            format!(
                "wire {tag} has {} segments; it needs at least 1",
                card.ints[1]
            ),
        ));
    }
    if card.is("GW") {
        let f = &card.floats;
        let length = ((f[3] - f[0]).powi(2) + (f[4] - f[1]).powi(2) + (f[5] - f[2]).powi(2)).sqrt();
        let scale = f[..6].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        if length <= 1e-12 * scale.max(1.0) {
            return Err(DeckError::at(
                line,
                format!(
                    "wire {tag} has zero length: both ends are at ({}, {}, {})",
                    f[0], f[1], f[2]
                ),
            ));
        }
    }
    Ok(())
}

fn unsupported(line: usize, mnemonic: &str) -> DeckError {
    let why = UNSUPPORTED
        .iter()
        .find(|(m, _)| *m == mnemonic)
        .map_or("is not supported", |(_, w)| w);
    DeckError::at(line, format!("this deck {why}"))
}

/// Replaces 4nec2's `EX 6` current sources with the network trick 4nec2
/// itself uses: each source drives a tiny dummy wire far from the antenna
/// with a voltage source, and an NT network (an admittance inverter, Y12 = j)
/// turns that voltage into a fixed current on the real feed segment. The
/// layout matches 4nec2's expanded decks so results agree with 4nec2.
fn convert_current_sources(deck: &mut NumericDeck) {
    let sources: Vec<usize> = deck
        .cards
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is("EX") && c.ints[0] == 6)
        .map(|(i, _)| i)
        .collect();
    if sources.is_empty() {
        return;
    }

    let half = CURRENT_SOURCE_HALF_LENGTH / design_frequency(deck);
    let radius = half / 20.0;

    let mut dummies = Vec::new();
    let mut networks = Vec::new();
    for (k, &index) in sources.iter().enumerate() {
        let tag = 9901 + k as i64;
        deck.helper_tags.push(tag);
        let z = tag as f64;
        dummies.push(NumCard::new(
            "GW",
            &[tag, 1],
            &[-half, 0.0, z, half, 0.0, z, radius],
        ));

        let ex = &deck.cards[index];
        let (feed_tag, feed_seg) = (ex.ints[1], ex.ints[2]);
        let (i_re, i_im) = (ex.floats[0], ex.floats[1]);
        let line = ex.line;
        // V = j·I on the dummy; the network then delivers I to the feed.
        let mut volt = NumCard::new("EX", &[0, tag, 1, 0], &[-i_im, i_re]);
        volt.line = line;
        let mut net = NumCard::new(
            "NT",
            &[tag, 1, feed_tag, feed_seg],
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        );
        net.line = line;
        deck.cards[index] = volt;
        networks.push(net);
    }

    // 4nec2 puts the networks after the deck's own cards, before any
    // calculation request.
    let at = deck
        .cards
        .iter()
        .position(|c| EXECUTE_CARDS.contains(&c.mnemonic.as_str()))
        .unwrap_or(deck.cards.len());
    for (k, card) in networks.into_iter().enumerate() {
        deck.cards.insert(at + k, card);
    }

    let ge = deck
        .cards
        .iter()
        .position(|c| c.is("GE"))
        .expect("checked above");
    for (k, card) in dummies.into_iter().enumerate() {
        deck.cards.insert(ge + k, card);
    }
}

impl NumericDeck {
    /// The wires fed by 4nec2 current sources, in the order of their
    /// helper sources: the far end of each helper's NT card.
    pub fn current_fed(&self) -> Vec<i64> {
        self.cards
            .iter()
            .filter(|c| c.is("EX") && self.helper_tags.contains(&c.ints[1]))
            .filter_map(|ex| {
                self.cards
                    .iter()
                    .find(|c| c.is("NT") && c.ints[0] == ex.ints[1])
                    .map(|nt| nt.ints[2])
            })
            .collect()
    }

    /// Number of the first card of the program-control section (after GE).
    pub fn control_start(&self) -> usize {
        self.cards
            .iter()
            .position(|c| c.is("GE"))
            .map_or(self.cards.len(), |i| i + 1)
    }

    /// About how many segments (and patches) the structure has, from the
    /// geometry cards alone (no engine run): wires, arcs and helices, GR
    /// and GX copies, and patches. For refusing models too big to solve.
    pub fn segment_estimate(&self) -> i64 {
        let mut total: i64 = 0;
        for c in &self.cards[..self.control_start()] {
            let int = |k: usize| c.ints.get(k).copied().unwrap_or(0).max(0);
            match c.mnemonic.as_str() {
                "GW" | "GA" | "GH" => total += int(1),
                "SP" => total += 1,
                "SM" => total += int(0).max(1) * int(1).max(1),
                // NR copies of the whole structure so far.
                "GR" => total *= int(1).max(1),
                // One mirror image per plane flagged in the digits.
                "GX" => {
                    let planes = int(1).to_string().chars().filter(|d| *d == '1').count();
                    total <<= planes.min(3);
                }
                _ => {}
            }
        }
        total
    }

    /// True if any card asks NEC to calculate something.
    pub fn has_execute(&self) -> bool {
        self.cards
            .iter()
            .any(|c| EXECUTE_CARDS.contains(&c.mnemonic.as_str()))
    }

    /// Adds an `XQ` card if nothing in the deck asks for a calculation, so a
    /// run always produces at least the input impedance.
    pub fn ensure_execute(&mut self) {
        if !self.has_execute() {
            self.cards.push(NumCard::new("XQ", &[0], &[]));
        }
    }

    /// The frequencies (MHz) the deck's FR cards ask for, in order.
    pub fn frequencies(&self) -> Vec<f64> {
        let mut out = Vec::new();
        for c in self.cards.iter().filter(|c| c.is("FR")) {
            let n = c.ints[1].max(1);
            let (start, step) = (c.floats[0], c.floats[1]);
            for k in 0..n {
                out.push(if c.ints[0] == 1 {
                    start * step.powi(k as i32)
                } else {
                    start + step * k as f64
                });
            }
        }
        out
    }

    /// A rough count of the unknowns NEC will solve for (segments, plus two
    /// per surface patch), worked out from the geometry cards without
    /// running the engine. Good enough to estimate run times; the engine's
    /// own count is exact.
    pub fn estimated_unknowns(&self) -> usize {
        let mut n: usize = 0;
        for c in &self.cards[..self.control_start()] {
            let i1 = c.ints[0].max(0) as usize;
            let i2 = c.ints[1].max(0) as usize;
            match c.mnemonic.as_str() {
                "GW" | "GA" | "GH" => n += i2,
                "SP" => n += 2,
                "SM" => n += 2 * i1 * i2,
                // GR repeats the structure i2 times round the Z axis.
                "GR" => n *= i2.max(1),
                // GX reflects in each plane whose digit is 1 (e.g. 110).
                "GX" => {
                    let planes = i2.to_string().chars().filter(|&d| d == '1').count();
                    n <<= planes;
                }
                // GM with copies adds i2 more copies (of all or part).
                "GM" => n += n * i2,
                _ => {}
            }
        }
        n
    }

    /// True if the deck uses Sommerfeld-Norton ground (GN 2), which costs
    /// extra time at every frequency.
    pub fn uses_sommerfeld(&self) -> bool {
        self.cards.iter().any(|c| c.is("GN") && c.ints[0] == 2)
    }
}

/// A card field's name for messages: GW's coordinates by name, others by
/// number ("field 5").
fn field_name(mnemonic: &str, k: usize) -> String {
    const GW: [&str; 9] = [
        "tag", "segments", "X1", "Y1", "Z1", "X2", "Y2", "Z2", "radius",
    ];
    match mnemonic {
        "GW" if k < GW.len() => GW[k].to_string(),
        _ => format!("field {}", k + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_unknowns_from_geometry() {
        let n =
            Deck::parse("CE\nGW 1 10 0 0 0 0 0 1 .01\nGR 1 4\nGX 10 110\nSM 2 3 0 0 0 1 0 0\nGE\n")
                .expand()
                .unwrap();
        // 10 segments × 4 (GR) × 4 (two GX planes) + 2 × 6 patches.
        assert_eq!(n.estimated_unknowns(), 172);
    }

    #[test]
    fn evaluates_symbols_and_units() {
        let deck = Deck::parse(
            "CM test\nCE\nSY S = 45.78 ft\nGW 1 5 -S 0 0 -S 0 61ft 1in\nGE 1\n\
             FR 0 1 0 0 3.8\nEN\nGW 9 9 9 9 9 9 9 9 9\n",
        );
        let n = deck.expand().unwrap();
        assert_eq!(n.comments, vec!["test".to_string()]);
        let gw = &n.cards[0];
        assert_eq!(gw.ints, vec![1, 5]);
        assert!((gw.floats[0] + 13.953744).abs() < 1e-6);
        assert!((gw.floats[5] - 18.5928).abs() < 1e-9);
        assert!((gw.floats[6] - 0.0254).abs() < 1e-12);
        // Cards after EN are ignored.
        assert_eq!(n.cards.len(), 3);
    }

    #[test]
    fn symbols_can_be_redefined_in_order() {
        let deck = Deck::parse("SY A=2\nSY A=A*3\nGW 1 1 0 0 0 0 0 A .001\nGE\n");
        let n = deck.expand().unwrap();
        assert_eq!(n.cards[0].floats[5], 6.0);
        assert_eq!(n.symbols, vec![("A".to_string(), 6.0)]);
    }

    #[test]
    fn reports_line_of_bad_expression() {
        let deck = Deck::parse("CE\nGW 1 5 0 0 0 0 0 nope .001\nGE\n");
        let e = deck.expand().unwrap_err();
        assert_eq!(e.line, Some(2));
        assert!(e.message.contains("nope"), "{e}");
    }

    #[test]
    fn current_sources_match_4nec2_layout() {
        // From 4nec2's 4square example and its expanded .inp file.
        let deck = Deck::parse(
            "CE\nGW 1 5 0 0 0 0 0 18.59 .0254\nGW 2 5 1 0 0 1 0 18.59 .0254\nGE 1\n\
             EX 6 1 1 00 1.0 0.0\nEX 6 2 1 00 .0 -1.0\nFR 0 1 0 0 3.8\nEN\n",
        );
        let n = deck.expand().unwrap();
        let text: Vec<String> = n
            .cards
            .iter()
            .map(|c| format!("{} {:?} {:?}", c.mnemonic, c.ints, c.floats))
            .collect();
        assert_eq!(n.cards[2].mnemonic, "GW");
        assert_eq!(n.cards[2].ints, vec![9901, 1]);
        assert!((n.cards[2].floats[3] - 0.04023632).abs() < 1e-7, "{text:?}");
        assert!((n.cards[2].floats[6] - 2.01182e-3).abs() < 1e-8);
        assert_eq!(n.cards[3].ints, vec![9902, 1]);
        assert!(n.cards[4].is("GE"));
        // EX 0 9901 1 0 0 1 and EX 0 9902 1 0 1 0, FR, then the networks:
        // NT 9901 1 1 1 0 0 0 1 and NT 9902 1 2 1 0 0 0 1.
        assert_eq!(n.cards[5].ints, vec![0, 9901, 1, 0]);
        assert_eq!(&n.cards[5].floats[..2], &[0.0, 1.0]);
        assert_eq!(n.cards[6].ints, vec![0, 9902, 1, 0]);
        assert_eq!(&n.cards[6].floats[..2], &[1.0, 0.0]);
        assert!(n.cards[7].is("FR"));
        assert!(n.cards[8].is("NT"));
        assert_eq!(n.cards[8].ints, vec![9901, 1, 1, 1]);
        assert_eq!(n.cards[8].floats[3], 1.0);
        assert_eq!(n.cards[9].ints, vec![9902, 1, 2, 1]);
    }

    #[test]
    fn rejects_zero_length_wires() {
        let e = Deck::parse("CE\nSY a=1\nGW 7 3 a 0 0 a 0 0 .001\nGE\n")
            .expand()
            .unwrap_err();
        assert_eq!(e.line, Some(3));
        assert!(e.message.contains("wire 7 has zero length"), "{e}");
    }

    #[test]
    fn percentage_picks_segment_along_wire() {
        let deck = Deck::parse(
            "CE
GW 1 21 0 0 0 0 0 10 .01
GW 2 1 0 0 0 1 0 0 .01
GE
EX 0 1 50% 0 1
LD 0 1 30% 100% 50
EX 0 2 50% 0 1
",
        );
        let n = deck.expand().unwrap();
        assert_eq!(n.cards[3].ints[2], 11);
        assert_eq!(&n.cards[4].ints[2..], &[7, 21]);
        assert_eq!(n.cards[5].ints[2], 1);
    }

    #[test]
    fn ge_keeps_only_its_first_field() {
        let n = Deck::parse("CE\nGW 1 5 0 0 1 0 0 2 .01\nGE 0 -1 0\n")
            .expand()
            .unwrap();
        assert_eq!(n.cards[1].to_nec_line(), "GE 0");
        assert_eq!(n.warnings.len(), 1);
    }

    #[test]
    fn trap_and_coating_loads_are_converted() {
        let deck = Deck::parse(
            "CE\nGW 1 21 0 -5 10 0 5 10 1mm\nGS 0 0 1\nGE 0\n\
             LD 6 1 5 5 200 5.5e-6 45e-12\nLD 6 1 7 7 0 1e-6 0 10\nLD 7 1 0 0 2.5 2mm\nFR 0 1 0 0 7.1\n",
        );
        let n = deck.expand().unwrap();
        let ld: Vec<&NumCard> = n.cards.iter().filter(|c| c.is("LD")).collect();
        assert_eq!(ld[0].ints, vec![1, 1, 5, 5]);
        let x = 2.0 * std::f64::consts::PI * 7.1e6 * 5.5e-6;
        assert!((ld[0].floats[0] - 200.0 * x).abs() < 1e-9);
        assert_eq!(&ld[0].floats[1..3], &[5.5e-6, 45e-12]);
        // Q defaults to 100 and the trap's own frequency (10 MHz) is used.
        let x2 = 2.0 * std::f64::consts::PI * 10e6 * 1e-6;
        assert!((ld[1].floats[0] - 100.0 * x2).abs() < 1e-9);
        // Coating: eps 2.5, R/r = 2.
        let l = 2e-7 * (2.5f64 * 2.0).powf(1.0 / 12.0) * (1.0 - 1.0 / 2.5) * 2f64.ln();
        assert_eq!(ld[2].ints, vec![2, 1, 0, 0]);
        assert!((ld[2].floats[1] - l).abs() < 1e-18);
    }

    #[test]
    fn mininec_ground_becomes_perfect_ground_plus_gd() {
        let deck = Deck::parse(
            "CE
GW 1 5 0 0 0 0 0 10 .01
GE 1
GN 3 0 0 0 13 0.005
FR 0 1 0 0 3.6
",
        );
        let n = deck.expand().unwrap();
        assert_eq!(n.cards[2].to_nec_line(), "GN 1");
        assert_eq!(n.cards[3].to_nec_line(), "GD 0 0 0 0 13 0.005");
    }

    #[test]
    fn lists_sweep_frequencies() {
        let deck = Deck::parse("CE\nGW 1 1 0 0 0 0 0 1 .001\nGE\nFR 0 3 0 0 14 0.5\n");
        assert_eq!(deck.expand().unwrap().frequencies(), vec![14.0, 14.5, 15.0]);
    }
}
