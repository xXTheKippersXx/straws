//! Templates: ready-made antennas as decks built from SY variables, so
//! every dimension has a name, a slider and can be optimised.
//!
//! Each template takes the design frequency, wire diameter, ground and
//! height, plus a few of its own settings, and writes a deck. Starting
//! dimensions come from the usual amateur references; the optimiser is the
//! way to fine-tune them for a particular build.

use std::f64::consts::PI;

use crate::units::wavelength;

/// What a template setting measures (for its label and entry box).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A plain number (a ratio, factor or count).
    Number,
    /// A whole number.
    Count,
    /// Degrees.
    Degrees,
    /// Wavelengths (shown with the metres it comes to).
    Wavelengths,
    /// Megahertz.
    Mhz,
    /// Ohms.
    Ohms,
}

/// One of a template's own settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setting {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub default: f64,
    pub help: &'static str,
}

/// The ground under a template's antenna.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateGround {
    FreeSpace,
    /// Average real ground (εr 13, σ 0.005 S/m), Sommerfeld-Norton.
    Real,
    Perfect,
}

/// The settings every template takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Common {
    pub mhz: f64,
    /// Wire or tube diameter, mm.
    pub diameter_mm: f64,
    pub ground: TemplateGround,
    /// Height above ground in wavelengths (of the lowest point for
    /// verticals, of the antenna's centre for horizontal ones).
    pub height_wl: f64,
}

pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    /// The ground it's usually modelled over.
    pub ground: TemplateGround,
    /// Its usual height, wavelengths.
    pub height_wl: f64,
    pub settings: &'static [Setting],
    build: fn(&Common, &[f64]) -> Deck,
}

impl Template {
    /// The settings' default values.
    pub fn defaults(&self) -> Vec<f64> {
        self.settings.iter().map(|s| s.default).collect()
    }

    /// The deck text for these choices.
    pub fn deck(&self, common: &Common, values: &[f64]) -> String {
        let mut v = self.defaults();
        for (k, x) in values.iter().enumerate().take(v.len()) {
            v[k] = *x;
        }
        (self.build)(common, &v).text(self.name, common)
    }
}

/// A deck being written: variables, wires and sources.
#[derive(Default)]
struct Deck {
    symbols: Vec<String>,
    wires: Vec<String>,
    after_ge: Vec<String>,
    /// Frequencies: (start, step, count).
    sweep: Option<(f64, f64, usize)>,
    next_tag: usize,
}

/// Segments for a length in metres: about 20 a wavelength, odd so there's
/// a centre segment, at least `min`.
fn segs(length: f64, mhz: f64, min: usize) -> usize {
    let n = (length / (wavelength(mhz) / 20.0)).ceil().max(min as f64) as usize;
    if n.is_multiple_of(2) { n + 1 } else { n }
}

fn num(v: f64) -> String {
    crate::deck::write::format_real_digits(v, 6)
}

impl Deck {
    /// `SY a=1, b=2  ' comment`.
    fn sy(&mut self, pairs: &[(&str, String)], comment: &str) {
        let body: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let mut line = format!("SY {}", body.join(", "));
        if !comment.is_empty() {
            line.push_str("\t' ");
            line.push_str(comment);
        }
        self.symbols.push(line);
    }

    /// A straight wire between two points written as formulas. Returns
    /// its tag.
    fn wire(&mut self, segments: usize, a: [&str; 3], b: [&str; 3], comment: &str) -> usize {
        self.next_tag += 1;
        let mut line = format!(
            "GW\t{}\t{segments}\t{}\t{}\t{}\t{}\t{}\t{}\trad",
            self.next_tag, a[0], a[1], a[2], b[0], b[1], b[2]
        );
        if !comment.is_empty() {
            line.push_str("\t' ");
            line.push_str(comment);
        }
        self.wires.push(line);
        self.next_tag
    }

    fn card(&mut self, line: String) {
        self.after_ge.push(line);
    }

    fn text(&self, name: &str, c: &Common) -> String {
        let mut out = format!("CM {name} for {} MHz (from a template)\nCE\n", num(c.mhz));
        out.push_str(&format!(
            "SY f={}, lam=299.792458/f, d={}, rad=d/2000\t' design MHz, wavelength (m), wire diameter (mm), radius (m)\n",
            num(c.mhz),
            num(c.diameter_mm)
        ));
        for s in &self.symbols {
            out.push_str(s);
            out.push('\n');
        }
        for w in &self.wires {
            out.push_str(w);
            out.push('\n');
        }
        let (ge, gn) = match c.ground {
            TemplateGround::FreeSpace => (0, "GN\t-1".to_string()),
            TemplateGround::Perfect => (1, "GN\t1".to_string()),
            TemplateGround::Real => (1, "GN\t2\t0\t0\t0\t13\t0.005".to_string()),
        };
        out.push_str(&format!("GE\t{ge}\n{gn}\n"));
        for l in &self.after_ge {
            out.push_str(l);
            out.push('\n');
        }
        match self.sweep {
            Some((start, step, n)) => out.push_str(&format!(
                "FR\t0\t{n}\t0\t0\t{}\t{}\n",
                num(start),
                num(step)
            )),
            None => out.push_str(&format!("FR\t0\t1\t0\t0\t{}\t0\n", num(c.mhz))),
        }
        out.push_str("EN\n");
        out
    }
}

/// Height in metres, with a floor for free space (where it doesn't matter).
fn height(c: &Common) -> f64 {
    c.height_wl * wavelength(c.mhz)
}

fn dipole(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    d.sy(
        &[("len", num(v[0] * lam)), ("h", num(height(c)))],
        "total length (m), height (m)",
    );
    let n = segs(v[0] * lam, c.mhz, 11);
    let t = d.wire(
        n,
        ["0", "-len/2", "h"],
        ["0", "len/2", "h"],
        "the dipole, along Y",
    );
    d.card(format!("EX\t0\t{t}\t{}\t0\t1\t0", n.div_ceil(2)));
    d
}

fn inverted_v(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    d.sy(
        &[
            ("leg", num(v[0] * lam)),
            ("ang", num(v[1])),
            ("h", num(height(c))),
            ("w", num(0.02 * lam)),
        ],
        "leg length (m), included angle (deg), apex height (m), feed gap (m)",
    );
    let n = segs(v[0] * lam, c.mhz, 7);
    let feed = d.wire(
        1,
        ["0", "-w/2", "h"],
        ["0", "w/2", "h"],
        "feed point at the apex",
    );
    d.wire(
        n,
        ["0", "-w/2", "h"],
        ["0", "-w/2-leg*sin(ang/2)", "h-leg*cos(ang/2)"],
        "left leg",
    );
    d.wire(
        n,
        ["0", "w/2", "h"],
        ["0", "w/2+leg*sin(ang/2)", "h-leg*cos(ang/2)"],
        "right leg",
    );
    d.card(format!("EX\t0\t{feed}\t1\t0\t1\t0"));
    d
}

fn end_fed(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    d.sy(
        &[
            ("len", num(v[0] * lam)),
            ("cp", num(v[1] * lam)),
            ("h", num(height(c))),
        ],
        "radiator length (m), counterpoise length (m), height (m)",
    );
    let n = segs(v[0] * lam, c.mhz, 11);
    let wire = d.wire(
        n,
        ["0", "0", "h"],
        ["0", "len", "h"],
        "radiator, fed at this end",
    );
    d.wire(
        segs(v[1] * lam, c.mhz, 1),
        ["0", "0", "h"],
        ["0", "-cp", "h"],
        "short counterpoise",
    );
    d.card(format!(
        "EX\t0\t{wire}\t1\t0\t1\t0\t' high impedance: use the Matching view for the transformer"
    ));
    d
}

fn vertical(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    let radials = (v[2].round() as usize).clamp(1, 16);
    d.sy(
        &[
            ("vl", num(v[0] * lam)),
            ("rl", num(v[1] * lam)),
            ("droop", num(v[3])),
            (
                "hb",
                num(height(c).max(v[1] * lam * (v[3].to_radians().sin()) + 0.02 * lam)),
            ),
        ],
        "vertical length (m), radial length (m), radial droop (deg), base height (m)",
    );
    let n = segs(v[0] * lam, c.mhz, 9);
    let vert = d.wire(
        n,
        ["0", "0", "hb"],
        ["0", "0", "hb+vl"],
        "the vertical, fed at its base",
    );
    let rn = segs(v[1] * lam, c.mhz, 5);
    for k in 0..radials {
        let az = 360.0 * k as f64 / radials as f64;
        let x = format!("rl*cos(droop)*cos({})", num(az));
        let y = format!("rl*cos(droop)*sin({})", num(az));
        d.wire(
            rn,
            ["0", "0", "hb"],
            [&x, &y, "hb-rl*sin(droop)"],
            &format!("radial at {az:.0}°"),
        );
    }
    d.card(format!("EX\t0\t{vert}\t1\t0\t1\t0"));
    d
}

fn phased_verticals(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    d.sy(
        &[
            ("vl", num(v[0] * lam)),
            ("sp", num(v[1] * lam)),
            ("ph", num(v[2])),
        ],
        "vertical length (m), spacing (m), phase of the front element (deg)",
    );
    let n = segs(v[0] * lam, c.mhz, 9);
    let back = d.wire(
        n,
        ["0", "0", "0"],
        ["0", "0", "vl"],
        "rear vertical (on the ground)",
    );
    let front = d.wire(
        n,
        ["sp", "0", "0"],
        ["sp", "0", "vl"],
        "front vertical, towards +X",
    );
    // Current sources (4nec2's EX 6) force equal currents: with voltage
    // sources the coupling between the verticals spoils the pattern.
    d.card(format!("EX\t6\t{back}\t1\t0\t1\t0\t' 1 A"));
    d.card(format!(
        "EX\t6\t{front}\t1\t0\tcos(ph)\tsin(ph)\t' 1 A at the phase ph"
    ));
    d
}

/// A square loop in the YZ plane at `x`, side `s` (a symbol), centred at
/// height `h`, fed (if `fed`) at the middle of its bottom side.
fn square_loop(
    d: &mut Deck,
    mhz: f64,
    side_m: f64,
    x: &str,
    s: &str,
    h: &str,
    what: &str,
) -> (usize, usize) {
    let n = segs(side_m, mhz, 5);
    let lo = format!("{h}-{s}/2");
    let hi = format!("{h}+{s}/2");
    let (l, r) = (format!("-{s}/2"), format!("{s}/2"));
    let bottom = d.wire(n, [x, &l, &lo], [x, &r, &lo], &format!("{what}: bottom"));
    d.wire(n, [x, &r, &lo], [x, &r, &hi], &format!("{what}: side"));
    d.wire(n, [x, &r, &hi], [x, &l, &hi], &format!("{what}: top"));
    d.wire(n, [x, &l, &hi], [x, &l, &lo], &format!("{what}: side"));
    (bottom, n.div_ceil(2))
}

fn loop_antenna(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    d.sy(
        &[("s", num(v[0] * lam / 4.0)), ("h", num(height(c)))],
        "side length (m), centre height (m)",
    );
    let (tag, seg) = square_loop(&mut d, c.mhz, v[0] * lam / 4.0, "0", "s", "h", "loop");
    d.card(format!("EX\t0\t{tag}\t{seg}\t0\t1\t0"));
    d
}

fn quad(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    d.sy(
        &[
            ("sd", num(v[0] * lam / 4.0)),
            ("sr", num(v[1] * lam / 4.0)),
            ("sp", num(v[2] * lam)),
            ("h", num(height(c))),
        ],
        "driven side (m), reflector side (m), spacing (m), centre height (m)",
    );
    let (tag, seg) = square_loop(
        &mut d,
        c.mhz,
        v[0] * lam / 4.0,
        "0",
        "sd",
        "h",
        "driven loop",
    );
    square_loop(
        &mut d,
        c.mhz,
        v[1] * lam / 4.0,
        "-sp",
        "sr",
        "h",
        "reflector loop",
    );
    d.card(format!("EX\t0\t{tag}\t{seg}\t0\t1\t0"));
    d
}

fn yagi(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    let n = (v[0].round() as usize).clamp(2, 12);
    // Reflector, driven element, then directors a little shorter each time.
    let lengths: Vec<f64> = (0..n)
        .map(|k| match k {
            0 => 0.495,
            1 => 0.473,
            k => 0.44 - 0.005 * (k - 2) as f64,
        })
        .collect();
    let gaps: Vec<f64> = (1..n)
        .map(|k| {
            if k == 1 {
                0.2
            } else {
                0.2 + 0.02 * (k - 2) as f64
            }
        })
        .collect();
    let lens: Vec<(String, String)> = lengths
        .iter()
        .enumerate()
        .map(|(k, l)| (format!("len{}", k + 1), num(l * lam)))
        .collect();
    let lens_ref: Vec<(&str, String)> = lens.iter().map(|(a, b)| (a.as_str(), b.clone())).collect();
    d.sy(
        &lens_ref,
        "element lengths (m): reflector, driven, directors",
    );
    let sps: Vec<(String, String)> = gaps
        .iter()
        .enumerate()
        .map(|(k, g)| (format!("sp{}", k + 2), num(g * lam)))
        .collect();
    let sps_ref: Vec<(&str, String)> = sps.iter().map(|(a, b)| (a.as_str(), b.clone())).collect();
    d.sy(&sps_ref, "gap from the element before (m)");
    let xs: Vec<(String, String)> = (2..=n)
        .map(|k| {
            let prev = if k == 2 {
                String::new()
            } else {
                format!("x{}+", k - 1)
            };
            (format!("x{k}"), format!("{prev}sp{k}"))
        })
        .collect();
    let xs_ref: Vec<(&str, String)> = xs.iter().map(|(a, b)| (a.as_str(), b.clone())).collect();
    d.sy(&xs_ref, "positions along the boom (worked out)");
    d.sy(&[("h", num(height(c)))], "height (m)");
    let mut driven = 0;
    for k in 1..=n {
        let x = if k == 1 {
            "0".to_string()
        } else {
            format!("x{k}")
        };
        let (a, b) = (format!("len{k}/2"), format!("-len{k}/2"));
        let segments = segs(lengths[k - 1] * lam, c.mhz, 11);
        let what = match k {
            1 => "reflector".to_string(),
            2 => "driven element".to_string(),
            k => format!("director {}", k - 2),
        };
        let tag = d.wire(segments, [&x, &a, "h"], [&x, &b, "h"], &what);
        if k == 2 {
            driven = tag;
            d.card(format!("EX\t0\t{tag}\t{}\t0\t1\t0", segments.div_ceil(2)));
        }
    }
    let _ = driven;
    d
}

/// Moxon rectangle dimensions in wavelengths from the wire diameter in
/// wavelengths (L. B. Cebik's regression fits, as used by MoxGen).
pub fn moxon_dimensions(diameter_wl: f64) -> [f64; 5] {
    let d1 = diameter_wl.log10();
    let a = -0.000_857_142_857_1 * d1 * d1 - 0.009_571_428_571 * d1 + 0.339_857_142_9;
    let b = -0.002_142_857_143 * d1 * d1 - 0.020_357_142_86 * d1 + 0.008_285_714_286;
    let c = 0.001_809_523_381 * d1 * d1 + 0.017_809_523_81 * d1 + 0.051_642_857_14;
    let d = 0.001 * d1 + 0.071_785_714_29;
    [a, b, c, d, b + c + d]
}

fn moxon(c: &Common, _v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    let [a, b, gap, dd, _] = moxon_dimensions(c.diameter_mm * 1e-3 / lam).map(|x| x * lam);
    d.sy(
        &[
            ("a", num(a)),
            ("b", num(b)),
            ("c", num(gap)),
            ("dt", num(dd)),
            ("h", num(height(c))),
        ],
        "width (m), driver tails (m), gap (m), reflector tails (m), height (m)",
    );
    d.sy(&[("e", "b+c+dt".into())], "depth (worked out)");
    let n = segs(a, c.mhz, 11);
    let tn = segs(b, c.mhz, 1);
    let rn = segs(dd, c.mhz, 1);
    let driver = d.wire(
        n,
        ["0", "-a/2", "h"],
        ["0", "a/2", "h"],
        "driven element (front)",
    );
    d.wire(tn, ["0", "a/2", "h"], ["-b", "a/2", "h"], "driver tail");
    d.wire(tn, ["0", "-a/2", "h"], ["-b", "-a/2", "h"], "driver tail");
    d.wire(n, ["-e", "-a/2", "h"], ["-e", "a/2", "h"], "reflector");
    d.wire(
        rn,
        ["-e", "a/2", "h"],
        ["-e+dt", "a/2", "h"],
        "reflector tail",
    );
    d.wire(
        rn,
        ["-e", "-a/2", "h"],
        ["-e+dt", "-a/2", "h"],
        "reflector tail",
    );
    d.card(format!("EX\t0\t{driver}\t{}\t0\t1\t0", n.div_ceil(2)));
    d
}

fn lpda(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let (f_low, f_high, tau, sigma, z0) = (v[0].min(v[1]), v[0].max(v[1]), v[2], v[3], v[4]);
    let tau = tau.clamp(0.7, 0.98);
    let l1 = wavelength(f_low) / 2.0 * 1.05;
    let shortest = wavelength(f_high) / 2.0 * 0.95;
    let mut lengths = vec![l1];
    while *lengths.last().unwrap_or(&0.0) > shortest && lengths.len() < 30 {
        lengths.push(lengths.last().unwrap() * tau);
    }
    let n = lengths.len();
    d.sy(
        &[
            ("tau", num(tau)),
            ("sig", num(sigma)),
            ("l1", num(l1)),
            ("h", num(height(c))),
        ],
        "scale factor τ, spacing factor σ, longest element (m), height (m)",
    );
    for k in 2..=n {
        d.sy(
            &[
                (&format!("l{k}"), format!("l{}*tau", k - 1)),
                (
                    &format!("x{k}"),
                    if k == 2 {
                        "2*sig*l1".into()
                    } else {
                        format!("x{}+2*sig*l{}", k - 1, k - 1)
                    },
                ),
            ],
            "",
        );
    }
    let mut tags = Vec::new();
    for (k, len) in lengths.iter().enumerate() {
        let x = if k == 0 {
            "0".to_string()
        } else {
            format!("x{}", k + 1)
        };
        let (a, b) = (format!("l{}/2", k + 1), format!("-l{}/2", k + 1));
        let segments = segs(*len, f_high, 5);
        let tag = d.wire(
            segments,
            [&x, &a, "h"],
            [&x, &b, "h"],
            &format!("element {}", k + 1),
        );
        tags.push((tag, segments.div_ceil(2)));
    }
    // Crossed feeder between neighbours (negative Z0 = crossed), fed at the
    // short (front) end.
    for w in tags.windows(2) {
        d.card(format!(
            "TL\t{}\t{}\t{}\t{}\t{}\t0",
            w[0].0,
            w[0].1,
            w[1].0,
            w[1].1,
            num(-z0)
        ));
    }
    let (front, seg) = tags[n - 1];
    d.card(format!("EX\t0\t{front}\t{seg}\t0\t1\t0"));
    let points = 41;
    d.sweep = Some((f_low, (f_high - f_low) / (points - 1) as f64, points));
    d
}

fn helix(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    let turns = v[0].max(1.0);
    let circ = v[2] * lam;
    let radius = circ / (2.0 * PI);
    let spacing = circ * v[1].to_radians().tan();
    let length = turns * spacing;
    d.sy(
        &[
            ("hr", num(radius)),
            ("s", num(spacing)),
            ("hl", num(length)),
            // Short, but at least 8 wire radii (NEC's thin-wire limit).
            ("hf", num((0.005 * lam).max(8.0 * c.diameter_mm / 2000.0))),
        ],
        "helix radius (m), turn spacing (m), length (m), feed height (m)",
    );
    let feed = d.wire(
        1,
        ["hr", "0", "0"],
        ["hr", "0", "hf"],
        "feed from the ground plane",
    );
    let segments = (turns * 16.0).round() as usize;
    d.next_tag += 1;
    let tag = d.next_tag;
    d.wires.push(format!(
        "GH\t{tag}\t{segments}\ts\thl\thr\thr\thr\thr\trad\t' the helix (GH card)"
    ));
    d.wires.push(format!(
        "GM\t0\t0\t0\t0\t0\t0\t0\thf\t{tag}\t' lift the helix onto the feed wire"
    ));
    d.card(format!("EX\t0\t{feed}\t1\t0\t1\t0"));
    d
}

fn j_pole(c: &Common, v: &[f64]) -> Deck {
    let mut d = Deck::default();
    let lam = wavelength(c.mhz);
    d.sy(
        &[
            ("rl", num(v[0] * lam)),
            ("st", num(v[1] * lam)),
            ("sp", num(v[2] * lam)),
            ("tap", num(v[3] * lam)),
            ("hb", num(height(c))),
        ],
        "radiator (m), stub (m), stub spacing (m), feed tap above the short (m), base height (m)",
    );
    let lam20 = |l: f64| segs(l, c.mhz, 1);
    d.wire(
        1,
        ["0", "0", "hb"],
        ["sp", "0", "hb"],
        "short at the bottom of the stub",
    );
    d.wire(
        lam20(v[3] * lam),
        ["0", "0", "hb"],
        ["0", "0", "hb+tap"],
        "long side, below the tap",
    );
    d.wire(
        lam20((v[1] - v[3]) * lam),
        ["0", "0", "hb+tap"],
        ["0", "0", "hb+st"],
        "long side, stub",
    );
    d.wire(
        segs(v[0] * lam, c.mhz, 9),
        ["0", "0", "hb+st"],
        ["0", "0", "hb+st+rl"],
        "radiator",
    );
    d.wire(
        lam20(v[3] * lam),
        ["sp", "0", "hb"],
        ["sp", "0", "hb+tap"],
        "short side, below the tap",
    );
    d.wire(
        lam20((v[1] - v[3]) * lam),
        ["sp", "0", "hb+tap"],
        ["sp", "0", "hb+st"],
        "short side, open end",
    );
    let feed = d.wire(
        1,
        ["0", "0", "hb+tap"],
        ["sp", "0", "hb+tap"],
        "feed across the stub",
    );
    d.card(format!("EX\t0\t{feed}\t1\t0\t1\t0"));
    d
}

const fn s(
    key: &'static str,
    label: &'static str,
    kind: Kind,
    default: f64,
    help: &'static str,
) -> Setting {
    Setting {
        key,
        label,
        kind,
        default,
        help,
    }
}

pub static TEMPLATES: &[Template] = &[
    Template {
        name: "Dipole",
        description: "A half-wave dipole, horizontal along Y, fed in the middle.",
        ground: TemplateGround::Real,
        height_wl: 0.5,
        settings: &[s(
            "len",
            "Length",
            Kind::Wavelengths,
            0.488,
            "Tip to tip; about 0.49 λ is resonant for thin wire",
        )],
        build: dipole,
    },
    Template {
        name: "Inverted V",
        description: "A dipole hung from one support, legs sloping down, fed at the apex.",
        ground: TemplateGround::Real,
        height_wl: 0.35,
        settings: &[
            s("leg", "Leg length", Kind::Wavelengths, 0.235, "Each leg"),
            s(
                "ang",
                "Included angle",
                Kind::Degrees,
                120.0,
                "Angle between the legs",
            ),
        ],
        build: inverted_v,
    },
    Template {
        name: "End-fed half-wave",
        description: "A half-wave wire fed at one end against a short counterpoise. Its feed impedance is high (thousands of ohms): design the transformer in the Matching view.",
        ground: TemplateGround::Real,
        height_wl: 0.35,
        settings: &[
            s(
                "len",
                "Radiator",
                Kind::Wavelengths,
                0.49,
                "Length of the radiating wire",
            ),
            s(
                "cp",
                "Counterpoise",
                Kind::Wavelengths,
                0.05,
                "The short wire on the other side of the feed",
            ),
        ],
        build: end_fed,
    },
    Template {
        name: "Vertical with radials",
        description: "A quarter-wave vertical over sloping radials (an elevated ground plane). The base is raised so the radials clear the ground.",
        ground: TemplateGround::Real,
        height_wl: 0.15,
        settings: &[
            s(
                "vl",
                "Vertical",
                Kind::Wavelengths,
                0.245,
                "Length of the vertical",
            ),
            s(
                "rl",
                "Radials",
                Kind::Wavelengths,
                0.25,
                "Length of each radial",
            ),
            s("nr", "Number of radials", Kind::Count, 4.0, "1 to 16"),
            s(
                "droop",
                "Radial droop",
                Kind::Degrees,
                30.0,
                "Below horizontal; about 30-45° brings the feed near 50 Ω",
            ),
        ],
        build: vertical,
    },
    Template {
        name: "Phased verticals",
        description: "Two quarter-wave verticals on (perfect) ground, a quarter-wave apart, fed 90° apart: a cardioid towards +X.",
        ground: TemplateGround::Perfect,
        height_wl: 0.0,
        settings: &[
            s(
                "vl",
                "Verticals",
                Kind::Wavelengths,
                0.245,
                "Length of each vertical",
            ),
            s(
                "sp",
                "Spacing",
                Kind::Wavelengths,
                0.25,
                "Between the verticals",
            ),
            s(
                "ph",
                "Front element phase",
                Kind::Degrees,
                -90.0,
                "Relative to the rear element",
            ),
        ],
        build: phased_verticals,
    },
    Template {
        name: "Loop",
        description: "A full-wave square loop, vertical, fed in the middle of the bottom side (horizontal polarisation).",
        ground: TemplateGround::Real,
        height_wl: 0.5,
        settings: &[s(
            "per",
            "Perimeter",
            Kind::Wavelengths,
            1.06,
            "About 1.06 λ is resonant",
        )],
        build: loop_antenna,
    },
    Template {
        name: "Quad (2 elements)",
        description: "A cubical quad: driven loop and reflector loop, beaming towards +X.",
        ground: TemplateGround::Real,
        height_wl: 0.5,
        settings: &[
            s(
                "pd",
                "Driven perimeter",
                Kind::Wavelengths,
                1.0,
                "Driven loop",
            ),
            s(
                "pr",
                "Reflector perimeter",
                Kind::Wavelengths,
                1.05,
                "Reflector loop",
            ),
            s(
                "sp",
                "Spacing",
                Kind::Wavelengths,
                0.15,
                "Between the loops",
            ),
        ],
        build: quad,
    },
    Template {
        name: "Yagi",
        description: "A Yagi-Uda beam: reflector, driven element and directors along a boom, beaming towards +X. Every length and gap is a variable, ready for the optimiser.",
        ground: TemplateGround::Real,
        height_wl: 1.0,
        settings: &[s("n", "Elements", Kind::Count, 3.0, "2 to 12")],
        build: yagi,
    },
    Template {
        name: "Moxon rectangle",
        description: "A 2-element Moxon rectangle with L. B. Cebik's dimensions for your wire size: good front-to-back and close to 50 Ω. Beams towards +X.",
        ground: TemplateGround::Real,
        height_wl: 0.5,
        settings: &[],
        build: moxon,
    },
    Template {
        name: "Log-periodic (LPDA)",
        description: "A log-periodic dipole array covering a band, with a crossed feeder (TL cards), fed at the short end, beaming towards +X.",
        ground: TemplateGround::FreeSpace,
        height_wl: 0.5,
        settings: &[
            s(
                "flo",
                "Lowest frequency",
                Kind::Mhz,
                13.5,
                "Bottom of the band covered",
            ),
            s(
                "fhi",
                "Highest frequency",
                Kind::Mhz,
                30.0,
                "Top of the band covered",
            ),
            s(
                "tau",
                "τ (scale factor)",
                Kind::Number,
                0.9,
                "Each element is this times the one behind; higher means more elements and more gain",
            ),
            s(
                "sig",
                "σ (spacing factor)",
                Kind::Number,
                0.06,
                "Spacing over twice the element length; about 0.06-0.17",
            ),
            s(
                "z0",
                "Feeder impedance",
                Kind::Ohms,
                100.0,
                "Characteristic impedance of the boom feeder",
            ),
        ],
        build: lpda,
    },
    Template {
        name: "Helix (axial mode)",
        description: "An axial-mode helix over a (perfect) ground plane: circular polarisation off the end, about 140 Ω.",
        ground: TemplateGround::Perfect,
        height_wl: 0.0,
        settings: &[
            s("turns", "Turns", Kind::Number, 6.0, "More turns, more gain"),
            s(
                "pitch",
                "Pitch angle",
                Kind::Degrees,
                12.5,
                "12-14° is usual",
            ),
            s(
                "circ",
                "Circumference",
                Kind::Wavelengths,
                1.0,
                "About 0.75-1.33 λ for axial mode",
            ),
        ],
        build: helix,
    },
    Template {
        name: "J-pole",
        description: "A half-wave vertical end-fed by a quarter-wave matching stub, fed across the stub near its shorted end (move the tap to tune the match).",
        ground: TemplateGround::Real,
        height_wl: 1.0,
        settings: &[
            s("rl", "Radiator", Kind::Wavelengths, 0.485, "Above the stub"),
            s("st", "Stub", Kind::Wavelengths, 0.245, "Matching section"),
            s(
                "sp",
                "Stub spacing",
                Kind::Wavelengths,
                0.025,
                "Between the stub's two sides",
            ),
            s(
                "tap",
                "Feed tap",
                Kind::Wavelengths,
                0.012,
                "Above the short",
            ),
        ],
        build: j_pole,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_expands() {
        for t in TEMPLATES {
            let c = Common {
                mhz: 14.2,
                diameter_mm: 2.0,
                ground: t.ground,
                height_wl: t.height_wl,
            };
            let text = t.deck(&c, &t.defaults());
            let deck = crate::deck::Deck::parse(&text);
            let n = deck
                .expand()
                .unwrap_or_else(|e| panic!("{}: {e}\n{text}", t.name));
            assert!(n.cards.iter().any(|c| c.is("EX")), "{}", t.name);
        }
    }
}
