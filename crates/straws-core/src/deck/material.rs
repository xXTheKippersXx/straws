//! What wires are made of: conductivity (NEC's LD 5 card) and, for tubes,
//! the wall thickness.
//!
//! NEC treats every wire as solid. That's right for tubing whose wall is a
//! few skin depths thick, which is nearly all real tubing (copper's skin
//! depth is about 17 µm at 14 MHz). A thinner wall carries the current in
//! less metal, so [`thin_walls`] swaps those LD 5 cards for an equivalent
//! distributed impedance (LD 2) worked out at the frequency being run.

use std::f64::consts::PI;

use num_complex::Complex64;

use super::edit::card_line;
use super::expand::{NumCard, NumericDeck};
use super::write::format_real;
use super::{Deck, LineKind};

/// Conductors and their conductivity in S/m.
pub const MATERIALS: &[(&str, f64)] = &[
    ("Copper", 5.8e7),
    ("Silver", 6.3e7),
    ("Gold", 4.1e7),
    ("Aluminium (pure)", 3.77e7),
    ("Aluminium 6061-T6", 2.5e7),
    ("Brass", 1.59e7),
    ("Zinc (galvanised surface)", 1.69e7),
    ("Phosphor bronze", 9.0e6),
    ("Stainless steel 304", 1.39e6),
];

/// Start of the comment that marks a tube's wall thickness on an LD 5 line.
pub const TUBE_MARK: &str = "straws: tube wall";

/// Permeability of free space.
const MU0: f64 = 4e-7 * PI;

/// A wire's material as the deck has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WireMaterial {
    /// S/m, or `None` for a perfect (lossless) conductor.
    pub conductivity: Option<f64>,
    /// Wall thickness in metres for a tube, `None` for a solid wire.
    pub wall: Option<f64>,
}

impl WireMaterial {
    pub const PERFECT: WireMaterial = WireMaterial {
        conductivity: None,
        wall: None,
    };

    /// The material's name if it's one of [`MATERIALS`].
    pub fn name(&self) -> String {
        match self.conductivity {
            None => "Perfect conductor".into(),
            Some(s) => MATERIALS
                .iter()
                .find(|(_, m)| (m - s).abs() <= 1e-3 * m)
                .map_or_else(|| format!("{s:.3e} S/m"), |(n, _)| (*n).to_string()),
        }
    }
}

/// Skin depth in metres at `mhz` for conductivity `sigma`.
pub fn skin_depth(mhz: f64, sigma: f64) -> f64 {
    1.0 / (PI * mhz * 1e6 * MU0 * sigma).sqrt()
}

/// Series impedance per metre (ohms/m) of a tube of outer radius `a` and
/// wall `t`, current on the outside (flat-conductor approximation, good
/// when the wall is thin compared with the radius).
pub fn tube_impedance(mhz: f64, sigma: f64, a: f64, t: f64) -> Complex64 {
    let delta = skin_depth(mhz, sigma);
    let k = Complex64::new(1.0, 1.0) / delta;
    let x = k * t;
    // coth(x), written to stay finite for thick walls.
    let coth = if x.re > 20.0 {
        Complex64::new(1.0, 0.0)
    } else {
        x.cosh() / x.sinh()
    };
    k / (sigma * 2.0 * PI * a) * coth
}

/// The material of the wire with tag `tag`: the last LD 5 card that covers
/// the whole wire (or every wire).
pub fn material_of(deck: &NumericDeck, tag: i64) -> WireMaterial {
    let mut m = WireMaterial::PERFECT;
    for c in &deck.cards[deck.control_start()..] {
        if c.is("LD") && c.ints[0] == -1 {
            m = WireMaterial::PERFECT;
        }
        if c.is("LD") && c.ints[0] == 5 && (c.ints[1] == tag || c.ints[1] == 0) {
            m = WireMaterial {
                conductivity: Some(c.floats[0]),
                wall: c
                    .line
                    .and_then(|l| deck.tubes.iter().find(|t| t.0 == l))
                    .map(|t| t.1),
            };
        }
    }
    m
}

/// The wall thickness written in an LD line's comment, as text (a number,
/// or a formula of the deck's variables), if it has one.
pub fn tube_wall_text(comment: Option<&str>) -> Option<&str> {
    Some(comment?.strip_prefix(TUBE_MARK)?.trim()).filter(|t| !t.is_empty())
}

/// The LD 5 line (index into `Deck::lines`) that sets the material of the
/// wire with tag `tag`, if any (the last one that covers it, as in
/// [`material_of`]).
pub fn material_line(deck: &NumericDeck, tag: i64) -> Option<usize> {
    let mut line = None;
    for c in &deck.cards[deck.control_start()..] {
        if c.is("LD") && c.ints[0] == -1 {
            line = None;
        }
        if c.is("LD") && c.ints[0] == 5 && (c.ints[1] == tag || c.ints[1] == 0) {
            line = c.line.map(|l| l - 1);
        }
    }
    line
}

fn whole_wire_ld5(kind: &LineKind) -> Option<i64> {
    match kind {
        LineKind::Card { mnemonic, fields } if mnemonic == "LD" => {
            let f = |k: usize| {
                fields
                    .get(k)
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            (f(0) == 5.0 && f(2) == 0.0 && f(3) == 0.0).then_some(f(1) as i64)
        }
        _ => None,
    }
}

impl Deck {
    /// Sets the material of wires `tags` (all wire tags are in `all_tags`,
    /// so a deck-wide LD 5 can be split up when only some wires change).
    pub fn set_material(&mut self, tags: &[i64], all_tags: &[i64], material: WireMaterial) {
        // One card per tag, however many GW cards share it (a mesh's line
        // of cells): a repeated LD 5 would count the loss again.
        let once = |list: &[i64]| {
            let mut out: Vec<i64> = Vec::new();
            for t in list {
                if !out.contains(t) {
                    out.push(*t);
                }
            }
            out
        };
        let (tags, all_tags) = (&once(tags)[..], &once(all_tags)[..]);
        // A deck-wide LD 5 (tag 0) becomes one card per other wire.
        let mut keep_for_others: Vec<String> = Vec::new();
        let mut remove = Vec::new();
        for (i, l) in self.lines.iter().enumerate() {
            match whole_wire_ld5(&l.kind) {
                Some(0) => {
                    remove.push(i);
                    if let LineKind::Card { fields, .. } = &l.kind {
                        for t in all_tags.iter().filter(|t| !tags.contains(t)) {
                            let mut f = fields.clone();
                            f.resize(5, "0".into());
                            f[1] = t.to_string();
                            keep_for_others.push(card_line("LD", &f, l.comment.as_deref()));
                        }
                    }
                }
                Some(t) if tags.contains(&t) => remove.push(i),
                _ => {}
            }
        }
        self.remove_lines(&remove);
        let mut lines = keep_for_others;
        if let Some(sigma) = material.conductivity {
            let comment = material
                .wall
                .map(|t| format!("{TUBE_MARK} {}", format_real(t)));
            for t in tags {
                let f: Vec<String> = ["5".to_string(), t.to_string(), "0".into(), "0".into()]
                    .into_iter()
                    .chain([format_real(sigma)])
                    .collect();
                lines.push(card_line("LD", &f, comment.as_deref()));
            }
        }
        if lines.is_empty() {
            return;
        }
        let at = self.ge_line().map_or(self.lines.len(), |g| g + 1);
        self.insert_lines(at, &lines);
    }
}

/// Replaces LD 5 cards for tubes whose wall is under three skin depths at
/// `mhz` with the tube's own impedance per metre (LD 2), worked out at
/// `mhz`. Thicker walls behave like solid wire and are left alone.
pub fn thin_walls(deck: &mut NumericDeck, mhz: f64) {
    if deck.tubes.is_empty() {
        return;
    }
    let geometry = deck.cards[..deck.control_start()].to_vec();
    // Radii in metres (GS scales the geometry).
    let scale: f64 = geometry
        .iter()
        .filter(|c| c.is("GS"))
        .map(|c| c.floats[0])
        .product();
    let radius = |tag: i64| {
        geometry
            .iter()
            .find(|c| c.is("GW") && c.ints[0] == tag)
            .map(|c| c.floats[6] * scale)
    };
    let mut out = Vec::with_capacity(deck.cards.len());
    for c in &deck.cards {
        let wall = c
            .line
            .and_then(|l| deck.tubes.iter().find(|t| t.0 == l))
            .map(|t| t.1);
        let (Some(t), true) = (wall, c.is("LD") && c.ints[0] == 5) else {
            out.push(c.clone());
            continue;
        };
        let sigma = c.floats[0];
        let tags: Vec<i64> = if c.ints[1] == 0 {
            geometry
                .iter()
                .filter(|g| g.is("GW"))
                .map(|g| g.ints[0])
                .collect()
        } else {
            vec![c.ints[1]]
        };
        for tag in tags {
            match radius(tag) {
                Some(a) if t < 3.0 * skin_depth(mhz, sigma) && a > 0.0 => {
                    let z = tube_impedance(mhz, sigma, a, t.min(a));
                    let l = z.im / (2.0 * PI * mhz * 1e6);
                    let mut ld = NumCard::new("LD", &[2, tag, 0, 0], &[z.re, l, 0.0]);
                    ld.line = c.line;
                    out.push(ld);
                }
                _ => {
                    let mut ld = c.clone();
                    ld.ints[1] = tag;
                    out.push(ld);
                }
            }
        }
    }
    deck.cards = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thick_tubes_act_solid_and_thin_ones_lose_more() {
        let a = 0.006;
        // A 1 mm copper wall at 14 MHz is ~60 skin depths: like solid.
        let solid = Complex64::new(1.0, 1.0) / (5.8e7 * skin_depth(14.0, 5.8e7) * 2.0 * PI * a);
        let thick = tube_impedance(14.0, 5.8e7, a, 1e-3);
        assert!((thick - solid).norm() / solid.norm() < 1e-6);
        // A wall much thinner than the skin depth: R ≈ 1/(σ·2πa·t).
        let thin = tube_impedance(14.0, 5.8e7, a, 1e-7);
        let dc = 1.0 / (5.8e7 * 2.0 * PI * a * 1e-7);
        assert!((thin.re - dc).abs() / dc < 0.01, "{thin} vs {dc}");
    }

    #[test]
    fn materials_round_trip_through_the_deck() {
        let mut deck = Deck::parse(
            "CE\nGW 1 11 0 -5 10 0 5 10 .006\nGW 2 11 1 -5 10 1 5 10 .006\nGE 0\nLD 5 0 0 0 5.8e7\nEX 0 1 6 0 1\nFR 0 1 0 0 14\nEN\n",
        );
        let alu = WireMaterial {
            conductivity: Some(2.5e7),
            wall: Some(1e-7),
        };
        deck.set_material(&[2], &[1, 2], alu);
        let n = deck.expand().unwrap();
        assert_eq!(material_of(&n, 1).name(), "Copper");
        assert_eq!(material_of(&n, 2), alu);
        assert_eq!(material_of(&n, 2).name(), "Aluminium 6061-T6");
        let mut run = n.clone();
        thin_walls(&mut run, 14.0);
        let lds: Vec<&NumCard> = run.cards.iter().filter(|c| c.is("LD")).collect();
        assert!(
            lds.iter().any(|c| c.ints[0] == 2 && c.ints[1] == 2),
            "{lds:?}"
        );
        assert!(lds.iter().any(|c| c.ints[0] == 5 && c.ints[1] == 1));
        deck.set_material(&[1, 2], &[1, 2], WireMaterial::PERFECT);
        let n = deck.expand().unwrap();
        assert!(!n.cards.iter().any(|c| c.is("LD")));
    }

    #[test]
    fn one_load_per_tag() {
        // Two GW cards on one tag (as in a mesh) get a single LD 5.
        let mut deck = Deck::parse(
            "GW 1 1 0 0 0 1 0 0 0.001
GW 1 1 1 0 0 2 0 0 0.001
GW 2 3 0 0 1 2 0 1 0.001
GE 0
EN
",
        );
        let copper = WireMaterial {
            conductivity: Some(5.8e7),
            wall: None,
        };
        deck.set_material(&[1, 1, 2], &[1, 1, 2], copper);
        let lds = deck.to_text().matches("LD").count();
        assert_eq!(lds, 2, "{}", deck.to_text());
    }
}
