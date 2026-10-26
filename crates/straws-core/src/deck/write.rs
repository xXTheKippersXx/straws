//! Writing a [`NumericDeck`] as plain NEC-2 text.
//!
//! The output suits both nec2c and 4nec2's engines: space-separated fields,
//! integers without decimal points, `CM`/`CE` comments first and `EN` last.
//! nec2c reads lines of up to 132 characters; the Fortran engines (4nec2's)
//! read only the first 80 columns, so decks meant for them are written
//! narrower, with fewer digits on the lines that need it.

use super::expand::{NumCard, NumericDeck};

/// Longest comment text written. nec2c and the Fortran engines read 80
/// columns of a comment card; keep well inside that.
const MAX_COMMENT: usize = 76;

/// Widest line nec2c reads.
pub const NEC2C_COLUMNS: usize = 132;
/// Widest line the Fortran NEC-2 engines (including 4nec2's) read.
pub const FORTRAN_COLUMNS: usize = 80;

impl NumericDeck {
    /// The deck as NEC-2 input text for nec2c, at full precision.
    pub fn to_nec_text(&self) -> String {
        self.to_nec_text_width(NEC2C_COLUMNS)
    }

    /// The deck as NEC-2 input text with lines no wider than `columns`.
    /// Use [`FORTRAN_COLUMNS`] when the same deck must also run in a
    /// Fortran engine.
    pub fn to_nec_text_width(&self, columns: usize) -> String {
        let mut out = String::new();
        let comments: Vec<&String> = self.comments.iter().filter(|c| !c.is_empty()).collect();
        for c in &comments {
            out.push_str("CM ");
            out.push_str(&truncate(c, MAX_COMMENT));
            out.push('\n');
        }
        out.push_str("CE\n");
        for card in &self.cards {
            out.push_str(&card.to_nec_line_width(columns));
            out.push('\n');
        }
        out.push_str("EN\n");
        out
    }
}

impl NumCard {
    /// The card as one line of NEC-2 text, without trailing zero fields.
    pub fn to_nec_line(&self) -> String {
        self.to_nec_line_width(NEC2C_COLUMNS)
    }

    /// The card as one line no wider than `columns` if possible, using up
    /// to 10 significant digits and fewer only when the line needs it.
    pub fn to_nec_line_width(&self, columns: usize) -> String {
        let mut line = String::new();
        for digits in (5..=10).rev() {
            line = self.line_with_digits(digits);
            if line.len() <= columns {
                break;
            }
        }
        line
    }

    fn line_with_digits(&self, digits: usize) -> String {
        let mut fields: Vec<String> = self.ints.iter().map(|i| i.to_string()).collect();
        fields.extend(self.floats.iter().map(|&f| format_real_digits(f, digits)));
        // Trailing zeros are the default; dropping them keeps lines short.
        while fields.len() > self.ints.len().min(1) && fields.last().is_some_and(|f| f == "0") {
            fields.pop();
        }
        let mut line = self.mnemonic.clone();
        for f in fields {
            line.push(' ');
            line.push_str(&f);
        }
        line
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Formats a real number compactly with up to 10 significant digits, which
/// is far finer than any physical dimension needs while keeping lines short.
pub fn format_real(v: f64) -> String {
    format_real_digits(v, 10)
}

/// Formats a real number compactly with at most `digits` significant digits.
pub fn format_real_digits(v: f64, digits: usize) -> String {
    if v == 0.0 || !v.is_finite() {
        return "0".to_string();
    }
    let shortest = format!("{v}");
    let significant = shortest
        .trim_start_matches('-')
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .trim_start_matches('0')
        .len();
    if shortest.len() <= digits + 2 && significant <= digits {
        return shortest;
    }
    let exp = v.abs().log10().floor() as i32;
    let last = digits as i32 - 1;
    let s = if (-4..=last).contains(&exp) {
        let decimals = (last - exp).max(0) as usize;
        let s = format!("{v:.decimals$}");
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    } else {
        let s = format!("{v:.prec$e}", prec = digits - 1);
        let (mantissa, exponent) = s.split_once('e').expect("exponent form");
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        format!("{mantissa}E{exponent}")
    };
    if s == "-0" { "0".to_string() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::Deck;

    #[test]
    fn formats_reals_compactly() {
        assert_eq!(format_real(0.0), "0");
        assert_eq!(format_real(18.5928), "18.5928");
        assert_eq!(format_real(-13.953744), "-13.953744");
        assert_eq!(format_real(0.1 + 0.2), "0.3");
        assert_eq!(format_real(1.0 / 3.0), "0.3333333333");
        assert_eq!(format_real(2.0 / 3.0 * 1e-7), "6.666666667E-8");
        assert_eq!(format_real(58001000.0), "58001000");
        assert_eq!(format_real(5e-3), "0.005");
    }

    #[test]
    fn keeps_fortran_lines_within_80_columns() {
        let deck = Deck::parse(
            "CE\nGW 11 11 -1.638261233 -1.400804456 -1.400804456 -1.638261233 1.400804456 -1.400804456 0.000527309\nGE\n",
        )
        .expand()
        .unwrap();
        let wide = deck.cards[0].to_nec_line();
        assert!(wide.len() > 80, "{wide}");
        assert!(wide.contains("0.000527309"));
        let narrow = deck.cards[0].to_nec_line_width(FORTRAN_COLUMNS);
        assert!(narrow.len() <= 80, "{narrow}");
        assert!(
            narrow.ends_with(" 0.0005273090") || narrow.ends_with(" 0.000527309"),
            "{narrow}"
        );
        assert_eq!(format_real_digits(-1.638261233, 7), "-1.638261");
        assert_eq!(format_real_digits(0.000527309, 7), "0.000527309");
    }

    #[test]
    fn writes_a_runnable_deck() {
        let deck = Deck::parse(
            "CM dipole\nCE\nSY len=10\nGW 1 21 0 -len/2 10 0 len/2 10 #14\nGE 0\n\
             EX 0 1 11 0 1 0\nFR 0 1 0 0 14.2\nEN\n",
        );
        let text = deck.expand().unwrap().to_nec_text();
        assert_eq!(
            text,
            "CM dipole\nCE\nGW 1 21 0 -5 10 0 5 10 0.00081405\nGE 0\n\
             EX 0 1 11 0 1\nFR 0 1 0 0 14.2\nEN\n"
        );
    }
}
