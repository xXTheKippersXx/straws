//! Reading NEC-2 text output into [`NecOutput`].
//!
//! Handles both nec2c's layout and the NEC-2D layout written by 4nec2's
//! engines (used for reference results). Sections are found by their
//! headings and rows are read as whitespace-separated numbers; NEC-2D runs
//! numbers together (`6.13467E+01-7.96113E+01`), which the tokenizer splits.

use num_complex::Complex64;

/// Everything read from one engine run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NecOutput {
    /// Segment geometry from the "SEGMENTATION DATA" table.
    pub segments: Vec<Segment>,
    /// Surface patches from the "SURFACE PATCH DATA" table.
    pub patches: Vec<Patch>,
    /// Results for each frequency, in the order computed.
    pub frequencies: Vec<FrequencyResult>,
    /// Error and warning lines the engine wrote.
    pub messages: Vec<String>,
    /// Tables whose rows are in the binary file, in order. See
    /// [`NecOutput::parse_with_binary`].
    pub(crate) binary_tables: Vec<BinaryTable>,
}

/// One wire segment, as the engine reports it (metres and degrees).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub number: u32,
    pub tag: u32,
    pub centre: [f64; 3],
    pub length: f64,
    /// Orientation angles: alpha is elevation from the XY plane, beta is
    /// azimuth from the X axis.
    pub alpha: f64,
    pub beta: f64,
    pub radius: f64,
    /// What each end joins (NEC's I- and I+): another segment's number
    /// (negative if it runs the other way), 0 for a free end, or this
    /// segment's own number where the end meets the ground.
    pub connections: [i64; 2],
}

impl Segment {
    /// Unit vector along the segment, from its orientation angles.
    pub fn direction(&self) -> [f64; 3] {
        let (a, b) = (self.alpha.to_radians(), self.beta.to_radians());
        [a.cos() * b.cos(), a.cos() * b.sin(), a.sin()]
    }

    /// The segment's two end points (metres).
    pub fn ends(&self) -> ([f64; 3], [f64; 3]) {
        let d = self.direction();
        let h = self.length / 2.0;
        let c = self.centre;
        (
            [c[0] - d[0] * h, c[1] - d[1] * h, c[2] - d[2] * h],
            [c[0] + d[0] * h, c[1] + d[1] * h, c[2] + d[2] * h],
        )
    }
}

/// A table whose rows went to the binary file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BinaryTable {
    /// A far-field pattern: frequency index, its table number, points.
    Pattern(usize, usize, usize),
    /// The current on every printed segment: frequency index, rows.
    Currents(usize, usize),
}

/// One surface patch, as the engine reports it (after every GM/GX/GR
/// copy and the subdivision where a wire joins it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Patch {
    pub number: u32,
    pub centre: [f64; 3],
    /// Unit normal, pointing out of the surface.
    pub normal: [f64; 3],
    /// Area, square metres.
    pub area: f64,
    /// Unit tangents (`t2` = normal x `t1`).
    pub t1: [f64; 3],
    pub t2: [f64; 3],
}

impl Patch {
    /// A square of the patch's area round its centre, along its tangents:
    /// how NEC sees a patch (it only uses the centre, normal and area).
    pub fn corners(&self) -> [[f64; 3]; 4] {
        let h = self.area.max(0.0).sqrt() / 2.0;
        let at = |a: f64, b: f64| {
            [0, 1, 2].map(|k| self.centre[k] + h * (a * self.t1[k] + b * self.t2[k]))
        };
        [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)]
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrequencyResult {
    pub mhz: f64,
    /// Antenna input parameters, one per voltage source.
    pub inputs: Vec<InputParams>,
    /// The same figures at network and transmission-line connection points
    /// ("structure excitation data at network connection points"); for
    /// 4nec2 current sources, the real feed points behind the helpers.
    pub network: Vec<InputParams>,
    /// Current on each segment (empty if the deck turned currents off).
    pub currents: Vec<SegmentCurrent>,
    pub power: Option<PowerBudget>,
    /// One table per RP card.
    pub patterns: Vec<PatternTable>,
    /// Near electric field points (from NE cards).
    pub near_e: Vec<NearField>,
    /// Near magnetic field points (NH card); `NearField::e` holds H (A/m).
    pub near_h: Vec<NearField>,
}

/// The electric field at one point near the antenna: position (metres)
/// and the x, y and z components (V/m, peak, for the sources as given).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearField {
    pub position: [f64; 3],
    pub e: [Complex64; 3],
}

impl NearField {
    /// Size of the field vector, |E| (V/m, peak).
    pub fn magnitude(&self) -> f64 {
        self.e.iter().map(|c| c.norm_sqr()).sum::<f64>().sqrt()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputParams {
    pub tag: u32,
    pub segment: u32,
    pub voltage: Complex64,
    pub current: Complex64,
    pub impedance: Complex64,
    pub admittance: Complex64,
    pub power: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentCurrent {
    pub segment: u32,
    pub tag: u32,
    /// Segment centre in wavelengths.
    pub centre_wl: [f64; 3],
    pub current: Complex64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PowerBudget {
    pub input: f64,
    pub radiated: f64,
    pub structure_loss: f64,
    pub network_loss: f64,
    pub efficiency_percent: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PatternTable {
    /// What the gain columns are, e.g. "POWER GAINS" or "DIRECTIVE GAINS".
    pub gain_kind: String,
    pub points: Vec<PatternPoint>,
    /// Average gain, if the RP card asked for it (a ratio, not dB).
    pub average_gain: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatternPoint {
    pub theta: f64,
    pub phi: f64,
    /// Gains in dBi; -999.99 means no field at all.
    pub gain_vert: f64,
    pub gain_hor: f64,
    pub gain_total: f64,
    pub axial_ratio: f64,
    pub tilt: f64,
    pub sense: Sense,
    /// E-theta and E-phi, magnitude (V/m) and phase (degrees).
    pub e_theta: (f64, f64),
    pub e_phi: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sense {
    None,
    Linear,
    Right,
    Left,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Word(String),
}

/// Splits a fixed-width chunk like `0.00009900.7531` (two numbers printed
/// with the same number of decimals and no space) into its numbers.
fn split_decimal_run(chunk: &str) -> Option<Vec<&str>> {
    if chunk.matches('.').count() < 2 || chunk.contains(['E', 'e', '+']) {
        return None;
    }
    let decimals = chunk.len() - chunk.rfind('.')? - 1;
    if decimals == 0 {
        return None;
    }
    let mut parts = Vec::new();
    let mut rest = chunk;
    while let Some(dot) = rest.find('.') {
        let end = (dot + 1 + decimals).min(rest.len());
        parts.push(&rest[..end]);
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        return None;
    }
    Some(parts)
}

/// Splits a line into numbers and words, separating run-together numbers.
fn tokens(line: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    for chunk in line.split_whitespace() {
        // NEC-2D marks some sources with a lone '*'; it carries no value.
        if chunk == "*" {
            continue;
        }
        // A run of '*' is a Fortran field too small for its number. Keep it
        // as "no value" so the rest of the row still lines up.
        if chunk.contains("**") {
            let mut rest = chunk;
            while !rest.is_empty() {
                if let Some(stripped) = rest.strip_prefix('*') {
                    let stars = rest.len() - stripped.trim_start_matches('*').len();
                    out.push(Tok::Num(f64::NAN));
                    rest = &rest[stars..];
                } else {
                    let end = rest.find('*').unwrap_or(rest.len());
                    out.extend(tokens(&rest[..end]));
                    rest = &rest[end..];
                }
            }
            continue;
        }
        if let Some(parts) = split_decimal_run(chunk) {
            for p in parts {
                out.push(match p.parse::<f64>() {
                    Ok(v) => Tok::Num(v),
                    Err(_) => Tok::Word(p.to_string()),
                });
            }
            continue;
        }
        let b = chunk.as_bytes();
        let mut start = 0;
        let mut i = 1;
        while i <= b.len() {
            // A sign starts a new number unless it follows an exponent mark.
            let split = i == b.len()
                || ((b[i] == b'-' || b[i] == b'+')
                    && b[i - 1] != b'E'
                    && b[i - 1] != b'e'
                    && b[i - 1].is_ascii_digit());
            if split {
                let piece = &chunk[start..i];
                out.push(match piece.parse::<f64>() {
                    Ok(v) => Tok::Num(v),
                    Err(_) => Tok::Word(piece.to_string()),
                });
                start = i;
            }
            i += 1;
        }
    }
    out
}

/// The numbers on a line, or `None` if it has any word in it.
fn numbers(line: &str) -> Option<Vec<f64>> {
    tokens(line)
        .into_iter()
        .map(|t| match t {
            Tok::Num(v) => Some(v),
            Tok::Word(_) => None,
        })
        .collect()
}

/// Reads rows of a table starting at `lines[i]`: skips headings, then
/// collects rows until a blank or non-matching line. Returns the rows and
/// the index after the table.
fn table<T>(
    lines: &[&str],
    mut i: usize,
    mut row: impl FnMut(&str) -> Option<T>,
) -> (Vec<T>, usize) {
    let mut out = Vec::new();
    // Headings: up to a few lines that don't parse as rows.
    let mut skipped = 0;
    while i < lines.len() && skipped < 8 {
        if let Some(r) = row(lines[i]) {
            out.push(r);
            i += 1;
            break;
        }
        i += 1;
        skipped += 1;
    }
    while i < lines.len() {
        match row(lines[i]) {
            Some(r) => out.push(r),
            None => break,
        }
        i += 1;
    }
    (out, i)
}

fn as_u32(v: f64) -> u32 {
    v as u32
}

impl NecOutput {
    /// True if the run produced any results at all (impedances, currents or
    /// patterns), rather than stopping with an error.
    pub fn has_results(&self) -> bool {
        self.frequencies
            .iter()
            .any(|f| !f.inputs.is_empty() || !f.currents.is_empty() || !f.patterns.is_empty())
    }

    /// Parses engine output text whose far-field pattern points and
    /// currents went to a binary file (`STRAWS_BINARY_PATTERN`, native-endian
    /// doubles: 12 a pattern point, 7 a segment current; tables in order,
    /// each marked in the text by its count). `None` if the file holds less
    /// than the text says.
    pub fn parse_with_binary(text: &str, binary: &[u8]) -> Option<NecOutput> {
        let mut out = NecOutput::parse(text);
        let mut values = binary
            .as_chunks::<8>()
            .0
            .iter()
            .map(|w| f64::from_ne_bytes(*w));
        let mut take = |n: usize| -> Option<Vec<f64>> {
            let v: Vec<f64> = values.by_ref().take(n).collect();
            (v.len() == n).then_some(v)
        };
        for table in std::mem::take(&mut out.binary_tables) {
            match table {
                BinaryTable::Pattern(f, t, count) => {
                    let mut points = Vec::with_capacity(count);
                    for _ in 0..count {
                        let v = take(12)?;
                        points.push(PatternPoint {
                            theta: v[0],
                            phi: v[1],
                            gain_vert: v[2],
                            gain_hor: v[3],
                            gain_total: v[4],
                            axial_ratio: v[5],
                            tilt: v[6],
                            sense: match v[7] as i32 {
                                1 => Sense::Linear,
                                2 => Sense::Right,
                                3 => Sense::Left,
                                _ => Sense::None,
                            },
                            e_theta: (v[8], v[9]),
                            e_phi: (v[10], v[11]),
                        });
                    }
                    if let Some(table) = out
                        .frequencies
                        .get_mut(f)
                        .and_then(|r| r.patterns.get_mut(t))
                    {
                        table.points = points;
                    }
                }
                BinaryTable::Currents(f, count) => {
                    let mut rows = Vec::with_capacity(count);
                    for _ in 0..count {
                        let v = take(7)?;
                        rows.push(SegmentCurrent {
                            segment: v[0] as u32,
                            tag: v[1] as u32,
                            centre_wl: [v[2], v[3], v[4]],
                            current: Complex64::new(v[5], v[6]),
                        });
                    }
                    if let Some(r) = out.frequencies.get_mut(f) {
                        r.currents = rows;
                    }
                }
            }
        }
        Some(out)
    }

    /// Parses engine output text.
    pub fn parse(text: &str) -> NecOutput {
        let lines: Vec<&str> = text.lines().collect();
        let mut out = NecOutput::default();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            let upper = line.trim();
            if upper.contains("SEGMENTATION DATA") {
                let (rows, next) = table(&lines, i + 1, |l| {
                    let n = numbers(l)?;
                    (n.len() == 12).then(|| Segment {
                        number: as_u32(n[0]),
                        centre: [n[1], n[2], n[3]],
                        length: n[4],
                        alpha: n[5],
                        beta: n[6],
                        radius: n[7],
                        connections: [n[8] as i64, n[10] as i64],
                        tag: as_u32(n[11]),
                    })
                });
                out.segments = rows;
                i = next;
                continue;
            }
            if upper.contains("SURFACE PATCH DATA") {
                let (rows, next) = table(&lines, i + 1, |l| {
                    let n = numbers(l)?;
                    (n.len() == 14).then(|| Patch {
                        number: as_u32(n[0]),
                        centre: [n[1], n[2], n[3]],
                        normal: [n[4], n[5], n[6]],
                        area: n[7],
                        t1: [n[8], n[9], n[10]],
                        t2: [n[11], n[12], n[13]],
                    })
                });
                out.patches = rows;
                i = next;
                continue;
            }
            if upper.starts_with("FREQUENCY") && upper.contains("MH") {
                if let Some(mhz) = tokens(upper).into_iter().find_map(|t| match t {
                    Tok::Num(v) => Some(v),
                    Tok::Word(_) => None,
                }) {
                    out.frequencies.push(FrequencyResult {
                        mhz,
                        ..Default::default()
                    });
                }
            } else if upper.contains("EXCITATION DATA AT NETWORK CONNECTION POINTS") {
                let (rows, next) = table(&lines, i + 1, input_row);
                if let Some(f) = out.frequencies.last_mut() {
                    f.network = rows;
                }
                i = next;
                continue;
            } else if upper.contains("ANTENNA INPUT PARAMETERS") {
                let (rows, next) = table(&lines, i + 1, |l| {
                    let n = numbers(l)?;
                    (n.len() == 11).then(|| InputParams {
                        tag: as_u32(n[0]),
                        segment: as_u32(n[1]),
                        voltage: Complex64::new(n[2], n[3]),
                        current: Complex64::new(n[4], n[5]),
                        impedance: Complex64::new(n[6], n[7]),
                        admittance: Complex64::new(n[8], n[9]),
                        power: n[10],
                    })
                });
                if let Some(f) = out.frequencies.last_mut() {
                    f.inputs = rows;
                }
                i = next;
                continue;
            } else if upper.contains("CURRENTS AND LOCATION")
                && let Some((k, count)) = lines[i + 1..lines.len().min(i + 8)]
                    .iter()
                    .enumerate()
                    .find_map(|(k, l)| {
                        Some((k, binary_count(l, "STRAWS BINARY CURRENTS", "SEGMENTS")?))
                    })
            {
                // The rows went to the binary file (see main.c).
                if !out.frequencies.is_empty() {
                    out.binary_tables
                        .push(BinaryTable::Currents(out.frequencies.len() - 1, count));
                }
                i += k + 2;
                continue;
            } else if upper.contains("CURRENTS AND LOCATION") {
                let (rows, next) = table(&lines, i + 1, |l| {
                    let n = numbers(l)?;
                    (n.len() == 10).then(|| SegmentCurrent {
                        segment: as_u32(n[0]),
                        tag: as_u32(n[1]),
                        centre_wl: [n[2], n[3], n[4]],
                        current: Complex64::new(n[6], n[7]),
                    })
                });
                if let Some(f) = out.frequencies.last_mut() {
                    f.currents = rows;
                }
                i = next;
                continue;
            } else if upper.contains("NEAR MAGNETIC FIELDS") {
                let (rows, next) = table(&lines, i + 1, |l| {
                    let n = numbers(l)?;
                    (n.len() == 9).then(|| NearField {
                        position: [n[0], n[1], n[2]],
                        e: [
                            Complex64::from_polar(n[3], n[4].to_radians()),
                            Complex64::from_polar(n[5], n[6].to_radians()),
                            Complex64::from_polar(n[7], n[8].to_radians()),
                        ],
                    })
                });
                if let Some(f) = out.frequencies.last_mut() {
                    f.near_h.extend(rows);
                }
                i = next;
                continue;
            } else if upper.contains("NEAR ELECTRIC FIELDS") {
                let (rows, next) = table(&lines, i + 1, |l| {
                    let n = numbers(l)?;
                    (n.len() == 9).then(|| NearField {
                        position: [n[0], n[1], n[2]],
                        e: [
                            Complex64::from_polar(n[3], n[4].to_radians()),
                            Complex64::from_polar(n[5], n[6].to_radians()),
                            Complex64::from_polar(n[7], n[8].to_radians()),
                        ],
                    })
                });
                if let Some(f) = out.frequencies.last_mut() {
                    f.near_e.extend(rows);
                }
                i = next;
                continue;
            } else if upper.contains("POWER BUDGET") {
                let mut budget = PowerBudget::default();
                let mut j = i + 1;
                while j < lines.len() && j < i + 12 {
                    let l = lines[j].trim().to_ascii_uppercase();
                    let value = l
                        .split_once('=')
                        .and_then(|(_, v)| v.split_whitespace().next()?.parse::<f64>().ok());
                    if let Some(v) = value {
                        if l.starts_with("INPUT POWER") {
                            budget.input = v;
                        } else if l.starts_with("RADIATED POWER") {
                            budget.radiated = v;
                        } else if l.starts_with("STRUCTURE LOSS") {
                            budget.structure_loss = v;
                        } else if l.starts_with("NETWORK LOSS") {
                            budget.network_loss = v;
                        } else if l.starts_with("EFFICIENCY") {
                            budget.efficiency_percent = v;
                            j += 1;
                            break;
                        }
                    }
                    j += 1;
                }
                if let Some(f) = out.frequencies.last_mut() {
                    f.power = Some(budget);
                }
                i = j;
                continue;
            } else if upper.contains("RADIATION PATTERNS")
                && let Some((k, count)) = lines[i + 1..lines.len().min(i + 8)]
                    .iter()
                    .enumerate()
                    .find_map(|(k, l)| Some((k, binary_marker(l)?)))
            {
                // The points went to the binary file (see radiation.c).
                let n = out.frequencies.len();
                if let Some(f) = out.frequencies.last_mut() {
                    f.patterns.push(PatternTable {
                        gain_kind: gain_kind(&lines[i + 1..lines.len().min(i + 4)]),
                        points: Vec::new(),
                        average_gain: None,
                    });
                    out.binary_tables.push(BinaryTable::Pattern(
                        n - 1,
                        f.patterns.len() - 1,
                        count,
                    ));
                }
                i += k + 2;
                continue;
            } else if upper.contains("RADIATION PATTERNS") {
                let gain_kind = gain_kind(&lines[i + 1..lines.len().min(i + 4)]);
                let (rows, next) = table(&lines, i + 1, pattern_row);
                if let Some(f) = out.frequencies.last_mut() {
                    f.patterns.push(PatternTable {
                        gain_kind,
                        points: rows,
                        average_gain: None,
                    });
                }
                i = next;
                continue;
            } else if upper.starts_with("AVERAGE POWER GAIN") {
                let value = upper
                    .trim_start_matches("AVERAGE POWER GAIN")
                    .trim_start_matches([':', '=', ' '])
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<f64>().ok());
                if let Some(table) = out
                    .frequencies
                    .last_mut()
                    .and_then(|f| f.patterns.last_mut())
                {
                    table.average_gain = value;
                }
            } else if upper.contains("ERROR") || upper.contains("WARNING") {
                out.messages.push(upper.to_string());
            }
            i += 1;
        }
        out
    }
}

/// A row of an input-parameters or network-connection table.
fn input_row(l: &str) -> Option<InputParams> {
    let n = numbers(l)?;
    (n.len() == 11).then(|| InputParams {
        tag: as_u32(n[0]),
        segment: as_u32(n[1]),
        voltage: Complex64::new(n[2], n[3]),
        current: Complex64::new(n[4], n[5]),
        impedance: Complex64::new(n[6], n[7]),
        admittance: Complex64::new(n[8], n[9]),
        power: n[10],
    })
}

/// What a pattern table's gain columns are, from its headings.
fn gain_kind(headings: &[&str]) -> String {
    headings
        .iter()
        .find_map(|l| {
            let l = l.to_ascii_uppercase();
            [
                "POWER GAINS",
                "DIRECTIVE GAINS",
                "VERTICAL GAINS",
                "MAJOR-MINOR",
                "GAINS",
            ]
            .iter()
            .find(|k| l.contains(*k))
            .map(|k| k.to_string())
        })
        .unwrap_or_default()
}

/// The point count of a "STRAWS BINARY PATTERN n POINTS" line.
fn binary_marker(line: &str) -> Option<usize> {
    binary_count(line, "STRAWS BINARY PATTERN", "POINTS")
}

/// The count of a "`prefix` n `suffix`" line.
fn binary_count(line: &str, prefix: &str, suffix: &str) -> Option<usize> {
    line.trim()
        .strip_prefix(prefix)?
        .trim()
        .strip_suffix(suffix)?
        .trim()
        .parse()
        .ok()
}

fn pattern_row(line: &str) -> Option<PatternPoint> {
    let toks = tokens(line);
    let mut nums = Vec::with_capacity(11);
    let mut sense = Sense::None;
    for t in toks {
        match t {
            Tok::Num(v) => nums.push(v),
            Tok::Word(w) => {
                // The sense word sits between the tilt and E-theta columns.
                if nums.len() != 7 {
                    return None;
                }
                sense = match w.as_str() {
                    "LINEAR" => Sense::Linear,
                    "RIGHT" => Sense::Right,
                    "LEFT" => Sense::Left,
                    _ => return None,
                };
            }
        }
    }
    (nums.len() == 11).then(|| PatternPoint {
        theta: nums[0],
        phi: nums[1],
        gain_vert: nums[2],
        gain_hor: nums[3],
        gain_total: nums[4],
        axial_ratio: nums[5],
        tilt: nums[6],
        sense,
        e_theta: (nums[7], nums[8]),
        e_phi: (nums[9], nums[10]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_run_together_numbers() {
        assert_eq!(
            numbers("    1    11 6.13467E+01-7.96113E+01 -2.5 1e-3").unwrap(),
            vec![1.0, 11.0, 61.3467, -79.6113, -2.5, 1e-3]
        );
        assert!(numbers("  TAG  SEG").is_none());
        // Fixed-width columns that touch, and NEC-2D's '*' source marker.
        assert_eq!(
            numbers("13 9901   0.0000   0.00009900.7531  0.00102").unwrap(),
            vec![13.0, 9901.0, 0.0, 0.0, 9900.7531, 0.00102]
        );
        assert_eq!(
            numbers("     1 *   1 1.0E+00").unwrap(),
            vec![1.0, 1.0, 1.0]
        );
        // A field too small for its number prints as stars.
        let row = numbers("    61 9901   0.0000   0.0000*********  0.00102").unwrap();
        assert_eq!(row.len(), 6);
        assert!(row[4].is_nan());
        assert_eq!(row[5], 0.00102);
    }

    const NEC2C: &str = "
                               --------- FREQUENCY --------
                                FREQUENCY : 1.3500E+01 MHz
                                WAVELENGTH: 2.2207E+01 Mtr

                        --------- ANTENNA INPUT PARAMETERS ---------
  TAG   SEG       VOLTAGE (VOLTS)         CURRENT (AMPS)         IMPEDANCE (OHMS)        ADMITTANCE (MHOS)     POWER
  No:   No:     REAL      IMAGINARY     REAL      IMAGINARY     REAL      IMAGINARY    REAL       IMAGINARY   (WATTS)
    1    11  1.0000E+00  0.0000E+00  6.0731E-03  7.8812E-03  6.1347E+01 -7.9611E+01  6.0731E-03  7.8812E-03  3.0365E-03


                               ---------- POWER BUDGET ---------
                               INPUT POWER   =  3.0365E-03 Watts
                               RADIATED POWER=  3.0365E-03 Watts
                               STRUCTURE LOSS=  0.0000E+00 Watts
                               NETWORK LOSS  =  0.0000E+00 Watts
                               EFFICIENCY    =  100.00 Percent

                             ---------- RADIATION PATTERNS -----------

 ---- ANGLES -----     ----- POWER GAINS -----       ---- POLARIZATION ----   ---- E(THETA) ----    ----- E(PHI) ------
  THETA      PHI       VERTC    HORIZ    TOTAL       AXIAL      TILT  SENSE   MAGNITUDE    PHASE    MAGNITUDE     PHASE
 DEGREES   DEGREES        DB       DB       DB       RATIO   DEGREES            VOLTS/M   DEGREES     VOLTS/M   DEGREES
    0.00      0.00   -999.99  -999.99  -999.99      0.0000      0.00         0.0000E+00      0.00  0.0000E+00      0.00
   10.00      0.00    -91.82    -9.84    -9.84      0.0001     90.00 LEFT    7.2252E-06    -11.43  9.0823E-02     43.42

  AVERAGE POWER GAIN:  4.0183E-01 - SOLID ANGLE USED IN AVERAGING: (+1.7500)*PI STERADIANS
";

    const NEC2D: &str = "
                                    FREQUENCY= 1.3500E+01 MHZ
                                    WAVELENGTH= 2.2207E+01 METERS

                                          - - - ANTENNA INPUT PARAMETERS - - -

   TAG   SEG.    VOLTAGE (VOLTS)         CURRENT (AMPS)         IMPEDANCE (OHMS)        ADMITTANCE (MHOS)      POWER
   NO.   NO.    REAL        IMAG.       REAL        IMAG.       REAL        IMAG.       REAL        IMAG.     (WATTS)
     1    11 1.00000E+00 0.00000E+00 6.07310E-03 7.88124E-03 6.13467E+01-7.96113E+01 6.07310E-03 7.88124E-03 3.03655E-03

                                                - - - RADIATION PATTERNS - - -

  - - ANGLES - -           - POWER GAINS -       - - - POLARIZATION - - -    - - - E(THETA) - - -    - - - E(PHI) - - -
  THETA     PHI        VERT.   HOR.    TOTAL      AXIAL     TILT   SENSE     MAGNITUDE    PHASE      MAGNITUDE    PHASE
 DEGREES  DEGREES       DB      DB      DB        RATIO     DEG.              VOLTS/M    DEGREES      VOLTS/M    DEGREES
   10.00     0.00     -91.82   -9.84   -9.84    0.00007    90.00  LEFT      7.22525E-06   -11.43    9.08226E-02    43.42

   AVERAGE POWER GAIN= 4.01828E-01       SOLID ANGLE USED IN AVERAGING=( 1.7500)*PI STERADIANS.
";

    #[test]
    fn reads_nec2c_output() {
        let out = NecOutput::parse(NEC2C);
        assert_eq!(out.frequencies.len(), 1);
        let f = &out.frequencies[0];
        assert_eq!(f.mhz, 13.5);
        assert_eq!(f.inputs[0].impedance, Complex64::new(61.347, -79.611));
        assert_eq!(f.power.unwrap().efficiency_percent, 100.0);
        let p = &f.patterns[0];
        assert_eq!(p.gain_kind, "POWER GAINS");
        assert_eq!(p.points.len(), 2);
        assert_eq!(p.points[0].sense, Sense::None);
        assert_eq!(p.points[1].sense, Sense::Left);
        assert_eq!(p.points[1].gain_total, -9.84);
        assert_eq!(p.average_gain, Some(0.40183));
    }

    #[test]
    fn reads_nec2d_output() {
        let out = NecOutput::parse(NEC2D);
        let f = &out.frequencies[0];
        assert_eq!(f.inputs[0].impedance, Complex64::new(61.3467, -79.6113));
        assert_eq!(f.patterns[0].points[0].e_phi, (9.08226e-2, 43.42));
        assert_eq!(f.patterns[0].average_gain, Some(0.401828));
    }
}
