//! Reading `.nec` decks, including 4nec2's extensions (`SY` symbols,
//! expressions in any field, unit suffixes, `'` comments, `EX 6` current
//! sources).
//!
//! A [`Deck`] keeps every line as written, so it can be edited and saved
//! without losing comments or formatting. [`Deck::expand`] evaluates it into
//! a plain numeric [`NumericDeck`] that any NEC-2 engine can run.

pub mod curves;
pub mod edit;
pub mod expand;
pub mod expr;
pub mod grids;
pub mod ground;
pub mod material;
pub mod patches;
pub mod plates;
pub mod radials;
pub mod reflectors;
pub mod shield;
pub mod sources;
pub mod vars;
pub mod write;

use std::fmt;
use std::path::Path;

pub use expand::{NumCard, NumericDeck};

/// A problem in a deck, tied to a line where possible.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckError {
    /// 1-based line number, if the problem is on a particular line.
    pub line: Option<usize>,
    pub message: String,
}

impl DeckError {
    pub fn at(line: usize, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            message: message.into(),
        }
    }

    pub fn general(message: impl Into<String>) -> Self {
        Self {
            line: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for DeckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(n) => write!(f, "line {n}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for DeckError {}

/// A parsed deck: every line of the file, in order.
#[derive(Debug, Clone, Default)]
pub struct Deck {
    pub lines: Vec<Line>,
}

/// One line of a deck.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// 1-based line number in the file.
    pub number: usize,
    /// The line exactly as written (without the line ending).
    pub raw: String,
    pub kind: LineKind,
    /// Text after a `'` on a card line, if any.
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LineKind {
    Blank,
    /// A whole-line `'` comment.
    Comment(String),
    /// A card whose content is free text: `CM`, `CE`, and the file name on
    /// 4nec2's `GF`/`WG` cards.
    Text {
        mnemonic: String,
        text: String,
    },
    /// `SY name=expr, name=expr, ...`
    Symbols(Vec<Assignment>),
    /// Any other card: a two-letter mnemonic and its fields as written.
    Card {
        mnemonic: String,
        fields: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub name: String,
    pub expr: String,
}

/// How a card's fields are laid out: integers first, then reals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub ints: usize,
    pub floats: usize,
}

/// Geometry cards: two integers and seven reals.
pub const GEOMETRY_CARDS: &[&str] = &[
    "GW", "GA", "GH", "GM", "GR", "GS", "GX", "GC", "GE", "SP", "SM", "SC", "GF",
];

/// Program control cards: four integers and six reals.
pub const CONTROL_CARDS: &[&str] = &[
    "FR", "LD", "GN", "EX", "NT", "TL", "XQ", "GD", "RP", "NX", "PT", "KH", "NE", "NH", "PQ", "EK",
    "CP", "PL", "EN", "WG",
];

/// The field layout of a NEC-2 card, or `None` if the card is not NEC-2.
pub fn layout(mnemonic: &str) -> Option<Layout> {
    if GEOMETRY_CARDS.contains(&mnemonic) {
        Some(Layout { ints: 2, floats: 7 })
    } else if CONTROL_CARDS.contains(&mnemonic) {
        Some(Layout { ints: 4, floats: 6 })
    } else {
        None
    }
}

/// How marker comments began before the app was called STRAWS.
const OLD_MARK: &str = concat!("nec-", "modeller: ");

impl Deck {
    /// Reads a deck from a file. Accepts UTF-8, or falls back to Latin-1 for
    /// older Windows files.
    pub fn from_file(path: impl AsRef<Path>) -> std::io::Result<Deck> {
        let bytes = std::fs::read(path)?;
        Ok(Deck::parse(&decode(&bytes)))
    }

    /// Splits deck text into lines. Never fails: problems in the content
    /// are reported later by [`Deck::expand`], with line numbers.
    pub fn parse(text: &str) -> Deck {
        // Windows (CR LF), Unix (LF) and old Mac (CR) line endings.
        let text = if text.contains('\r') {
            std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            std::borrow::Cow::Borrowed(text)
        };
        // Marker comments written before the app was called STRAWS.
        let text = if text.contains(OLD_MARK) {
            std::borrow::Cow::Owned(text.replace(OLD_MARK, "straws: "))
        } else {
            text
        };
        let lines = text
            .lines()
            .enumerate()
            .map(|(i, raw)| parse_line(i + 1, raw.trim_end_matches('\r')))
            .collect();
        Deck { lines }
    }

    /// Evaluates symbols and expressions into a plain numeric deck.
    pub fn expand(&self) -> Result<NumericDeck, DeckError> {
        expand::expand(self, &[])
    }

    /// Like [`Deck::expand`], with some symbols given new values. Each
    /// override replaces that symbol's first definition; later lines that
    /// redefine it from itself still apply.
    pub fn expand_with(&self, overrides: &[(String, f64)]) -> Result<NumericDeck, DeckError> {
        expand::expand(self, overrides)
    }

    /// The deck's variables: the first definition of each SY symbol, in
    /// order. A variable is `adjustable` if it's a plain number (with or
    /// without a unit), rather than worked out from other symbols.
    pub fn variables(&self) -> Vec<Variable> {
        let mut out: Vec<Variable> = Vec::new();
        let units = expr::Symbols::default();
        for line in &self.lines {
            if let LineKind::Symbols(assignments) = &line.kind {
                for a in assignments {
                    if a.name.is_empty() || out.iter().any(|v| v.name.eq_ignore_ascii_case(&a.name))
                    {
                        continue;
                    }
                    let literal = expr::eval(&a.expr, &units).ok();
                    out.push(Variable {
                        name: a.name.clone(),
                        line: line.number,
                        expr: a.expr.clone(),
                        literal,
                    });
                }
            }
        }
        out
    }

    /// Rewrites the first definition of `name` to `value` (in metres or
    /// whatever plain number the deck uses), keeping the rest of the line.
    /// Returns false if there's no such symbol.
    pub fn set_variable(&mut self, name: &str, value: f64) -> bool {
        for line in &mut self.lines {
            let LineKind::Symbols(assignments) = &mut line.kind else {
                continue;
            };
            let Some(a) = assignments
                .iter_mut()
                .find(|a| a.name.eq_ignore_ascii_case(name))
            else {
                continue;
            };
            a.expr = write::format_real(value);
            let mut raw = format!(
                "SY {}",
                assignments
                    .iter()
                    .map(|a| format!("{}={}", a.name, a.expr))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if let Some(c) = &line.comment {
                raw.push_str("\t' ");
                raw.push_str(c);
            }
            line.raw = raw;
            return true;
        }
        false
    }

    /// The deck as text, line by line as stored.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(&line.raw);
            // Windows line endings, as 4nec2 writes its decks.
            out.push_str("\r\n");
        }
        out
    }
}

/// One SY symbol, as first defined.
#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    pub name: String,
    pub line: usize,
    pub expr: String,
    /// The value, if the definition is a plain number (so a slider can set it).
    pub literal: Option<f64>,
}

/// Decodes file bytes as UTF-8, or Latin-1 if that fails.
pub fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn parse_line(number: usize, raw: &str) -> Line {
    let trimmed = raw.trim_start();
    let line = |kind, comment| Line {
        number,
        raw: raw.to_string(),
        kind,
        comment,
    };

    if trimmed.is_empty() {
        return line(LineKind::Blank, None);
    }
    if let Some(rest) = trimmed.strip_prefix('\'') {
        return line(LineKind::Comment(rest.to_string()), None);
    }

    let split = trimmed
        .char_indices()
        .nth(2)
        .map_or(trimmed.len(), |(i, _)| i);
    let mnemonic = trimmed[..split].to_ascii_uppercase();
    let rest = &trimmed[split..];

    match mnemonic.as_str() {
        "CM" | "CE" => line(
            LineKind::Text {
                mnemonic,
                text: rest.trim().to_string(),
            },
            None,
        ),
        _ => {
            let (body, comment) = split_comment(rest);
            if mnemonic == "SY" {
                let assignments = split_top_level(body, |c| c == ',')
                    .into_iter()
                    .map(|part| match part.split_once('=') {
                        Some((name, expr)) => Assignment {
                            name: name.trim().to_string(),
                            expr: expr.trim().to_string(),
                        },
                        None => Assignment {
                            name: part.trim().to_string(),
                            expr: String::new(),
                        },
                    })
                    .collect();
                return line(LineKind::Symbols(assignments), comment);
            }
            let fields = card_fields(body);
            // 4nec2's GF/WG cards may name a file instead of giving numbers.
            if (mnemonic == "GF" || mnemonic == "WG")
                && fields
                    .iter()
                    .any(|f| f.contains('.') && f.chars().any(|c| c.is_ascii_alphabetic()))
            {
                return line(
                    LineKind::Text {
                        mnemonic,
                        text: body.trim().to_string(),
                    },
                    comment,
                );
            }
            line(LineKind::Card { mnemonic, fields }, comment)
        }
    }
}

/// A card's fields. Spaces, tabs and commas all separate fields, except that
/// in a tab-separated line a unit after a space stays with its number (4nec2
/// decks write `-51 ft` inside one tab field; `GS 0 0 in` with spaces is
/// three fields).
fn card_fields(body: &str) -> Vec<String> {
    let split = |t: &str| split_top_level(t, |c| c.is_whitespace() || c == ',');
    if !body.contains('\t') {
        return split(body);
    }
    split_top_level(body, |c| c == '\t')
        .iter()
        .flat_map(|chunk| {
            // Only a tab field holding one value and its unit is joined:
            // `0 0 ft` before a tab is still three fields.
            let parts = split(chunk);
            if parts.len() == 2 {
                join_units(parts)
            } else {
                parts
            }
        })
        .collect()
}

/// Puts a unit written after a space back on its number.
fn join_units(fields: Vec<String>) -> Vec<String> {
    const UNITS: [&str; 10] = ["mm", "cm", "in", "ft", "pf", "nf", "uf", "nh", "uh", "mh"];
    let mut out: Vec<String> = Vec::with_capacity(fields.len());
    for f in fields {
        let unit = UNITS.contains(&f.to_ascii_lowercase().as_str());
        match out.last_mut() {
            Some(prev)
                if unit
                    && prev
                        .ends_with(|c: char| c.is_ascii_alphanumeric() || c == '.' || c == ')') =>
            {
                prev.push(' ');
                prev.push_str(&f);
            }
            _ => out.push(f),
        }
    }
    out
}

/// Splits off a trailing `'` comment.
fn split_comment(text: &str) -> (&str, Option<String>) {
    match text.find('\'') {
        Some(i) => (&text[..i], Some(text[i + 1..].trim().to_string())),
        None => (text, None),
    }
}

/// Splits on separator characters that are outside parentheses, dropping
/// empty pieces.
fn split_top_level(text: &str, is_sep: impl Fn(char) -> bool) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in text.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth <= 0 && is_sep(c) {
            if !current.trim().is_empty() {
                parts.push(current.trim().to_string());
            }
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_after_a_space_stays_with_its_number() {
        // From 4nec2's G5RV example: tab-separated fields, `-51 ft` in one.
        let deck = Deck::parse("GW\t1\t31\t0\t-51 ft\t0\t0\t51 ft\t0\t#12\nGE 0\n");
        let n = deck.expand().unwrap();
        let gw = &n.cards[0];
        assert!(n.warnings.is_empty(), "{:?}", n.warnings);
        assert!(
            (gw.floats[1] + 51.0 * 0.3048).abs() < 1e-9,
            "{:?}",
            gw.floats
        );
        assert!(
            (gw.floats[4] - 51.0 * 0.3048).abs() < 1e-9,
            "{:?}",
            gw.floats
        );
        assert_eq!(gw.floats[2], 0.0);
        // With spaces only, a unit is a field of its own (a GS scale).
        let n = Deck::parse("GW 1 3 0 0 0 0 0 10 .1\nGS 0 0 in\nGE 0\n")
            .expand()
            .unwrap();
        assert_eq!(n.cards[1].floats[0], 0.0254);
        // Also with tabs before a comment (from 4nec2's BYDipole example).
        let n = Deck::parse("GW 1 3 0 0 0 0 0 10 .1\nGS 0 0 ft\t\t' All in ft.\nGE 0\n")
            .expand()
            .unwrap();
        assert_eq!(n.cards[1].floats[0], 0.3048);
    }

    #[test]
    fn parses_cards_symbols_and_comments() {
        let deck = Deck::parse(
            "CM test\nCE\nSY len=20, Z=len*cos(55)  ' note\n\
             GW\t1\t20\t-X\t0\thgh-Z\t-0.1\t0\thgh\t#12 ' radius\n'GN -1\ngn 2,0,0,0,14,.006\n",
        );
        assert_eq!(deck.lines.len(), 6);
        assert_eq!(
            deck.lines[2].kind,
            LineKind::Symbols(vec![
                Assignment {
                    name: "len".into(),
                    expr: "20".into()
                },
                Assignment {
                    name: "Z".into(),
                    expr: "len*cos(55)".into()
                },
            ])
        );
        assert_eq!(deck.lines[2].comment.as_deref(), Some("note"));
        match &deck.lines[3].kind {
            LineKind::Card { mnemonic, fields } => {
                assert_eq!(mnemonic, "GW");
                assert_eq!(fields.len(), 9);
                assert_eq!(fields[4], "hgh-Z");
                assert_eq!(fields[8], "#12");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(deck.lines[4].kind, LineKind::Comment("GN -1".into()));
        match &deck.lines[5].kind {
            LineKind::Card { mnemonic, fields } => {
                assert_eq!(mnemonic, "GN");
                assert_eq!(fields, &["2", "0", "0", "0", "14", ".006"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn variables_can_be_listed_overridden_and_saved() {
        let mut deck = Deck::parse(
            "CE
SY len=10, h = 5 ft
SY half=len/2  ' derived
SY len=len*1
GW 1 9 0 -half h 0 half h .001
GE
",
        );
        let vars = deck.variables();
        assert_eq!(vars.len(), 3);
        assert_eq!(vars[0].literal, Some(10.0));
        assert!((vars[1].literal.unwrap() - 1.524).abs() < 1e-12);
        assert_eq!(vars[2].literal, None);
        let n = deck.expand_with(&[("LEN".to_string(), 12.0)]).unwrap();
        assert_eq!(n.cards[0].floats[4], 6.0);
        assert!(deck.set_variable("len", 11.5));
        assert_eq!(deck.lines[1].raw, "SY len=11.5, h=5 ft");
        assert!(
            deck.to_text()
                .starts_with("CE\r\nSY len=11.5, h=5 ft\r\nSY half=len/2")
        );
        let reparsed = Deck::parse(&deck.to_text());
        assert_eq!(reparsed.expand().unwrap().cards[0].floats[4], 5.75);
    }

    #[test]
    fn commas_inside_functions_do_not_split() {
        let deck = Deck::parse("SY a=mod(7,3), b=2\n");
        match &deck.lines[0].kind {
            LineKind::Symbols(a) => {
                assert_eq!(a.len(), 2);
                assert_eq!(a[0].expr, "mod(7,3)");
            }
            other => panic!("{other:?}"),
        }
    }
}
