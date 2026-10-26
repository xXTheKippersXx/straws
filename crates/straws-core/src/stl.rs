//! Solid models as STL meshes: straight round tubes found in them become
//! wires (their axis and radius).

use std::collections::HashMap;

use crate::dxf::DrawnWire;

type P3 = [f64; 3];

/// Reads an STL file (binary or ASCII) as triangles, in its own units.
pub fn read_stl(bytes: &[u8]) -> Result<Vec<[P3; 3]>, String> {
    // Binary: 80-byte header, a triangle count, then 50 bytes a triangle.
    // Some binary files start with "solid" too, so the size decides.
    if bytes.len() >= 84 {
        let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        if n.checked_mul(50).and_then(|b| b.checked_add(84)) == Some(bytes.len()) {
            let float = |at: usize| {
                f64::from(f32::from_le_bytes([
                    bytes[at],
                    bytes[at + 1],
                    bytes[at + 2],
                    bytes[at + 3],
                ]))
            };
            return Ok((0..n)
                .map(|k| {
                    let at = 84 + 50 * k + 12; // after the normal
                    [0, 1, 2].map(|v| [0, 1, 2].map(|i| float(at + 12 * v + 4 * i)))
                })
                .collect());
        }
    }
    let text = String::from_utf8_lossy(bytes);
    if !text.trim_start().starts_with("solid") {
        return Err(
            "it's neither an ASCII STL (starting \"solid\") nor a binary one of the right size"
                .into(),
        );
    }
    let mut points = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut words = line.split_whitespace();
        if words.next() == Some("vertex") {
            let p: Vec<f64> = words.filter_map(|w| w.parse().ok()).collect();
            if p.len() != 3 {
                return Err(format!("line {}: a vertex needs three numbers", n + 1));
            }
            points.push([p[0], p[1], p[2]]);
        }
    }
    if points.is_empty() || points.len() % 3 != 0 {
        return Err("no complete triangles in it".into());
    }
    Ok(points.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
}

/// A straight round tube found in a mesh: its axis ends and outer radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tube {
    pub a: P3,
    pub b: P3,
    pub radius: f64,
}

/// What a mesh holds: its tubes, and what else was left out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parts {
    pub tubes: Vec<Tube>,
    /// Flat pieces (tube ends, plates, faces of blocks).
    pub flat: usize,
    /// Curved parts that aren't straight round tubes (bent tubes, dishes).
    pub other: usize,
}

/// Neighbouring triangles folded by more than this (degrees) are in
/// different parts: a tube's side wall stays whole (12 sides fold 30°
/// each), its flat ends and anything joined at an angle come apart.
const CREASE_DEG: f64 = 70.0;

/// Splits the mesh into parts (triangles sharing edges without a sharp
/// fold) and turns each straight round tube into its axis and radius. A
/// tube inside a larger one on the same axis (a pipe's inner wall) is left
/// out.
pub fn tubes(triangles: &[[P3; 3]]) -> Parts {
    // Weld corners that are the same point (CAD exports repeat them).
    let corners: Vec<P3> = triangles.iter().flatten().copied().collect();
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &corners {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let size = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f64>().sqrt();
    let id = crate::dxf::merge_points(&corners, 1e-7 * size.max(1e-12));
    let normals: Vec<Option<P3>> = triangles
        .iter()
        .map(|t| {
            let n = cross(
                [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]],
                [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]],
            );
            (length(n) > 0.0).then(|| unit(n))
        })
        .collect();
    // Triangles sharing an edge, without a sharp fold, are one part.
    let mut parent: Vec<usize> = (0..triangles.len()).collect();
    let mut edges: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for t in 0..triangles.len() {
        let v = [id[3 * t], id[3 * t + 1], id[3 * t + 2]];
        for (x, y) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
            if x != y {
                edges.entry((x.min(y), x.max(y))).or_default().push(t);
            }
        }
    }
    let smooth = CREASE_DEG.to_radians().cos();
    for sharing in edges.values() {
        for (k, &a) in sharing.iter().enumerate() {
            for &b in &sharing[k + 1..] {
                if let (Some(na), Some(nb)) = (normals[a], normals[b])
                    && dot(na, nb) > smooth
                {
                    union(&mut parent, a, b);
                }
            }
        }
    }
    let mut parts: HashMap<usize, Vec<usize>> = HashMap::new();
    for (t, n) in normals.iter().enumerate() {
        if n.is_some() {
            let root = find(&mut parent, t);
            parts.entry(root).or_default().push(t);
        }
    }
    let mut roots: Vec<usize> = parts.keys().copied().collect();
    roots.sort_unstable();
    let mut out = Parts::default();
    let mut found = Vec::new();
    for r in roots {
        let tris = &parts[&r];
        let first = normals[tris[0]].unwrap_or([0.0, 0.0, 1.0]);
        if tris
            .iter()
            .all(|&t| normals[t].is_some_and(|n| dot(n, first) > 0.9999))
        {
            out.flat += 1;
            continue;
        }
        let mut vertices: Vec<usize> = tris
            .iter()
            .flat_map(|&t| [id[3 * t], id[3 * t + 1], id[3 * t + 2]])
            .collect();
        vertices.sort_unstable();
        vertices.dedup();
        let points: Vec<P3> = vertices.iter().map(|&v| corners[v]).collect();
        match tube_of(&points) {
            Some(t) => found.push(t),
            None => out.other += 1,
        }
    }
    // A tube inside a larger one with the same ends is the larger one's
    // inner wall.
    let near = |p: P3, q: P3, tol: f64| length([p[0] - q[0], p[1] - q[1], p[2] - q[2]]) <= tol;
    for (k, t) in found.iter().enumerate() {
        let inner = found.iter().enumerate().any(|(j, o)| {
            j != k
                && (o.radius > t.radius || (o.radius == t.radius && j < k))
                && ((near(o.a, t.a, o.radius) && near(o.b, t.b, o.radius))
                    || (near(o.a, t.b, o.radius) && near(o.b, t.a, o.radius)))
        });
        if !inner {
            out.tubes.push(*t);
        }
    }
    out
}

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

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (a, b) = (find(parent, a), find(parent, b));
    if a != b {
        parent[a.max(b)] = a.min(b);
    }
}

/// The tube these points (one part's corners) make, if they make one: the
/// longest principal axis, with most points at one radius from it, spread
/// all the way round.
fn tube_of(points: &[P3]) -> Option<Tube> {
    let n = points.len() as f64;
    if points.len() < 12 {
        return None;
    }
    let c = [0, 1, 2].map(|i| points.iter().map(|p| p[i]).sum::<f64>() / n);
    let mut cov = [[0.0; 3]; 3];
    for p in points {
        let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j] / n;
            }
        }
    }
    // The axis: the strongest direction (power iteration, from the
    // covariance's largest column so it can't start square to it).
    let mut axis = (0..3)
        .map(|j| [cov[0][j], cov[1][j], cov[2][j]])
        .max_by(|a, b| length(*a).total_cmp(&length(*b)))?;
    for _ in 0..200 {
        let next = [0, 1, 2].map(|i| (0..3).map(|j| cov[i][j] * axis[j]).sum::<f64>());
        let l = length(next);
        if l == 0.0 {
            return None;
        }
        axis = next.map(|v| v / l);
    }
    // Along and out from the axis.
    let helper = if axis[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let e1 = unit(cross(axis, helper));
    let e2 = cross(axis, e1);
    let mut along = (f64::MAX, f64::MIN);
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for p in points {
        let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
        let t = dot(d, axis);
        along = (along.0.min(t), along.1.max(t));
        let (x, y) = (dot(d, e1), dot(d, e2));
        out.push(((x * x + y * y).sqrt(), y.atan2(x)));
    }
    let radius = out.iter().map(|o| o.0).fold(0.0, f64::max);
    if radius <= 0.0 || along.1 - along.0 < 2.0 * radius {
        return None;
    }
    // Round: many points on the outside, all round it.
    let rim: Vec<f64> = out
        .iter()
        .filter(|o| o.0 >= 0.97 * radius)
        .map(|o| o.1)
        .collect();
    let mut bins = [false; 64];
    for a in &rim {
        let k = ((a + std::f64::consts::PI) / (2.0 * std::f64::consts::PI) * 64.0) as usize;
        bins[k.min(63)] = true;
    }
    if (rim.len() as f64) < 0.4 * n || bins.iter().filter(|b| **b).count() < 6 {
        return None;
    }
    let at = |t: f64| [0, 1, 2].map(|i| c[i] + t * axis[i]);
    Some(Tube {
        a: at(along.0),
        b: at(along.1),
        radius,
    })
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

fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The tubes as wires in metres (`metres_per_unit` of the mesh's units),
/// with ends within `join_m` of each other made one point.
pub fn wires(tubes: &[Tube], metres_per_unit: f64, join_m: f64) -> Vec<DrawnWire> {
    let ends: Vec<P3> = tubes
        .iter()
        .flat_map(|t| [t.a, t.b])
        .map(|p| p.map(|v| v * metres_per_unit))
        .collect();
    let id = crate::dxf::merge_points(&ends, join_m.max(0.0));
    let ends = crate::dxf::merged_positions(&ends, &id);
    tubes
        .iter()
        .enumerate()
        .filter(|(k, _)| id[2 * k] != id[2 * k + 1])
        .map(|(k, t)| DrawnWire {
            a: ends[id[2 * k]],
            b: ends[id[2 * k + 1]],
            layer: format!("tube {}", k + 1),
            radius: Some(t.radius * metres_per_unit),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P3, b: P3, tol: f64) -> bool {
        length([a[0] - b[0], a[1] - b[1], a[2] - b[2]]) < tol
    }

    /// A closed tube of `sides` sides from a to b (corners at radius r).
    fn tube_mesh(a: P3, b: P3, r: f64, sides: usize) -> Vec<[P3; 3]> {
        let d = unit([b[0] - a[0], b[1] - a[1], b[2] - a[2]]);
        let helper = if d[2].abs() < 0.9 {
            [0.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let e1 = unit(cross(d, helper));
        let e2 = cross(d, e1);
        let ring = |c: P3, k: usize| {
            let t = 2.0 * std::f64::consts::PI * k as f64 / sides as f64;
            [0, 1, 2].map(|i| c[i] + r * (t.cos() * e1[i] + t.sin() * e2[i]))
        };
        let mut out = Vec::new();
        for k in 0..sides {
            let (p0, p1, q0, q1) = (ring(a, k), ring(a, k + 1), ring(b, k), ring(b, k + 1));
            out.extend([[p0, p1, q1], [p0, q1, q0], [a, p1, p0], [b, q0, q1]]);
        }
        out
    }

    fn binary(tris: &[[P3; 3]]) -> Vec<u8> {
        let mut out = vec![0u8; 80];
        out.extend((tris.len() as u32).to_le_bytes());
        for t in tris {
            out.extend([0u8; 12]);
            for p in t {
                for v in p {
                    out.extend((*v as f32).to_le_bytes());
                }
            }
            out.extend([0u8; 2]);
        }
        out
    }

    #[test]
    fn tubes_are_found_and_joined() {
        // An L of two tubes meeting at a point, a plate, and a square bar.
        let mut mesh = tube_mesh([0.0, 0.0, 0.0], [0.0, 0.0, 500.0], 5.0, 16);
        mesh.extend(tube_mesh([0.0, 0.0, 500.0], [300.0, 0.0, 500.0], 3.0, 12));
        mesh.extend(tube_mesh([0.0, 100.0, 0.0], [0.0, 400.0, 0.0], 50.0, 4));
        let parts = tubes(&mesh);
        assert_eq!(parts.tubes.len(), 2);
        // The square bar's four sides and every end are flat.
        assert_eq!(parts.other, 0);
        assert_eq!(parts.flat, 4 + 2 * 2 + 2);
        let t = parts.tubes[0];
        assert!((t.radius - 5.0).abs() < 1e-9);
        let w = wires(&parts.tubes, 0.001, 0.001);
        assert_eq!(w.len(), 2);
        assert!(close(w[0].b, [0.0, 0.0, 0.5], 1e-9) || close(w[0].a, [0.0, 0.0, 0.5], 1e-9));
        assert!((w[1].radius.unwrap() - 0.003).abs() < 1e-12);
        // The two tubes share their corner point exactly.
        let corner = [w[0].a, w[0].b]
            .into_iter()
            .find(|p| [w[1].a, w[1].b].contains(p));
        assert!(corner.is_some());
    }

    #[test]
    fn a_feed_gap_closes_in_the_middle() {
        let mut mesh = tube_mesh([0.0, -490.0, 0.0], [0.0, -5.0, 0.0], 5.0, 16);
        mesh.extend(tube_mesh([0.0, 5.0, 0.0], [0.0, 490.0, 0.0], 5.0, 16));
        let w = wires(&tubes(&mesh).tubes, 0.001, 0.012);
        assert_eq!(w.len(), 2);
        for wire in &w {
            let long = length([
                wire.b[0] - wire.a[0],
                wire.b[1] - wire.a[1],
                wire.b[2] - wire.a[2],
            ]);
            assert!((long - 0.490).abs() < 1e-9, "{long}");
        }
    }

    #[test]
    fn binary_and_ascii_files() {
        let mesh = tube_mesh([0.0, 0.0, 0.0], [100.0, 0.0, 0.0], 2.0, 12);
        let b = read_stl(&binary(&mesh)).unwrap();
        assert_eq!(b.len(), mesh.len());
        assert!(close(b[0][0], mesh[0][0], 1e-4));
        let mut text = String::from("solid t\n");
        for t in &mesh {
            text += "facet normal 0 0 0\nouter loop\n";
            for p in t {
                text += &format!("vertex {} {} {}\n", p[0], p[1], p[2]);
            }
            text += "endloop\nendfacet\n";
        }
        text += "endsolid t\n";
        let a = read_stl(text.as_bytes()).unwrap();
        assert_eq!(a, mesh);
        let found = tubes(&a).tubes;
        assert_eq!(found.len(), 1);
        assert!(
            close(found[0].a, [0.0, 0.0, 0.0], 1e-9) || close(found[0].b, [0.0, 0.0, 0.0], 1e-9)
        );
        assert!(read_stl(b"hello").is_err());
    }

    #[test]
    fn hollow_tubes_use_their_outside() {
        // A pipe: outer and inner walls as two tubes in one part (joined by end rings).
        let (a, b) = ([0.0, 0.0, 0.0], [0.0, 200.0, 0.0]);
        let outer = tube_mesh(a, b, 10.0, 24);
        let inner = tube_mesh(a, b, 8.0, 24);
        // Keep the side walls only and join them with end rings sharing edges.
        let sides =
            |m: &[[P3; 3]]| -> Vec<[P3; 3]> { m.chunks(4).flat_map(|c| [c[0], c[1]]).collect() };
        let mut mesh = sides(&outer);
        mesh.extend(sides(&inner));
        for k in 0..24 {
            let o = &outer[4 * k];
            let i = &inner[4 * k];
            // Annulus between the rings at each end.
            mesh.push([o[0], o[1], i[1]]);
            mesh.push([o[0], i[1], i[0]]);
            mesh.push([o[2], o[1], i[2]]);
        }
        let found = tubes(&mesh);
        assert_eq!(found.tubes.len(), 1, "{found:?}");
        assert!((found.tubes[0].radius - 10.0).abs() < 1e-9);
    }
}
