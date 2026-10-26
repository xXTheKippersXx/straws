//! Expressions in 4nec2-style decks: `SY` symbol definitions and any card
//! field, e.g. `GW 1 21 0 -len/2 hgh 0 len/2 hgh #12`.
//!
//! Rules follow 4nec2 (a Visual Basic program):
//! - operators `+ - * / ^`, plus `\` (integer divide), `mod`, comparisons
//!   (`= <> < > <= >=`) and `and or not xor eqv`; true is -1, false is 0;
//! - `^` binds tightest and is left-associative; unary minus is weaker than
//!   `^`, so `-2^2` is -4, as in Visual Basic;
//! - functions `sin cos tan atn sqr exp log log10 abs sgn fix int mod`, with
//!   angles in degrees and `log` the natural log;
//! - names are case-insensitive and may start with `#` (AWG sizes, `#12`);
//! - a number directly followed by a name multiplies them, so `61ft` is
//!   61 × ft and `30mm` is 30 × mm.

use std::collections::HashMap;
use std::fmt;

/// An error while reading or evaluating an expression.
#[derive(Debug, Clone, PartialEq)]
pub struct ExprError(pub String);

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExprError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError(msg.into()))
}

/// Symbol values, keyed by lower-case name.
#[derive(Debug, Clone)]
pub struct Symbols {
    values: HashMap<String, f64>,
}

impl Default for Symbols {
    fn default() -> Self {
        Self::with_4nec2_defaults()
    }
}

impl Symbols {
    /// An empty table with no predefined names.
    pub fn empty() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// The names 4nec2 predefines: units (`mm cm in ft pF nF uF nH uH mH`),
    /// `pi`, and AWG wire radii `#0`..`#20` in metres (from 4nec2's
    /// data/Symbols.txt).
    pub fn with_4nec2_defaults() -> Self {
        let mut s = Self::empty();
        for (name, value) in [
            ("mm", 1e-3),
            ("cm", 1e-2),
            ("in", 0.0254),
            ("ft", 0.3048),
            ("pf", 1e-12),
            ("nf", 1e-9),
            ("uf", 1e-6),
            ("nh", 1e-9),
            ("uh", 1e-6),
            // Not in 4nec2's documentation, but accepted by it.
            ("mh", 1e-3),
            ("pi", std::f64::consts::PI),
        ] {
            s.set(name, value);
        }
        // AWG diameters in mm, as listed in 4nec2's Symbols.txt.
        const AWG_DIAMETER_MM: [f64; 21] = [
            8.2525, 7.3482, 6.5430, 5.8268, 5.1892, 4.6203, 4.1148, 3.6652, 3.2639, 2.9058, 2.5883,
            2.3038, 2.0523, 1.8288, 1.6281, 1.4503, 1.2903, 1.1506, 1.0236, 0.9119, 0.8128,
        ];
        for (gauge, d) in AWG_DIAMETER_MM.iter().enumerate() {
            s.set(&format!("#{gauge}"), d * 1e-3 / 2.0);
        }
        s
    }

    pub fn set(&mut self, name: &str, value: f64) {
        self.values.insert(name.to_ascii_lowercase(), value);
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        self.values.get(&name.to_ascii_lowercase()).copied()
    }
}

/// Evaluates an expression against a symbol table.
pub fn eval(text: &str, symbols: &Symbols) -> Result<f64, ExprError> {
    let tokens = tokenize(text)?;
    let mut p = Parser {
        tokens: &tokens,
        pos: 0,
        symbols,
    };
    let value = p.expr()?;
    if p.pos != tokens.len() {
        return err(format!("unexpected '{}' in \"{text}\"", tokens[p.pos]));
    }
    Ok(value)
}

/// Names the expression language reserves: functions, word operators and
/// the predefined units (`mm`, `ft`, `pi`, ...). Variables can't use them.
pub const RESERVED: &[&str] = &[
    "sin", "cos", "tan", "atn", "sqr", "exp", "log", "log10", "abs", "sgn", "fix", "int", "mod",
    "and", "or", "not", "xor", "eqv", "mm", "cm", "in", "ft", "pf", "nf", "uf", "nh", "uh", "mh",
    "pi",
];

/// Byte ranges of the names in `text`, found the same way the expression
/// reader finds them (so `1e5` is a number, `61ft` is a number and a name).
pub fn name_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_digit() || (c == '.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let mut j = i + 1;
                if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
                    j += 1;
                }
                if j < bytes.len() && bytes[j].is_ascii_digit() {
                    i = j;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
        } else if is_name_start(c) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_name_char(bytes[i] as char) {
                i += 1;
            }
            out.push(start..i);
        } else {
            // Any other character, including the bytes of non-ASCII ones.
            i += 1;
        }
    }
    out
}

/// The distinct names used in `text`, lower-case, leaving out functions,
/// operators and units.
pub fn names_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in name_spans(text) {
        let name = text[r].to_ascii_lowercase();
        if !name.starts_with('#') && !RESERVED.contains(&name.as_str()) && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// `text` with every use of the name `from` (any case) replaced by `to`.
pub fn replace_name(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for r in name_spans(text) {
        if text[r.clone()].eq_ignore_ascii_case(from) {
            out.push_str(&text[last..r.start]);
            // A number right before a name multiplies it (`2len`); keep
            // that meaning if the name becomes a number.
            if text[..r.start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_digit() || c == '.')
            {
                out.push('*');
            }
            out.push_str(to);
            last = r.end;
        }
    }
    out.push_str(&text[last..]);
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Num(v) => write!(f, "{v}"),
            Tok::Name(n) => f.write_str(n),
            Tok::Op(o) => f.write_str(o),
            Tok::LParen => f.write_str("("),
            Tok::RParen => f.write_str(")"),
            Tok::Comma => f.write_str(","),
        }
    }
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '#'
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '#'
}

fn tokenize(text: &str) -> Result<Vec<Tok>, ExprError> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // Exponent only if 'e' is followed by a digit or a signed digit,
            // so `1in` or `2em` stay a number times a name.
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let s: String = chars[start..i].iter().collect();
            let v: f64 = s
                .parse()
                .map_err(|_| ExprError(format!("bad number \"{s}\"")))?;
            out.push(Tok::Num(v));
        } else if is_name_start(c) {
            let start = i;
            i += 1;
            while i < chars.len() && is_name_char(chars[i]) {
                i += 1;
            }
            out.push(Tok::Name(chars[start..i].iter().collect()));
        } else {
            let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let op: &'static str = match two.as_str() {
                "<=" => "<=",
                ">=" => ">=",
                "<>" => "<>",
                _ => "",
            };
            if !op.is_empty() {
                out.push(Tok::Op(op));
                i += 2;
                continue;
            }
            out.push(match c {
                '+' => Tok::Op("+"),
                '-' => Tok::Op("-"),
                '*' => Tok::Op("*"),
                '/' => Tok::Op("/"),
                '\\' => Tok::Op("\\"),
                '^' => Tok::Op("^"),
                '<' => Tok::Op("<"),
                '>' => Tok::Op(">"),
                '=' => Tok::Op("="),
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                ',' => Tok::Comma,
                _ => return err(format!("unexpected character '{c}' in \"{text}\"")),
            });
            i += 1;
        }
    }
    // A name right after a number is a unit multiplier, with or without a
    // space: `61ft` and `45.78 ft` both mean number × ft.
    let mut with_units = Vec::with_capacity(out.len());
    for tok in out {
        if let (Some(Tok::Num(_)), Tok::Name(name)) = (with_units.last(), &tok) {
            let is_operator = ["mod", "and", "or", "xor", "eqv", "not"]
                .iter()
                .any(|w| name.eq_ignore_ascii_case(w));
            if !is_operator && !name.starts_with('#') {
                with_units.push(Tok::Op("*"));
            }
        }
        with_units.push(tok);
    }
    Ok(with_units)
}

struct Parser<'a> {
    tokens: &'a [Tok],
    pos: usize,
    symbols: &'a Symbols,
}

fn truth(b: bool) -> f64 {
    if b { -1.0 } else { 0.0 }
}

fn as_int(v: f64) -> i64 {
    v.round() as i64
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn peek_word(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Tok::Name(n)) if n.eq_ignore_ascii_case(word))
    }

    fn peek_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(o)) if *o == op)
    }

    // Precedence, loosest first: eqv, xor, or, and, not, comparisons, + -,
    // mod, \, * /, unary -, ^.
    fn expr(&mut self) -> Result<f64, ExprError> {
        let mut v = self.xor()?;
        while self.peek_word("eqv") {
            self.pos += 1;
            let r = self.xor()?;
            v = !(as_int(v) ^ as_int(r)) as f64;
        }
        Ok(v)
    }

    fn xor(&mut self) -> Result<f64, ExprError> {
        let mut v = self.or()?;
        while self.peek_word("xor") {
            self.pos += 1;
            let r = self.or()?;
            v = (as_int(v) ^ as_int(r)) as f64;
        }
        Ok(v)
    }

    fn or(&mut self) -> Result<f64, ExprError> {
        let mut v = self.and()?;
        while self.peek_word("or") {
            self.pos += 1;
            let r = self.and()?;
            v = (as_int(v) | as_int(r)) as f64;
        }
        Ok(v)
    }

    fn and(&mut self) -> Result<f64, ExprError> {
        let mut v = self.not()?;
        while self.peek_word("and") {
            self.pos += 1;
            let r = self.not()?;
            v = (as_int(v) & as_int(r)) as f64;
        }
        Ok(v)
    }

    fn not(&mut self) -> Result<f64, ExprError> {
        if self.peek_word("not") {
            self.pos += 1;
            let v = self.not()?;
            return Ok(!as_int(v) as f64);
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<f64, ExprError> {
        let mut v = self.additive()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o)) if matches!(*o, "=" | "<>" | "<" | ">" | "<=" | ">=") => *o,
                _ => return Ok(v),
            };
            self.pos += 1;
            let r = self.additive()?;
            v = truth(match op {
                "=" => v == r,
                "<>" => v != r,
                "<" => v < r,
                ">" => v > r,
                "<=" => v <= r,
                _ => v >= r,
            });
        }
    }

    fn additive(&mut self) -> Result<f64, ExprError> {
        let mut v = self.modulo()?;
        loop {
            if self.peek_op("+") {
                self.pos += 1;
                v += self.modulo()?;
            } else if self.peek_op("-") {
                self.pos += 1;
                v -= self.modulo()?;
            } else {
                return Ok(v);
            }
        }
    }

    fn modulo(&mut self) -> Result<f64, ExprError> {
        let mut v = self.int_divide()?;
        // `mod` as an operator, unless it is the function form `mod(a, b)`.
        while self.peek_word("mod") && !matches!(self.tokens.get(self.pos + 1), Some(Tok::LParen)) {
            self.pos += 1;
            let r = self.int_divide()?;
            v = vb_mod(v, r)?;
        }
        Ok(v)
    }

    fn int_divide(&mut self) -> Result<f64, ExprError> {
        let mut v = self.term()?;
        while self.peek_op("\\") {
            self.pos += 1;
            let r = as_int(self.term()?);
            if r == 0 {
                return err("division by zero");
            }
            v = (as_int(v) / r) as f64;
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, ExprError> {
        let mut v = self.unary()?;
        loop {
            if self.peek_op("*") {
                self.pos += 1;
                v *= self.unary()?;
            } else if self.peek_op("/") {
                self.pos += 1;
                let r = self.unary()?;
                if r == 0.0 {
                    return err("division by zero");
                }
                v /= r;
            } else {
                return Ok(v);
            }
        }
    }

    fn unary(&mut self) -> Result<f64, ExprError> {
        if self.peek_op("-") {
            self.pos += 1;
            return Ok(-self.unary()?);
        }
        if self.peek_op("+") {
            self.pos += 1;
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<f64, ExprError> {
        let mut v = self.atom()?;
        while self.peek_op("^") {
            self.pos += 1;
            // The exponent may carry its own sign: 10^-3.
            let r = if self.peek_op("-") {
                self.pos += 1;
                -self.atom()?
            } else {
                self.atom()?
            };
            v = v.powf(r);
        }
        Ok(v)
    }

    fn atom(&mut self) -> Result<f64, ExprError> {
        match self.peek().cloned() {
            Some(Tok::Num(v)) => {
                self.pos += 1;
                Ok(v)
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let v = self.expr()?;
                self.expect_rparen()?;
                Ok(v)
            }
            Some(Tok::Name(name)) => {
                self.pos += 1;
                if matches!(self.peek(), Some(Tok::LParen)) {
                    self.pos += 1;
                    let mut args = vec![self.expr()?];
                    while matches!(self.peek(), Some(Tok::Comma)) {
                        self.pos += 1;
                        args.push(self.expr()?);
                    }
                    self.expect_rparen()?;
                    return call(&name, &args);
                }
                self.symbols
                    .get(&name)
                    .ok_or_else(|| ExprError(format!("no variable called \"{name}\"")))
            }
            Some(t) => err(format!("unexpected '{t}'")),
            None => err("expression ends too early"),
        }
    }

    fn expect_rparen(&mut self) -> Result<(), ExprError> {
        if matches!(self.peek(), Some(Tok::RParen)) {
            self.pos += 1;
            Ok(())
        } else {
            err("missing ')'")
        }
    }
}

fn vb_mod(a: f64, b: f64) -> Result<f64, ExprError> {
    let b = as_int(b);
    if b == 0 {
        return err("division by zero in mod");
    }
    Ok((as_int(a) % b) as f64)
}

fn call(name: &str, args: &[f64]) -> Result<f64, ExprError> {
    let lower = name.to_ascii_lowercase();
    let one = |f: fn(f64) -> f64| -> Result<f64, ExprError> {
        match args {
            [x] => Ok(f(*x)),
            _ => err(format!("{name}() takes one value")),
        }
    };
    match lower.as_str() {
        "sin" => one(|x| x.to_radians().sin()),
        "cos" => one(|x| x.to_radians().cos()),
        "tan" => one(|x| x.to_radians().tan()),
        "atn" => one(|x| x.atan().to_degrees()),
        "sqr" => one(f64::sqrt),
        "exp" => one(f64::exp),
        "log" => one(f64::ln),
        "log10" => one(f64::log10),
        "abs" => one(f64::abs),
        "sgn" => one(|x| {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }
        }),
        "fix" => one(f64::trunc),
        "int" => one(f64::floor),
        "mod" => match args {
            [a, b] => vb_mod(*a, *b),
            _ => err("mod() takes two values"),
        },
        _ => err(format!("unknown function \"{name}\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_found_and_replaced() {
        assert_eq!(names_in("2*len+cos(ang)-1e5+30mm+#12"), ["len", "ang"]);
        assert_eq!(
            replace_name("Len/2+len2-LEN", "len", "half"),
            "half/2+len2-half"
        );
        assert_eq!(replace_name("2len+x", "len", "(-3)"), "2*(-3)+x");
    }

    fn ev(s: &str) -> f64 {
        eval(s, &Symbols::default()).unwrap()
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("1+2*3"), 7.0);
        assert_eq!(ev("(1+2)*3"), 9.0);
        assert_eq!(ev("-2^2"), -4.0);
        assert_eq!(ev("2^3^2"), 64.0);
        assert_eq!(ev("10^-3"), 1e-3);
        assert_eq!(ev("7 mod 3"), 1.0);
        assert_eq!(ev("mod(7,3)"), 1.0);
        assert_eq!(ev("7\\2"), 3.0);
        assert_eq!(ev("5.e-3"), 5e-3);
        assert_eq!(ev(".5"), 0.5);
    }

    #[test]
    fn units_and_awg() {
        assert!((ev("61ft") - 18.5928).abs() < 1e-12);
        assert!((ev("1in") - 0.0254).abs() < 1e-15);
        assert!((ev("30mm") - 0.03).abs() < 1e-15);
        assert!((ev("#12") - 1.02615e-3).abs() < 1e-12);
        assert!((ev("2.5e1mm") - 0.025).abs() < 1e-15);
        assert!((ev("45.78 ft") - 13.953744).abs() < 1e-9);
        assert_eq!(ev("7 mod 3"), 1.0);
    }

    #[test]
    fn functions_use_degrees() {
        assert!((ev("cos(60)") - 0.5).abs() < 1e-12);
        assert!((ev("atn(1)") - 45.0).abs() < 1e-12);
        assert!((ev("sqr(16)") - 4.0).abs() < 1e-12);
        assert!((ev(".4342945*LOG(10)") - 1.0).abs() < 1e-6);
        assert_eq!(ev("int(-1.5)"), -2.0);
        assert_eq!(ev("fix(-1.5)"), -1.0);
    }

    #[test]
    fn symbols_ignore_case_and_compare() {
        let mut s = Symbols::default();
        s.set("Z", 3.0);
        assert_eq!(
            eval("hgh-z", &{
                let mut t = s.clone();
                t.set("hgh", 10.0);
                t
            })
            .unwrap(),
            7.0
        );
        assert_eq!(eval("z > 2", &s).unwrap(), -1.0);
        assert_eq!(eval("not (z = 3)", &s).unwrap(), 0.0);
    }

    #[test]
    fn errors_are_reported() {
        assert!(eval("nope*2", &Symbols::default()).is_err());
        assert!(eval("(1+2", &Symbols::default()).is_err());
        assert!(eval("1/0", &Symbols::default()).is_err());
    }
}
