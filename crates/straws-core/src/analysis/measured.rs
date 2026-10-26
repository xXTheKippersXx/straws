//! Measured sweeps from a VNA (Touchstone .s1p/.s2p files), to overlay on
//! the model's results.

use num_complex::Complex64;

use super::feedsystem::Stage;

/// One measured frequency: the impedance the instrument saw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredPoint {
    pub mhz: f64,
    pub z: Complex64,
}

/// Reads a Touchstone file (version 1 or 2) with `ports` ports (1 for .s1p,
/// 2 for .s2p; a version 2 `[Number of Ports]` line overrides it). Only the
/// first port's reflection (S11, or Z11/Y11 turned into an impedance) is
/// kept, in order of frequency.
pub fn read_touchstone(text: &str, ports: usize) -> Result<Vec<MeasuredPoint>, String> {
    let mut ports = ports.max(1);
    let mut unit = 1e3; // MHz per file unit: GHz is the default.
    let mut param = 'S';
    let mut format = "MA".to_string();
    let mut r: f64 = 50.0;
    let mut version2 = false;
    let mut options_seen = false;
    let mut numbers = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('!').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("[version]") {
                version2 = true;
            } else if let Some(rest) = lower.strip_prefix("[number of ports]") {
                ports = rest
                    .trim()
                    .parse()
                    .map_err(|_| format!("line {}: can't read the number of ports", n + 1))?;
            } else if let Some(rest) = lower.strip_prefix("[reference]") {
                if let Some(v) = rest.split_whitespace().next().and_then(|t| t.parse().ok()) {
                    r = v;
                }
            } else if lower.starts_with("[end]") {
                break;
            }
            continue;
        }
        if let Some(opts) = line.strip_prefix('#') {
            if options_seen {
                continue; // Only the first option line counts.
            }
            options_seen = true;
            let mut words = opts.split_whitespace().map(|w| w.to_ascii_uppercase());
            while let Some(w) = words.next() {
                match w.as_str() {
                    "HZ" => unit = 1e-6,
                    "KHZ" => unit = 1e-3,
                    "MHZ" => unit = 1.0,
                    "GHZ" => unit = 1e3,
                    "S" | "Z" | "Y" => param = w.chars().next().unwrap_or('S'),
                    "H" | "G" => {
                        return Err(format!(
                            "{w} parameters aren't supported: export S parameters from the instrument"
                        ));
                    }
                    "MA" | "DB" | "RI" => format = w,
                    "R" => {
                        r = words
                            .next()
                            .and_then(|v| v.parse().ok())
                            .ok_or("the option line's R has no number after it")?;
                    }
                    other => {
                        return Err(format!(
                            "the option line has \"{other}\", which isn't a Touchstone option"
                        ));
                    }
                }
            }
            continue;
        }
        for t in line.split_whitespace() {
            numbers.push(
                t.parse::<f64>()
                    .map_err(|_| format!("line {}: \"{t}\" isn't a number", n + 1))?,
            );
        }
    }
    if !(r.is_finite() && r > 0.0) {
        return Err("the reference impedance must be above 0 Ω".into());
    }
    let per = 1 + 2 * ports * ports;
    if numbers.is_empty() {
        return Err("no measurements in the file".into());
    }
    if numbers.len() % per != 0 {
        return Err(format!(
            "the numbers don't come in groups of {per} (a frequency and {} values for {ports} port{}): is it the right kind of file?",
            per - 1,
            if ports == 1 { "" } else { "s" }
        ));
    }
    let mut out = Vec::with_capacity(numbers.len() / per);
    for g in numbers.chunks(per) {
        let (a, b) = (g[1], g[2]);
        let v = match format.as_str() {
            "RI" => Complex64::new(a, b),
            "DB" => Complex64::from_polar(10f64.powf(a / 20.0), b.to_radians()),
            _ => Complex64::from_polar(a, b.to_radians()),
        };
        let z = match param {
            // Version 1 normalises Z and Y to R; version 2 gives them in ohms and siemens.
            'Z' if version2 => v,
            'Z' => v * r,
            'Y' if version2 => 1.0 / v,
            'Y' => r / v,
            _ => r * (1.0 + v) / (1.0 - v),
        };
        let mhz = g[0] * unit;
        if mhz.is_finite() && mhz > 0.0 && z.re.is_finite() && z.im.is_finite() {
            out.push(MeasuredPoint { mhz, z });
        }
    }
    if out.is_empty() {
        return Err(
            "none of the measurements could be used (open circuits or zero frequency)".into(),
        );
    }
    out.sort_by(|a, b| a.mhz.total_cmp(&b.mhz));
    Ok(out)
}

/// The measured impedance at `mhz`, interpolated between the nearest
/// measurements (`None` outside the measured range).
pub fn interpolate(points: &[MeasuredPoint], mhz: f64) -> Option<Complex64> {
    let (first, last) = (points.first()?.mhz, points.last()?.mhz);
    // Frequencies worked out by steps may miss the ends by a rounding error.
    let tolerance = 1e-9 * last.abs().max(1.0);
    let mhz = if (mhz - first).abs() <= tolerance {
        first
    } else if (mhz - last).abs() <= tolerance {
        last
    } else {
        mhz
    };
    let k = points.partition_point(|p| p.mhz < mhz);
    let after = points.get(k)?;
    if after.mhz == mhz {
        return Some(after.z);
    }
    let before = points.get(k.checked_sub(1)?)?;
    let t = (mhz - before.mhz) / (after.mhz - before.mhz);
    Some(before.z + (after.z - before.z) * t)
}

/// The ports a file name says it has (`.s1p` → 1, `.s2p` → 2), if any.
pub fn ports_from_name(name: &str) -> Option<usize> {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next()?;
    ext.strip_prefix('s')?.strip_suffix('p')?.parse().ok()
}

/// The antenna's impedance when `z_radio` was measured at the radio end of
/// `stages` (listed from the antenna, as for
/// [`super::feedsystem::evaluate`]): the feed taken off again.
pub fn at_antenna(stages: &[Stage], z_radio: Complex64, mhz: f64) -> Complex64 {
    let m = super::feedsystem::chain_matrix(stages, mhz);
    // z_radio = (A·Z + B) / (C·Z + D), solved for Z.
    (m[1][1] * z_radio - m[0][1]) / (m[0][0] - m[1][0] * z_radio)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::feedline::CABLES;

    fn close(a: Complex64, b: Complex64) -> bool {
        (a - b).norm() < 1e-6 * (1.0 + b.norm())
    }

    #[test]
    fn formats_and_units() {
        // A 50 Ω match, a short and 100 Ω, written three ways.
        let ri = "! NanoVNA\n# MHz S RI R 50\n14.0 0 0\n14.1 -1 0\n14.2 0.333333333333 0\n";
        let p = read_touchstone(ri, 1).unwrap();
        assert_eq!(p.len(), 3);
        assert!(close(p[0].z, Complex64::new(50.0, 0.0)));
        assert!(p[1].z.norm() < 1e-9);
        assert!(close(p[2].z, Complex64::new(100.0, 0.0)));
        assert_eq!(p[2].mhz, 14.2);
        let ma = "# GHz S MA R 50\n0.0142 0.333333333333 0\n";
        assert!(close(
            read_touchstone(ma, 1).unwrap()[0].z,
            Complex64::new(100.0, 0.0)
        ));
        let db = "# Hz S DB R 50\n14200000 -9.5424250944 0\n";
        let p = read_touchstone(db, 1).unwrap();
        assert!(close(p[0].z, Complex64::new(100.0, 0.0)));
        assert!((p[0].mhz - 14.2).abs() < 1e-12);
        // No option line: GHz, S, MA, 50 Ω.
        let p = read_touchstone("1 0 0\n", 1).unwrap();
        assert_eq!(p[0].mhz, 1000.0);
        // Z normalised to R (version 1), and Y.
        let z = "# MHz Z RI R 75\n7 1 0.5\n";
        assert!(close(
            read_touchstone(z, 1).unwrap()[0].z,
            Complex64::new(75.0, 37.5)
        ));
        let y = "# MHz Y RI R 50\n7 0.5 0\n";
        assert!(close(
            read_touchstone(y, 1).unwrap()[0].z,
            Complex64::new(100.0, 0.0)
        ));
    }

    #[test]
    fn two_ports_and_version_2() {
        // S11 first, the others ignored; the frequency comes out sorted.
        let s2p = "# MHz S RI R 50\n146 0.2 0.1 0.9 0 0.9 0 0.3 0\n144 0 0 1 0 1 0 0 0\n";
        let p = read_touchstone(s2p, 2).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].mhz, 144.0);
        let g = Complex64::new(0.2, 0.1);
        assert!(close(p[1].z, 50.0 * (1.0 + g) / (1.0 - g)));
        let v2 = "[Version] 2.0\n# MHz Z RI R 50\n[Number of Ports] 1\n[Network Data]\n7 36 -12\n[End]\n";
        assert!(close(
            read_touchstone(v2, 2).unwrap()[0].z,
            Complex64::new(36.0, -12.0)
        ));
        assert_eq!(ports_from_name("Dipole 20m.S1P"), Some(1));
        assert_eq!(ports_from_name("x.s2p"), Some(2));
        assert_eq!(ports_from_name("x.csv"), None);
    }

    #[test]
    fn bad_files_say_why() {
        assert!(read_touchstone("", 1).is_err());
        assert!(
            read_touchstone("# MHz S RI\n7 0\n", 1)
                .unwrap_err()
                .contains("groups of 3")
        );
        assert!(read_touchstone("# MHz H RI\n7 0 0\n", 1).is_err());
        assert!(
            read_touchstone("# MHz S RI\n7 x 0\n", 1)
                .unwrap_err()
                .contains("line 2")
        );
        assert!(read_touchstone("# MHz S RI R 0\n7 0 0\n", 1).is_err());
        // An open circuit alone can't be used.
        assert!(read_touchstone("# MHz S RI\n7 1 0\n", 1).is_err());
    }

    #[test]
    fn interpolation() {
        let p = [
            MeasuredPoint {
                mhz: 14.0,
                z: Complex64::new(50.0, -10.0),
            },
            MeasuredPoint {
                mhz: 14.2,
                z: Complex64::new(70.0, 30.0),
            },
        ];
        assert_eq!(interpolate(&p, 14.0), Some(p[0].z));
        assert_eq!(interpolate(&p, 14.2), Some(p[1].z));
        assert!(close(
            interpolate(&p, 14.05).unwrap(),
            Complex64::new(55.0, 0.0)
        ));
        assert_eq!(interpolate(&p, 13.9), None);
        assert_eq!(interpolate(&p, 14.3), None);
    }

    #[test]
    fn the_feed_comes_off_again() {
        let cable = CABLES[2];
        let stages = [
            Stage::Balun {
                ratio: 4.0,
                loss_db: 0.3,
            },
            Stage::Line {
                cable,
                length: 12.3,
            },
        ];
        let antenna = Complex64::new(180.0, -65.0);
        let (radio, _, _) = super::super::feedsystem::evaluate(&stages, antenna, 14.1, 50.0);
        assert!(close(at_antenna(&stages, radio, 14.1), antenna));
        assert!(close(at_antenna(&[], radio, 14.1), radio));
    }
}
