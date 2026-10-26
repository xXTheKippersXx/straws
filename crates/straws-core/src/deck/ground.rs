//! Choosing the ground and the antenna's height above it, as the deck's GN
//! card (and a GM card that lifts the whole antenna).

use super::edit::card_line;
use super::expand::NumericDeck;
use super::write::format_real;
use super::{Deck, LineKind};

/// How a real ground is worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Fresnel reflection coefficients (GN 0): quick, fine for antennas
    /// more than about 0.1 λ up.
    Reflection,
    /// Sommerfeld-Norton (GN 2): exact near the ground, slower to set up.
    Sommerfeld,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ground {
    FreeSpace,
    /// A perfectly conducting ground.
    Perfect,
    /// A real ground: relative permittivity and conductivity (S/m).
    Real {
        method: Method,
        permittivity: f64,
        conductivity: f64,
    },
}

/// Common grounds (permittivity, conductivity), from the usual amateur
/// references (ARRL Antenna Book, EZNEC and 4nec2 lists).
pub const PRESETS: &[(&str, f64, f64)] = &[
    ("City, industrial (extremely poor)", 3.0, 0.001),
    ("Rocky, mountainous (very poor)", 5.0, 0.001),
    ("Sandy, dry", 10.0, 0.002),
    ("Poor (rocky, steep hills)", 13.0, 0.002),
    ("Average (pastoral, medium hills, forest)", 13.0, 0.005),
    ("Good (flat, marshy, densely wooded)", 14.0, 0.01),
    ("Very good (pastoral, low hills, rich soil)", 20.0, 0.0303),
    ("Fresh water", 80.0, 0.001),
    ("Salt water", 81.0, 5.0),
];

/// Comment on a GN card whose radial screen is only there for 4nec2 (this
/// app estimates the radials on or under the ground itself, and ignores it).
pub const SCREEN_MARK: &str = "straws: 4nec2's stand-in for the radials on or under the ground";

/// Start of the comment that remembers a real ground's variables while the
/// ground is free space or perfect.
pub const GROUND_MEMORY: &str = "straws: real ground's variables";

/// Comment that marks the GM card this module writes for the height.
pub const HEIGHT_MARK: &str = "straws: antenna height above ground";

/// The ground the deck uses (its last GN card wins, as in NEC).
pub fn ground(deck: &NumericDeck) -> Ground {
    let Some(gn) = deck.cards.iter().rfind(|c| c.is("GN")) else {
        return Ground::FreeSpace;
    };
    match gn.ints[0] {
        -1 => Ground::FreeSpace,
        1 => Ground::Perfect,
        t => Ground::Real {
            method: if t == 2 {
                Method::Sommerfeld
            } else {
                Method::Reflection
            },
            permittivity: gn.floats[0],
            conductivity: gn.floats[1],
        },
    }
}

/// NEC's buried radial-wire ground screen (GN's NRADL, SCRWL and SCRWR):
/// radials in the ground round the origin, approximated through the
/// ground's reflection coefficients (GN 0 only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialScreen {
    pub count: i64,
    /// Length of each radial (metres).
    pub length: f64,
    /// Wire radius (metres).
    pub wire_radius: f64,
}

/// The radial screen the deck's ground has, if any.
pub fn radial_screen(deck: &NumericDeck) -> Option<RadialScreen> {
    let gn = deck.cards.iter().rfind(|c| c.is("GN"))?;
    (gn.ints[0] == 0 && gn.ints[1] > 0).then(|| RadialScreen {
        count: gn.ints[1],
        length: gn.floats[2],
        wire_radius: gn.floats[3],
    })
}

/// How far the height GM card lifts the antenna now (metres), worked out
/// in `numeric` (so a height variable's slider counts).
pub fn height_now(numeric: &NumericDeck, deck: &Deck) -> f64 {
    let Some(line) = deck.height_line() else {
        return 0.0;
    };
    numeric
        .cards
        .iter()
        .find(|c| c.is("GM") && c.line == Some(line + 1))
        .map_or(0.0, |c| c.floats[5])
}

fn is_card(kind: &LineKind, mnemonic: &str) -> bool {
    matches!(kind, LineKind::Card { mnemonic: m, .. } if m == mnemonic)
}

impl Deck {
    /// Sets the ground: one GN card just after GE, replacing any GN and GD
    /// cards, and GE's ground flag to match. Variables that set a real
    /// ground's permittivity and conductivity are remembered (in a comment)
    /// while the ground is free space or perfect, and come back with it.
    pub fn set_ground(&mut self, ground: Ground) {
        // The real ground's variable fields, from its card or the memory.
        let gn = self.lines.iter().rposition(|l| is_card(&l.kind, "GN"));
        let mut kept: Option<[String; 2]> = gn.and_then(|l| match &self.lines[l].kind {
            LineKind::Card { fields, .. }
                if matches!(fields.first().map(String::as_str), Some("0" | "2")) =>
            {
                let f = |k: usize| fields.get(k).cloned().unwrap_or_default();
                let pair = [f(4), f(5)];
                pair.iter()
                    .any(|t| !t.is_empty() && super::edit::plain_number(t).is_none())
                    .then_some(pair)
            }
            _ => None,
        });
        let memory = self
            .lines
            .iter()
            .position(|l| matches!(&l.kind, LineKind::Comment(c) if c.starts_with(GROUND_MEMORY)));
        if let Some(m) = memory
            && let LineKind::Comment(c) = &self.lines[m].kind
        {
            let mut words = c[GROUND_MEMORY.len()..].split_whitespace();
            if kept.is_none()
                && let (Some(e), Some(s)) = (words.next(), words.next())
            {
                kept = Some([e.to_string(), s.to_string()]);
            }
        }
        let mut old: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| is_card(&l.kind, "GN") || is_card(&l.kind, "GD"))
            .map(|(i, _)| i)
            .collect();
        old.extend(memory);
        self.remove_lines(&old);
        let real = matches!(ground, Ground::Real { .. });
        let fields: Vec<String> = match ground {
            Ground::FreeSpace => vec!["-1".into()],
            Ground::Perfect => vec!["1".into()],
            Ground::Real {
                method,
                permittivity,
                conductivity,
            } => {
                let t = if method == Method::Sommerfeld {
                    "2"
                } else {
                    "0"
                };
                [t, "0", "0", "0"]
                    .iter()
                    .map(|s| s.to_string())
                    .chain([format_real(permittivity), format_real(conductivity)])
                    .collect()
            }
        };
        let Some(ge) = self.ge_line() else {
            return;
        };
        // GE 1: a ground is present (NEC joins wires that touch it).
        let flag = if ground == Ground::FreeSpace {
            "0"
        } else {
            "1"
        };
        self.set_field(ge, 0, flag);
        let mut lines = vec![card_line("GN", &fields, None)];
        match &kept {
            // Back to a real ground: its variables again ("-" was a number).
            Some(pair) if real => {
                let mut fields = fields.clone();
                for (k, t) in [4, 5].into_iter().zip(pair) {
                    if t != "-" && !t.is_empty() {
                        fields[k] = t.clone();
                    }
                }
                lines[0] = card_line("GN", &fields, None);
            }
            Some(pair) => {
                let shown = pair.clone().map(|t| {
                    if t.is_empty() || super::edit::plain_number(&t).is_some() {
                        "-".to_string()
                    } else {
                        t
                    }
                });
                lines.push(format!("'{GROUND_MEMORY} {} {}", shown[0], shown[1]));
            }
            None => {}
        }
        self.insert_lines(ge + 1, &lines);
    }

    /// Adds, changes or (with `None`) removes the radial screen on the
    /// deck's last GN card. Only a reflection-coefficient ground (GN 0) can
    /// have one; others are left alone.
    pub fn set_radial_screen(&mut self, screen: Option<RadialScreen>) {
        let Some(line) = self.lines.iter().rposition(|l| is_card(&l.kind, "GN")) else {
            return;
        };
        let LineKind::Card { fields, .. } = &self.lines[line].kind else {
            return;
        };
        if fields.first().and_then(|f| f.parse::<f64>().ok()) != Some(0.0) {
            return;
        }
        let mut fields = fields.clone();
        fields.resize(fields.len().max(6), "0".into());
        // A screen set by hand isn't the 4nec2 stand-in any more.
        let comment = self.lines[line]
            .comment
            .clone()
            .filter(|c| c != SCREEN_MARK);
        match screen {
            Some(s) => {
                fields.truncate(6);
                fields[1] = s.count.max(1).to_string();
                fields.extend([format_real(s.length), format_real(s.wire_radius)]);
            }
            None => {
                fields[1] = "0".into();
                fields.truncate(6);
            }
        }
        self.lines[line].raw = card_line("GN", &fields, comment.as_deref());
        self.reparse();
    }

    /// Writes (or with `None` removes) NEC's own GN radial screen as 4nec2's
    /// stand-in for radials on or under the ground, which 4nec2 can't model
    /// any other way. Only a reflection-coefficient ground (GN 0) without a
    /// screen of its own gets one. This app ignores it (see `SCREEN_MARK`).
    pub fn set_stand_in_screen(&mut self, screen: Option<RadialScreen>) {
        let Some(line) = self.lines.iter().rposition(|l| is_card(&l.kind, "GN")) else {
            return;
        };
        let marked = self.lines[line].comment.as_deref() == Some(SCREEN_MARK);
        if marked {
            self.set_radial_screen(None);
        }
        let LineKind::Card { fields, .. } = &self.lines[line].kind else {
            return;
        };
        let reflection = fields.first().and_then(|f| f.parse::<f64>().ok()) == Some(0.0);
        let own = fields
            .get(1)
            .and_then(|f| f.parse::<f64>().ok())
            .unwrap_or(0.0)
            != 0.0;
        if let Some(s) = screen
            && reflection
            && !own
        {
            self.set_radial_screen(Some(s));
            let LineKind::Card { fields, .. } = &self.lines[line].kind else {
                return;
            };
            self.lines[line].raw = card_line("GN", fields, Some(SCREEN_MARK));
            self.reparse();
        }
    }

    /// Switches a reflection-coefficient ground (GN 0) to Sommerfeld-Norton
    /// (GN 2), keeping its soil, when wires come close to the ground: the
    /// quick method is wrong there. Returns whether it changed.
    pub fn use_sommerfeld(&mut self) -> bool {
        let Some(line) = self.lines.iter().rposition(|l| is_card(&l.kind, "GN")) else {
            return false;
        };
        let LineKind::Card { fields, .. } = &self.lines[line].kind else {
            return false;
        };
        if fields.first().and_then(|f| f.parse::<f64>().ok()) != Some(0.0) {
            return false;
        }
        // NEC's radial screen only works with the quick method.
        self.set_radial_screen(None);
        self.set_field(line, 0, "2");
        true
    }

    /// The extra height this module's GM card adds (metres), if any.
    pub fn height_shift(&self) -> f64 {
        self.lines
            .iter()
            .find(|l| is_card(&l.kind, "GM") && l.comment.as_deref() == Some(HEIGHT_MARK))
            .and_then(|l| match &l.kind {
                LineKind::Card { fields, .. } => fields.get(7)?.parse::<f64>().ok(),
                _ => None,
            })
            .unwrap_or(0.0)
    }

    /// Lifts the whole antenna by `dz` metres with a GM card just before GE
    /// (replacing an earlier one), or removes it for 0.
    pub fn set_height_shift(&mut self, dz: f64) {
        let old: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| is_card(&l.kind, "GM") && l.comment.as_deref() == Some(HEIGHT_MARK))
            .map(|(i, _)| i)
            .collect();
        self.remove_lines(&old);
        if dz.abs() < 1e-12 {
            return;
        }
        let Some(ge) = self.ge_line() else {
            return;
        };
        let fields: Vec<String> = ["0", "0", "0", "0", "0", "0", "0"]
            .iter()
            .map(|s| s.to_string())
            .chain([format_real(dz), "0".into()])
            .collect();
        self.insert_lines(ge, &[card_line("GM", &fields, Some(HEIGHT_MARK))]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::edit::wires;

    #[test]
    fn ground_and_height_round_trip() {
        let mut deck = Deck::parse(
            "CE\nGW 1 11 0 -5 10 0 5 10 .001\nGE 0\nGN 2 0 0 0 13 .005\nGD 0 0 0 0 1 1 1 1\nEX 0 1 6 0 1\nFR 0 1 0 0 14\nEN\n",
        );
        let n = deck.expand().unwrap();
        assert_eq!(
            ground(&n),
            Ground::Real {
                method: Method::Sommerfeld,
                permittivity: 13.0,
                conductivity: 0.005
            }
        );
        deck.set_ground(Ground::Perfect);
        let n = deck.expand().unwrap();
        assert_eq!(ground(&n), Ground::Perfect);
        assert!(!n.cards.iter().any(|c| c.is("GD")));
        assert_eq!(n.cards.iter().find(|c| c.is("GE")).unwrap().ints[0], 1);

        // A radial screen only on a reflection-coefficient ground.
        let screen = RadialScreen {
            count: 32,
            length: 5.2,
            wire_radius: 0.001,
        };
        deck.set_radial_screen(Some(screen));
        assert_eq!(radial_screen(&deck.expand().unwrap()), None);
        deck.set_ground(Ground::Real {
            method: Method::Reflection,
            permittivity: 13.0,
            conductivity: 0.005,
        });
        deck.set_radial_screen(Some(screen));
        let n = deck.expand().unwrap();
        assert_eq!(radial_screen(&n), Some(screen));
        assert_eq!(
            n.cards.iter().rfind(|c| c.is("GN")).unwrap().floats[1],
            0.005
        );
        deck.set_radial_screen(None);
        assert_eq!(radial_screen(&deck.expand().unwrap()), None);
        // The 4nec2 stand-in for estimated radials: written, but ignored here.
        deck.set_stand_in_screen(Some(screen));
        assert!(deck.to_text().contains(SCREEN_MARK));
        assert_eq!(radial_screen(&deck.expand().unwrap()), None);
        deck.set_stand_in_screen(None);
        assert!(!deck.to_text().contains(SCREEN_MARK));
        // Low wires: the quick method gives way to Sommerfeld-Norton.
        assert!(deck.use_sommerfeld());
        assert!(matches!(
            ground(&deck.expand().unwrap()),
            Ground::Real {
                method: Method::Sommerfeld,
                ..
            }
        ));
        deck.set_ground(Ground::Perfect);

        // A real ground's variables come back after free space.
        let mut d = Deck::parse(
            "SY er=13, sg=.005\nGW 1 11 0 -5 10 0 5 10 .001\nGE 1\nGN 2 0 0 0 er sg\nEN\n",
        );
        d.set_ground(Ground::FreeSpace);
        assert!(d.to_text().contains(GROUND_MEMORY));
        d.set_ground(Ground::Perfect);
        d.set_ground(Ground::Real {
            method: Method::Reflection,
            permittivity: 5.0,
            conductivity: 0.001,
        });
        let text = d.to_text();
        assert!(text.contains("GN\t0\t0\t0\t0\ter\tsg"), "{text}");
        assert!(!text.contains(GROUND_MEMORY));

        deck.set_height_shift(2.5);
        assert_eq!(deck.height_shift(), 2.5);
        let n = deck.expand().unwrap();
        let w = wires(&deck, &n);
        assert!(!w[0].moved, "a plain shift keeps wires editable");
        assert_eq!(w[0].a[2], 12.5);
        deck.set_height_shift(0.0);
        assert_eq!(deck.height_shift(), 0.0);
        deck.set_ground(Ground::FreeSpace);
        let n = deck.expand().unwrap();
        assert_eq!(ground(&n), Ground::FreeSpace);
        assert_eq!(n.cards.iter().find(|c| c.is("GE")).unwrap().ints[0], 0);
    }
}
