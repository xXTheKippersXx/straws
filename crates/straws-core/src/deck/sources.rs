//! Sources (EX cards) for the Feeds view: listing them, setting amplitude
//! and phase, voltage or current, adding and removing them, and making
//! amplitude and phase variables.
//!
//! - A voltage source is `EX 0` (or `EX 5`); a current source is 4nec2's
//!   `EX 6` (the deck reader turns it into a helper wire and a network, see
//!   `expand`). Amplitude is volts or amps, phase degrees; the card holds
//!   them as real and imaginary parts (fields 5 and 6).
//! - Variables: `NAME_amp` and `NAME_phase`, with the card's fields written
//!   `NAME_amp*cos(NAME_phase)` and `NAME_amp*sin(NAME_phase)` (degrees).

use num_complex::Complex64;

use super::edit::card_line;
use super::expr::{self, Symbols};
use super::vars::VarNotes;
use super::write::format_real;
use super::{Deck, LineKind, NumericDeck};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Voltage,
    Current,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Voltage => "voltage",
            SourceKind::Current => "current",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            SourceKind::Voltage => "V",
            SourceKind::Current => "A",
        }
    }

    fn ex_type(self) -> &'static str {
        match self {
            SourceKind::Voltage => "0",
            SourceKind::Current => "6",
        }
    }
}

/// A source in the deck.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// The EX card's line.
    pub line: usize,
    pub kind: SourceKind,
    pub tag: i64,
    /// Segment on the wire (1-based).
    pub segment: i64,
    /// Its value worked out (`None` if a field can't be).
    pub value: Option<Complex64>,
    /// The amplitude and phase variables, if it has them.
    pub variables: Option<(String, String)>,
}

impl Source {
    pub fn amplitude(&self) -> Option<f64> {
        self.value.map(Complex64::norm)
    }

    /// Phase in degrees.
    pub fn phase(&self) -> Option<f64> {
        self.value.map(|v| v.arg().to_degrees())
    }
}

fn fields(deck: &Deck, line: usize) -> Option<&Vec<String>> {
    match &deck.lines.get(line)?.kind {
        LineKind::Card { mnemonic, fields } if mnemonic == "EX" => Some(fields),
        _ => None,
    }
}

/// `NAME_amp*cos(NAME_phase)` and `NAME_amp*sin(NAME_phase)`: the names.
fn variables_of(re: &str, im: &str) -> Option<(String, String)> {
    let (amp, rest) = re.split_once("*cos(")?;
    let phase = rest.strip_suffix(')')?;
    (im == format!("{amp}*sin({phase})")).then(|| (amp.to_string(), phase.to_string()))
}

/// The voltage and current sources in a deck, in order.
pub fn sources(deck: &Deck, numeric: &NumericDeck) -> Vec<Source> {
    let mut symbols = Symbols::default();
    for (name, v) in &numeric.symbols {
        symbols.set(name, *v);
    }
    let num = |t: &str| expr::eval(t, &symbols).ok();
    deck.lines
        .iter()
        .enumerate()
        .filter_map(|(line, _)| {
            let f = fields(deck, line)?;
            let kind = match f.first()?.parse::<f64>().ok()? as i64 {
                0 | 5 => SourceKind::Voltage,
                6 => SourceKind::Current,
                _ => return None,
            };
            let get = |k: usize| f.get(k).map_or("0", String::as_str);
            let value = Some(Complex64::new(num(get(4))?, num(get(5))?));
            Some(Source {
                line,
                kind,
                tag: num(get(1))? as i64,
                segment: num(get(2))? as i64,
                value: value.filter(|v| v.re.is_finite() && v.im.is_finite()),
                variables: variables_of(get(4), get(5)),
            })
        })
        .collect()
}

/// `value` as the card's two number fields.
fn parts(value: Complex64) -> [String; 2] {
    // No floating-point dust from the polar form (cos 90° isn't exactly 0).
    let tidy = |v: f64| {
        let v = (v * 1e12).round() / 1e12;
        if v == 0.0 { 0.0 } else { v }
    };
    [format_real(tidy(value.re)), format_real(tidy(value.im))]
}

impl Deck {
    /// Sets source `line` to `value` (volts or amps), as plain numbers.
    pub fn set_source(&mut self, line: usize, value: Complex64) -> Result<(), String> {
        let f = fields(self, line).ok_or("that line isn't a source")?;
        if variables_of(
            f.get(4).map_or("", String::as_str),
            f.get(5).map_or("", String::as_str),
        )
        .is_some()
        {
            return Err("its amplitude and phase are variables: change them instead".into());
        }
        let [re, im] = parts(value);
        self.set_field(line, 4, &re);
        self.set_field(line, 5, &im);
        Ok(())
    }

    /// Makes source `line` a voltage or current source (same numbers).
    pub fn set_source_kind(&mut self, line: usize, kind: SourceKind) -> Result<(), String> {
        fields(self, line).ok_or("that line isn't a source")?;
        self.set_field(line, 0, kind.ex_type());
        Ok(())
    }

    /// Adds a source on segment `segment` of wire `tag`, after the last EX
    /// card (or just after GE). Returns its line.
    pub fn add_source(
        &mut self,
        tag: i64,
        segment: i64,
        kind: SourceKind,
        value: Complex64,
    ) -> usize {
        let [re, im] = parts(value);
        let f: Vec<String> = [
            kind.ex_type().to_string(),
            tag.to_string(),
            segment.to_string(),
            "0".into(),
            re,
            im,
        ]
        .into_iter()
        .collect();
        let last_ex = self
            .lines
            .iter()
            .rposition(|l| matches!(&l.kind, LineKind::Card { mnemonic, .. } if mnemonic == "EX"));
        let at = last_ex
            .map(|l| l + 1)
            .or_else(|| self.ge_line().map(|g| g + 1))
            .unwrap_or(self.lines.len());
        self.insert_lines(at, &[card_line("EX", &f, None)]);
        at
    }

    /// Removes source `line` (and its variables, if nothing else uses them).
    pub fn remove_source(&mut self, line: usize) {
        let vars = fields(self, line).and_then(|f| {
            variables_of(
                f.get(4).map_or("", String::as_str),
                f.get(5).map_or("", String::as_str),
            )
        });
        if fields(self, line).is_none() {
            return;
        }
        self.remove_lines(&[line]);
        if let Some((a, p)) = vars {
            for v in [a, p] {
                if self.uses(&v).is_empty() {
                    let _ = self.delete_variable(&v, None);
                }
            }
        }
    }

    /// Makes source `line`'s amplitude and phase variables `NAME_amp` and
    /// `NAME_phase` (grouped under NAME), with their current values.
    pub fn make_source_variables(
        &mut self,
        line: usize,
        name: &str,
        numeric: &NumericDeck,
    ) -> Result<(String, String), String> {
        let s = sources(self, numeric)
            .into_iter()
            .find(|s| s.line == line)
            .ok_or("that line isn't a source")?;
        if s.variables.is_some() {
            return Err("its amplitude and phase are variables already".into());
        }
        let v = s.value.ok_or("its value can't be worked out")?;
        // Which EX card it is (the new SY lines go above it).
        let nth = (0..line).filter(|&l| fields(self, l).is_some()).count();
        let (amp, phase) = (format!("{name}_amp"), format!("{name}_phase"));
        self.check_new_name(&amp)?;
        self.check_new_name(&phase)?;
        let round = |x: f64| (x * 1e9).round() / 1e9;
        let notes = |what: &str, unit: &str| VarNotes {
            description: format!("{name} {what}"),
            unit: unit.to_string(),
            group: name.to_string(),
            ..VarNotes::default()
        };
        self.add_variable(
            &amp,
            &format_real(round(v.norm())),
            &notes("amplitude", s.kind.unit()),
        )?;
        self.add_variable(
            &phase,
            &format_real(round(v.arg().to_degrees())),
            &notes("phase", "deg"),
        )?;
        let line = (0..self.lines.len())
            .filter(|&l| fields(self, l).is_some())
            .nth(nth)
            .ok_or("the source moved")?;
        self.set_field(line, 4, &format!("{amp}*cos({phase})"));
        self.set_field(line, 5, &format!("{amp}*sin({phase})"));
        Ok((amp, phase))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck() -> Deck {
        Deck::parse(
            "GW 1 9 0 0 0 0 0 10 .01\nGW 2 9 5 0 0 5 0 10 .01\nGE 1\nGN 1\nEX 6 1 1 0 1 0\nEX 0 2 1 0 0 -1\nFR 0 1 0 0 7.1 0\nEN\n",
        )
    }

    #[test]
    fn sources_are_read_set_and_made_variables() {
        let mut d = deck();
        let n = d.expand().unwrap();
        let s = sources(&d, &n);
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].kind, s[0].tag, s[0].segment),
            (SourceKind::Current, 1, 1)
        );
        assert!((s[1].phase().unwrap() + 90.0).abs() < 1e-9);
        d.set_source(s[1].line, Complex64::from_polar(2.0, 45f64.to_radians()))
            .unwrap();
        d.set_source_kind(s[1].line, SourceKind::Current).unwrap();
        let n = d.expand().unwrap();
        let s = sources(&d, &n);
        assert_eq!(s[1].kind, SourceKind::Current);
        assert!((s[1].amplitude().unwrap() - 2.0).abs() < 1e-9);
        // Variables: the card follows them.
        let (amp, phase) = d.make_source_variables(s[0].line, "feed1", &n).unwrap();
        let n = d.expand().unwrap();
        let s = sources(&d, &n);
        assert_eq!(s[0].variables, Some((amp.clone(), phase)));
        assert!(d.set_source(s[0].line, Complex64::new(1.0, 0.0)).is_err());
        let n = d.expand_with(&[(amp, 3.0)]).unwrap();
        assert!((sources(&d, &n)[0].amplitude().unwrap() - 3.0).abs() < 1e-9);
        // Add and remove.
        let line = d.add_source(2, 5, SourceKind::Voltage, Complex64::new(1.0, 0.0));
        let n = d.expand().unwrap();
        assert_eq!(sources(&d, &n).len(), 3);
        d.remove_source(line);
        d.remove_source(sources(&d, &n)[0].line);
        let n = d.expand().unwrap();
        assert_eq!(sources(&d, &n).len(), 1);
        assert!(d.variables().is_empty(), "{:?}", d.variables());
    }
}
