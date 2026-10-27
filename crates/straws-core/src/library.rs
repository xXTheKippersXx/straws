//! The design library: proven antennas from their designers' published
//! dimensions, with build notes, made for any band.
//!
//! A published design is made for another frequency by scaling every
//! dimension, the element diameter included, so the copy behaves exactly as
//! the original. Designs worked out from formulas (Cebik's Moxon, the ARRL
//! quad) are worked out afresh for the frequency and wire size instead.

use crate::deck::Deck;
use crate::deck::plates::{PlateKind, PlateSpec};
use crate::templates::{Common, TEMPLATES, TemplateGround};
use crate::units::wavelength;

/// How a design is made for a frequency.
#[derive(Clone, Copy)]
pub enum Make {
    /// Published dimensions at `mhz` with this element diameter (mm),
    /// scaled to the frequency asked for.
    Scaled {
        mhz: f64,
        diameter_mm: f64,
        deck: fn(&Scaled) -> String,
    },
    /// Worked out from formulas for any frequency and wire diameter.
    Formula {
        mhz: f64,
        diameter_mm: f64,
        deck: fn(f64, f64) -> String,
    },
}

/// A published design being made for a frequency.
pub struct Scaled {
    /// The frequency it's made for.
    pub mhz: f64,
    /// Lengths in the original times this.
    pub scale: f64,
    pub name: &'static str,
}

impl Scaled {
    /// Millimetres in the original to metres in the copy.
    fn m(&self, mm: f64) -> f64 {
        mm * 1e-3 * self.scale
    }
}

/// One design in the library.
pub struct Design {
    pub name: &'static str,
    /// What sort of antenna ("Yagi", "Quad"...), for grouping.
    pub kind: &'static str,
    /// Who designed it.
    pub credit: &'static str,
    /// Where the dimensions come from.
    pub source: &'static str,
    pub summary: &'static str,
    /// What its designer says it does.
    pub published: &'static str,
    pub build: &'static [&'static str],
    /// The feed impedance it's built for (the SWR's reference).
    pub z0: f64,
    pub make: Make,
}

impl Design {
    /// The frequency it was published for (or a usual one).
    pub fn mhz(&self) -> f64 {
        match self.make {
            Make::Scaled { mhz, .. } | Make::Formula { mhz, .. } => mhz,
        }
    }

    /// The deck made for `mhz`; formula designs use this wire diameter.
    pub fn deck(&self, mhz: f64, diameter_mm: f64) -> String {
        match self.make {
            Make::Scaled { mhz: f0, deck, .. } => deck(&Scaled {
                mhz,
                scale: f0 / mhz,
                name: self.name,
            }),
            Make::Formula { deck, .. } => deck(mhz, diameter_mm),
        }
    }

    /// The element diameter (mm) when made for `mhz`: scaled for published
    /// designs, else as given.
    pub fn diameter_at(&self, mhz: f64, chosen_mm: f64) -> f64 {
        match self.make {
            Make::Scaled {
                mhz: f0,
                diameter_mm,
                ..
            } => diameter_mm * f0 / mhz,
            Make::Formula { .. } => chosen_mm,
        }
    }
}

fn num(v: f64) -> String {
    crate::deck::write::format_real_digits(v, 6)
}

/// Segments for a length (m): about 20 a wavelength, odd, at least `min`.
fn segs(length: f64, mhz: f64, min: usize) -> usize {
    let n = (length / (wavelength(mhz) / 20.0)).ceil().max(min as f64) as usize;
    if n.is_multiple_of(2) { n + 1 } else { n }
}

/// The deck's opening lines: what it is and where it's from.
fn heading(s: &Scaled, credit: &str, source: &str, f0: f64) -> String {
    let made = if (s.scale - 1.0).abs() < 1e-9 {
        format!("CM {} for {} MHz, as published\n", s.name, num(s.mhz))
    } else {
        format!(
            "CM {} made for {} MHz: the {} MHz design, every dimension scaled by {}\n",
            s.name,
            num(s.mhz),
            num(f0),
            num(s.scale)
        )
    };
    format!("{made}CM Design: {credit}. Dimensions: {source}\nCE\n")
}

/// A Yagi along +X from its published table: (name, position mm, length
/// mm), the driven element at `driven`, in free space.
fn yagi(
    s: &Scaled,
    credit: &str,
    source: &str,
    f0: f64,
    diameter_mm: f64,
    elements: &[(&str, f64, f64)],
    driven: usize,
) -> String {
    let mut out = heading(s, credit, source, f0);
    out.push_str(&format!(
        "SY f={}, d={}, rad=d/2000\t' frequency (MHz), element diameter (mm; {} mm in the original), radius (m)\n",
        num(s.mhz),
        num(diameter_mm * s.scale),
        num(diameter_mm)
    ));
    for (name, pos, len) in elements {
        out.push_str(&format!(
            "SY {name}_x={}, {name}_len={}\t' {name}: position and length (m); published {} and {} mm\n",
            num(s.m(*pos)),
            num(s.m(*len)),
            num(*pos),
            num(*len)
        ));
    }
    let mut ex = String::new();
    for (k, (name, _, len)) in elements.iter().enumerate() {
        let n = segs(s.m(*len), s.mhz, 11);
        out.push_str(&format!(
            "GW\t{}\t{n}\t{name}_x\t-{name}_len/2\t0\t{name}_x\t{name}_len/2\t0\trad\t' {name}\n",
            k + 1
        ));
        if k == driven {
            ex = format!("EX\t0\t{}\t{}\t0\t1\t0\n", k + 1, n.div_ceil(2));
        }
    }
    out.push_str("GE\t0\nGN\t-1\n");
    out.push_str(&ex);
    out.push_str(&sweep(s.mhz, 0.02));
    out.push_str("EN\n");
    out
}

/// An FR card sweeping ±share around `mhz` in 41 steps.
fn sweep(mhz: f64, share: f64) -> String {
    let start = mhz * (1.0 - share);
    let step = 2.0 * share * mhz / 40.0;
    format!("FR\t0\t41\t0\t0\t{}\t{}\n", num(start), num(step))
}

const DK7ZB: &str = "DK7ZB";

fn dk7zb_3(s: &Scaled) -> String {
    yagi(
        s,
        DK7ZB,
        "qsl.net/dk7zb, 3-element 28 Ohm Yagi (2 m very short Yagis), 10 mm elements",
        144.3,
        10.0,
        &[
            ("refl", 0.0, 1016.0),
            ("rad", 480.0, 957.0),
            ("dir1", 860.0, 899.0),
        ],
        1,
    )
}

fn dk7zb_5(s: &Scaled) -> String {
    yagi(
        s,
        DK7ZB,
        "qsl.net/dk7zb, 5-element 28 Ohm Yagi (2 m short Yagis), 10 mm elements",
        144.3,
        10.0,
        &[
            ("refl", 0.0, 1022.0),
            ("rad", 410.0, 958.0),
            ("dir1", 815.0, 911.0),
            ("dir2", 1475.0, 916.0),
            ("dir3", 1970.0, 880.0),
        ],
        1,
    )
}

fn dk7zb_6(s: &Scaled) -> String {
    yagi(
        s,
        DK7ZB,
        "qsl.net/dk7zb, 6-element 50 Ohm high-gain Yagi (2 m short Yagis), 10 mm elements",
        144.35,
        10.0,
        &[
            ("refl", 0.0, 1014.0),
            ("rad", 280.0, 976.0),
            ("dir1", 415.0, 950.0),
            ("dir2", 840.0, 928.0),
            ("dir3", 1485.0, 921.0),
            ("dir4", 2075.0, 880.0),
        ],
        1,
    )
}

/// Trevor Marshall's biquad: two squares of 1.2 mm wire, 30.5 mm a side,
/// 15 mm in front of a 123 mm square reflector, fed where they meet.
fn biquad(s: &Scaled) -> String {
    let mut out = heading(
        s,
        "Trevor Marshall",
        "his biquad design (30.5 mm sides of 1.2 mm wire, 123 mm reflector), with the 15 mm spacing builders settled on (martybugs.net)",
        2442.0,
    );
    out.push_str(&format!(
        "SY f={}, d={}, rad=d/2000\t' frequency (MHz), wire diameter (mm; 1.2 mm in the original), radius (m)\n",
        num(s.mhz),
        num(1.2 * s.scale)
    ));
    out.push_str(&format!(
        "SY side={}, gap={}\t' side of each square (m), gap between the feed terminals (m); published 30.5 mm sides\n",
        num(s.m(30.5)),
        num(s.m(5.0))
    ));
    out.push_str("SY q=side/sqr(2)\t' how far each corner is from the middle line (worked out)\n");
    // The two squares stand side by side (along Y) in the X = 0 plane;
    // their inner corners split into the feed terminals T (top) and B.
    let n = segs(s.m(30.5), s.mhz, 5);
    let pts = [
        ("0", "gap/2"),
        ("-q", "q"),
        ("-2*q", "0"),
        ("-q", "-q"),
        ("0", "-gap/2"),
        ("q", "-q"),
        ("2*q", "0"),
        ("q", "q"),
        ("0", "gap/2"),
    ];
    for k in 0..8 {
        let (a, b) = (pts[k], pts[k + 1]);
        out.push_str(&format!(
            "GW\t{}\t{n}\t0\t{}\t{}\t0\t{}\t{}\trad\t' {} square, side {}\n",
            k + 1,
            a.0,
            a.1,
            b.0,
            b.1,
            if k < 4 { "left" } else { "right" },
            k % 4 + 1
        ));
    }
    out.push_str("GW\t9\t1\t0\t0\t-gap/2\t0\t0\tgap/2\trad\t' feed: between the terminals\n");
    out.push_str("GE\t0\nGN\t-1\nEX\t0\t9\t1\t0\t1\t0\n");
    out.push_str(&sweep(s.mhz, 0.04));
    out.push_str("EN\n");
    // The reflector as a STRAWS plate (editable in its own panel).
    let mut deck = Deck::parse(&out);
    let lam = wavelength(s.mhz);
    let cells = ((s.m(123.0) / (lam / 10.0)).ceil() as usize).clamp(2, 40);
    let spec = PlateSpec {
        name: "reflector".into(),
        kind: PlateKind::Flat,
        centre: [-s.m(15.0), 0.0, 0.0],
        width: s.m(123.0),
        height: s.m(123.0),
        facing: 0.0,
        angle: 0.0,
        cells: [cells, cells],
        segments: 1,
        wire_radius: s.m(0.6),
    };
    if deck.add_plate(&spec, 10).is_ok() {
        return deck.to_text().replace("\r\n", "\n");
    }
    out
}

/// A template's deck for these choices (the classics come from them).
fn template(name: &str, c: &Common, values: &[(usize, f64)]) -> String {
    let Some(t) = TEMPLATES.iter().find(|t| t.name == name) else {
        return String::new();
    };
    let mut v = t.defaults();
    for (k, x) in values {
        if let Some(slot) = v.get_mut(*k) {
            *slot = *x;
        }
    }
    t.deck(c, &v)
}

fn moxon(mhz: f64, diameter_mm: f64) -> String {
    let deck = template(
        "Moxon rectangle",
        &Common {
            mhz,
            diameter_mm,
            ground: TemplateGround::FreeSpace,
            height_wl: 0.5,
        },
        &[],
    );
    format!(
        "CM Moxon rectangle for {} MHz, L. B. Cebik's (W4RNL) dimensions for this wire size\n{deck}",
        num(mhz)
    )
}

fn ground_plane(mhz: f64, diameter_mm: f64) -> String {
    // The radiator trimmed to resonance with the radials drooped 45°:
    // 0.2366 λ for wire 9.5e-5 λ thick (2 mm at 14.2 MHz), 0.0043 λ
    // shorter for each tenfold thicker (fitted to STRAWS's own runs).
    let thick = (diameter_mm * 1e-3 / wavelength(mhz)).max(1e-9);
    let vl = 0.2366 - 0.0043 * (thick / 9.47e-5).log10();
    let deck = template(
        "Vertical with radials",
        &Common {
            mhz,
            diameter_mm,
            ground: TemplateGround::FreeSpace,
            height_wl: 0.0,
        },
        &[(0, vl), (2, 4.0), (3, 45.0)],
    );
    format!(
        "CM Quarter-wave ground plane for {} MHz: four radials drooping 45 degrees\n{deck}",
        num(mhz)
    )
}

fn quad(mhz: f64, diameter_mm: f64) -> String {
    // The ARRL formulas: perimeters 1005/f and 1030/f feet.
    let lam = wavelength(mhz);
    let feet = |k: f64| k / mhz * 0.3048 / lam;
    let deck = template(
        "Quad (2 elements)",
        &Common {
            mhz,
            diameter_mm,
            ground: TemplateGround::FreeSpace,
            height_wl: 0.5,
        },
        &[(0, feet(1005.0)), (1, feet(1030.0)), (2, 0.15)],
    );
    format!(
        "CM Two-element cubical quad for {} MHz: driven loop 1005/f ft, reflector 1030/f ft, 0.15 wavelength apart\n{deck}",
        num(mhz)
    )
}

pub static LIBRARY: &[Design] = &[
    Design {
        name: "DK7ZB 3-element Yagi, 28 Ω",
        kind: "Yagi",
        credit: "DK7ZB",
        source: "https://www.qsl.net/d/dk7zb/2m_veryshort/3ele_28.htm",
        summary: "A short 2 m Yagi on an 85 cm boom, its 28 Ω feed matched to 50 Ω with DK7ZB's quarter-wave coax match.",
        published: "6.4 dBd (8.5 dBi), front-to-back 16 dB, beamwidth 61.4° (E plane) by 93.2°, at 144.3 MHz; 28 Ω.",
        build: &[
            "Elements of 10 mm tube; DK7ZB also gives lengths for 8 and 6 mm (on his page). For 145 MHz he makes every element 5 mm shorter.",
            "Mounted on insulating (polyamide) clamps above the boom, so no boom correction is needed.",
            "The radiator is fed in the middle at 28 Ω. DK7ZB's match: two 75 Ω coax cables in parallel (37.5 Ω), a quarter wave long, from the radiator to the 50 Ω feeder: 42.5 cm of 7 mm satellite TV cable (velocity factor 0.82) at 2 m, measured on the braid.",
            "Keep the two match cables exactly parallel; the line also acts as a sleeve balun. Ground the coax socket to a metal boom.",
        ],
        z0: 28.0,
        make: Make::Scaled {
            mhz: 144.3,
            diameter_mm: 10.0,
            deck: dk7zb_3,
        },
    },
    Design {
        name: "DK7ZB 5-element Yagi, 28 Ω",
        kind: "Yagi",
        credit: "DK7ZB",
        source: "https://www.qsl.net/d/dk7zb/2m-short/5-ele.htm",
        summary: "A 2 m Yagi on a 2 m boom, 28 Ω, with DK7ZB's quarter-wave coax match.",
        published: "9 dBd (11.1 dBi), front-to-back over 20 dB; 28 Ω. Stacking distance 2.40 m (vertical) or 2.55 m (horizontal).",
        build: &[
            "Elements of 10 mm tube; DK7ZB also gives lengths for 8, 6 and 4 mm (on his page). The radiator is always 10 mm.",
            "Mounted on insulating clamps above the boom (no boom correction).",
            "The radiator is fed in the middle at 28 Ω. DK7ZB's match: two 75 Ω coax cables in parallel (37.5 Ω), a quarter wave long, to the 50 Ω feeder: 42.5 cm of 7 mm satellite TV cable (velocity factor 0.82) at 2 m, measured on the braid.",
            "ON7EQ's tip for fine tuning: lengthening the second director about 3 mm a side mainly moves the resonance; trim it to centre the match where you want it.",
        ],
        z0: 28.0,
        make: Make::Scaled {
            mhz: 144.3,
            diameter_mm: 10.0,
            deck: dk7zb_5,
        },
    },
    Design {
        name: "DK7ZB 6-element high-gain Yagi, 50 Ω",
        kind: "Yagi",
        credit: "DK7ZB",
        source: "https://www.qsl.net/d/dk7zb/2m-short/5-ele.htm",
        summary: "A 2 m Yagi on a 2.1 m boom, fed directly at 50 Ω through a choke.",
        published: "9.46 dBd (11.6 dBi), front-to-back 30 dB or more, beamwidth 45.0° by 53.2°, at 144.35 MHz; 50 Ω. Stacking distance 2.45 m.",
        build: &[
            "Elements of 10 mm tube; DK7ZB also gives lengths for 8 and 6 mm (on his page).",
            "Mounted on insulating clamps above the boom (no boom correction).",
            "Fed directly with 50 Ω coax through DK7ZB's choke at the radiator (a few turns of the coax).",
            "STRAWS's model of the straight 10 mm radiator gives about 40 Ω at 144.35 MHz (SWR 1.25 on 50 Ω).",
        ],
        z0: 50.0,
        make: Make::Scaled {
            mhz: 144.35,
            diameter_mm: 10.0,
            deck: dk7zb_6,
        },
    },
    Design {
        name: "Biquad with reflector",
        kind: "Biquad",
        credit: "Trevor Marshall",
        source: "https://martybugs.net/wireless/biquad/",
        summary: "Two square loops in front of a square reflector: a small, easy panel antenna, made famous for 2.4 GHz Wi-Fi and as a dish feed.",
        published: "About 11 dBi, beamwidth about 40-50°, at 2.4 GHz.",
        build: &[
            "One 244 mm piece of 1.2 mm copper wire, bent into two squares of 30.5 mm a side (measured centre to centre of the wire); its middle goes to the coax centre conductor, its ends to the shield.",
            "Reflector: 123 x 123 mm of copper-clad board or any metal sheet (110 x 110 mm when it feeds a dish). Lips on two sides cut the rear lobes.",
            "The element stands about 15 mm in front of the reflector; the coax (with its shield in a copper tube soldered to the reflector) comes through the middle.",
            "The model's 5 mm feed gap stands for the few millimetres the centre conductor sticks out of the tube.",
            "STRAWS's model gives about 10.2 dBi and 65 + j11 Ω at 2442 MHz, a little under the builders' 11 dBi.",
        ],
        z0: 50.0,
        make: Make::Scaled {
            mhz: 2442.0,
            diameter_mm: 1.2,
            deck: biquad,
        },
    },
    Design {
        name: "Moxon rectangle",
        kind: "Moxon",
        credit: "L. B. Cebik, W4RNL (after Les Moxon, G6XN)",
        source: "Cebik's MoxGen regression formulas, for the wire size you choose",
        summary: "A two-element beam folded into a rectangle: wide bandwidth, a deep rear null and close to 50 Ω, with a small turning radius.",
        published: "Front-to-back over 20 dB at the design frequency, feed close to 50 Ω (Cebik).",
        build: &[
            "The driven element is the front side with its tails bent back; the reflector is the back side with its tails bent forward. The gap between the tail tips is critical: keep it as modelled.",
            "Built of wire on a fibreglass X frame for HF, or tube or rod on an insulating frame for VHF.",
            "Feed the middle of the driven element with 50 Ω coax through a choke balun.",
        ],
        z0: 50.0,
        make: Make::Formula {
            mhz: 14.15,
            diameter_mm: 2.0,
            deck: moxon,
        },
    },
    Design {
        name: "Two-element cubical quad",
        kind: "Quad",
        credit: "the ARRL Antenna Book's formulas",
        source: "driven loop 1005/f feet, reflector 1030/f feet (f in MHz)",
        summary: "A driven loop and a reflector loop: more gain than a two-element Yagi and a quieter, lower-angle signal.",
        published: "Loops 1005/f and 1030/f feet round; spacing 0.14-0.25 wavelength (0.15 here).",
        build: &[
            "Square loops on fibreglass spreaders (a diamond or square), fed at the middle of the bottom side for horizontal polarisation.",
            "In STRAWS's model these loops resonate about 1% below the design frequency, at about 60 Ω: shorten the driven loop a little, or let the optimiser trim it.",
        ],
        z0: 50.0,
        make: Make::Formula {
            mhz: 14.2,
            diameter_mm: 2.0,
            deck: quad,
        },
    },
    Design {
        name: "Quarter-wave ground plane",
        kind: "Vertical",
        credit: "the classic elevated ground plane",
        source: "four quarter-wave radials drooping 45°, the radiator trimmed to resonance for the wire size",
        summary: "The classic vertical: drooping the radials 45° brings the feed to about 50 Ω, so it feeds straight from coax.",
        published: "About 50 Ω with the radials drooping 45° (ARRL Antenna Book).",
        build: &[
            "Radiator and radials a quarter wave long; trim the radiator for the lowest SWR.",
            "The coax centre to the radiator, the shield to the radials; a choke on the coax below the feed keeps it from becoming part of the antenna.",
        ],
        z0: 50.0,
        make: Make::Formula {
            mhz: 14.2,
            diameter_mm: 2.0,
            deck: ground_plane,
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_design_reads_at_its_frequency_and_scaled() {
        for d in LIBRARY {
            for mhz in [d.mhz(), d.mhz() / 3.0] {
                let text = d.deck(mhz, 2.0);
                let deck = Deck::parse(&text);
                let n = deck
                    .expand()
                    .unwrap_or_else(|e| panic!("{}: {e:?}\n{text}", d.name));
                assert!(n.cards.iter().any(|c| c.is("EX")), "{}", d.name);
                assert!(n.cards.iter().any(|c| c.is("GW")), "{}", d.name);
            }
        }
    }

    #[test]
    fn scaling_keeps_the_shape() {
        let d = &LIBRARY[1];
        assert_eq!(d.mhz(), 144.3);
        let text = d.deck(50.1, 0.0);
        // Everything 144.3/50.1 = 2.88 times as long, the tube too.
        assert!(text.contains("scaled by 2.88024"), "{text}");
        assert!((d.diameter_at(50.1, 0.0) - 28.8024).abs() < 1e-3);
        let as_published = d.deck(144.3, 0.0);
        assert!(as_published.contains("as published"));
        assert!(
            as_published.contains("SY refl_x=0, refl_len=1.022"),
            "{as_published}"
        );
    }
}
