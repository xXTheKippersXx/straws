//! Building the decks the app runs from a user's deck: their geometry,
//! sources, loads and ground, with our own frequency plan and pattern
//! request in place of whatever output cards the deck had.

use crate::deck::edit::card_line;
use crate::deck::expand::EXECUTE_CARDS;
use crate::deck::write::format_real;
use crate::deck::{Deck, LineKind, NumCard, NumericDeck};

/// A linear frequency sweep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrequencyPlan {
    pub start_mhz: f64,
    pub stop_mhz: f64,
    /// Number of frequencies, including both ends (at least 1).
    pub points: usize,
}

impl FrequencyPlan {
    /// One frequency only.
    pub fn single(mhz: f64) -> Self {
        Self {
            start_mhz: mhz,
            stop_mhz: mhz,
            points: 1,
        }
    }

    /// A sweep of ±5% around a design frequency, with 51 points, on round
    /// numbers where possible.
    pub fn around(mhz: f64) -> Self {
        let span = nice_step(mhz * 0.10 / 50.0) * 50.0;
        let start = (mhz - span / 2.0).max(span / 100.0);
        Self {
            start_mhz: round_to(start, span / 50.0),
            stop_mhz: round_to(start, span / 50.0) + span,
            points: 51,
        }
    }

    /// The deck's own sweep range if it has one, otherwise a sweep around
    /// its design frequency (NEC's default of 299.8 MHz if it has no FR
    /// card). A coarse sweep is filled in with round-sized steps (30-75
    /// across the range) so the frequency slider moves smoothly.
    pub fn from_deck(deck: &NumericDeck) -> Self {
        let freqs = deck.frequencies();
        match freqs.as_slice() {
            [] => Self::around(299.8),
            [f] => Self::around(*f),
            [first, .., last] => {
                let (lo, hi) = if first <= last {
                    (*first, *last)
                } else {
                    (*last, *first)
                };
                let span = hi - lo;
                let points = if freqs.len() >= 21 || span <= 0.0 {
                    freqs.len()
                } else {
                    let step = nice_step(span / 30.0);
                    (span / step).round() as usize + 1
                };
                Self {
                    start_mhz: lo,
                    stop_mhz: hi,
                    points: points.clamp(1, 201),
                }
            }
        }
    }

    pub fn step_mhz(&self) -> f64 {
        if self.points > 1 {
            (self.stop_mhz - self.start_mhz) / (self.points - 1) as f64
        } else {
            0.0
        }
    }

    pub fn frequencies(&self) -> Vec<f64> {
        let step = self.step_mhz();
        (0..self.points.max(1))
            .map(|k| self.start_mhz + step * k as f64)
            .collect()
    }
}

/// Rounds a positive step to 1, 2 or 5 times a power of ten.
fn nice_step(x: f64) -> f64 {
    if x <= 0.0 || !x.is_finite() {
        return 1.0;
    }
    let p = 10f64.powf(x.log10().floor());
    let m = x / p;
    let nice = if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * p
}

fn round_to(x: f64, step: f64) -> f64 {
    (x / step).round() * step
}

/// A far-field pattern request on a regular grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatternRequest {
    /// Grid step in degrees for both theta and phi.
    pub step_deg: f64,
}

impl PatternRequest {
    pub fn new(step_deg: f64) -> Self {
        Self { step_deg }
    }

    /// The RP card: theta from the zenith down to the horizon over ground,
    /// or to the nadir in free space; phi all the way round (0 and 360 both
    /// included, so the mesh closes).
    pub fn rp_card(&self, over_ground: bool) -> NumCard {
        let step = self.step_deg.clamp(0.25, 90.0);
        let theta_end = if over_ground { 90.0 } else { 180.0 };
        let n_theta = (theta_end / step).round() as i64 + 1;
        let n_phi = (360.0 / step).round() as i64 + 1;
        // XNDA 1000: vertical/horizontal components, no normalisation,
        // power gain, no averaging.
        NumCard::new("RP", &[0, n_theta, n_phi, 1000], &[0.0, 0.0, step, step])
    }
}

/// True if the deck puts the antenna over a ground (GN with any type other
/// than -1, the "no ground" type).
pub fn has_ground(deck: &NumericDeck) -> bool {
    let mut ground = false;
    for c in deck.cards.iter().filter(|c| c.is("GN")) {
        ground = c.ints[0] != -1;
    }
    ground
}

/// Cards that only ask for output or end a run; the app supplies its own.
const OUTPUT_CARDS: &[&str] = &["FR", "PT", "PQ", "CP", "PL", "WG", "NX", "EN"];

/// The deck to run: the model up to its first calculation request, then
/// our frequencies and pattern request (or a plain calculation).
pub fn run_deck(
    deck: &NumericDeck,
    plan: &FrequencyPlan,
    pattern: Option<PatternRequest>,
) -> NumericDeck {
    let mut out = deck.clone();
    let start = deck.control_start();
    let mut cards: Vec<NumCard> = deck.cards[..start].to_vec();
    for c in &deck.cards[start..] {
        if EXECUTE_CARDS.contains(&c.mnemonic.as_str()) {
            // Anything after the first calculation belongs to a later run of
            // a multi-step deck; the app shows the first.
            break;
        }
        if !OUTPUT_CARDS.contains(&c.mnemonic.as_str()) {
            cards.push(c.clone());
        }
    }
    cards.push(NumCard::new(
        "FR",
        &[0, plan.points.max(1) as i64],
        &[plan.start_mhz, plan.step_mhz()],
    ));
    match pattern {
        Some(p) => cards.push(p.rp_card(has_ground(deck))),
        None => cards.push(NumCard::new("XQ", &[0], &[])),
    }
    out.cards = cards;
    // Thin-walled tubes: their loss depends on frequency, worked out at the
    // middle of the plan (exact for single-frequency runs).
    crate::deck::material::thin_walls(&mut out, (plan.start_mhz + plan.stop_mhz) / 2.0);
    out
}

/// A deck that only reads the geometry: the engine lists every segment
/// and stops, without solving anything. Used to show a model straight away.
pub fn geometry_deck(deck: &NumericDeck) -> NumericDeck {
    let mut out = deck.clone();
    out.cards.truncate(deck.control_start());
    out
}

/// Writes `plan` into the deck's FR card, so the file opens (here and in
/// 4nec2) with the same sweep. Leaves the deck alone when it already opens
/// with that sweep, which keeps FR cards written with variables (`f`) and
/// unchanged files as they are. Otherwise the first FR card becomes a
/// linear sweep with plain numbers (its comment kept) and any later FR
/// cards go; with no FR card one is added before the first card that asks
/// for results. Returns whether the deck changed.
pub fn write_sweep(deck: &mut Deck, plan: &FrequencyPlan) -> bool {
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1e-6);
    if let Ok(n) = deck.expand() {
        let now = FrequencyPlan::from_deck(&n);
        if now.points == plan.points
            && close(now.start_mhz, plan.start_mhz)
            && close(now.stop_mhz, plan.stop_mhz)
        {
            return false;
        }
    }
    let fields: Vec<String> = [
        "0".to_string(),
        plan.points.max(1).to_string(),
        "0".into(),
        "0".into(),
        format_real(plan.start_mhz),
        format_real(plan.step_mhz()),
    ]
    .into();
    let is = |l: &crate::deck::Line, m: &[&str]| matches!(&l.kind, LineKind::Card { mnemonic, .. } if m.contains(&mnemonic.as_str()));
    let frs: Vec<usize> = (0..deck.lines.len())
        .filter(|&i| is(&deck.lines[i], &["FR"]))
        .collect();
    match frs.split_first() {
        Some((&first, rest)) => {
            let comment = deck.lines[first].comment.clone();
            deck.lines[first].raw = card_line("FR", &fields, comment.as_deref());
            deck.reparse();
            deck.remove_lines(rest);
        }
        None => {
            let after = deck.ge_line().map_or(0, |g| g + 1);
            let at = (after..deck.lines.len())
                .find(|&i| is(&deck.lines[i], &[EXECUTE_CARDS, &["EN"]].concat()))
                .unwrap_or(deck.lines.len());
            deck.insert_lines(at, &[card_line("FR", &fields, None)]);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deck(text: &str) -> NumericDeck {
        Deck::parse(text).expand().unwrap()
    }

    #[test]
    fn sweeps_around_a_design_frequency() {
        let p = FrequencyPlan::around(14.2);
        assert_eq!(p.points, 51);
        assert!((p.step_mhz() - 0.02).abs() < 1e-12, "{p:?}");
        assert!(p.start_mhz < 14.2 && p.stop_mhz > 14.2);
        assert!((p.start_mhz - 13.7).abs() < 1e-9, "{p:?}");
        let f = p.frequencies();
        assert_eq!(f.len(), 51);
        assert!((f[50] - p.stop_mhz).abs() < 1e-9);
    }

    #[test]
    fn uses_the_decks_own_sweep() {
        let d = deck("CE\nGW 1 9 0 0 -1 0 0 1 .001\nGE\nFR 0 31 0 0 7 0.01\n");
        let p = FrequencyPlan::from_deck(&d);
        assert_eq!((p.points, p.start_mhz), (31, 7.0));
        assert!((p.stop_mhz - 7.3).abs() < 1e-9);
        // A coarse 7-point sweep, 13.9-14.5 MHz, gets 0.02 MHz steps, which
        // still land on the deck's own 0.1 MHz points.
        let d = deck("CE\nGW 1 9 0 0 -1 0 0 1 .001\nGE\nFR 0 7 0 0 13.9 0.1\n");
        let p = FrequencyPlan::from_deck(&d);
        assert_eq!(p.points, 31);
        assert!((p.step_mhz() - 0.02).abs() < 1e-9);
    }

    #[test]
    fn save_writes_the_sweep_into_the_fr_card() {
        let opens_with = |d: &Deck| FrequencyPlan::from_deck(&d.expand().unwrap());
        // A sweep that matches what the deck opens with leaves it alone,
        // formula and all.
        let text = "SY f=14.2
GW 1 9 0 0 -1 0 0 1 .001
GE
EX 0 1 5 0 1
FR 0 1 0 0 f 0 ' design
XQ
EN
";
        let mut d = Deck::parse(text);
        let same = opens_with(&d);
        assert!(!write_sweep(&mut d, &same));
        assert_eq!(d.to_text(), Deck::parse(text).to_text());
        // A new range: plain numbers, comment kept, and it reopens the same.
        let plan = FrequencyPlan {
            start_mhz: 174.0,
            stop_mhz: 230.0,
            points: 57,
        };
        assert!(write_sweep(&mut d, &plan));
        assert!(
            d.to_text().contains("FR	0	57	0	0	174	1	' design"),
            "{}",
            d.to_text()
        );
        assert_eq!(opens_with(&d), plan);
        // Several FR cards become one; none at all gets one before XQ.
        let mut d = Deck::parse(
            "GW 1 9 0 0 -1 0 0 1 .001
GE
FR 0 1 0 0 7 0
XQ
FR 0 1 0 0 14 0
XQ
EN
",
        );
        assert!(write_sweep(&mut d, &plan));
        assert_eq!(
            d.expand()
                .unwrap()
                .cards
                .iter()
                .filter(|c| c.is("FR"))
                .count(),
            1
        );
        assert_eq!(opens_with(&d), plan);
        let mut d = Deck::parse(
            "GW 1 9 0 0 -1 0 0 1 .001
GE
EX 0 1 5 0 1
XQ
EN
",
        );
        assert!(write_sweep(&mut d, &plan));
        let names: Vec<String> = d
            .expand()
            .unwrap()
            .cards
            .iter()
            .map(|c| c.mnemonic.clone())
            .collect();
        assert_eq!(names[names.len() - 3..], ["EX", "FR", "XQ"]);
        assert_eq!(opens_with(&d), plan);
    }

    #[test]
    fn builds_a_run_deck_with_our_output_cards() {
        let d = deck(
            "CE\nGW 1 9 0 0 1 0 0 3 .001\nGE 1\nGN 2 0 0 0 13 .005\nEX 0 1 5 0 1\n\
             FR 0 1 0 0 14 0\nPT -1\nRP 0 19 37 1000 0 0 5 10\nLD 5 0 0 0 5.8e7\nXQ\n",
        );
        let run = run_deck(
            &d,
            &FrequencyPlan::single(14.1),
            Some(PatternRequest::new(5.0)),
        );
        let names: Vec<&str> = run.cards.iter().map(|c| c.mnemonic.as_str()).collect();
        // The LD after the deck's RP belongs to a later run and is dropped.
        assert_eq!(names, ["GW", "GE", "GN", "EX", "FR", "RP"]);
        let rp = run.cards.last().unwrap();
        assert_eq!(rp.ints, vec![0, 19, 73, 1000]);
        assert!(has_ground(&d));
        let free = run_deck(
            &deck("CE\nGW 1 9 0 0 1 0 0 3 .001\nGE\n"),
            &FrequencyPlan::single(14.0),
            None,
        );
        assert!(free.cards.last().unwrap().is("XQ"));
    }

    #[test]
    fn free_space_pattern_covers_the_whole_sphere() {
        let rp = PatternRequest::new(5.0).rp_card(false);
        assert_eq!(rp.ints, vec![0, 37, 73, 1000]);
    }
}
