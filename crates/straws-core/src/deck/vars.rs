//! Making and managing variables without writing deck text: create one from
//! any number, rename it everywhere, delete it (writing its value back in),
//! keep notes with it, and drive measurements (a wire's length, the spacing
//! between wires, the height, an angle) from one.
//!
//! Everything stays ordinary 4nec2 deck text: a variable is an `SY` line,
//! and its notes go in that line's comment, e.g.
//! `SY refl=1.05  ' Reflector length [unit=m group=Reflector range=0.9..1.2]`.
//! 4nec2 ignores comments, so files stay compatible.

use super::edit::{Wire, plain_number};
use super::expr::{self, RESERVED};
use super::ground::HEIGHT_MARK;
use super::material::{TUBE_MARK, tube_wall_text};
use super::write::format_real;
use super::{Assignment, Deck, LineKind};

/// What's kept with a variable besides its value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VarNotes {
    pub description: String,
    /// `m` means a length in metres (shown in the chosen length unit);
    /// anything else (`deg`, `MHz`, `mm`, ...) is shown as written.
    pub unit: String,
    /// A heading to list it under, e.g. "Reflector".
    pub group: String,
    /// The slider's range.
    pub range: Option<(f64, f64)>,
    /// The slider's step.
    pub step: Option<f64>,
    /// The optimiser leaves it alone.
    pub fixed: bool,
    /// A setting outside the deck's cards that this variable sets, e.g.
    /// `feed_length` for the Feedline view (empty: none).
    pub role: String,
}

impl VarNotes {
    /// Reads notes from an SY line's comment. A comment without our
    /// `[...]` part is all description (as in decks from 4nec2).
    pub fn parse(comment: &str) -> VarNotes {
        let comment = comment.trim();
        let mut notes = VarNotes::default();
        if let Some(inner) = comment.strip_suffix(']')
            && let Some(open) = inner.rfind('[')
        {
            let mut parsed = VarNotes {
                description: inner[..open].trim().to_string(),
                ..VarNotes::default()
            };
            if words(&inner[open + 1..])
                .iter()
                .all(|w| parsed.read_word(w))
            {
                return parsed;
            }
        }
        notes.description = comment.to_string();
        notes
    }

    fn read_word(&mut self, word: &str) -> bool {
        if word == "fixed" {
            self.fixed = true;
            return true;
        }
        let Some((key, value)) = word.split_once('=') else {
            return false;
        };
        match key {
            "unit" => self.unit = value.to_string(),
            "group" => self.group = value.to_string(),
            "use" => self.role = value.to_string(),
            "step" => match plain_number(value) {
                Some(s) => self.step = Some(s),
                None => return false,
            },
            "range" => match value
                .split_once("..")
                .and_then(|(a, b)| Some((plain_number(a)?, plain_number(b)?)))
            {
                Some(r) => self.range = Some(r),
                None => return false,
            },
            _ => return false,
        }
        true
    }

    /// The comment to write, or `None` if there's nothing to keep.
    pub fn to_comment(&self) -> Option<String> {
        let mut items = Vec::new();
        let quote = |s: &str| {
            let s: String = s
                .chars()
                .filter(|c| !matches!(c, '"' | '[' | ']'))
                .collect();
            if s.contains(char::is_whitespace) {
                format!("\"{s}\"")
            } else {
                s
            }
        };
        if !self.unit.trim().is_empty() {
            items.push(format!("unit={}", quote(self.unit.trim())));
        }
        if !self.group.trim().is_empty() {
            items.push(format!("group={}", quote(self.group.trim())));
        }
        if let Some((a, b)) = self.range {
            items.push(format!("range={}..{}", format_real(a), format_real(b)));
        }
        if let Some(s) = self.step {
            items.push(format!("step={}", format_real(s)));
        }
        if self.fixed {
            items.push("fixed".into());
        }
        if !self.role.trim().is_empty() {
            items.push(format!("use={}", quote(self.role.trim())));
        }
        let description = self.description.trim();
        match (description.is_empty(), items.is_empty()) {
            (true, true) => None,
            (false, true) => Some(description.to_string()),
            (true, false) => Some(format!("[{}]", items.join(" "))),
            (false, false) => Some(format!("{description} [{}]", items.join(" "))),
        }
    }
}

/// Splits on spaces outside double quotes, dropping the quotes.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Where a wire's length is measured from when a variable drives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// Both ends move; the middle stays put.
    Centre,
    /// The start (X1, Y1, Z1) stays put.
    Start,
    /// The end (X2, Y2, Z2) stays put.
    End,
}

/// An SY line's text.
fn symbols_raw(assignments: &[Assignment], comment: Option<&str>) -> String {
    let mut raw = format!(
        "SY {}",
        assignments
            .iter()
            .map(|a| format!("{}={}", a.name, a.expr))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(c) = comment {
        raw.push_str("\t' ");
        raw.push_str(c);
    }
    raw
}

/// Splits a raw card line into (before the fields, the fields, the comment),
/// so names in the fields can be changed without touching the rest.
fn split_raw(raw: &str) -> (&str, &str, &str) {
    let lead = raw.len() - raw.trim_start().len();
    let body = raw[lead..]
        .char_indices()
        .nth(2)
        .map_or(raw.len(), |(i, _)| lead + i);
    let end = raw[body..].find('\'').map_or(raw.len(), |i| body + i);
    (&raw[..body], &raw[body..end], &raw[end..])
}

/// A raw line's comment part (`tail`, from the `'`) with `from` renamed in
/// a tube wall formula, the one comment that holds a formula.
fn replace_in_wall(tail: &str, from: &str, to: &str) -> String {
    match tail.find(TUBE_MARK) {
        Some(i) => {
            let at = i + TUBE_MARK.len();
            format!(
                "{}{}",
                &tail[..at],
                expr::replace_name(&tail[at..], from, to)
            )
        }
        None => tail.to_string(),
    }
}

/// `base + coef × term` as tidy formula text.
fn lin(base: f64, coef: f64, term: &str) -> String {
    let base = if base.abs() < 1e-12 {
        String::new()
    } else {
        format_real(base)
    };
    lin_text(&base, coef, term)
}

/// `base + coef × term`, with `base` written as text (empty for zero). A
/// formula base is kept as it is, so whatever it depends on still applies.
fn lin_text(base: &str, coef: f64, term: &str) -> String {
    let near = |x: f64, y: f64| (x - y).abs() < 1e-12;
    let t = if near(coef, 0.0) {
        String::new()
    } else if near(coef, 1.0) {
        term.to_string()
    } else if near(coef, -1.0) {
        format!("-{term}")
    } else if near(coef, 0.5) {
        format!("{term}/2")
    } else if near(coef, -0.5) {
        format!("-{term}/2")
    } else {
        format!("{}*{term}", format_real(coef))
    };
    let base = if base.chars().skip(1).any(|c| "+-*/\\^<>= ".contains(c)) {
        format!("({base})")
    } else {
        base.to_string()
    };
    match (base.is_empty(), t.is_empty()) {
        (true, true) => "0".into(),
        (false, true) => base,
        (true, false) => t,
        (false, false) if t.starts_with('-') => format!("{base}{t}"),
        (false, false) => format!("{base}+{t}"),
    }
}

const COORD: [&str; 6] = ["X1", "Y1", "Z1", "X2", "Y2", "Z2"];

/// One coordinate a new variable will drive.
struct Move {
    /// Where the end is now (metres).
    at: [f64; 3],
    /// The coordinate (0-2): its value becomes `base + coef × term` metres.
    k: usize,
    base: f64,
    coef: f64,
    term: String,
    /// The other end's coordinate as written, if it's a formula and the
    /// base comes from it.
    from_text: Option<String>,
    /// The driven wire's GS scale and GM shift along `k`.
    scale: f64,
    offset: f64,
}

impl Move {
    /// The field text for this coordinate on a wire with `scale` and
    /// `offset` (along `k`).
    fn text(&self, scale: f64, offset: f64) -> String {
        match &self.from_text {
            Some(t) if scale == self.scale && (offset - self.offset).abs() < 1e-12 => {
                lin_text(t, self.coef / scale, &self.term)
            }
            _ => lin((self.base - offset) / scale, self.coef / scale, &self.term),
        }
    }
}

/// Planned field changes, (line, field, new text), and the coordinates
/// they drive (so joined wire ends can follow).
#[derive(Default)]
struct Plan {
    changes: Vec<(usize, usize, String)>,
    moves: Vec<Move>,
}

/// Sets coordinate `k` (0-2) of end `end` of `wire` to `base + coef × term`
/// metres, checking the field can change. If the base is the other end's
/// coordinate (`from`, a field) and that's a formula, the formula is used,
/// so the end stays linked to it.
#[allow(clippy::too_many_arguments)]
fn set_coord(
    plan: &mut Plan,
    wire: &Wire,
    end: usize,
    k: usize,
    base: f64,
    coef: f64,
    term: &str,
    from: Option<usize>,
) -> Result<(), String> {
    if coef.abs() < 1e-12 {
        return Ok(());
    }
    let field = 2 + end * 3 + k;
    if !wire.is_number(field) {
        return Err(format!(
            "wire {}'s {} is already a formula ({}); type a number in the wire table first",
            wire.tag,
            COORD[end * 3 + k],
            wire.fields[field]
        ));
    }
    let m = Move {
        at: wire.end(end),
        k,
        base,
        coef,
        term: term.to_string(),
        from_text: from
            .filter(|&f| !wire.is_number(f))
            .map(|f| wire.fields[f].clone()),
        scale: wire.scale,
        offset: wire.offset[k],
    };
    plan.changes
        .push((wire.line, field, m.text(wire.scale, wire.offset[k])));
    plan.moves.push(m);
    Ok(())
}

fn movable(wire: &Wire) -> Result<(), String> {
    if wire.moved {
        Err(format!(
            "wire {} is moved by a GM card, so its position can't be driven from here",
            wire.tag
        ))
    } else {
        Ok(())
    }
}

fn centre(w: &Wire) -> [f64; 3] {
    [0, 1, 2].map(|k| (w.a[k] + w.b[k]) / 2.0)
}

impl Deck {
    /// Every definition of `name`: (line index, position on the line).
    fn definitions(&self, name: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            if let LineKind::Symbols(a) = &line.kind {
                for (k, a) in a.iter().enumerate() {
                    if a.name.eq_ignore_ascii_case(name) {
                        out.push((i, k));
                    }
                }
            }
        }
        out
    }

    /// Whether `name` is an SY variable.
    pub fn has_variable(&self, name: &str) -> bool {
        !self.definitions(name).is_empty()
    }

    /// Checks `name` can be a new variable.
    pub fn check_new_name(&self, name: &str) -> Result<(), String> {
        let mut chars = name.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!(
                "\"{name}\" can't be a name: use letters, digits and _ only, starting with a letter"
            ));
        }
        if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(format!(
                "\"{name}\" is already a function or unit in formulas; pick another name"
            ));
        }
        if self.has_variable(name) {
            return Err(format!("there's already a variable called \"{name}\""));
        }
        Ok(())
    }

    /// `base`, or `base_2`, `base_3`, ... if that's taken.
    pub fn unique_name(&self, base: &str) -> String {
        let mut clean: String = base
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        if !clean.starts_with(|c: char| c.is_ascii_alphabetic()) {
            clean.insert(0, 'v');
        }
        if self.check_new_name(&clean).is_ok() {
            return clean;
        }
        (2..)
            .map(|n| format!("{clean}_{n}"))
            .find(|n| self.check_new_name(n).is_ok())
            .unwrap_or(clean)
    }

    /// The notes kept with variable `name`. Notes belong to a line that
    /// defines only that variable.
    pub fn notes(&self, name: &str) -> VarNotes {
        let Some(&(line, _)) = self.definitions(name).first() else {
            return VarNotes::default();
        };
        let l = &self.lines[line];
        match &l.kind {
            LineKind::Symbols(a) if a.len() == 1 => l
                .comment
                .as_deref()
                .map(VarNotes::parse)
                .unwrap_or_default(),
            _ => VarNotes::default(),
        }
    }

    /// Replaces the notes of `name`, giving it a line of its own if it
    /// shared one. Returns false if there's no such variable.
    pub fn set_notes(&mut self, name: &str, notes: &VarNotes) -> bool {
        let Some(&(line, _)) = self.definitions(name).first() else {
            return false;
        };
        self.split_symbols(line);
        let Some(&(line, k)) = self.definitions(name).first() else {
            return false;
        };
        if let LineKind::Symbols(a) = &self.lines[line].kind {
            self.lines[line].raw = symbols_raw(&a[k..=k], notes.to_comment().as_deref());
        }
        self.reparse();
        true
    }

    /// Puts each assignment of an SY line on a line of its own (in the
    /// same order, so nothing changes meaning). The comment stays with the
    /// first one.
    fn split_symbols(&mut self, line: usize) {
        let l = &self.lines[line];
        let LineKind::Symbols(a) = &l.kind else {
            return;
        };
        if a.len() < 2 {
            return;
        }
        let raws: Vec<String> = a
            .iter()
            .enumerate()
            .map(|(k, one)| {
                symbols_raw(
                    std::slice::from_ref(one),
                    if k == 0 { l.comment.as_deref() } else { None },
                )
            })
            .collect();
        self.remove_lines(&[line]);
        self.insert_lines(line, &raws);
    }

    /// Where new variables go: after the SY lines at the top of the deck,
    /// or just before the first card.
    pub(crate) fn variables_line(&self) -> usize {
        let first_card = self
            .lines
            .iter()
            .position(|l| matches!(l.kind, LineKind::Card { .. }))
            .unwrap_or(self.lines.len());
        self.lines[..first_card]
            .iter()
            .rposition(|l| matches!(l.kind, LineKind::Symbols(_)))
            .map_or(first_card, |i| i + 1)
    }

    /// Adds `SY name=expr` with its notes. Returns the new line's index.
    pub fn add_variable(
        &mut self,
        name: &str,
        expr: &str,
        notes: &VarNotes,
    ) -> Result<usize, String> {
        self.check_new_name(name)?;
        if expr.trim().is_empty() {
            return Err("a variable needs a value".into());
        }
        let at = self.variables_line();
        let a = Assignment {
            name: name.to_string(),
            expr: expr.split_whitespace().collect::<Vec<_>>().join(" "),
        };
        self.insert_lines(at, &[symbols_raw(&[a], notes.to_comment().as_deref())]);
        Ok(at)
    }

    /// Rewrites the first definition of `name` as `expr` (a number or a
    /// formula). Returns false if there's no such variable.
    pub fn set_expression(&mut self, name: &str, expr: &str) -> bool {
        let Some(&(line, k)) = self.definitions(name).first() else {
            return false;
        };
        let l = &mut self.lines[line];
        let mut alone = false;
        if let LineKind::Symbols(a) = &mut l.kind {
            a[k].expr = expr.split_whitespace().collect::<Vec<_>>().join(" ");
            l.raw = symbols_raw(a, l.comment.as_deref());
            alone = a.len() == 1;
        }
        self.reparse();
        // Variables are worked out in order: one whose formula now uses a
        // variable defined further down moves to just below it.
        let latest = expr::names_in(expr)
            .iter()
            .filter(|u| !u.eq_ignore_ascii_case(name))
            .filter_map(|u| self.definitions(u).first().map(|d| d.0))
            .max();
        if let Some(last) = latest
            && last > line
            && alone
        {
            let raw = self.lines[line].raw.clone();
            self.remove_lines(&[line]);
            self.insert_lines(last, &[raw]);
        }
        true
    }

    /// Renames a variable everywhere: its own definitions and every formula
    /// that uses it. Comments are left as they are.
    pub fn rename_variable(&mut self, old: &str, new: &str) -> Result<(), String> {
        if !self.has_variable(old) {
            return Err(format!("there's no variable called \"{old}\""));
        }
        if !old.eq_ignore_ascii_case(new) {
            self.check_new_name(new)?;
        } else if old == new {
            return Ok(());
        }
        for line in &mut self.lines {
            if matches!(line.kind, LineKind::Symbols(_) | LineKind::Card { .. }) {
                let (head, body, tail) = split_raw(&line.raw);
                line.raw = format!(
                    "{head}{}{}",
                    expr::replace_name(body, old, new),
                    replace_in_wall(tail, old, new)
                );
            }
        }
        self.reparse();
        Ok(())
    }

    /// Lines (indices) that use `name` directly, other than its own
    /// definitions.
    pub fn uses(&self, name: &str) -> Vec<usize> {
        let lower = name.to_ascii_lowercase();
        let mut out = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            let used = match &line.kind {
                LineKind::Symbols(a) => a.iter().any(|a| {
                    !a.name.eq_ignore_ascii_case(name) && expr::names_in(&a.expr).contains(&lower)
                }),
                LineKind::Card { fields, .. } => {
                    fields.iter().any(|f| expr::names_in(f).contains(&lower))
                        || tube_wall_text(line.comment.as_deref())
                            .is_some_and(|t| expr::names_in(t).contains(&lower))
                }
                _ => false,
            };
            if used {
                out.push(i);
            }
        }
        out
    }

    /// Card lines (indices) that depend on `name`, directly or through
    /// other variables worked out from it.
    pub fn uses_through(&self, name: &str) -> Vec<usize> {
        let mut names = vec![name.to_ascii_lowercase()];
        loop {
            let before = names.len();
            for line in &self.lines {
                if let LineKind::Symbols(a) = &line.kind {
                    for a in a {
                        let n = a.name.to_ascii_lowercase();
                        if !names.contains(&n)
                            && expr::names_in(&a.expr).iter().any(|u| names.contains(u))
                        {
                            names.push(n);
                        }
                    }
                }
            }
            if names.len() == before {
                break;
            }
        }
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, l)| match &l.kind {
                LineKind::Card { fields, .. } => fields
                    .iter()
                    .map(String::as_str)
                    .chain(tube_wall_text(l.comment.as_deref()))
                    .any(|f| expr::names_in(f).iter().any(|u| names.contains(u))),
                _ => false,
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Deletes a variable. If something still uses it, `bake` is the value
    /// written in its place (`None` refuses instead).
    pub fn delete_variable(&mut self, name: &str, bake: Option<f64>) -> Result<(), String> {
        let defs = self.definitions(name);
        match defs.len() {
            0 => return Err(format!("there's no variable called \"{name}\"")),
            1 => {}
            _ => {
                return Err(format!(
                    "\"{name}\" is set on more than one SY line; delete it in the Deck view"
                ));
            }
        }
        let used = self.uses(name);
        if !used.is_empty() {
            let Some(value) = bake else {
                return Err(format!(
                    "\"{name}\" is still used on {} line{}",
                    used.len(),
                    if used.len() == 1 { "" } else { "s" }
                ));
            };
            let text = if value < 0.0 {
                format!("({})", format_real(value))
            } else {
                format_real(value)
            };
            for &i in &used {
                let line = &mut self.lines[i];
                let (head, body, tail) = split_raw(&line.raw);
                line.raw = format!(
                    "{head}{}{}",
                    expr::replace_name(body, name, &text),
                    replace_in_wall(tail, name, &text)
                );
            }
        }
        let (line, k) = defs[0];
        let l = &self.lines[line];
        if let LineKind::Symbols(a) = &l.kind {
            if a.len() == 1 {
                self.remove_lines(&[line]);
                return Ok(());
            }
            let mut rest = a.clone();
            rest.remove(k);
            self.lines[line].raw = symbols_raw(&rest, l.comment.as_deref());
        }
        self.reparse();
        Ok(())
    }

    /// The variable a card field is set to, if the field is just its name.
    pub fn field_variable(&self, line: usize, field: usize) -> Option<String> {
        let LineKind::Card { fields, .. } = &self.lines.get(line)?.kind else {
            return None;
        };
        let text = fields.get(field)?.trim();
        let spans = expr::name_spans(text);
        (spans.len() == 1 && spans[0] == (0..text.len()) && self.has_variable(text))
            .then(|| text.to_string())
    }

    /// Turns the number in a card field into a new variable: adds
    /// `SY name=number` and puts the name in the field.
    pub fn make_field_variable(
        &mut self,
        line: usize,
        field: usize,
        name: &str,
        notes: &VarNotes,
    ) -> Result<(), String> {
        self.check_new_name(name)?;
        let Some(LineKind::Card { fields, .. }) = self.lines.get(line).map(|l| &l.kind) else {
            return Err("that isn't a card line".into());
        };
        let text = fields.get(field).cloned().unwrap_or_else(|| "0".into());
        if !expr::names_in(&text).is_empty() {
            return Err(format!(
                "that number is already a formula ({text}); change the variables it uses instead"
            ));
        }
        expr::eval(&text, &expr::Symbols::default()).map_err(|e| e.to_string())?;
        self.set_field(line, field, name);
        self.add_variable(name, &text, notes).map(|_| ())
    }

    /// Makes the planned changes and adds the variable. Ends of `joined`
    /// wires at a point that moves follow it, so junctions stay joined.
    /// Makes the wall thickness of the tube set by LD line `line` a new
    /// variable (metres).
    pub fn make_wall_variable(
        &mut self,
        line: usize,
        name: &str,
        notes: &VarNotes,
    ) -> Result<(), String> {
        self.check_new_name(name)?;
        let l = self.lines.get(line).ok_or("that line has gone")?;
        let text = tube_wall_text(l.comment.as_deref())
            .ok_or("that wire isn't a hollow tube")?
            .to_string();
        if !expr::names_in(&text).is_empty() {
            return Err(format!("the wall is already a formula ({text})"));
        }
        let comment = format!("{TUBE_MARK} {name}");
        let LineKind::Card { mnemonic, fields } = &l.kind else {
            return Err("that isn't a card line".into());
        };
        self.lines[line].raw = super::edit::card_line(mnemonic, fields, Some(&comment));
        self.reparse();
        self.add_variable(name, &text, notes).map(|_| ())
    }

    /// Makes coordinate `field` (2-7) of `wire` a new variable; ends of
    /// `joined` wires at that point follow it.
    pub fn make_coord_variable(
        &mut self,
        wire: &Wire,
        field: usize,
        joined: &[Wire],
        name: &str,
        notes: &VarNotes,
    ) -> Result<(), String> {
        if !(2..=7).contains(&field) {
            return Err("that isn't a coordinate".into());
        }
        let value = plain_number(&wire.fields[field]).ok_or_else(|| {
            format!(
                "that number is already a formula ({}); change the variables it uses instead",
                wire.fields[field]
            )
        })?;
        movable(wire)?;
        let (end, k) = ((field - 2) / 3, (field - 2) % 3);
        let mut plan = Plan::default();
        // In metres: name × scale + offset, so the field is just the name.
        set_coord(
            &mut plan,
            wire,
            end,
            k,
            wire.offset[k],
            wire.scale,
            name,
            None,
        )?;
        self.apply(name, value, notes, plan, joined)
    }

    fn apply(
        &mut self,
        name: &str,
        value: f64,
        notes: &VarNotes,
        plan: Plan,
        joined: &[Wire],
    ) -> Result<(), String> {
        self.check_new_name(name)?;
        let mut changes = plan.changes;
        for m in &plan.moves {
            let near = 1e-6 * m.at.iter().fold(1.0_f64, |a, x| a.max(x.abs()));
            for w in joined.iter().filter(|w| !w.moved) {
                for end in 0..2 {
                    let p = w.end(end);
                    if (0..3).any(|i| (p[i] - m.at[i]).abs() > near) {
                        continue;
                    }
                    let field = 2 + end * 3 + m.k;
                    if changes.iter().any(|c| c.0 == w.line && c.1 == field) {
                        continue;
                    }
                    if !w.is_number(field) {
                        return Err(format!(
                            "wire {} is joined there, but its {} is a formula ({}); turn off \
                             \"move joined ends too\", or type a number there first",
                            w.tag,
                            COORD[end * 3 + m.k],
                            w.fields[field]
                        ));
                    }
                    changes.push((w.line, field, m.text(w.scale, w.offset[m.k])));
                }
            }
        }
        for (line, field, text) in changes {
            self.set_field(line, field, &text);
        }
        self.add_variable(name, &format_real(value), notes)?;
        // Curves on the ends that moved follow them too.
        if !joined.is_empty() {
            self.link_curves();
        }
        Ok(())
    }

    /// A new variable (metres) for the length of `wires[0]`, driving the
    /// length of every wire given (so several wires share one length).
    pub fn drive_length(
        &mut self,
        wires: &[Wire],
        joined: &[Wire],
        anchor: Anchor,
        name: &str,
        notes: &VarNotes,
    ) -> Result<(), String> {
        let first = wires.first().ok_or("select a wire first")?;
        let value = first.length();
        let mut changes = Plan::default();
        for w in wires {
            movable(w)?;
            let len = w.length();
            if len < 1e-12 {
                return Err(format!("wire {} has no length", w.tag));
            }
            let c = centre(w);
            for (k, &ck) in c.iter().enumerate() {
                let u = (w.b[k] - w.a[k]) / len;
                match anchor {
                    Anchor::Centre => {
                        set_coord(&mut changes, w, 0, k, ck, -u / 2.0, name, None)?;
                        set_coord(&mut changes, w, 1, k, ck, u / 2.0, name, None)?;
                    }
                    Anchor::Start => {
                        set_coord(&mut changes, w, 1, k, w.a[k], u, name, Some(2 + k))?
                    }
                    Anchor::End => set_coord(&mut changes, w, 0, k, w.b[k], -u, name, Some(5 + k))?,
                }
            }
        }
        self.apply(name, value, notes, changes, joined)
    }

    /// A new variable (metres) for the spacing between wires. Along an
    /// axis (0-2), the wires are spaced equally, the first (lowest along
    /// the axis) staying put; with no axis, exactly two wires, and the
    /// second moves along the line between their centres.
    pub fn drive_spacing(
        &mut self,
        wires: &[Wire],
        joined: &[Wire],
        axis: Option<usize>,
        name: &str,
        notes: &VarNotes,
    ) -> Result<(), String> {
        if wires.len() < 2 {
            return Err("select at least two wires".into());
        }
        let mut changes = Plan::default();
        let value = match axis {
            Some(k) => {
                let mut sorted = wires.to_vec();
                sorted.sort_by(|p, q| centre(p)[k].total_cmp(&centre(q)[k]));
                let c0 = centre(&sorted[0])[k];
                let span = centre(&sorted[sorted.len() - 1])[k] - c0;
                if span.abs() < 1e-9 {
                    return Err(format!(
                        "the wires are all at the same {} position",
                        ["x", "y", "z"][k]
                    ));
                }
                for (i, w) in sorted.iter().enumerate().skip(1) {
                    movable(w)?;
                    let ci = centre(w)[k];
                    for end in 0..2 {
                        let base = c0 + (w.end(end)[k] - ci);
                        set_coord(&mut changes, w, end, k, base, i as f64, name, None)?;
                    }
                }
                span / (sorted.len() - 1) as f64
            }
            None => {
                if wires.len() != 2 {
                    return Err("select exactly two wires for the distance between them".into());
                }
                let (c1, c2) = (centre(&wires[0]), centre(&wires[1]));
                let d = [0, 1, 2].map(|k| c2[k] - c1[k]);
                let dist = d.iter().map(|x| x * x).sum::<f64>().sqrt();
                if dist < 1e-9 {
                    return Err("the two wires have the same centre".into());
                }
                let w = &wires[1];
                movable(w)?;
                for (k, dk) in d.iter().enumerate() {
                    let u = dk / dist;
                    for end in 0..2 {
                        let base = w.end(end)[k] - u * dist;
                        set_coord(&mut changes, w, end, k, base, u, name, None)?;
                    }
                }
                dist
            }
        };
        self.apply(name, value, notes, changes, joined)
    }

    /// A new variable (metres) for the height of the lowest point of these
    /// wires; they all move up and down with it.
    pub fn drive_height(
        &mut self,
        wires: &[Wire],
        joined: &[Wire],
        name: &str,
        notes: &VarNotes,
    ) -> Result<(), String> {
        let low = wires
            .iter()
            .flat_map(|w| [w.a[2], w.b[2]])
            .fold(f64::MAX, f64::min);
        if wires.is_empty() {
            return Err("select a wire first".into());
        }
        let mut changes = Plan::default();
        for w in wires {
            movable(w)?;
            for end in 0..2 {
                set_coord(
                    &mut changes,
                    w,
                    end,
                    2,
                    w.end(end)[2] - low,
                    1.0,
                    name,
                    None,
                )?;
            }
        }
        self.apply(name, low, notes, changes, joined)
    }

    /// A new variable (a scale factor, 1 now) for the size of `wires`:
    /// every end becomes centre + (end - centre) x scale, about the middle
    /// of their bounding box, so the whole group grows or shrinks together
    /// (an element's size for the optimiser).
    pub fn drive_scale(
        &mut self,
        wires: &[Wire],
        joined: &[Wire],
        name: &str,
        notes: &VarNotes,
    ) -> Result<(), String> {
        if wires.is_empty() {
            return Err("select some wires first".into());
        }
        let ends: Vec<[f64; 3]> = wires.iter().flat_map(|w| [w.a, w.b]).collect();
        let centre = [0, 1, 2].map(|k| {
            let lo = ends.iter().map(|e| e[k]).fold(f64::MAX, f64::min);
            let hi = ends.iter().map(|e| e[k]).fold(f64::MIN, f64::max);
            let c = (lo + hi) / 2.0;
            // No floating-point dust in the formulas.
            (c * 1e9).round() / 1e9
        });
        let mut changes = Plan::default();
        for w in wires {
            movable(w)?;
            for end in 0..2 {
                for (k, &c) in centre.iter().enumerate() {
                    let d = ((w.end(end)[k] - c) * 1e9).round() / 1e9;
                    set_coord(&mut changes, w, end, k, c, d, name, None)?;
                }
            }
        }
        self.apply(name, 1.0, notes, changes, joined)
    }

    /// The line of the GM card that sets the whole antenna's height.
    pub fn height_line(&self) -> Option<usize> {
        self.lines.iter().position(|l| {
            matches!(&l.kind, LineKind::Card { mnemonic, .. } if mnemonic == "GM")
                && l.comment.as_deref() == Some(HEIGHT_MARK)
        })
    }

    /// A new variable (metres) for the height of the whole antenna's lowest
    /// point (`lowest` metres now), set through the height GM card.
    pub fn drive_height_shift(
        &mut self,
        lowest: f64,
        name: &str,
        notes: &VarNotes,
    ) -> Result<(), String> {
        self.check_new_name(name)?;
        if let Some(line) = self.height_line()
            && let LineKind::Card { fields, .. } = &self.lines[line].kind
            && fields.get(7).is_some_and(|f| plain_number(f).is_none())
        {
            return Err(format!(
                "the height is already set by a formula ({})",
                fields[7]
            ));
        }
        let shift = self.height_shift();
        if self.height_line().is_none() {
            self.set_height_shift(1.0);
        }
        let line = self.height_line().ok_or("the deck has no GE card")?;
        self.set_field(line, 7, &lin(shift - lowest, 1.0, name));
        self.add_variable(name, &format_real(lowest), notes)
            .map(|_| ())
    }

    /// A new variable (degrees) for the angle of `wire` above the
    /// horizontal, pivoting about end `pivot` (0 = start, 1 = end); its
    /// length and direction across the ground stay the same.
    pub fn drive_angle(
        &mut self,
        wire: &Wire,
        joined: &[Wire],
        pivot: usize,
        name: &str,
        notes: &VarNotes,
    ) -> Result<(), String> {
        movable(wire)?;
        let a = wire.end(pivot);
        let b = wire.end(1 - pivot);
        let d = [0, 1, 2].map(|k| b[k] - a[k]);
        let flat = d[0].hypot(d[1]);
        let len = (flat * flat + d[2] * d[2]).sqrt();
        if len < 1e-12 {
            return Err(format!("wire {} has no length", wire.tag));
        }
        let h = if flat < 1e-9 {
            [1.0, 0.0]
        } else {
            [d[0] / flat, d[1] / flat]
        };
        let angle = d[2].atan2(flat).to_degrees();
        let mut changes = Plan::default();
        let moving = 1 - pivot;
        let cos = format!("cos({name})");
        set_coord(
            &mut changes,
            wire,
            moving,
            0,
            a[0],
            len * h[0],
            &cos,
            Some(2 + pivot * 3),
        )?;
        set_coord(
            &mut changes,
            wire,
            moving,
            1,
            a[1],
            len * h[1],
            &cos,
            Some(3 + pivot * 3),
        )?;
        set_coord(
            &mut changes,
            wire,
            moving,
            2,
            a[2],
            len,
            &format!("sin({name})"),
            Some(4 + pivot * 3),
        )?;
        self.apply(name, angle, notes, changes, joined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::edit::wires;

    const DECK: &str = "CM test\nCE\nSY f=14.2, lam=300/f\nGW 1 11 0 -5 10 0 5 10 .001\nGW 2 11 -2 -5.2 10 -2 5.2 10 .001\nGW 3 11 2 -4.8 10 2 4.8 10 .001\nGE 0\nEX 0 1 6 0 1\nFR 0 1 0 0 f\nEN\n";

    fn ws(deck: &Deck) -> Vec<Wire> {
        wires(deck, &deck.expand().unwrap())
    }

    #[test]
    fn notes_round_trip_and_old_comments_are_descriptions() {
        let notes = VarNotes {
            description: "Reflector length".into(),
            unit: "m".into(),
            group: "Boom parts".into(),
            range: Some((0.9, 1.2)),
            step: Some(0.001),
            fixed: true,
            role: "feed_length".into(),
        };
        let c = notes.to_comment().unwrap();
        assert_eq!(
            c,
            "Reflector length [unit=m group=\"Boom parts\" range=0.9..1.2 step=0.001 fixed use=feed_length]"
        );
        assert_eq!(VarNotes::parse(&c), notes);
        assert_eq!(VarNotes::parse("height [m]").description, "height [m]");
        assert_eq!(VarNotes::default().to_comment(), None);
    }

    #[test]
    fn create_rename_delete() {
        let mut deck = Deck::parse(DECK);
        let w = ws(&deck);
        deck.make_field_variable(w[0].line, 4, "z_feed", &VarNotes::default())
            .unwrap();
        assert_eq!(deck.lines[3].raw, "SY z_feed=10");
        assert!(deck.check_new_name("Z_FEED").is_err());
        assert!(deck.check_new_name("cos").is_err());
        assert!(deck.check_new_name("2a").is_err());
        assert_eq!(deck.unique_name("z feed"), "z_feed_2");
        assert_eq!(ws(&deck)[0].a[2], 10.0);

        // Notes on a shared line split it without changing the order.
        let notes = VarNotes {
            unit: "MHz".into(),
            ..VarNotes::default()
        };
        assert!(deck.set_notes("f", &notes));
        assert_eq!(deck.lines[2].raw, "SY f=14.2\t' [unit=MHz]");
        assert_eq!(deck.lines[3].raw, "SY lam=300/f");
        assert_eq!(deck.notes("f"), notes);

        deck.rename_variable("f", "freq").unwrap();
        assert_eq!(deck.lines[3].raw, "SY lam=300/freq");
        assert!(deck.lines.iter().any(|l| l.raw == "FR 0 1 0 0 freq"));
        assert_eq!(deck.uses("freq").len(), 2);
        assert_eq!(deck.uses_through("freq").len(), 1, "only the FR card");

        assert!(deck.delete_variable("freq", None).is_err());
        deck.delete_variable("freq", Some(14.25)).unwrap();
        assert_eq!(deck.lines[2].raw, "SY lam=300/14.25");
        let n = deck.expand().unwrap();
        assert_eq!(n.frequencies(), vec![14.25]);
        assert!(!deck.has_variable("freq"));
    }

    #[test]
    fn measurements_drive_the_geometry() {
        let mut deck = Deck::parse(DECK);
        let w = ws(&deck);
        deck.drive_length(&w[..1], &[], Anchor::Centre, "dlen", &VarNotes::default())
            .unwrap();
        let w2 = ws(&deck);
        assert_eq!(w2[0].fields[3], "-dlen/2");
        assert_eq!(w2[0].fields[6], "dlen/2");
        let n = deck.expand_with(&[("dlen".into(), 12.0)]).unwrap();
        assert!((wires(&deck, &n)[0].length() - 12.0).abs() < 1e-12);

        // Equal spacing along x: the lowest wire stays, the others follow.
        let w = ws(&deck);
        deck.drive_spacing(&w[1..], &[], Some(0), "pitch", &VarNotes::default())
            .unwrap();
        let n = deck.expand_with(&[("pitch".into(), 3.0)]).unwrap();
        let w3 = wires(&deck, &n);
        assert_eq!(w3[1].a[0], -2.0);
        assert!((w3[2].a[0] - 1.0).abs() < 1e-12);

        // Formulas already in place are refused, with a reason.
        let w = ws(&deck);
        let e = deck
            .drive_height(&w, &[], "h", &VarNotes::default())
            .map(|_| ());
        assert!(e.is_ok(), "{e:?}");
        let w = ws(&deck);
        let e = deck.drive_length(&w[..1], &[], Anchor::Start, "x", &VarNotes::default());
        assert!(e.unwrap_err().contains("already a formula"));
        let n = deck.expand_with(&[("h".into(), 4.0)]).unwrap();
        assert!(wires(&deck, &n).iter().all(|w| w.a[2] == 4.0));
    }

    #[test]
    fn a_fixed_end_set_by_a_formula_stays_linked() {
        let mut deck = Deck::parse(DECK);
        let w = ws(&deck);
        deck.make_field_variable(w[0].line, 6, "tip", &VarNotes::default())
            .unwrap();
        let w = ws(&deck);
        deck.drive_length(&w[..1], &[], Anchor::End, "len", &VarNotes::default())
            .unwrap();
        assert_eq!(ws(&deck)[0].fields[3], "tip-len");
        let n = deck
            .expand_with(&[("tip".into(), 6.0), ("len".into(), 4.0)])
            .unwrap();
        let w = wires(&deck, &n);
        assert_eq!((w[0].a[1], w[0].b[1]), (2.0, 6.0));
    }

    #[test]
    fn distance_and_angle() {
        let mut deck = Deck::parse(
            "CE\nGW 1 5 0 0 0 3 0 4 .001\nGW 2 5 1 1 0 1 2 0 .001\nGE 0\nEX 0 1 3 0 1\nFR 0 1 0 0 14\nEN\n",
        );
        let w = ws(&deck);
        deck.drive_angle(&w[0], &[], 0, "ang", &VarNotes::default())
            .unwrap();
        assert_eq!(deck.variables()[0].literal.map(|a| a.round()), Some(53.0));
        let n = deck.expand_with(&[("ang".into(), 90.0)]).unwrap();
        let w1 = wires(&deck, &n);
        assert!(w1[0].b[0].abs() < 1e-9 && (w1[0].b[2] - 5.0).abs() < 1e-9);

        let w = ws(&deck);
        let pair = [w[0].clone(), w[1].clone()];
        deck.drive_spacing(&pair, &[], None, "gap", &VarNotes::default())
            .unwrap();
        let n = deck.expand().unwrap();
        let w2 = wires(&deck, &n);
        assert!((w2[1].a[0] - 1.0).abs() < 1e-9 && (w2[1].b[1] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn tube_wall_variable() {
        let mut deck = Deck::parse(DECK);
        let n = deck.expand().unwrap();
        let all: Vec<i64> = ws(&deck).iter().map(|w| w.tag).collect();
        deck.set_material(
            &[1],
            &all,
            crate::deck::material::WireMaterial {
                conductivity: Some(3.77e7),
                wall: Some(0.001),
            },
        );
        let n2 = deck.expand().unwrap();
        let line = crate::deck::material::material_line(&n2, 1).unwrap();
        assert!(crate::deck::material::material_line(&n, 1).is_none());
        deck.make_wall_variable(line, "wall", &VarNotes::default())
            .unwrap();
        let n = deck.expand_with(&[("wall".into(), 0.002)]).unwrap();
        assert_eq!(n.tubes.len(), 1);
        assert_eq!(n.tubes[0].1, 0.002);
        assert_eq!(deck.uses("wall").len(), 1);
        deck.rename_variable("wall", "tube_t").unwrap();
        assert_eq!(deck.expand().unwrap().tubes[0].1, 0.001);
        deck.delete_variable("tube_t", Some(0.0015)).unwrap();
        assert_eq!(deck.expand().unwrap().tubes[0].1, 0.0015);
    }

    #[test]
    fn joined_ends_follow() {
        let text = "CE\nGW 1 5 0 0 0 0 0 5 .001\nGW 2 5 0 0 5 3 0 5 .001\nGE 0\nEX 0 1 3 0 1\nFR 0 1 0 0 14\nEN\n";
        let mut deck = Deck::parse(text);
        let w = ws(&deck);
        deck.drive_length(&w[..1], &w, Anchor::Start, "len", &VarNotes::default())
            .unwrap();
        let n = deck.expand_with(&[("len".into(), 7.0)]).unwrap();
        let w2 = wires(&deck, &n);
        assert_eq!((w2[0].b[2], w2[1].a[2], w2[1].b[2]), (7.0, 7.0, 5.0));

        // One coordinate made a variable: the joined end follows too.
        let mut deck = Deck::parse(text);
        let w = ws(&deck);
        deck.make_coord_variable(&w[0], 7, &w, "top", &VarNotes::default())
            .unwrap();
        assert_eq!(ws(&deck)[1].fields[4], "top");
        // Without the joined wires, it doesn't.
        let mut deck = Deck::parse(text);
        let w = ws(&deck);
        deck.make_coord_variable(&w[0], 7, &[], "top", &VarNotes::default())
            .unwrap();
        assert_eq!(ws(&deck)[1].fields[4], "5");
    }

    #[test]
    fn whole_antenna_height() {
        let mut deck = Deck::parse(
            "CE\nGW 1 5 0 -1 2 0 1 2 .001\nGE 1\nGN 1\nEX 0 1 3 0 1\nFR 0 1 0 0 14\nEN\n",
        );
        deck.set_height_shift(0.5);
        deck.drive_height_shift(2.5, "height", &VarNotes::default())
            .unwrap();
        let line = deck.height_line().unwrap();
        assert_eq!(
            deck.field_variable(line, 7),
            None,
            "a formula, not just the name"
        );
        let n = deck.expand_with(&[("height".into(), 7.0)]).unwrap();
        assert_eq!(wires(&deck, &n)[0].a[2], 7.0);
        assert!(
            deck.drive_height_shift(2.5, "h2", &VarNotes::default())
                .is_err()
        );
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    #[test]
    fn a_formula_using_a_later_variable_moves_below_it() {
        let mut d = Deck::parse("SY a=1\nSY b=2\nSY c=3\nGW 1 3 0 0 0 0 0 a .001\nGE 0\nEN\n");
        assert!(d.set_expression("a", "-c"));
        let n = d.expand().unwrap();
        let get = |k: &str| n.symbols.iter().find(|(s, _)| s == k).map(|(_, v)| *v);
        assert_eq!(get("a"), Some(-3.0));
        let order: Vec<String> = d.variables().into_iter().map(|v| v.name).collect();
        assert_eq!(order, ["b", "c", "a"]);
    }
}

#[cfg(test)]
mod scale_tests {
    use super::*;
    use crate::deck::edit::wires;

    #[test]
    fn a_group_of_wires_scales_together() {
        let mut d = Deck::parse(
            "GW 1 5 0 0 0 0 1 1 .001\nGW 2 5 0 1 1 0 2 0 .001\nGW 3 3 5 5 5 6 5 5 .001\nGE 0\nEN\n",
        );
        let n = d.expand().unwrap();
        let w = wires(&d, &n);
        d.drive_scale(&w[..2], &[], "size", &VarNotes::default())
            .unwrap();
        // About the middle of the pair's box (0, 1, 0.5): doubled.
        let n = d.expand_with(&[("size".into(), 2.0)]).unwrap();
        let w = wires(&d, &n);
        assert!((w[0].a[1] - -1.0).abs() < 1e-9 && (w[0].a[2] - -0.5).abs() < 1e-9);
        assert!((w[0].b[1] - 1.0).abs() < 1e-9 && (w[1].a[1] - 1.0).abs() < 1e-9);
        // Still joined, and the unselected wire is untouched.
        assert_eq!(w[0].b, w[1].a);
        assert_eq!(w[2].a, [5.0, 5.0, 5.0]);
    }
}
