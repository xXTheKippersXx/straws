//! The feedline's shield as a wire, to see common-mode current: current
//! that flows back down the outside of the coax instead of into the
//! antenna. A choke (a current balun) is a series impedance at its top.

use super::edit::card_line;
use super::expand::NumericDeck;
use super::write::format_real;
use super::{Deck, LineKind};

/// Comment marking the shield's GW line.
pub const SHIELD_MARK: &str = "straws: feedline shield";
/// Comment marking the choke's LD line.
pub const CHOKE_MARK: &str = "straws: common-mode choke";

fn marked(deck: &Deck, mark: &str) -> Vec<usize> {
    deck.lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.comment.as_deref() == Some(mark))
        .map(|(i, _)| i)
        .collect()
}

/// The shield wire's tag, if the deck has one.
pub fn shield_tag(deck: &Deck) -> Option<i64> {
    marked(deck, SHIELD_MARK)
        .first()
        .and_then(|&i| match &deck.lines[i].kind {
            LineKind::Card { fields, .. } => fields.first()?.parse::<f64>().ok().map(|t| t as i64),
            _ => None,
        })
}

/// The choke's resistance and reactance (ohms), if there is one.
pub fn choke(numeric: &NumericDeck, deck: &Deck) -> Option<(f64, f64)> {
    let line = *marked(deck, CHOKE_MARK).first()?;
    numeric
        .cards
        .iter()
        .find(|c| c.line == Some(line + 1))
        .map(|c| (c.floats[0], c.floats[1]))
}

/// The choke's LD line (index into `Deck::lines`), if there is one.
pub fn choke_line(deck: &Deck) -> Option<usize> {
    marked(deck, CHOKE_MARK).first().copied()
}

impl Deck {
    /// Removes the shield wire and its choke.
    pub fn remove_shield(&mut self) {
        let mut lines = marked(self, SHIELD_MARK);
        lines.extend(marked(self, CHOKE_MARK));
        self.remove_lines(&lines);
    }

    /// Adds (or replaces) the shield: a wire from `top` (an end of the fed
    /// segment) to `bottom` (metres), and a choke of `r` + j`x` ohms on its
    /// top segment if either isn't zero.
    #[allow(clippy::too_many_arguments)]
    pub fn add_shield(
        &mut self,
        tag: i64,
        top: [f64; 3],
        bottom: [f64; 3],
        segments: i64,
        radius: f64,
        r: f64,
        x: f64,
    ) {
        self.remove_shield();
        let fields: Vec<String> = [tag as f64, segments as f64]
            .into_iter()
            .chain(top)
            .chain(bottom)
            .chain([radius])
            // No floating-point dust like 1e-17 for a zero coordinate.
            .map(|v| format_real(if v.abs() < 1e-12 { 0.0 } else { v }))
            .collect();
        let at = self.ge_line().unwrap_or(self.lines.len());
        self.insert_lines(at, &[card_line("GW", &fields, Some(SHIELD_MARK))]);
        if r != 0.0 || x != 0.0 {
            let ge = self.ge_line().unwrap_or(self.lines.len());
            let f: Vec<String> = ["4".to_string(), tag.to_string(), "1".into(), "1".into()]
                .into_iter()
                .chain([format_real(r), format_real(x)])
                .collect();
            self.insert_lines(ge + 1, &[card_line("LD", &f, Some(CHOKE_MARK))]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shield_round_trip() {
        let mut deck = Deck::parse(
            "CE\nGW 1 11 0 -5 10 0 5 10 .001\nGE 1\nGN 1\nEX 0 1 6 0 1\nFR 0 1 0 0 14\nEN\n",
        );
        deck.add_shield(
            2,
            [0.0, -0.4545, 10.0],
            [0.0, -0.4545, 0.0],
            9,
            0.005,
            1000.0,
            0.0,
        );
        assert_eq!(shield_tag(&deck), Some(2));
        let n = deck.expand().unwrap();
        assert_eq!(choke(&n, &deck), Some((1000.0, 0.0)));
        deck.remove_shield();
        assert_eq!(shield_tag(&deck), None);
        assert!(deck.expand().unwrap().cards.iter().all(|c| !c.is("LD")));
    }
}
