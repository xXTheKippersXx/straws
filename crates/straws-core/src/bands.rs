//! Amateur bands, for sweeps, optimiser goals and plot shading.
//!
//! The built-in lists are the ITU's amateur allocations in its three
//! regions (Radio Regulations Article 5, as tabulated in Wikipedia's
//! "Amateur radio frequency allocations", checked 2026-10-28), from 2200 m
//! to 13 cm. Each country's licence decides what an operator may actually
//! use (often less, sometimes more: e.g. 160 m above 1.85 MHz in Region 1
//! and 70 cm beyond 430-440 MHz are national arrangements), so users can
//! keep their own list as well.

/// One band: a name ("20 m") and its edges.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub name: String,
    pub start_mhz: f64,
    pub stop_mhz: f64,
}

impl Band {
    pub fn new(name: &str, start_mhz: f64, stop_mhz: f64) -> Band {
        Band {
            name: name.to_string(),
            start_mhz,
            stop_mhz,
        }
    }

    pub fn centre_mhz(&self) -> f64 {
        (self.start_mhz + self.stop_mhz) / 2.0
    }

    pub fn contains(&self, mhz: f64) -> bool {
        (self.start_mhz..=self.stop_mhz).contains(&mhz)
    }
}

/// (name, edges in Regions 1, 2 and 3; `None`: no allocation there).
type Row = (&'static str, [Option<(f64, f64)>; 3]);

const fn all(a: f64, b: f64) -> [Option<(f64, f64)>; 3] {
    [Some((a, b)), Some((a, b)), Some((a, b))]
}

#[rustfmt::skip]
const ITU: [Row; 19] = [
    ("2200 m", all(0.1357, 0.1378)),
    ("630 m", all(0.472, 0.479)),
    ("160 m", [Some((1.81, 2.0)), Some((1.8, 2.0)), Some((1.8, 2.0))]),
    ("80 m", [Some((3.5, 3.8)), Some((3.5, 4.0)), Some((3.5, 3.9))]),
    ("60 m", all(5.3515, 5.3665)),
    ("40 m", [Some((7.0, 7.2)), Some((7.0, 7.3)), Some((7.0, 7.2))]),
    ("30 m", all(10.1, 10.15)),
    ("20 m", all(14.0, 14.35)),
    ("17 m", all(18.068, 18.168)),
    ("15 m", all(21.0, 21.45)),
    ("12 m", all(24.89, 24.99)),
    ("10 m", all(28.0, 29.7)),
    ("6 m", [Some((50.0, 52.0)), Some((50.0, 54.0)), Some((50.0, 54.0))]),
    ("2 m", [Some((144.0, 146.0)), Some((144.0, 148.0)), Some((144.0, 148.0))]),
    ("1.25 m", [None, Some((220.0, 225.0)), None]),
    ("70 cm", all(430.0, 440.0)),
    ("33 cm", [None, Some((902.0, 928.0)), None]),
    ("23 cm", all(1240.0, 1300.0)),
    ("13 cm", all(2300.0, 2450.0)),
];

/// The ITU amateur bands in `region` (1, 2 or 3), lowest first.
pub fn itu_region(region: u8) -> Vec<Band> {
    let k = usize::from(region.clamp(1, 3) - 1);
    ITU.iter()
        .filter_map(|(name, edges)| edges[k].map(|(a, b)| Band::new(name, a, b)))
        .collect()
}

/// The band holding `mhz`, if any.
pub fn containing(bands: &[Band], mhz: f64) -> Option<&Band> {
    bands.iter().find(|b| b.contains(mhz))
}

/// The bands that overlap `lo..=hi`.
pub fn overlapping(bands: &[Band], lo: f64, hi: f64) -> impl Iterator<Item = &Band> {
    bands
        .iter()
        .filter(move |b| b.stop_mhz >= lo && b.start_mhz <= hi)
}

/// A user's own list made usable: blank names named from their edges,
/// edges in order, empty or nonsensical rows left out, sorted by frequency.
pub fn tidy(bands: &[Band]) -> Vec<Band> {
    let mut out: Vec<Band> = bands
        .iter()
        .filter(|b| b.start_mhz.is_finite() && b.stop_mhz.is_finite())
        .filter(|b| b.start_mhz > 0.0 && b.stop_mhz > 0.0 && b.start_mhz != b.stop_mhz)
        .map(|b| {
            let (lo, hi) = if b.start_mhz < b.stop_mhz {
                (b.start_mhz, b.stop_mhz)
            } else {
                (b.stop_mhz, b.start_mhz)
            };
            let name = b.name.trim();
            let name = if name.is_empty() {
                format!("{lo}-{hi} MHz")
            } else {
                name.to_string()
            };
            Band::new(&name, lo, hi)
        })
        .collect();
    out.sort_by(|a, b| a.start_mhz.total_cmp(&b.start_mhz));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_are_ordered_and_differ_where_they_should() {
        for region in 1..=3 {
            let bands = itu_region(region);
            for pair in bands.windows(2) {
                assert!(pair[0].start_mhz < pair[0].stop_mhz);
                assert!(pair[0].stop_mhz < pair[1].start_mhz, "{pair:?}");
            }
        }
        let band = |r: u8, name: &str| itu_region(r).into_iter().find(|b| b.name == name);
        assert_eq!(band(1, "80 m").unwrap().stop_mhz, 3.8);
        assert_eq!(band(2, "80 m").unwrap().stop_mhz, 4.0);
        assert_eq!(band(3, "80 m").unwrap().stop_mhz, 3.9);
        assert_eq!(band(2, "40 m").unwrap().stop_mhz, 7.3);
        assert!(band(1, "33 cm").is_none() && band(2, "33 cm").is_some());
        let r3 = itu_region(3);
        assert_eq!(containing(&r3, 14.2).unwrap().name, "20 m");
        assert!(containing(&r3, 14.5).is_none());
        let names: Vec<_> = overlapping(&r3, 13.0, 19.0)
            .map(|b| b.name.as_str())
            .collect();
        assert_eq!(names, ["20 m", "17 m"]);
    }

    #[test]
    fn own_lists_are_tidied() {
        let mine = [
            Band::new("", 7.3, 7.0),
            Band::new("bad", 0.0, 5.0),
            Band::new(" 20 m ", 14.0, 14.35),
        ];
        let t = tidy(&mine);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0], Band::new("7-7.3 MHz", 7.0, 7.3));
        assert_eq!(t[1].name, "20 m");
    }
}
