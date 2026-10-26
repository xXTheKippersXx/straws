//! Far-field patterns on a regular theta/phi grid, with the field
//! components the views offer, and 2D cuts through them.

use num_complex::Complex64;

use crate::output::PatternTable;

/// Gain values below this (dBi) mean "no field".
pub const NO_FIELD_DBI: f64 = -999.0;

/// Which part of the field a gain refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Component {
    Total,
    Vertical,
    Horizontal,
    /// Right-hand circular (IEEE convention).
    Rhcp,
    /// Left-hand circular.
    Lhcp,
}

impl Component {
    pub const ALL: [Component; 5] = [
        Component::Total,
        Component::Vertical,
        Component::Horizontal,
        Component::Rhcp,
        Component::Lhcp,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Component::Total => "Total",
            Component::Vertical => "Vertical",
            Component::Horizontal => "Horizontal",
            Component::Rhcp => "Right-hand circular",
            Component::Lhcp => "Left-hand circular",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A pattern on a regular grid: `thetas` from the zenith (0°) down, `phis`
/// counter-clockwise from +X. Gains are dBi, stored row by row (theta).
#[derive(Debug, Clone, PartialEq)]
pub struct PatternGrid {
    pub thetas: Vec<f64>,
    pub phis: Vec<f64>,
    gains: [Vec<f32>; 5],
}

/// dBi of one field component, from the total gain and the share of the
/// field's power in that component.
fn component_db(total_dbi: f64, share: f64) -> f64 {
    if total_dbi <= NO_FIELD_DBI || share <= 1e-30 {
        NO_FIELD_DBI - 0.99
    } else {
        total_dbi + 10.0 * share.log10()
    }
}

impl PatternGrid {
    /// Builds a grid from an engine pattern table, in whatever order the
    /// points come. Returns `None` if they don't form a full regular
    /// theta × phi grid.
    pub fn from_table(table: &PatternTable) -> Option<PatternGrid> {
        let pts = &table.points;
        if pts.is_empty() {
            return None;
        }
        let mut thetas: Vec<f64> = pts.iter().map(|p| p.theta).collect();
        let mut phis: Vec<f64> = pts.iter().map(|p| p.phi).collect();
        for v in [&mut thetas, &mut phis] {
            v.sort_by(f64::total_cmp);
            v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        }
        let (nt, np) = (thetas.len(), phis.len());
        if nt * np != pts.len() {
            return None;
        }
        let find = |v: &[f64], x: f64| v.iter().position(|y| (y - x).abs() < 1e-6);
        let mut gains: [Vec<f32>; 5] = std::array::from_fn(|_| vec![f32::NAN; nt * np]);
        for p in pts {
            let (it, ip) = (find(&thetas, p.theta)?, find(&phis, p.phi)?);
            let k = it * np + ip;
            let e_theta = Complex64::from_polar(p.e_theta.0, p.e_theta.1.to_radians());
            let e_phi = Complex64::from_polar(p.e_phi.0, p.e_phi.1.to_radians());
            let power = e_theta.norm_sqr() + e_phi.norm_sqr();
            // RHCP = (Eθ + jEφ)/√2 and LHCP = (Eθ − jEφ)/√2 for NEC's exp(+jωt).
            let j = Complex64::new(0.0, 1.0);
            let (rhcp, lhcp) = if power > 0.0 {
                (
                    (e_theta + j * e_phi).norm_sqr() / 2.0 / power,
                    (e_theta - j * e_phi).norm_sqr() / 2.0 / power,
                )
            } else {
                (0.0, 0.0)
            };
            gains[Component::Total.index()][k] = p.gain_total as f32;
            gains[Component::Vertical.index()][k] = p.gain_vert as f32;
            gains[Component::Horizontal.index()][k] = p.gain_hor as f32;
            gains[Component::Rhcp.index()][k] = component_db(p.gain_total, rhcp) as f32;
            gains[Component::Lhcp.index()][k] = component_db(p.gain_total, lhcp) as f32;
        }
        Some(PatternGrid {
            thetas,
            phis,
            gains,
        })
    }

    /// dBi at grid point (theta index, phi index).
    pub fn gain(&self, c: Component, it: usize, ip: usize) -> f64 {
        f64::from(self.gains[c.index()][it * self.phis.len() + ip])
    }

    /// Adds `db` to every gain (a change of efficiency, same shape).
    pub fn shift_db(&mut self, db: f64) {
        for g in self.gains.iter_mut().flatten() {
            if f64::from(*g) > NO_FIELD_DBI {
                *g += db as f32;
            }
        }
    }

    /// The strongest direction: (dBi, theta, phi).
    pub fn max(&self, c: Component) -> (f64, f64, f64) {
        let mut best = (f64::MIN, 0.0, 0.0);
        for (it, &t) in self.thetas.iter().enumerate() {
            for (ip, &p) in self.phis.iter().enumerate() {
                let g = self.gain(c, it, ip);
                if g > best.0 {
                    best = (g, t, p);
                }
            }
        }
        best
    }

    /// True if the grid only covers the upper half (a pattern over ground).
    pub fn upper_half_only(&self) -> bool {
        self.thetas.last().is_some_and(|&t| t <= 90.0 + 1e-6)
    }

    fn nearest(v: &[f64], x: f64) -> usize {
        v.iter()
            .enumerate()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(0, |(i, _)| i)
    }

    /// Gain at any direction, by bilinear interpolation between grid points
    /// (in dB, which is what the plots show).
    pub fn sample(&self, c: Component, theta: f64, phi: f64) -> f64 {
        let phi = phi.rem_euclid(360.0);
        let (t0, t1, ft) = bracket(&self.thetas, theta);
        let (p0, p1, fp) = bracket(&self.phis, phi);
        let g = |it, ip| self.gain(c, it, ip).max(NO_FIELD_DBI);
        let top = g(t0, p0) * (1.0 - fp) + g(t0, p1) * fp;
        let bottom = g(t1, p0) * (1.0 - fp) + g(t1, p1) * fp;
        top * (1.0 - ft) + bottom * ft
    }

    /// The vertical-plane cut through azimuth `phi`: points of
    /// (elevation angle measured from the horizon towards `phi` and on over
    /// the zenith to the far side, 0-180° over ground or -90-270° in free
    /// space, gain dBi). Elevation 0 is the horizon in the `phi` direction,
    /// 90 straight up and 180 the horizon behind.
    pub fn elevation_cut(&self, c: Component, phi: f64) -> Cut {
        let front = Self::nearest(&self.phis, phi.rem_euclid(360.0));
        let back = Self::nearest(&self.phis, (phi + 180.0).rem_euclid(360.0));
        let mut points = Vec::new();
        // Front half: elevation = 90 - theta.
        for (it, &t) in self.thetas.iter().enumerate().rev() {
            points.push((90.0 - t, self.gain(c, it, front)));
        }
        // Back half: elevation = 90 + theta.
        for (it, &t) in self.thetas.iter().enumerate().skip(1) {
            points.push((90.0 + t, self.gain(c, it, back)));
        }
        Cut { points }
    }

    /// The horizontal cut at `elevation` degrees above the horizon: points
    /// of (azimuth phi, gain dBi).
    pub fn azimuth_cut(&self, c: Component, elevation: f64) -> Cut {
        let theta = 90.0 - elevation;
        let points = self
            .phis
            .iter()
            .map(|&p| (p, self.sample(c, theta, p)))
            .collect();
        Cut { points }
    }
}

/// Indices either side of `x` in sorted `v`, and the fraction between them.
fn bracket(v: &[f64], x: f64) -> (usize, usize, f64) {
    if v.len() < 2 || x <= v[0] {
        return (0, 0, 0.0);
    }
    for i in 1..v.len() {
        if x <= v[i] {
            let f = (x - v[i - 1]) / (v[i] - v[i - 1]);
            return (i - 1, i, f);
        }
    }
    let last = v.len() - 1;
    (last, last, 0.0)
}

/// A 2D pattern cut: (angle in degrees, gain in dBi).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cut {
    pub points: Vec<(f64, f64)>,
}

/// Key figures of a cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutFigures {
    pub max_dbi: f64,
    pub max_angle: f64,
    /// Width of the main lobe between its -3 dB points, if both are found.
    pub beamwidth: Option<f64>,
    /// Peak gain minus the gain 180° away, if the cut covers both.
    pub front_to_back: Option<f64>,
}

impl Cut {
    /// Peak gain, -3 dB beamwidth and front-to-back ratio.
    pub fn figures(&self) -> Option<CutFigures> {
        let (imax, &(max_angle, max_dbi)) = self
            .points
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.1.total_cmp(&b.1.1))?;
        if max_dbi <= NO_FIELD_DBI {
            return None;
        }
        let target = max_dbi - 3.0;
        let n = self.points.len();
        // Walk each way from the peak until the gain falls below -3 dB.
        let crossing = |step: isize| -> Option<f64> {
            let mut i = imax as isize;
            for _ in 0..n {
                let j = i + step;
                if j < 0 || j >= n as isize {
                    return None;
                }
                let (a0, g0) = self.points[i as usize];
                let (a1, g1) = self.points[j as usize];
                if g1 < target {
                    let f = (g0 - target) / (g0 - g1);
                    return Some(a0 + (a1 - a0) * f);
                }
                i = j;
            }
            None
        };
        let beamwidth = match (crossing(-1), crossing(1)) {
            (Some(lo), Some(hi)) => Some((hi - lo).abs()),
            _ => None,
        };
        let back_angle = max_angle + 180.0;
        let back = self.points.iter().find(|(a, _)| {
            let d = (a - back_angle).rem_euclid(360.0);
            d < 1e-6 || d > 360.0 - 1e-6
        });
        let front_to_back = back.map(|&(_, g)| max_dbi - g.max(NO_FIELD_DBI));
        Some(CutFigures {
            max_dbi,
            max_angle,
            beamwidth,
            front_to_back,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{PatternPoint, Sense};

    fn point(
        theta: f64,
        phi: f64,
        e_theta: (f64, f64),
        e_phi: (f64, f64),
        total: f64,
    ) -> PatternPoint {
        PatternPoint {
            theta,
            phi,
            gain_vert: total,
            gain_hor: NO_FIELD_DBI - 0.99,
            gain_total: total,
            axial_ratio: 0.0,
            tilt: 0.0,
            sense: Sense::Linear,
            e_theta,
            e_phi,
        }
    }

    /// A made-up dipole-like pattern: gain = 2 + 10 log10(sin²θ) dBi.
    fn table(step: f64) -> PatternTable {
        let mut points = Vec::new();
        let n_t = (180.0 / step) as usize;
        let n_p = (360.0 / step) as usize;
        for ip in 0..=n_p {
            for it in 0..=n_t {
                let (t, p) = (it as f64 * step, ip as f64 * step);
                let s = t.to_radians().sin().powi(2);
                let g = if s > 1e-12 {
                    2.0 + 10.0 * s.log10()
                } else {
                    -999.99
                };
                points.push(point(t, p, (s.sqrt(), 0.0), (0.0, 0.0), g));
            }
        }
        PatternTable {
            gain_kind: "POWER GAINS".into(),
            points,
            average_gain: None,
        }
    }

    #[test]
    fn builds_grid_and_finds_peak() {
        let g = PatternGrid::from_table(&table(10.0)).unwrap();
        assert_eq!((g.thetas.len(), g.phis.len()), (19, 37));
        let (max, theta, _) = g.max(Component::Total);
        assert!((max - 2.0).abs() < 1e-6);
        assert_eq!(theta, 90.0);
        assert!(!g.upper_half_only());
        // A purely theta-polarised field splits evenly into RHCP and LHCP.
        assert!((g.gain(Component::Rhcp, 9, 0) - (2.0 - 3.0103)).abs() < 1e-3);
    }

    #[test]
    fn circular_components_follow_the_ieee_convention() {
        // Eθ = 1, Eφ = -j: pure right-hand circular for exp(+jωt).
        let mut t = table(90.0);
        for p in &mut t.points {
            p.e_theta = (1.0, 0.0);
            p.e_phi = (1.0, -90.0);
            p.gain_total = 0.0;
        }
        let g = PatternGrid::from_table(&t).unwrap();
        assert!(g.gain(Component::Rhcp, 1, 0).abs() < 1e-6);
        assert!(g.gain(Component::Lhcp, 1, 0) < -200.0);
    }

    #[test]
    fn cuts_and_figures() {
        let g = PatternGrid::from_table(&table(5.0)).unwrap();
        let el = g.elevation_cut(Component::Total, 0.0);
        let f = el.figures().unwrap();
        // Peak at the horizon (elevation 0 or 180), beamwidth of a short
        // dipole is 90°.
        assert!((f.max_dbi - 2.0).abs() < 1e-6);
        assert!((f.beamwidth.unwrap() - 90.0).abs() < 1.0, "{f:?}");
        let az = g.azimuth_cut(Component::Total, 0.0);
        let f = az.figures().unwrap();
        assert!((f.max_dbi - 2.0).abs() < 1e-6);
        assert_eq!(f.beamwidth, None); // omnidirectional
        assert!(f.front_to_back.unwrap().abs() < 1e-9);
        // Interpolation half way between grid rows.
        assert!(
            (g.sample(Component::Total, 92.5, 7.5) - g.gain(Component::Total, 18, 1)).abs() < 0.02
        );
    }
}
