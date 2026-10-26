//! Wire-frame drawings from CAD programs (DXF files), turned into wires.
//!
//! Lines, polylines (with their arc bulges), arcs, circles, ellipses and
//! splines become chains of straight pieces; everything else (text,
//! dimensions, hatches, faces, block references) is counted and left out.

use std::collections::HashMap;
use std::f64::consts::PI;

type P3 = [f64; 3];

/// A chain of points from one drawing entity, in drawing units. A closed
/// shape repeats its first point at the end.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub layer: String,
    pub points: Vec<P3>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    /// Metres per drawing unit, when the file says ($INSUNITS).
    pub metres_per_unit: Option<f64>,
    pub paths: Vec<Path>,
    /// Entities left out, by kind, and how many.
    pub skipped: Vec<(String, usize)>,
}

impl Drawing {
    /// The layers with their number of shapes, by name.
    pub fn layers(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for p in &self.paths {
            match counts.iter_mut().find(|(n, _)| *n == p.layer) {
                Some(c) => c.1 += 1,
                None => counts.push((p.layer.clone(), 1)),
            }
        }
        counts.sort_by(|a, b| a.0.cmp(&b.0));
        counts
    }
}

/// Metres per unit for DXF's $INSUNITS codes (`None` for unitless or odd ones).
pub fn insunits_metres(code: i64) -> Option<f64> {
    Some(match code {
        1 => 0.0254,
        2 => 0.3048,
        4 => 0.001,
        5 => 0.01,
        6 => 1.0,
        7 => 1000.0,
        9 => 2.54e-5,
        10 => 0.9144,
        14 => 0.1,
        _ => return None,
    })
}

/// Reads an ASCII DXF file. Curves are cut into straight pieces turning at
/// most `step_deg` each.
pub fn read_dxf(text: &str, step_deg: f64) -> Result<Drawing, String> {
    if text.starts_with("AutoCAD Binary DXF") {
        return Err("this is a binary DXF: save it as an ASCII DXF instead".into());
    }
    let mut lines = text.lines();
    let mut pairs = Vec::new();
    while let Some(code) = lines.next() {
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        let code: i32 = code
            .parse()
            .map_err(|_| format!("\"{code}\" isn't a DXF group code: is it a DXF file?"))?;
        let value = lines
            .next()
            .ok_or("the file ends half way through an entry")?;
        pairs.push((code, value.trim()));
    }
    let step = step_deg.clamp(0.5, 90.0).to_radians();
    let mut drawing = Drawing::default();
    let mut skipped: HashMap<String, usize> = HashMap::new();
    let mut section = "";
    let mut k = 0;
    while k < pairs.len() {
        let (code, value) = pairs[k];
        if code != 0 {
            k += 1;
            continue;
        }
        // An entity (or section marker): its groups run to the next code 0.
        let end = (k + 1..pairs.len())
            .find(|&j| pairs[j].0 == 0)
            .unwrap_or(pairs.len());
        let groups = &pairs[k + 1..end];
        match value {
            "SECTION" => {
                section = groups.iter().find(|g| g.0 == 2).map_or("", |g| match g.1 {
                    "HEADER" => "HEADER",
                    "ENTITIES" => "ENTITIES",
                    _ => "OTHER",
                });
                // The header's variables are all in this one group.
                if section == "HEADER"
                    && let Some(at) = groups
                        .iter()
                        .position(|g| g.0 == 9 && g.1.eq_ignore_ascii_case("$INSUNITS"))
                    && let Some((70, v)) = groups.get(at + 1)
                {
                    drawing.metres_per_unit = v.parse().ok().and_then(insunits_metres);
                }
            }
            "ENDSEC" => section = "",
            "EOF" => break,
            kind if section == "ENTITIES" => {
                let e = Entity(groups);
                let layer = e.text(8).unwrap_or("0").to_string();
                let mut next = end;
                let points = match kind {
                    "LINE" => Some(vec![e.point(10), e.point(11)]),
                    "LWPOLYLINE" => Some(lwpolyline(&e, step)),
                    "POLYLINE" => {
                        // VERTEX entities follow until SEQEND.
                        let mut vertices = Vec::new();
                        let mut j = end;
                        while j < pairs.len() {
                            let to = (j + 1..pairs.len())
                                .find(|&i| pairs[i].0 == 0)
                                .unwrap_or(pairs.len());
                            match pairs[j].1 {
                                "VERTEX" => vertices.push(Entity(&pairs[j + 1..to])),
                                _ => {
                                    if pairs[j].1 == "SEQEND" {
                                        j = to;
                                    }
                                    break;
                                }
                            }
                            j = to;
                        }
                        next = j;
                        polyline(&e, &vertices, step)
                    }
                    "ARC" => Some(arc(&e, step)),
                    "CIRCLE" => Some(circle(&e, step)),
                    "ELLIPSE" => Some(ellipse(&e, step)),
                    "SPLINE" => spline(&e),
                    other => {
                        *skipped.entry(other.to_string()).or_default() += 1;
                        None
                    }
                };
                if let Some(points) = points {
                    let points: Vec<P3> = points
                        .into_iter()
                        .filter(|p| p.iter().all(|v| v.is_finite()))
                        .collect();
                    if points.len() >= 2 {
                        drawing.paths.push(Path { layer, points });
                    }
                }
                k = next;
                continue;
            }
            _ => {}
        }
        k = end;
    }
    let mut skipped: Vec<_> = skipped.into_iter().collect();
    skipped.sort();
    drawing.skipped = skipped;
    Ok(drawing)
}

/// One entity's groups.
struct Entity<'a>(&'a [(i32, &'a str)]);

impl Entity<'_> {
    fn text(&self, code: i32) -> Option<&str> {
        self.0.iter().find(|g| g.0 == code).map(|g| g.1)
    }

    fn num(&self, code: i32, default: f64) -> f64 {
        self.text(code)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    fn int(&self, code: i32) -> i64 {
        self.text(code).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    /// A point from codes x, x + 10, x + 20.
    fn point(&self, x: i32) -> P3 {
        [
            self.num(x, 0.0),
            self.num(x + 10, 0.0),
            self.num(x + 20, 0.0),
        ]
    }

    /// Every value of `code`, in order.
    fn all(&self, code: i32) -> Vec<f64> {
        self.0
            .iter()
            .filter(|g| g.0 == code)
            .filter_map(|g| g.1.parse().ok())
            .collect()
    }

    /// The entity's plane (extrusion direction) as world axes.
    fn ocs(&self) -> Ocs {
        Ocs::new([self.num(210, 0.0), self.num(220, 0.0), self.num(230, 1.0)])
    }
}

/// DXF's object coordinate system: 2D entities live in a plane given by
/// its normal, with axes from the "arbitrary axis algorithm".
struct Ocs {
    x: P3,
    y: P3,
    z: P3,
}

impl Ocs {
    fn new(n: P3) -> Ocs {
        let len = norm(n);
        let z = if len > 0.0 {
            scale(n, 1.0 / len)
        } else {
            [0.0, 0.0, 1.0]
        };
        let x = if z[0].abs() < 1.0 / 64.0 && z[1].abs() < 1.0 / 64.0 {
            cross([0.0, 1.0, 0.0], z)
        } else {
            cross([0.0, 0.0, 1.0], z)
        };
        let x = scale(x, 1.0 / norm(x));
        let y = cross(z, x);
        Ocs { x, y, z }
    }

    fn world(&self, p: P3) -> P3 {
        [0, 1, 2].map(|i| p[0] * self.x[i] + p[1] * self.y[i] + p[2] * self.z[i])
    }
}

fn norm(a: P3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn scale(a: P3, s: f64) -> P3 {
    a.map(|v| v * s)
}

fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Points round an arc in its plane, from `start` turning `sweep` radians
/// (anticlockwise when positive), `start` included.
fn arc_points(centre: [f64; 2], r: f64, start: f64, sweep: f64, step: f64) -> Vec<[f64; 2]> {
    let n = (sweep.abs() / step).ceil().max(1.0) as usize;
    (0..=n)
        .map(|k| {
            let a = start + sweep * k as f64 / n as f64;
            [centre[0] + r * a.cos(), centre[1] + r * a.sin()]
        })
        .collect()
}

/// The arc a polyline's bulge makes from `a` to `b` (points after `a`).
fn bulge_points(a: [f64; 2], b: [f64; 2], bulge: f64, step: f64) -> Vec<[f64; 2]> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let d = (dx * dx + dy * dy).sqrt();
    if bulge.abs() < 1e-12 || d == 0.0 {
        return vec![b];
    }
    let sweep = 4.0 * bulge.atan();
    // The centre is off the chord's middle, to the left for an anticlockwise arc.
    let h = (d / 2.0) / (sweep / 2.0).tan();
    let centre = [
        (a[0] + b[0]) / 2.0 - dy / d * h,
        (a[1] + b[1]) / 2.0 + dx / d * h,
    ];
    let r = ((a[0] - centre[0]).powi(2) + (a[1] - centre[1]).powi(2)).sqrt();
    let start = (a[1] - centre[1]).atan2(a[0] - centre[0]);
    let mut pts = arc_points(centre, r, start, sweep, step);
    pts.remove(0);
    if let Some(last) = pts.last_mut() {
        *last = b; // exactly on the next vertex
    }
    pts
}

/// Vertices (x, y, bulge) of a 2D polyline at height `z` in `ocs`, as points.
fn flat_polyline(
    vertices: &[(f64, f64, f64)],
    closed: bool,
    z: f64,
    ocs: &Ocs,
    step: f64,
) -> Vec<P3> {
    let mut out: Vec<[f64; 2]> = Vec::new();
    let Some(first) = vertices.first() else {
        return Vec::new();
    };
    out.push([first.0, first.1]);
    let count = if closed {
        vertices.len()
    } else {
        vertices.len() - 1
    };
    for i in 0..count {
        let (x, y, bulge) = vertices[i];
        let next = vertices[(i + 1) % vertices.len()];
        out.extend(bulge_points([x, y], [next.0, next.1], bulge, step));
    }
    out.into_iter()
        .map(|p| ocs.world([p[0], p[1], z]))
        .collect()
}

fn lwpolyline(e: &Entity, step: f64) -> Vec<P3> {
    // Each vertex starts with code 10; a bulge (42) belongs to the vertex before it.
    let mut vertices: Vec<(f64, f64, f64)> = Vec::new();
    for &(code, value) in e.0 {
        let v: f64 = value.parse().unwrap_or(0.0);
        match code {
            10 => vertices.push((v, 0.0, 0.0)),
            20 => {
                if let Some(last) = vertices.last_mut() {
                    last.1 = v;
                }
            }
            42 => {
                if let Some(last) = vertices.last_mut() {
                    last.2 = v;
                }
            }
            _ => {}
        }
    }
    flat_polyline(
        &vertices,
        e.int(70) & 1 == 1,
        e.num(38, 0.0),
        &e.ocs(),
        step,
    )
}

fn polyline(e: &Entity, vertices: &[Entity], step: f64) -> Option<Vec<P3>> {
    let flags = e.int(70);
    if flags & (16 | 64) != 0 {
        return None; // polygon and polyface meshes are surfaces
    }
    let closed = flags & 1 == 1;
    if flags & 8 != 0 {
        // A 3D polyline: world coordinates, no bulges.
        let mut pts: Vec<P3> = vertices.iter().map(|v| v.point(10)).collect();
        if closed && let Some(&first) = pts.first() {
            pts.push(first);
        }
        return Some(pts);
    }
    let v: Vec<(f64, f64, f64)> = vertices
        .iter()
        .map(|v| (v.num(10, 0.0), v.num(20, 0.0), v.num(42, 0.0)))
        .collect();
    Some(flat_polyline(&v, closed, e.num(30, 0.0), &e.ocs(), step))
}

fn arc(e: &Entity, step: f64) -> Vec<P3> {
    let c = e.point(10);
    let start = e.num(50, 0.0).to_radians();
    let mut sweep = e.num(51, 360.0).to_radians() - start;
    while sweep <= 0.0 {
        sweep += 2.0 * PI;
    }
    let ocs = e.ocs();
    arc_points([c[0], c[1]], e.num(40, 0.0), start, sweep, step)
        .into_iter()
        .map(|p| ocs.world([p[0], p[1], c[2]]))
        .collect()
}

fn circle(e: &Entity, step: f64) -> Vec<P3> {
    let c = e.point(10);
    let ocs = e.ocs();
    let mut pts: Vec<P3> = arc_points([c[0], c[1]], e.num(40, 0.0), 0.0, 2.0 * PI, step)
        .into_iter()
        .map(|p| ocs.world([p[0], p[1], c[2]]))
        .collect();
    if let Some(&first) = pts.first()
        && let Some(last) = pts.last_mut()
    {
        *last = first; // closed exactly
    }
    pts
}

fn ellipse(e: &Entity, step: f64) -> Vec<P3> {
    let c = e.point(10);
    let major = e.point(11);
    let ocs = e.ocs();
    let minor = scale(cross(ocs.z, major), e.num(40, 1.0));
    let start = e.num(41, 0.0);
    let mut end = e.num(42, 2.0 * PI);
    while end <= start {
        end += 2.0 * PI;
    }
    let n = ((end - start) / step).ceil().max(1.0) as usize;
    (0..=n)
        .map(|k| {
            let t = start + (end - start) * k as f64 / n as f64;
            [0, 1, 2].map(|i| c[i] + t.cos() * major[i] + t.sin() * minor[i])
        })
        .collect()
}

fn spline(e: &Entity) -> Option<Vec<P3>> {
    let degree = e.int(71).max(1) as usize;
    let knots = e.all(40);
    let weights = e.all(41);
    let control: Vec<P3> = {
        let (x, y, z) = (e.all(10), e.all(20), e.all(30));
        (0..x.len())
            .map(|i| [x[i], *y.get(i).unwrap_or(&0.0), *z.get(i).unwrap_or(&0.0)])
            .collect()
    };
    let n = control.len();
    if n <= degree || knots.len() != n + degree + 1 {
        // No usable control polygon: the fit points, joined straight.
        let (x, y, z) = (e.all(11), e.all(21), e.all(31));
        let fit: Vec<P3> = (0..x.len())
            .map(|i| [x[i], *y.get(i).unwrap_or(&0.0), *z.get(i).unwrap_or(&0.0)])
            .collect();
        return (fit.len() >= 2).then_some(fit);
    }
    let w = |i: usize| if weights.len() == n { weights[i] } else { 1.0 };
    let (lo, hi) = (knots[degree], knots[n]);
    let samples = (8 * n).max(16);
    Some(
        (0..=samples)
            .map(|s| {
                let t = lo + (hi - lo) * s as f64 / samples as f64;
                de_boor(t, degree, &knots, &control, &w)
            })
            .collect(),
    )
}

/// A point on a (rational) B-spline at parameter `t`.
fn de_boor(t: f64, p: usize, knots: &[f64], control: &[P3], w: &dyn Fn(usize) -> f64) -> P3 {
    let n = control.len();
    // The knot span holding t (the last span for t at the very end).
    let mut k = p;
    while k + 1 < n && t >= knots[k + 1] {
        k += 1;
    }
    let mut d: Vec<[f64; 4]> = (0..=p)
        .map(|j| {
            let i = j + k - p;
            let wi = w(i);
            [
                control[i][0] * wi,
                control[i][1] * wi,
                control[i][2] * wi,
                wi,
            ]
        })
        .collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = j + k - p;
            let span = knots[i + p + 1 - r] - knots[i];
            let a = if span == 0.0 {
                0.0
            } else {
                (t - knots[i]) / span
            };
            let before = d[j - 1];
            for (v, b) in d[j].iter_mut().zip(before) {
                *v = (1.0 - a) * b + a * *v;
            }
        }
    }
    let h = d[p];
    let wt = if h[3] == 0.0 { 1.0 } else { h[3] };
    [h[0] / wt, h[1] / wt, h[2] / wt]
}

/// How a drawing becomes wires.
#[derive(Debug, Clone, PartialEq)]
pub struct WireOptions {
    /// Metres per drawing unit.
    pub metres_per_unit: f64,
    /// Ends closer than this (metres) become one point, so the wires join.
    pub join_m: f64,
    /// Only these layers (all when empty).
    pub layers: Vec<String>,
}

/// A straight wire from the drawing, in metres.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnWire {
    pub a: P3,
    pub b: P3,
    pub layer: String,
    /// The wire's own radius (metres) when the source says, as a solid
    /// model's tube does.
    pub radius: Option<f64>,
}

/// The drawing's pieces as straight wires in metres: points within
/// `join_m` of each other merged (so touching shapes join), pieces that
/// end up with no length or that repeat another dropped.
pub fn wires(drawing: &Drawing, options: &WireOptions) -> Vec<DrawnWire> {
    let s = options.metres_per_unit;
    let paths: Vec<&Path> = drawing
        .paths
        .iter()
        .filter(|p| options.layers.is_empty() || options.layers.contains(&p.layer))
        .collect();
    // Every point, then merged by a grid of cells the size of the tolerance.
    let mut points: Vec<P3> = Vec::new();
    let mut index: Vec<Vec<usize>> = Vec::new();
    for p in &paths {
        let mut ids = Vec::new();
        for q in &p.points {
            ids.push(points.len());
            points.push(scale(*q, s));
        }
        index.push(ids);
    }
    let merged = merge_points(&points, options.join_m.max(0.0));
    let points = merged_positions(&points, &merged);
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (p, ids) in paths.iter().zip(&index) {
        for pair in ids.windows(2) {
            let (i, j) = (merged[pair[0]], merged[pair[1]]);
            if i == j {
                continue;
            }
            if !seen.insert((i.min(j), i.max(j))) {
                continue;
            }
            let (a, b) = (points[i], points[j]);
            if norm([b[0] - a[0], b[1] - a[1], b[2] - a[2]]) <= options.join_m.max(1e-9) {
                continue;
            }
            out.push(DrawnWire {
                a,
                b,
                layer: p.layer.clone(),
                radius: None,
            });
        }
    }
    out
}

/// Where each merged group of points ends up: the middle of its points
/// (so a gap closed by joining closes from both sides), stored at the
/// group's index from [`merge_points`].
pub(crate) fn merged_positions(points: &[P3], merged: &[usize]) -> Vec<P3> {
    let mut sum = vec![[0.0; 3]; points.len()];
    let mut count = vec![0usize; points.len()];
    for (p, &g) in points.iter().zip(merged) {
        for i in 0..3 {
            sum[g][i] += p[i];
        }
        count[g] += 1;
    }
    sum.iter()
        .zip(&count)
        .zip(points)
        .map(|((s, &n), p)| if n == 0 { *p } else { s.map(|v| v / n as f64) })
        .collect()
}

/// For each point, the index of the point it's merged into (the first of
/// its group). Points chain together within `tol`.
pub(crate) fn merge_points(points: &[P3], tol: f64) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..points.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while parent[r] != r {
            r = parent[r];
        }
        let mut i = i;
        while parent[i] != r {
            let next = parent[i];
            parent[i] = r;
            i = next;
        }
        r
    }
    let cell = tol.max(1e-9);
    let key = |p: &P3| p.map(|v| (v / cell).floor() as i64);
    let mut grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for (i, p) in points.iter().enumerate() {
        let k = key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(others) = grid.get(&[k[0] + dx, k[1] + dy, k[2] + dz]) {
                        for &j in others {
                            let q = points[j];
                            if norm([p[0] - q[0], p[1] - q[1], p[2] - q[2]]) <= tol {
                                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                                if a != b {
                                    parent[a.max(b)] = a.min(b);
                                }
                            }
                        }
                    }
                }
            }
        }
        grid.entry(k).or_default().push(i);
    }
    (0..points.len()).map(|i| find(&mut parent, i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DXF file from (code, value) lines.
    fn dxf(entities: &str, units: Option<i64>) -> String {
        let header = units.map_or(String::new(), |u| {
            format!("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{u}\n0\nENDSEC\n")
        });
        format!("{header}0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n")
    }

    fn close(a: P3, b: P3) -> bool {
        norm([a[0] - b[0], a[1] - b[1], a[2] - b[2]]) < 1e-9
    }

    #[test]
    fn lines_units_and_skipped_entities() {
        let text = dxf(
            "0\nLINE\n8\nDriven\n10\n0\n20\n0\n30\n0\n11\n100\n21\n0\n31\n50\n0\nTEXT\n8\n0\n1\nhello\n0\nINSERT\n2\nBLOCK1\n",
            Some(4),
        );
        let d = read_dxf(&text, 10.0).unwrap();
        assert_eq!(d.metres_per_unit, Some(0.001));
        assert_eq!(d.paths.len(), 1);
        assert_eq!(d.paths[0].layer, "Driven");
        assert!(close(d.paths[0].points[1], [100.0, 0.0, 50.0]));
        assert_eq!(d.skipped, vec![("INSERT".into(), 1), ("TEXT".into(), 1)]);
        let w = wires(
            &d,
            &WireOptions {
                metres_per_unit: 0.001,
                join_m: 0.0001,
                layers: vec![],
            },
        );
        assert_eq!(w.len(), 1);
        assert!(close(w[0].b, [0.1, 0.0, 0.05]));
        // CRLF line endings and no header: no units.
        let d = read_dxf(
            &dxf("0\nLINE\n10\n1\n11\n2\n", None).replace('\n', "\r\n"),
            10.0,
        )
        .unwrap();
        assert_eq!(d.metres_per_unit, None);
        assert_eq!(d.paths.len(), 1);
        assert!(read_dxf("AutoCAD Binary DXF\r\n", 10.0).is_err());
        assert!(read_dxf("hello\nworld\n", 10.0).is_err());
    }

    #[test]
    fn curves_lie_where_they_should() {
        // A closed polyline: a semicircle bulge (bulge 1) from (1,0) to (-1,0), then straight back.
        let text = dxf(
            "0\nLWPOLYLINE\n8\nL\n90\n2\n70\n1\n10\n1\n20\n0\n42\n1\n10\n-1\n20\n0\n\
             0\nARC\n10\n5\n20\n5\n30\n2\n40\n2\n50\n0\n51\n90\n\
             0\nCIRCLE\n10\n0\n20\n0\n30\n0\n40\n3\n210\n0\n220\n0\n230\n-1\n\
             0\nELLIPSE\n10\n0\n20\n0\n30\n0\n11\n4\n21\n0\n31\n0\n40\n0.5\n41\n0\n42\n6.283185307179586\n",
            None,
        );
        let d = read_dxf(&text, 10.0).unwrap();
        assert_eq!(d.paths.len(), 4);
        // The bulge: every point on the unit circle above the axis, then back to the start.
        let p = &d.paths[0].points;
        assert!(close(p[0], [1.0, 0.0, 0.0]) && close(*p.last().unwrap(), [1.0, 0.0, 0.0]));
        for q in &p[..p.len() - 1] {
            assert!((norm(*q) - 1.0).abs() < 1e-9 && q[1] >= -1e-9);
        }
        assert_eq!(p.len(), 18 + 2); // 180° in 10° steps, then the straight side
        // The arc from 0° to 90° about (5,5) at z = 2.
        let p = &d.paths[1].points;
        assert!(close(p[0], [7.0, 5.0, 2.0]) && close(*p.last().unwrap(), [5.0, 7.0, 2.0]));
        // A circle seen from below (normal -z) is mirrored in x: it starts at (-3, 0).
        assert!(close(d.paths[2].points[0], [-3.0, 0.0, 0.0]));
        // The ellipse reaches (0, 2) a quarter of the way round.
        let p = &d.paths[3].points;
        assert!(
            close(p[p.len() / 4], [0.0, 2.0, 0.0]) || close(p[(p.len() - 1) / 4], [0.0, 2.0, 0.0])
        );
    }

    #[test]
    fn splines_and_old_polylines() {
        // A quadratic Bezier as a clamped spline: its middle is P0/4 + P1/2 + P2/4.
        let text = dxf(
            "0\nSPLINE\n71\n2\n72\n6\n73\n3\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n\
             10\n0\n20\n0\n30\n0\n10\n1\n20\n2\n30\n0\n10\n2\n20\n0\n30\n0\n\
             0\nPOLYLINE\n70\n8\n0\nVERTEX\n10\n0\n20\n0\n30\n0\n0\nVERTEX\n10\n0\n20\n0\n30\n3\n0\nSEQEND\n\
             0\nLINE\n10\n9\n11\n10\n",
            None,
        );
        let d = read_dxf(&text, 10.0).unwrap();
        assert_eq!(d.paths.len(), 3);
        let p = &d.paths[0].points;
        assert!(close(p[0], [0.0, 0.0, 0.0]) && close(*p.last().unwrap(), [2.0, 0.0, 0.0]));
        assert!(close(p[p.len() / 2], [1.0, 1.0, 0.0]));
        assert_eq!(d.paths[1].points, vec![[0.0, 0.0, 0.0], [0.0, 0.0, 3.0]]);
        // The LINE after SEQEND is still read.
        assert!(close(d.paths[2].points[1], [10.0, 0.0, 0.0]));
    }

    #[test]
    fn ends_join_and_repeats_go() {
        // Two lines meeting 0.4 mm apart, a third drawn twice (once reversed),
        // and a 0.1 mm stub that vanishes.
        let text = dxf(
            "0\nLINE\n8\nA\n10\n0\n11\n100\n\
             0\nLINE\n8\nB\n10\n100.4\n11\n100.4\n21\n100\n\
             0\nLINE\n8\nA\n10\n0\n20\n50\n11\n0\n21\n150\n\
             0\nLINE\n8\nA\n10\n0\n20\n150\n11\n0\n21\n50\n\
             0\nLINE\n8\nA\n10\n500\n11\n500.1\n",
            Some(4),
        );
        let d = read_dxf(&text, 10.0).unwrap();
        let mut options = WireOptions {
            metres_per_unit: 0.001,
            join_m: 0.001,
            layers: vec![],
        };
        let w = wires(&d, &options);
        assert_eq!(w.len(), 3);
        assert!(close(w[0].b, w[1].a));
        options.layers = vec!["B".into()];
        assert_eq!(wires(&d, &options).len(), 1);
        assert_eq!(d.layers(), vec![("A".into(), 4), ("B".into(), 1)]);
    }
}
