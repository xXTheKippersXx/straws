//! Turning the model and its pattern into 3D geometry.

use eframe::egui::Color32;
use glam::Vec3;
use straws_core::analysis::pattern::{Component, NO_FIELD_DBI, PatternGrid};

use crate::colormap::viridis;
use crate::model::LoadedModel;
use crate::render3d::{LineInstance, MeshVertex, Scene};

/// Colours for the 3D views in the current theme.
#[derive(Clone, Copy)]
pub struct Palette {
    pub wire: Color32,
    pub feed: Color32,
    pub ground: Color32,
    pub axes: [Color32; 3],
    /// Wires with a geometry problem.
    pub problem: Color32,
    /// Selected wires and grabbed ends.
    pub selected: Color32,
    /// Surface patches.
    pub patch: Color32,
}

impl Palette {
    pub fn for_theme(dark: bool) -> Palette {
        if dark {
            Palette {
                wire: Color32::from_rgb(205, 214, 228),
                feed: Color32::from_rgb(255, 140, 60),
                ground: Color32::from_rgba_unmultiplied(120, 150, 120, 70),
                axes: [
                    Color32::from_rgb(230, 85, 85),
                    Color32::from_rgb(95, 200, 110),
                    Color32::from_rgb(90, 140, 240),
                ],
                problem: Color32::from_rgb(240, 90, 90),
                selected: Color32::from_rgb(90, 200, 255),
                patch: Color32::from_rgb(112, 124, 146),
            }
        } else {
            Palette {
                wire: Color32::from_rgb(40, 48, 62),
                feed: Color32::from_rgb(220, 90, 20),
                ground: Color32::from_rgba_unmultiplied(60, 110, 60, 70),
                axes: [
                    Color32::from_rgb(200, 40, 40),
                    Color32::from_rgb(30, 150, 50),
                    Color32::from_rgb(40, 90, 210),
                ],
                problem: Color32::from_rgb(210, 40, 40),
                selected: Color32::from_rgb(0, 120, 215),
                patch: Color32::from_rgb(168, 178, 196),
            }
        }
    }
}

fn v3(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
}

/// Wires, feed points, ground grid and axes.
pub fn model_scene(
    model: &LoadedModel,
    over_ground: bool,
    palette: Palette,
    problem_tags: &[u32],
    currents: Option<&[straws_core::output::SegmentCurrent]>,
    colours: Option<&[(u32, f32)]>,
) -> Scene {
    // Current magnitude on each segment, 0..1 of the largest.
    let largest = currents.map_or(0.0, |c| {
        c.iter().map(|s| s.current.norm()).fold(0.0, f64::max)
    });
    let current_of = |n: u32| -> Option<f32> {
        let c = currents?;
        let s = c.iter().find(|s| s.segment == n)?;
        (largest > 0.0).then(|| (s.current.norm() / largest) as f32)
    };
    let mut scene = Scene::default();
    let feeds = model.feed_segments();
    // Until the engine has laid the wires out (or if it couldn't), draw
    // them straight from the deck.
    if model.segments.is_empty() {
        for w in &model.wires {
            scene
                .lines
                .push(LineInstance::new(v3(w.a), v3(w.b), 2.5, palette.wire));
        }
    }
    for s in model.visible_segments() {
        let (a, b) = s.ends();
        let is_feed = feeds.contains(&s.number);
        let by_wire = colours.map(|c| c.iter().find(|(t, _)| *t == s.tag).map(|(_, v)| *v));
        let (width, colour) = if let Some(v) = by_wire {
            // Wires the sensitivity check didn't try stay plain.
            match v {
                Some(v) => (4.0, viridis(v)),
                None => (2.5, palette.wire.gamma_multiply(0.5)),
            }
        } else if let Some(i) = current_of(s.number) {
            (if is_feed { 6.0 } else { 4.0 }, viridis(i))
        } else if is_feed {
            (6.0, palette.feed)
        } else if problem_tags.contains(&s.tag) {
            (3.0, palette.problem)
        } else {
            (2.5, palette.wire)
        };
        scene
            .lines
            .push(LineInstance::new(v3(a), v3(b), width, colour));
    }
    // Surface patches: a square of each patch's area, outlined.
    for p in &model.patches {
        let corners = p.corners().map(v3);
        let base = scene.mesh.len() as u32;
        for c in corners {
            scene.mesh.push(MeshVertex {
                pos: c.to_array(),
                normal: v3(p.normal).to_array(),
                color: crate::render3d::rgba(palette.patch),
            });
        }
        scene
            .mesh_indices
            .extend([0, 1, 2, 0, 2, 3].map(|k| base + k));
        for k in 0..4 {
            scene.lines.push(LineInstance::new(
                corners[k],
                corners[(k + 1) % 4],
                1.0,
                palette.wire.gamma_multiply(0.6),
            ));
        }
    }
    let (centre, radius) = model.bounds();
    let reach = radius * 1.3 + (Vec3::from(centre).truncate().length());
    if over_ground {
        // A grid on z = 0 under the antenna, in round-number steps.
        let step = nice(reach / 6.0);
        let n = (reach / step).ceil() as i32;
        let extent = n as f32 * step;
        let (cx, cy) = (
            (centre[0] / step).round() * step,
            (centre[1] / step).round() * step,
        );
        for i in -n..=n {
            let o = i as f32 * step;
            scene.lines.push(LineInstance::new(
                Vec3::new(cx + o, cy - extent, 0.0),
                Vec3::new(cx + o, cy + extent, 0.0),
                1.0,
                palette.ground,
            ));
            scene.lines.push(LineInstance::new(
                Vec3::new(cx - extent, cy + o, 0.0),
                Vec3::new(cx + extent, cy + o, 0.0),
                1.0,
                palette.ground,
            ));
        }
    }
    axes(&mut scene, radius * 0.35, palette);
    scene
}

fn axes(scene: &mut Scene, length: f32, palette: Palette) {
    for (k, dir) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
        scene.lines.push(LineInstance::new(
            Vec3::ZERO,
            dir * length,
            2.0,
            palette.axes[k],
        ));
    }
}

/// Rounds to 1, 2 or 5 times a power of ten.
pub fn nice(x: f32) -> f32 {
    if x.is_nan() || x <= 0.0 {
        return 1.0;
    }
    let p = 10f32.powf(x.log10().floor());
    let m = x / p;
    p * if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    }
}

/// How gains map to the pattern's radius and colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GainScale {
    pub max_dbi: f64,
    /// dB below the peak that maps to the centre.
    pub range_db: f64,
}

impl GainScale {
    /// 0 at `range_db` below the peak (or below), 1 at the peak.
    pub fn unit(&self, dbi: f64) -> f32 {
        if dbi <= NO_FIELD_DBI {
            return 0.0;
        }
        (((dbi - (self.max_dbi - self.range_db)) / self.range_db).clamp(0.0, 1.0)) as f32
    }
}

/// Direction for theta (from zenith) and phi (from +X), degrees.
pub fn direction(theta: f64, phi: f64) -> Vec3 {
    let (t, p) = (theta.to_radians(), phi.to_radians());
    Vec3::new(
        (t.sin() * p.cos()) as f32,
        (t.sin() * p.sin()) as f32,
        t.cos() as f32,
    )
}

/// The pattern surface: radius and colour from gain. `size` is the radius
/// of the peak, `centre` where it sits.
pub fn pattern_scene(
    grid: &PatternGrid,
    component: Component,
    scale: GainScale,
    centre: Vec3,
    size: f32,
    alpha: u8,
) -> Scene {
    let (nt, np) = (grid.thetas.len(), grid.phis.len());
    let mut scene = Scene {
        mesh_translucent: alpha < 255,
        ..Scene::default()
    };
    let mut positions = Vec::with_capacity(nt * np);
    for (it, &t) in grid.thetas.iter().enumerate() {
        for (ip, &p) in grid.phis.iter().enumerate() {
            let u = scale.unit(grid.gain(component, it, ip));
            positions.push(centre + direction(t, p) * (u * size));
            let c = viridis(u);
            let [r, g, b, _] = c.to_srgba_unmultiplied();
            scene.mesh.push(MeshVertex {
                pos: [0.0; 3],
                normal: [0.0; 3],
                color: [r, g, b, alpha].map(|v| f32::from(v) / 255.0),
            });
        }
    }
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for it in 0..nt.saturating_sub(1) {
        for ip in 0..np.saturating_sub(1) {
            let k = |i: usize, j: usize| (i * np + j) as u32;
            let quad = [k(it, ip), k(it + 1, ip), k(it + 1, ip + 1), k(it, ip + 1)];
            for tri in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
                let [a, b, c] = tri.map(|i| positions[i as usize]);
                let n = (b - a).cross(c - a);
                for &i in &tri {
                    normals[i as usize] += n;
                }
                scene.mesh_indices.extend_from_slice(&tri);
            }
        }
    }
    for (k, v) in scene.mesh.iter_mut().enumerate() {
        v.pos = positions[k].to_array();
        v.normal = normals[k].normalize_or(Vec3::Z).to_array();
    }
    scene
}

/// Where a ray from the camera first meets the pattern surface, as
/// (theta, phi) of that point seen from the pattern's centre.
pub fn pick_pattern(scene: &Scene, centre: Vec3, origin: Vec3, dir: Vec3) -> Option<(f64, f64)> {
    let mut best: Option<(f32, Vec3)> = None;
    for tri in scene.mesh_indices.as_chunks::<3>().0 {
        let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(scene.mesh[tri[k] as usize].pos));
        if let Some(t) = ray_triangle(origin, dir, a, b, c)
            && best.is_none_or(|(bt, _)| t < bt)
        {
            best = Some((t, origin + dir * t));
        }
    }
    let (_, hit) = best?;
    let d = (hit - centre).normalize_or_zero();
    if d == Vec3::ZERO {
        return None;
    }
    let theta = f64::from(d.z.clamp(-1.0, 1.0).acos().to_degrees());
    let phi = f64::from(d.y.atan2(d.x).to_degrees()).rem_euclid(360.0);
    Some((theta, phi))
}

/// Möller–Trumbore ray/triangle intersection: distance along the ray.
fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_triangle() {
        let t = ray_triangle(
            Vec3::new(0.2, 0.2, 5.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
        );
        assert!((t.unwrap() - 5.0).abs() < 1e-6);
        assert!(
            ray_triangle(
                Vec3::new(2.0, 2.0, 5.0),
                -Vec3::Z,
                Vec3::ZERO,
                Vec3::X,
                Vec3::Y
            )
            .is_none()
        );
    }

    #[test]
    fn gain_scale_and_directions() {
        let s = GainScale {
            max_dbi: 10.0,
            range_db: 40.0,
        };
        assert_eq!(s.unit(10.0), 1.0);
        assert_eq!(s.unit(-30.0), 0.0);
        assert_eq!(s.unit(-999.99), 0.0);
        assert!((s.unit(-10.0) - 0.5).abs() < 1e-6);
        assert!((direction(0.0, 0.0) - Vec3::Z).length() < 1e-6);
        assert!((direction(90.0, 90.0) - Vec3::Y).length() < 1e-6);
        assert_eq!(nice(3.7), 5.0);
        assert_eq!(nice(0.12), 0.1);
    }
}
