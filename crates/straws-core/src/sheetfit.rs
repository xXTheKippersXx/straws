//! Thin solid sheets (a plate, a corner reflector, a dish from a CAD
//! model) recognised from points on their surface and fitted as one of
//! STRAWS's reflectors: which way it faces, its profile (flat, V, parabolic
//! cylinder, dish, two curvatures, or a formula), its outline, aperture and
//! thickness.

use crate::deck::reflectors::{Outline, Profile, ReflectorSpec};

type P3 = [f64; 3];

/// A sheet fitted as a reflector, in the units of the points it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetFit {
    pub outline: Outline,
    pub shape: Shape,
    /// The vertex (the surface's turning point, or the aperture's middle).
    pub vertex: P3,
    /// Azimuth and elevation of the direction it faces (degrees).
    pub facing: [f64; 2],
    /// Aperture across and up.
    pub size: [f64; 2],
    /// The aperture's middle off the vertex (across, up).
    pub offset: [f64; 2],
    pub thickness: f64,
    /// How far the surface strays from the fitted shape, besides the
    /// thickness (the typical distance).
    pub misfit: f64,
}

/// A fitted surface, the depth towards the focus at (across, up) from the
/// vertex.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Flat,
    /// Folded along the up line; the angle between its sides (degrees).
    Vee(f64),
    /// u²/(4f): curved across.
    ParabolicCylinder(f64),
    /// (u² + v²)/(4f).
    Paraboloid(f64),
    /// u²/(4f) + v²/(4f2).
    TwoCurvatures(f64, f64),
    /// A circular cylinder of this radius, curved across.
    Cylinder(f64),
    /// A sphere of this radius.
    Sphere(f64),
    /// a u² + b uv + c v² (per unit length).
    Quadric([f64; 3]),
}

impl Shape {
    pub fn label(&self) -> &'static str {
        match self {
            Shape::Flat => "flat",
            Shape::Vee(_) => "V (corner)",
            Shape::ParabolicCylinder(_) => "parabolic cylinder",
            Shape::Paraboloid(_) => "dish (paraboloid)",
            Shape::TwoCurvatures(..) => "two curvatures",
            Shape::Cylinder(_) => "circular cylinder",
            Shape::Sphere(_) => "sphere",
            Shape::Quadric(_) => "curved (formula)",
        }
    }
}

impl SheetFit {
    /// The same fit in metres (`m` metres per unit).
    pub fn scaled(&self, m: f64) -> SheetFit {
        let shape = match &self.shape {
            Shape::ParabolicCylinder(f) => Shape::ParabolicCylinder(f * m),
            Shape::Paraboloid(f) => Shape::Paraboloid(f * m),
            Shape::TwoCurvatures(f, g) => Shape::TwoCurvatures(f * m, g * m),
            Shape::Cylinder(r) => Shape::Cylinder(r * m),
            Shape::Sphere(r) => Shape::Sphere(r * m),
            Shape::Quadric(q) => Shape::Quadric(q.map(|c| c / m)),
            s => s.clone(),
        };
        SheetFit {
            shape,
            vertex: self.vertex.map(|v| v * m),
            size: self.size.map(|v| v * m),
            offset: self.offset.map(|v| v * m),
            thickness: self.thickness * m,
            misfit: self.misfit * m,
            ..self.clone()
        }
    }

    /// Cells across and up for a mesh no coarser than `mesh` (V shapes get
    /// an even count across, so a mesh line runs along the fold).
    pub fn cells(&self, mesh: f64) -> [usize; 2] {
        let n = |s: f64| ((s / mesh.max(1e-12)).ceil() as usize).clamp(2, 80);
        let mut across = n(self.size[0]);
        if matches!(self.shape, Shape::Vee(_)) && across % 2 == 1 {
            across = (across + 1).min(80);
        }
        [across, n(self.size[1])]
    }

    /// A reflector made of wires this thick (metres; the fit must be in
    /// metres too).
    pub fn reflector(&self, name: &str, cells: [usize; 2]) -> ReflectorSpec {
        let mut radius = self.size[0].max(self.size[1]);
        let (profile, focal, angle) = match &self.shape {
            Shape::Cylinder(r) => {
                radius = *r;
                (Profile::Cylinder, [r / 2.0, r / 2.0], 90.0)
            }
            Shape::Sphere(r) => {
                radius = *r;
                (Profile::Sphere, [r / 2.0, r / 2.0], 90.0)
            }
            Shape::Flat => (Profile::Flat, [1.0, 1.0], 90.0),
            Shape::Vee(a) => (Profile::Vee, [1.0, 1.0], *a),
            Shape::ParabolicCylinder(f) => (Profile::ParabolicCylinder, [*f, *f], 90.0),
            Shape::Paraboloid(f) => (Profile::Paraboloid, [*f, *f], 90.0),
            Shape::TwoCurvatures(f, g) => (Profile::TwoCurvatures, [*f, *g], 90.0),
            Shape::Quadric(q) => {
                let term = |c: f64, t: &str| (c != 0.0).then(|| format!("{}*{t}", round_sig(c)));
                let parts: Vec<String> = [term(q[0], "x^2"), term(q[1], "x*y"), term(q[2], "y^2")]
                    .into_iter()
                    .flatten()
                    .collect();
                let f = if parts.is_empty() {
                    "0".into()
                } else {
                    parts.join("+").replace("+-", "-")
                };
                (Profile::Formula(f), [1.0, 1.0], 90.0)
            }
        };
        ReflectorSpec {
            name: name.to_string(),
            outline: self.outline,
            profile,
            vertex: self.vertex.map(round_um),
            facing: self.facing.map(snap_angle),
            size: self.size.map(round_um),
            offset: self.offset.map(round_um),
            focal: focal.map(round_um),
            radius: round_um(radius),
            angle: (angle * 1e4).round() / 1e4,
            cells,
            segments: 1,
            wire_radius: round_um(self.thickness / 2.0).max(1e-6),
        }
    }
}

/// An angle in degrees, made whole when within 0.02° of it (CAD rounding
/// and the fit's own wobble), else to a millionth of a degree.
pub fn snap_angle(a: f64) -> f64 {
    let whole = a.round();
    let a = if (a - whole).abs() < 0.02 {
        whole
    } else {
        (a * 1e6).round() / 1e6
    };
    a + 0.0 // never -0
}

/// Metres to the micrometre.
fn round_um(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// Six significant figures.
fn round_sig(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let p = 5 - v.abs().log10().floor() as i32;
    let s = 10f64.powi(p);
    (v * s).round() / s
}

/// Fits a thin sheet to points on both sides of its surface. `towards`
/// says which way a flat sheet should face (the rest of the antenna); a
/// curved one faces its concave side. None when the points aren't a thin
/// sheet (thicker than a tenth of its smaller width, or too few).
pub fn fit_sheet(points: &[P3], towards: Option<P3>) -> Option<SheetFit> {
    if points.len() < 8 {
        return None;
    }
    let n = points.len() as f64;
    let c = [0, 1, 2].map(|i| points.iter().map(|p| p[i]).sum::<f64>() / n);
    let normal = smallest_axis(points, c)?;
    let size = points
        .iter()
        .map(|p| length(sub(*p, c)))
        .fold(0.0, f64::max)
        .max(1e-30);
    let mut best: Option<(SheetFit, P3, usize)> = None;
    for axis in [normal, normal.map(|v| -v)] {
        let Some(fit) = fit_facing(points, c, axis, size) else {
            continue;
        };
        // Curved sheets face their hollow side; flat ones the antenna.
        let ok = match &fit.0.shape {
            Shape::Flat => towards.is_none_or(|t| dot(sub(t, c), axis) >= 0.0),
            _ => fit.1,
        };
        if ok {
            let kind = match fit.0.shape {
                Shape::Flat => 0,
                Shape::Vee(_) => 2,
                _ => 1,
            };
            best = Some((fit.0, axis, kind));
            break;
        }
    }
    let (mut fit, axis, kind) = best?;
    // A curved sheet that isn't symmetric (an offset dish) tilts the
    // least-spread direction off its axis: turn the axis until the shape
    // fits best (a tilted parabola isn't one).
    if kind != 0 {
        // Every few points are enough to steer by.
        let every = (points.len() / 1500).max(1);
        let some: Vec<P3> = points.iter().step_by(every).copied().collect();
        let score = |a: P3| shape_squares(&some, c, a, size, kind);
        let (mut a, mut now) = (axis, score(axis));
        let mut step = 2f64.to_radians();
        let mut tries = 0;
        while step > 1e-8 && tries < 300 {
            tries += 1;
            let mut better = false;
            let e1 = square_to(a);
            let e2 = cross(a, e1);
            for (e, sign) in [(e1, 1.0), (e1, -1.0), (e2, 1.0), (e2, -1.0)] {
                let tried = unit(add(scale(a, step.cos()), scale(e, sign * step.sin())));
                let sc = score(tried);
                // Real gains only: a straight shape doesn't change when
                // turned along its straight direction.
                if sc < now * (1.0 - 1e-9) {
                    (a, now, better) = (tried, sc, true);
                }
            }
            if !better {
                step /= 2.0;
            }
        }
        // A V or a parabolic cylinder is straight along its up line, so
        // tilting it that way changes nothing: stand the axis square to
        // that line.
        let (_, [_, up], local) = local_coords(&some, c, a, size);
        let slope = if kind == 2 {
            fit_vee(&local).map(|v| v.3)
        } else {
            least_squares(&local, |u, v| vec![u * u, u * v, v * v, u, v, 1.0])
                .filter(|q| q[2].abs() < 0.05 * q[0].abs() && q[1].abs() < 0.05 * q[0].abs())
                .map(|q| q[4])
        };
        if let Some(e) = slope {
            a = unit(sub(a, scale(up, e)));
        }
        if let Some(f) = fit_facing(points, c, a, size)
            && f.1
        {
            fit = f.0;
        }
    }
    let thin = fit.thickness < 0.1 * fit.size[0].min(fit.size[1]);
    // A sheet's points sit on its two faces, at about ±t/2; a round
    // cross-section (a bent tube) spreads them all the way between.
    let two_faced = fit.misfit < 0.3 * fit.thickness;
    (thin && two_faced && fit.misfit < 0.05 * fit.size[0].max(fit.size[1])).then_some(fit)
}

/// The direction the points spread least in (power iteration on the
/// covariance's inverse, by deflating the two largest).
fn smallest_axis(points: &[P3], c: P3) -> Option<P3> {
    let n = points.len() as f64;
    let mut cov = [[0.0; 3]; 3];
    for p in points {
        let d = sub(*p, c);
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j] / n;
            }
        }
    }
    let mul =
        |m: &[[f64; 3]; 3], v: P3| [0, 1, 2].map(|i| (0..3).map(|j| m[i][j] * v[j]).sum::<f64>());
    let mut found: Vec<P3> = Vec::new();
    for _ in 0..2 {
        let mut v = (0..3)
            .map(|j| [cov[0][j], cov[1][j], cov[2][j]])
            .map(|col| {
                let mut col = col;
                for f in &found {
                    col = sub(col, scale(*f, dot(col, *f)));
                }
                col
            })
            .max_by(|a, b| length(*a).total_cmp(&length(*b)))?;
        if length(v) == 0.0 {
            v = [1.0, 0.0, 0.0];
            for f in &found {
                v = sub(v, scale(*f, dot(v, *f)));
            }
        }
        for _ in 0..300 {
            let mut next = mul(&cov, v);
            for f in &found {
                next = sub(next, scale(*f, dot(next, *f)));
            }
            let l = length(next);
            if l == 0.0 {
                break;
            }
            v = scale(next, 1.0 / l);
        }
        found.push(unit(v));
    }
    let last = cross(found[0], found[1]);
    (length(last) > 0.5).then(|| unit(last))
}

/// Sum of squared distances from the best quadric (`kind` 1) or V (2)
/// with the sheet facing `axis`.
fn shape_squares(points: &[P3], c: P3, axis: P3, size: f64, kind: usize) -> f64 {
    let local = local_coords(points, c, axis, size).2;
    if kind == 2 {
        let Some((k, uc, f0, e)) = fit_vee(&local) else {
            return f64::MAX;
        };
        return normal_squares(&local, &|u: f64, v: f64| k * (u - uc).abs() + e * v + f0);
    }
    let Some(q) = least_squares(&local, |u, v| vec![u * u, u * v, v * v, u, v, 1.0]) else {
        return f64::MAX;
    };
    normal_squares(&local, &|u: f64, v: f64| {
        q[0] * u * u + q[1] * u * v + q[2] * v * v + q[3] * u + q[4] * v + q[5]
    })
}

/// Squared distances square to the surface depth = f(u, v), summed.
fn normal_squares(local: &[[f64; 3]], f: &dyn Fn(f64, f64) -> f64) -> f64 {
    let e = 1e-6;
    local
        .iter()
        .map(|p| {
            let gu = (f(p[0] + e, p[1]) - f(p[0] - e, p[1])) / (2.0 * e);
            let gv = (f(p[0], p[1] + e) - f(p[0], p[1] - e)) / (2.0 * e);
            (p[2] - f(p[0], p[1])).powi(2) / (1.0 + gu * gu + gv * gv)
        })
        .sum()
}

/// The facing angles, across and up directions for `axis`, and the points
/// in (across, up, depth) about `c`, divided by `size`.
fn local_coords(points: &[P3], c: P3, axis: P3, size: f64) -> ([f64; 2], [P3; 2], Vec<[f64; 3]>) {
    let el = axis[2].clamp(-1.0, 1.0).asin();
    let az = if axis[0].abs() < 1e-12 && axis[1].abs() < 1e-12 {
        0.0
    } else {
        axis[1].atan2(axis[0])
    };
    let across = [-az.sin(), az.cos(), 0.0];
    let up = [-el.sin() * az.cos(), -el.sin() * az.sin(), el.cos()];
    let local = points
        .iter()
        .map(|p| {
            let d = sub(*p, c);
            [
                dot(d, across) / size,
                dot(d, up) / size,
                dot(d, axis) / size,
            ]
        })
        .collect();
    ([az, el], [across, up], local)
}

/// Fits the shapes with the sheet facing `axis`; returns the best,
/// whether it curves towards that side (concave), and how well each shape
/// fits (sums of squared distances: flat, quadric, V).
fn fit_facing(points: &[P3], c: P3, axis: P3, size: f64) -> Option<(SheetFit, bool, [f64; 3])> {
    // Local coordinates, scaled to about 1 for well-conditioned sums.
    let ([az, el], [across, up], local) = local_coords(points, c, axis, size);

    let flat = least_squares(&local, |_, _| vec![1.0])?;
    let quad = least_squares(&local, |u, v| vec![u * u, u * v, v * v, u, v, 1.0])?;
    let vee = fit_vee(&local);
    // Distances from the surface, square to it.
    let away = |f: &dyn Fn(f64, f64) -> f64| -> Vec<f64> {
        let e = 1e-6;
        local
            .iter()
            .map(|p| {
                let gu = (f(p[0] + e, p[1]) - f(p[0] - e, p[1])) / (2.0 * e);
                let gv = (f(p[0], p[1] + e) - f(p[0], p[1] - e)) / (2.0 * e);
                (p[2] - f(p[0], p[1])).abs() / (1.0 + gu * gu + gv * gv).sqrt()
            })
            .collect()
    };
    let med = |f: &dyn Fn(f64, f64) -> f64| {
        let mut r = away(f);
        r.sort_by(f64::total_cmp);
        r[r.len() / 2]
    };
    // The thickness: most points are on the two faces, the rest on the
    // edges between them, nearer the middle.
    let thick = |f: &dyn Fn(f64, f64) -> f64| {
        let mut r = away(f);
        r.sort_by(f64::total_cmp);
        2.0 * r[r.len() * 3 / 4]
    };
    let squares = |f: &dyn Fn(f64, f64) -> f64| away(f).iter().map(|d| d * d).sum::<f64>();
    let spread = |f: &dyn Fn(f64, f64) -> f64, t: f64| {
        // The typical distance from ±t/2 (how badly the shape fits).
        let mut r: Vec<f64> = away(f).iter().map(|d| (d - t / 2.0).abs()).collect();
        r.sort_by(f64::total_cmp);
        r[r.len() / 2]
    };
    let qf = |u: f64, v: f64| {
        quad[0] * u * u + quad[1] * u * v + quad[2] * v * v + quad[3] * u + quad[4] * v + quad[5]
    };
    let ff = |_: f64, _: f64| flat[0];
    let m_flat = med(&ff);
    // A curve far tighter than the sheet is wide only threads a few
    // points (a box's corners), not a real surface.
    let mut uv: Vec<(i64, i64)> = local
        .iter()
        .map(|p| ((p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64))
        .collect();
    uv.sort_unstable();
    uv.dedup();
    let wild = quad[..3].iter().any(|q| q.abs() > 12.5) || uv.len() < 12;
    let m_quad = if wild { f64::MAX } else { med(&qf) };
    let vf = |u: f64, v: f64| vee.map_or(0.0, |(k, uc, f0, e)| k * (u - uc).abs() + e * v + f0);
    let m_vee = vee.map(|_| med(&vf));
    let fits = [
        squares(&ff),
        squares(&qf),
        vee.map_or(f64::MAX, |_| squares(&vf)),
    ];

    // The aperture, in local units.
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &local {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let quiet = 1e-9;
    let m_vee = if uv.len() < 12 { None } else { m_vee };
    let curved = m_flat > 1.5 * m_quad.min(m_vee.unwrap_or(f64::MAX)) + quiet;
    // How far the edges lean out: at a sloping rim the sheet's edge face
    // sticks out by t/2 sin(slope) beyond the middle surface.
    let lean = |f: &dyn Fn(f64, f64) -> f64, t: f64| {
        let e = 1e-6;
        let slope = |u: f64, v: f64, along_u: bool| {
            let g = if along_u {
                (f(u + e, v) - f(u - e, v)) / (2.0 * e)
            } else {
                (f(u, v + e) - f(u, v - e)) / (2.0 * e)
            };
            t / 2.0 * g.abs() / (1.0 + g * g).sqrt()
        };
        [
            slope(lo[0], mid[1], true),
            slope(hi[0], mid[1], true),
            slope(mid[0], lo[1], false),
            slope(mid[0], hi[1], false),
        ]
    };
    let (shape, vertex_uv, concave, t, misfit, edges);
    if !curved {
        shape = Shape::Flat;
        vertex_uv = [mid[0], mid[1], flat[0]];
        concave = true;
        t = thick(&ff);
        misfit = spread(&ff, t);
        edges = [0.0; 4];
    } else if let (Some((k, uc, _, _)), Some(mv)) = (vee, m_vee)
        && mv < 0.7 * m_quad
    {
        shape = Shape::Vee(2.0 * (1.0 / k.abs()).atan().to_degrees());
        vertex_uv = [uc, mid[1], vf(uc, mid[1])];
        concave = k > 0.0;
        t = thick(&vf);
        misfit = spread(&vf, t);
        let _ = mv;
        edges = lean(&vf, t);
    } else {
        let [a, b, cc, d, e, _] = [quad[0], quad[1], quad[2], quad[3], quad[4], quad[5]];
        let big = a.abs().max(cc.abs()).max(1e-300);
        let small = |x: f64| x.abs() < 0.05 * big;
        // The turning point (or the aperture's middle along a straight
        // direction).
        let det = 4.0 * a * cc - b * b;
        let (u0, v0) = if !small(b) || (!small(a) && !small(cc)) {
            if det.abs() > 1e-12 * big * big {
                ((-2.0 * cc * d + b * e) / det, (-2.0 * a * e + b * d) / det)
            } else {
                (mid[0], mid[1])
            }
        } else if !small(a) {
            (-d / (2.0 * a), mid[1])
        } else {
            (mid[0], -e / (2.0 * cc))
        };
        let mut vuv = [u0, v0, qf(u0, v0)];
        concave = a + cc > 0.0;
        // Back in real units: depth = coefficient × size⁻¹ × length².
        let s = size;
        let parabolic = if small(b) && !small(a) && !small(cc) && (a - cc).abs() < 0.05 * big {
            Shape::Paraboloid(s / (2.0 * (a + cc)))
        } else if small(b) && !small(a) && small(cc) {
            Shape::ParabolicCylinder(s / (4.0 * a))
        } else if small(b) && !small(a) && !small(cc) && a * cc > 0.0 {
            Shape::TwoCurvatures(s / (4.0 * a), s / (4.0 * cc))
        } else {
            Shape::Quadric([a / s, b / s, cc / s])
        };
        let tq = thick(&qf);
        let mq = spread(&qf, tq);
        // A sphere or circular cylinder, if it fits better than the
        // parabola (radius searched about twice the focal length).
        let round = match parabolic {
            Shape::Paraboloid(f) => Some((f / s, true)),
            Shape::ParabolicCylinder(f) => Some((f / s, false)),
            _ => None,
        };
        let circle = round.and_then(|(fl, both)| {
            let (u0, v0) = (vuv[0], vuv[1]);
            let reach = local
                .iter()
                .map(|p| {
                    let du = p[0] - u0;
                    let dv = if both { p[1] - v0 } else { 0.0 };
                    du.hypot(dv)
                })
                .fold(0.0, f64::max);
            let surface = move |r: f64, h: f64| {
                move |u: f64, v: f64| {
                    let du = u - u0;
                    let dv = if both { v - v0 } else { 0.0 };
                    r - (r * r - du * du - dv * dv).max(0.0).sqrt() + h
                }
            };
            // Points clear of the rim (where the sheet's edge face is).
            let inner: Vec<[f64; 3]> = local
                .iter()
                .filter(|p| {
                    let du = p[0] - u0;
                    let dv = if both { p[1] - v0 } else { 0.0 };
                    du.hypot(dv) <= 0.85 * reach
                })
                .copied()
                .collect();
            if inner.len() < 12 {
                return None;
            }
            // For a radius, the depth that centres the points on it; then
            // the thickness and how far the points stray from ±t/2.
            let cost = |r: f64| {
                let mut h = vuv[2];
                for _ in 0..4 {
                    let f = surface(r, h);
                    let mean = inner.iter().map(|p| p[2] - f(p[0], p[1])).sum::<f64>()
                        / inner.len() as f64;
                    h += mean;
                }
                let f = surface(r, h);
                let mut d: Vec<f64> = inner
                    .iter()
                    .map(|p| {
                        let du = p[0] - u0;
                        let dv = if both { p[1] - v0 } else { 0.0 };
                        let cr = du.hypot(dv) / r;
                        (p[2] - f(p[0], p[1])).abs() * (1.0 - cr * cr).max(0.0).sqrt()
                    })
                    .collect();
                d.sort_by(f64::total_cmp);
                let t = 2.0 * d[d.len() * 3 / 4];
                let mut off: Vec<f64> = d.iter().map(|x| (x - t / 2.0).abs()).collect();
                off.sort_by(f64::total_cmp);
                (off[off.len() / 2], t, h)
            };
            let (mut lo, mut hi) = ((2.0 * fl * 0.5).max(reach * 1.0001), 2.0 * fl * 2.0);
            if hi <= lo {
                return None;
            }
            for _ in 0..80 {
                let (a1, a2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
                if cost(a1).0 < cost(a2).0 {
                    hi = a2;
                } else {
                    lo = a1;
                }
            }
            let r = (lo + hi) / 2.0;
            let (sp, t, h) = cost(r);
            (sp < 0.5 * mq).then_some((r, both, sp, t, h))
        });
        match circle {
            Some((r, both, sp, tc, h)) => {
                vuv[2] = h;
                shape = if both {
                    Shape::Sphere(r * s)
                } else {
                    Shape::Cylinder(r * s)
                };
                t = tc;
                misfit = sp;
                let (u0, v0, h0) = (vuv[0], vuv[1], vuv[2]);
                let f = move |u: f64, v: f64| {
                    let du = u - u0;
                    let dv = if both { v - v0 } else { 0.0 };
                    r - (r * r - du * du - dv * dv).max(0.0).sqrt() + h0
                };
                edges = lean(&f, t);
            }
            None => {
                shape = parabolic;
                t = tq;
                misfit = mq;
                edges = lean(&qf, t);
            }
        }
        vertex_uv = vuv;
    }
    let lo = [lo[0] + edges[0], lo[1] + edges[2]];
    let hi = [hi[0] - edges[1], hi[1] - edges[3]];
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let offset = [mid[0] - vertex_uv[0], mid[1] - vertex_uv[1]];
    let vertex = add(
        c,
        scale(
            add(
                add(scale(across, vertex_uv[0]), scale(up, vertex_uv[1])),
                scale(axis, vertex_uv[2]),
            ),
            size,
        ),
    );
    let outline = outline_of(&local, lo, hi);
    Some((
        SheetFit {
            outline,
            shape,
            vertex,
            facing: [az.to_degrees(), el.to_degrees()],
            size: [(hi[0] - lo[0]) * size, (hi[1] - lo[1]) * size],
            offset: offset.map(|o| o * size),
            thickness: t * size,
            misfit: misfit * size,
        },
        concave,
        fits,
    ))
}

/// A rectangle when the points fill their bounding box's corners, else an
/// ellipse.
fn outline_of(local: &[[f64; 3]], lo: [f64; 2], hi: [f64; 2]) -> Outline {
    let mut pts: Vec<(f64, f64)> = local.iter().map(|p| (p[0], p[1])).collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    pts.dedup();
    if pts.len() < 3 {
        return Outline::Rectangle;
    }
    // Convex hull (monotone chain) and its area.
    let turn = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut hull: Vec<(f64, f64)> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &(f64, f64)>> = if pass == 0 {
            Box::new(pts.iter())
        } else {
            Box::new(pts.iter().rev())
        };
        for &p in iter {
            while hull.len() >= start + 2
                && turn(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    let area: f64 = (0..hull.len())
        .map(|i| {
            let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f64>()
        .abs()
        / 2.0;
    let boxed = (hi[0] - lo[0]) * (hi[1] - lo[1]);
    if boxed <= 0.0 || area / boxed > 0.9 {
        Outline::Rectangle
    } else {
        Outline::Ellipse
    }
}

/// A V folded along an up line: depth = k |u - uc| + e v + f0 (least
/// squares for k, e and f0, the fold found by a search).
fn fit_vee(local: &[[f64; 3]]) -> Option<(f64, f64, f64, f64)> {
    let (lo, hi) = local
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), p| (l.min(p[0]), h.max(p[0])));
    if hi <= lo {
        return None;
    }
    let fit_at = |uc: f64| {
        let sol = least_squares(local, |u, v| vec![(u - uc).abs(), v, 1.0])?;
        let err: f64 = local
            .iter()
            .map(|p| (p[2] - sol[0] * (p[0] - uc).abs() - sol[1] * p[1] - sol[2]).powi(2))
            .sum();
        Some((err, sol[0], sol[2], sol[1]))
    };
    let mut best = (f64::MAX, 0.0, 0.0, 0.0, 0.0);
    for k in 1..100 {
        let uc = lo + (hi - lo) * k as f64 / 100.0;
        if let Some((e, a, f0, slope)) = fit_at(uc)
            && e < best.0
        {
            best = (e, uc, a, f0, slope);
        }
    }
    // Narrow it down.
    let mut step = (hi - lo) / 100.0;
    for _ in 0..40 {
        step /= 2.0;
        for uc in [best.1 - step, best.1 + step] {
            if let Some((e, a, f0, slope)) = fit_at(uc)
                && e < best.0
            {
                best = (e, uc, a, f0, slope);
            }
        }
    }
    (best.0 < f64::MAX && best.2 != 0.0).then_some((best.2, best.1, best.3, best.4))
}

/// Least squares for depth = Σ cᵢ·termᵢ(u, v).
fn least_squares(local: &[[f64; 3]], terms: impl Fn(f64, f64) -> Vec<f64>) -> Option<Vec<f64>> {
    let n = terms(0.0, 0.0).len();
    let mut m = vec![vec![0.0; n + 1]; n];
    for p in local {
        let t = terms(p[0], p[1]);
        for i in 0..n {
            for j in 0..n {
                m[i][j] += t[i] * t[j];
            }
            m[i][n] += t[i] * p[2];
        }
    }
    // Gaussian elimination with partial pivoting.
    for col in 0..n {
        let pivot = (col..n).max_by(|a, b| m[*a][col].abs().total_cmp(&m[*b][col].abs()))?;
        if m[pivot][col].abs() < 1e-300 {
            return None;
        }
        m.swap(col, pivot);
        for row in 0..n {
            if row != col {
                let f = m[row][col] / m[col][col];
                let pivot_row = m[col].clone();
                for (x, p) in m[row][col..].iter_mut().zip(&pivot_row[col..]) {
                    *x -= f * p;
                }
            }
        }
    }
    Some((0..n).map(|i| m[i][n] / m[i][i]).collect())
}

fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: P3, k: f64) -> P3 {
    a.map(|v| v * k)
}
fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: P3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: P3) -> P3 {
    let l = length(a).max(1e-300);
    a.map(|v| v / l)
}
/// A unit direction square to d.
fn square_to(d: P3) -> P3 {
    let helper = if d[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    unit(cross(helper, d))
}

fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step::{Mesh, sheet_solid};

    /// A wire mesh over the grid points f(i, j) that exist (metres), and
    /// the corners of the solid sheet it makes (millimetres).
    fn sheet(nu: usize, nv: usize, t: f64, f: impl Fn(f64, f64) -> Option<P3>) -> Vec<P3> {
        let at = |i: usize, j: usize| {
            f(
                i as f64 / (nu - 1) as f64 - 0.5,
                j as f64 / (nv - 1) as f64 - 0.5,
            )
        };
        let mut wires = Vec::new();
        for i in 0..nu {
            for j in 0..nv {
                let Some(p) = at(i, j) else { continue };
                if let Some(q) = (i + 1 < nu).then(|| at(i + 1, j)).flatten() {
                    wires.push((p, q));
                }
                if let Some(q) = (j + 1 < nv).then(|| at(i, j + 1)).flatten() {
                    wires.push((p, q));
                }
            }
        }
        let mesh = Mesh {
            name: "test".into(),
            wires,
            thickness: t,
            radius: t / 2.0,
        };
        sheet_solid(&mesh).unwrap().points
    }

    /// The point at (across, up, depth) from `o` facing (az, el) degrees.
    fn placed(o: P3, az: f64, el: f64, u: f64, v: f64, w: f64) -> P3 {
        let (az, el) = (az.to_radians(), el.to_radians());
        let across = [-az.sin(), az.cos(), 0.0];
        let up = [-el.sin() * az.cos(), -el.sin() * az.sin(), el.cos()];
        let axis = [el.cos() * az.cos(), el.cos() * az.sin(), el.sin()];
        [0, 1, 2].map(|i| o[i] + u * across[i] + v * up[i] + w * axis[i])
    }

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn a_flat_plate_facing_anywhere() {
        // 1.2 m x 0.8 m, 3 mm thick, facing azimuth 30, elevation 10.
        let pts = sheet(13, 9, 0.003, |a, b| {
            Some(placed([1.0, 2.0, 3.0], 30.0, 10.0, 1.2 * a, 0.8 * b, 0.0))
        });
        let ahead = placed([1000.0, 2000.0, 3000.0], 30.0, 10.0, 0.0, 0.0, 500.0);
        let fit = fit_sheet(&pts, Some(ahead)).unwrap();
        assert_eq!(fit.shape, Shape::Flat);
        assert_eq!(fit.outline, Outline::Rectangle);
        assert!(
            near(fit.facing[0], 30.0, 1e-6) && near(fit.facing[1], 10.0, 1e-6),
            "{fit:?}"
        );
        assert!(
            near(fit.size[0], 1200.0, 1e-3) && near(fit.size[1], 800.0, 1e-3),
            "{fit:?}"
        );
        assert!(near(fit.thickness, 3.0, 1e-6), "{fit:?}");
        for i in 0..3 {
            assert!(
                near(fit.vertex[i], [1000.0, 2000.0, 3000.0][i], 1e-3),
                "{fit:?}"
            );
        }
        // Facing the other way when the antenna is behind it.
        let behind = placed([1000.0, 2000.0, 3000.0], 30.0, 10.0, 0.0, 0.0, -500.0);
        let back = fit_sheet(&pts, Some(behind)).unwrap();
        assert!(
            near(back.facing[0], -150.0, 1e-6) && near(back.facing[1], -10.0, 1e-6),
            "{back:?}"
        );
    }

    #[test]
    fn a_corner_reflector() {
        // A 90 degree V, sides 0.5 m, 0.6 m tall, fold up z, facing +x.
        let pts = sheet(11, 4, 0.002, |a, b| {
            let u = a; // -0.5 .. 0.5 m across
            Some([u.abs(), u, 0.6 * b])
        });
        let fit = fit_sheet(&pts, None).unwrap();
        let Shape::Vee(angle) = fit.shape else {
            panic!("{fit:?}")
        };
        assert!(near(angle, 90.0, 0.01), "{fit:?}");
        assert!(
            near(fit.facing[0], 0.0, 0.01) && near(fit.facing[1], 0.0, 0.01),
            "{fit:?}"
        );
        assert!(
            near(fit.vertex[0], 0.0, 0.1) && near(fit.vertex[1], 0.0, 0.1),
            "{fit:?}"
        );
        assert!(near(fit.thickness, 2.0, 0.05), "{fit:?}");
        assert!(near(fit.size[1], 600.0, 0.5), "{fit:?}");
    }

    #[test]
    fn a_round_dish() {
        // A paraboloid 1 m across, f = 0.4 m, facing straight up.
        let pts = sheet(21, 21, 0.003, |a, b| {
            (a * a + b * b <= 0.25 + 1e-9).then(|| [a, b, (a * a + b * b) / 1.6])
        });
        let fit = fit_sheet(&pts, None).unwrap();
        let Shape::Paraboloid(f) = fit.shape else {
            panic!("{fit:?}")
        };
        assert!(near(f, 400.0, 0.5), "{fit:?}");
        assert_eq!(fit.outline, Outline::Ellipse);
        assert!(near(fit.facing[1], 90.0, 0.01), "{fit:?}");
        assert!(fit.vertex.iter().all(|v| v.abs() < 0.5), "{fit:?}");
        assert!(near(fit.thickness, 3.0, 0.05), "{fit:?}");
    }

    #[test]
    fn an_offset_curved_reflector() {
        // A parabolic cylinder, f = 0.3 m, its aperture 50 mm off the
        // vertex, facing azimuth 120, made in metres.
        let pts = sheet(17, 9, 0.002, |a, b| {
            let u = 0.05 + 0.8 * a;
            Some(placed(
                [0.5, -0.2, 1.0],
                120.0,
                0.0,
                u,
                0.6 * b,
                u * u / 1.2,
            ))
        });
        let fit = fit_sheet(&pts, None).unwrap();
        let Shape::ParabolicCylinder(f) = fit.shape else {
            panic!("{fit:?}")
        };
        assert!(near(f, 300.0, 0.3), "{fit:?}");
        assert!(
            near(fit.facing[0], 120.0, 0.05) && near(fit.facing[1], 0.0, 0.05),
            "{fit:?}"
        );
        assert!(near(fit.offset[0], 50.0, 0.5), "{fit:?}");
        assert!(
            near(fit.size[0], 800.0, 0.5) && near(fit.size[1], 600.0, 0.5),
            "{fit:?}"
        );
        for i in 0..3 {
            assert!(
                near(fit.vertex[i], [500.0, -200.0, 1000.0][i], 0.5),
                "{fit:?}"
            );
        }
        // As a reflector, in metres.
        let spec = fit.scaled(0.001).reflector("dish1", [10, 8]);
        assert_eq!(spec.profile, Profile::ParabolicCylinder);
        assert!(near(spec.focal[0], 0.3, 0.0005) && near(spec.wire_radius, 0.001, 5e-5));
    }

    #[test]
    fn solid_blocks_and_rods_arent_sheets() {
        // A 10 x 20 x 30 block's corners.
        let mut block = Vec::new();
        for x in [0.0, 10.0] {
            for y in [0.0, 20.0] {
                for z in [0.0, 30.0] {
                    block.push([x, y, z]);
                }
            }
        }
        block.extend(block.clone());
        assert!(fit_sheet(&block, None).is_none());
    }

    /// Prints what a STEP file's sheets fit as (STRAWS_STEP_FILE).
    #[test]
    #[ignore]
    fn show_sheets() {
        let path = std::env::var("STRAWS_STEP_FILE").unwrap();
        let read = crate::step::read_step(&std::fs::read_to_string(path).unwrap()).unwrap();
        println!(
            "{} tubes, {:?} flat {} other",
            read.parts.tubes.len(),
            read.parts.flat,
            read.parts.other
        );
        for s in &read.sheets {
            println!("{s:#?}");
        }
    }
}
