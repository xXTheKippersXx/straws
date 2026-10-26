//! The 3D model view and the 3D pattern view.

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};
use glam::{Mat4, Vec3, Vec4};
use straws_core::analysis::checks::Severity;
use straws_core::analysis::pattern::PatternGrid;
use straws_core::deck::edit::moved_end;
use straws_core::deck::radials::radial_sets;

use crate::app::{Data, Settings};
use crate::colormap::viridis;
use crate::render3d::{Gpu, OrbitCamera, Scene, SceneBuffers, Viewport};
use crate::scene::{self, GainScale, Palette};
use crate::views::curves::auto_segments as curve_segments;
use crate::views::newvar;
use crate::views::wires::{auto_segments, show_length};
use straws_core::deck::curves::{CurveKind, CurveSpec, arc_between, curves, facing, sweep_through};

use super::empty_note;

/// Identifies what a cached scene was built from, so it's rebuilt only
/// when something it depends on changes.
#[derive(Clone, Copy, PartialEq, Debug)]
struct PatternKey {
    generation: u64,
    mhz: u64,
    step: u64,
    component: usize,
    range: u64,
    dark: bool,
}

fn pattern_key(data: &Data, settings: &Settings, dark: bool) -> Option<PatternKey> {
    let p = data.shown_point()?;
    p.pattern.as_ref()?;
    Some(PatternKey {
        generation: data.generation,
        mhz: p.mhz.to_bits(),
        step: p.pattern_step.unwrap_or(0.0).to_bits(),
        component: settings.component,
        range: settings.range_db.to_bits(),
        dark,
    })
}

/// Draws the area for a 3D view and renders into it. Returns the area and
/// the mouse response for overlays.
fn show_3d(
    ui: &mut egui::Ui,
    gpu: &Gpu,
    viewport: &mut Option<Viewport>,
    camera: &mut OrbitCamera,
    home: OrbitCamera,
    scenes: &[&SceneBuffers],
) -> (Rect, egui::Response) {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    camera.handle_input(ui, &response);
    if response.double_clicked() {
        *camera = home;
    }
    render_3d(ui, gpu, viewport, camera, rect, scenes);
    (rect, response)
}

/// Renders the scenes into `rect`.
fn render_3d(
    ui: &mut egui::Ui,
    gpu: &Gpu,
    viewport: &mut Option<Viewport>,
    camera: &OrbitCamera,
    rect: Rect,
    scenes: &[&SceneBuffers],
) {
    let ppp = ui.ctx().pixels_per_point();
    let size = [
        (rect.width() * ppp).round().max(1.0) as u32,
        (rect.height() * ppp).round().max(1.0) as u32,
    ];
    let vp = viewport.get_or_insert_with(|| Viewport::new(gpu, size));
    vp.render(gpu, size, camera, ui.visuals().extreme_bg_color, scenes);
    ui.painter().image(
        vp.texture_id,
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// Text in a corner of a 3D view, on a faint panel so it stays readable.
fn corner_text(ui: &egui::Ui, rect: Rect, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let font = egui::FontId::proportional(13.0);
    let colour = ui.visuals().text_color();
    let painter = ui.painter_at(rect);
    let mut y = rect.top() + 8.0;
    for line in lines {
        let galley = painter.layout_no_wrap(line.clone(), font.clone(), colour);
        let r = Rect::from_min_size(pos2(rect.left() + 8.0, y), galley.size()).expand(3.0);
        painter.rect_filled(r, 3.0, ui.visuals().extreme_bg_color.gamma_multiply(0.75));
        painter.galley(pos2(rect.left() + 8.0, y), galley, colour);
        y += 19.0;
    }
}

/// View buttons along the top of a 3D view.
fn view_buttons(ui: &mut egui::Ui, camera: &mut OrbitCamera, home: OrbitCamera) {
    use std::f32::consts::FRAC_PI_2;
    let mut set = |yaw: f32, pitch: f32| {
        camera.yaw = yaw;
        camera.pitch = pitch;
    };
    if ui
        .small_button("Front")
        .on_hover_text("Look along +X")
        .clicked()
    {
        set(FRAC_PI_2, 0.0);
    }
    if ui
        .small_button("Side")
        .on_hover_text("Look along +Y")
        .clicked()
    {
        set(0.0, 0.0);
    }
    if ui
        .small_button("Top")
        .on_hover_text("Look straight down")
        .clicked()
    {
        set(0.0, 1.55);
    }
    if ui
        .small_button("Reset")
        .on_hover_text("Back to the starting view (or double-click the view)")
        .clicked()
    {
        *camera = home;
    }
}

/// What clicks in the model view do.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Click wires to select them, drag wire ends to move them.
    #[default]
    Select,
    /// Click to place wire ends; each click starts the next wire.
    Draw,
}

/// A wire end being dragged: every end at that point moves together, so
/// junctions stay joined.
struct EndDrag {
    ends: Vec<(usize, usize)>,
    /// The plane the end moves in: coordinate `axis` stays at `value`.
    axis: usize,
    value: f64,
}

/// World-to-screen projection for one frame of a 3D view.
struct Projector {
    view_proj: Mat4,
    rect: Rect,
    camera: OrbitCamera,
}

impl Projector {
    fn new(camera: &OrbitCamera, rect: Rect) -> Projector {
        Projector {
            view_proj: camera.view_proj(rect.width() / rect.height().max(1.0)),
            rect,
            camera: *camera,
        }
    }

    fn screen(&self, p: [f64; 3]) -> Option<Pos2> {
        let c = self.view_proj * Vec4::new(p[0] as f32, p[1] as f32, p[2] as f32, 1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let (x, y) = (c.x / c.w, c.y / c.w);
        Some(pos2(
            self.rect.left() + (x + 1.0) / 2.0 * self.rect.width(),
            self.rect.top() + (1.0 - y) / 2.0 * self.rect.height(),
        ))
    }

    /// How far `p` is from the camera, for picking the nearest of several.
    fn depth(&self, p: [f64; 3]) -> f32 {
        let c = self.view_proj * Vec4::new(p[0] as f32, p[1] as f32, p[2] as f32, 1.0);
        c.w
    }

    /// Where the mouse ray meets the plane where coordinate `axis` = `value`.
    fn on_plane(&self, pos: Pos2, axis: usize, value: f64) -> Option<[f64; 3]> {
        let ndc = vec2(
            (pos.x - self.rect.left()) / self.rect.width() * 2.0 - 1.0,
            1.0 - (pos.y - self.rect.top()) / self.rect.height() * 2.0,
        );
        let (o, d) = self
            .camera
            .ray(ndc, self.rect.width() / self.rect.height().max(1.0));
        let (o, d) = (o.to_array().map(f64::from), d.to_array().map(f64::from));
        if d[axis].abs() < 1e-4 {
            return None;
        }
        let t = (value - o[axis]) / d[axis];
        (t > 0.0).then(|| [0, 1, 2].map(|k| if k == axis { value } else { o[k] + d[k] * t }))
    }

    /// The axis-aligned plane most nearly facing the camera.
    fn facing_axis(&self) -> usize {
        let f = self.camera.target - self.camera.eye();
        let a = f.abs();
        if a.z >= a.x && a.z >= a.y {
            2
        } else if a.y >= a.x {
            1
        } else {
            0
        }
    }
}

/// Whether `p` is inside the convex polygon `corners` (either winding).
fn inside(p: Pos2, corners: &[Pos2]) -> bool {
    let n = corners.len();
    let side = |k: usize| {
        let (a, b) = (corners[k], corners[(k + 1) % n]);
        (b - a).x * (p - a).y - (b - a).y * (p - a).x
    };
    let sides: Vec<f32> = (0..n).map(side).collect();
    sides.iter().all(|&s| s >= 0.0) || sides.iter().all(|&s| s <= 0.0)
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

/// Pixels within which the mouse grabs or snaps to a wire end.
const GRAB_PX: f32 = 9.0;

/// What the model's wire scene was built from: generation, geometry,
/// theme, problem wires and (if shown) which currents.
type WiresKey = (u64, u64, bool, Vec<u32>, Option<(u64, u64)>);

#[derive(Default)]
pub struct ModelView {
    /// The selected reflector's z = f(x, y) box.
    formula: crate::views::reflectors::FormulaEdit,
    camera: Option<OrbitCamera>,
    viewport: Option<Viewport>,
    wires: Option<SceneBuffers>,
    wires_key: Option<WiresKey>,
    overlay: Option<SceneBuffers>,
    overlay_key: Option<PatternKey>,
    pub tool: Tool,
    drag: Option<EndDrag>,
    /// The end of the wire being drawn so far.
    draw_from: Option<[f64; 3]>,
    /// What the Draw tool draws.
    shape: Shape,
    /// Points clicked so far for a curve: an arc's two ends (then the
    /// bulge), or a circle's, helix's or spiral's centre.
    curve_points: Vec<[f64; 3]>,
    /// The plane a curve is being drawn on (fixed at its first click).
    curve_axis: Option<usize>,
    /// The last point placed while drawing: new points go on the plane
    /// through it, unless the mouse is on a wire or curve end.
    last_point: Option<[f64; 3]>,
    /// Where the drawing plane is this frame (for the hint).
    plane_value: Option<f64>,
    /// Grid step for snapping, metres (worked out from the model at first).
    grid: Option<f64>,
}

impl ModelView {
    pub fn invalidate(&mut self) {
        self.wires_key = None;
        self.overlay_key = None;
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        gpu: Option<&Gpu>,
        data: &mut Data,
        settings: &mut Settings,
    ) {
        let Some(gpu) = gpu else {
            return empty_note(ui, "3D views need a GPU renderer, which isn't available.");
        };
        let Some(model) = &data.model else {
            return welcome(ui, data, settings);
        };
        if model.segments.is_empty()
            && model.patches.is_empty()
            && model.has_structure()
            && model.wires.is_empty()
            && model.geometry_error.is_none()
        {
            return empty_note(ui, "Reading the geometry…");
        }
        let dark = ui.visuals().dark_mode;
        let palette = Palette::for_theme(dark);
        let mut problems: Vec<u32> = data
            .checks
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .flat_map(|i| i.tags.iter().copied())
            .collect();
        problems.sort_unstable();
        problems.dedup();
        // Currents at the slider frequency, if they're shown and match the
        // geometry on screen.
        let point = data.shown_point();
        let currents = point
            .filter(|_| settings.show_currents)
            .map(|p| p.currents.as_slice())
            .filter(|c| !c.is_empty());
        // The sensitivity check's colours, while they belong to this model.
        let colours = data
            .wire_colours
            .as_ref()
            .filter(|c| c.generation == data.model_generation);
        let current_key = match colours {
            Some(c) => Some((u64::MAX, c.version)),
            None => currents
                .and(point)
                .map(|p| (p.mhz.to_bits(), p.pattern_step.unwrap_or(0.0).to_bits())),
        };
        let key = (
            data.model_generation,
            model.geometry_version,
            dark,
            problems,
            current_key,
        );
        if self.wires_key.as_ref() != Some(&key) {
            self.wires = Some(gpu.upload(&scene::model_scene(
                model,
                data.over_ground,
                palette,
                &key.3,
                currents,
                colours.map(|c| c.values.as_slice()),
            )));
            self.wires_key = Some(key);
        }
        let (centre, radius) = model.bounds();
        let home = OrbitCamera::looking_at(centre, radius);
        let camera = self.camera.get_or_insert(home);
        let unit = settings.length();
        let mhz = data.mhz.max(1e-3);
        let grid = *self
            .grid
            .get_or_insert_with(|| f64::from(scene::nice(radius / 10.0)));

        ui.horizontal(|ui| {
            view_buttons(ui, camera, home);
            ui.separator();
            ui.selectable_value(&mut self.tool, Tool::Select, "Select")
                .on_hover_text("Click a wire to select it; drag a wire end to move it");
            ui.selectable_value(&mut self.tool, Tool::Draw, "Draw")
                .on_hover_text(
                    "Click to place wire ends, one wire after another; Esc or right-click stops. \
                     Wires are drawn on the grid plane facing you (use Front, Side or Top).",
                );
            if self.tool == Tool::Draw {
                let before = self.shape;
                egui::ComboBox::from_id_salt("draw shape")
                    .selected_text(self.shape.label())
                    .width(90.0)
                    // Every shape in view, without scrolling.
                    .height(430.0)
                    .show_ui(ui, |ui| {
                        for s in Shape::ALL {
                            ui.selectable_value(&mut self.shape, s, s.label())
                                .on_hover_text(s.help());
                        }
                    })
                    .response
                    .on_hover_text(
                        "What to draw, on the grid plane facing you (Front, Side or Top). \
                         Click a curve afterwards to adjust its numbers.",
                    );
                if self.shape != before {
                    self.curve_points.clear();
                    self.curve_axis = None;
                    self.draw_from = None;
                }
            }
            ui.checkbox(&mut settings.snap, "Snap").on_hover_text(
                "Snap to wire and curve ends and the grid; arcs snap to 15° steps. \
                 Hold Shift to keep a wire to 15° steps, Alt to turn snapping off for a moment.",
            );
            let mut g = unit.from_metres(grid, mhz);
            let speed = g * 0.02;
            if ui
                .add(
                    crate::views::num(&mut g)
                        .speed(speed)
                        .range(1e-9..=1e9)
                        .max_decimals(6)
                        .suffix(format!(" {}", unit.symbol())),
                )
                .on_hover_text("Grid step for snapping")
                .changed()
            {
                self.grid = Some(unit.to_metres(g, mhz));
            }
            ui.separator();
            ui.checkbox(&mut settings.show_currents, "Currents")
                .on_hover_text("Colour the wires by the current on them (yellow strongest); hover a wire for its current and phase");
            ui.checkbox(&mut settings.pattern_on_model, "Pattern on antenna")
                .on_hover_text("Draw the radiation pattern over the antenna, see-through");
        });
        if self.tool != Tool::Draw {
            self.draw_from = None;
        }

        let mut scenes: Vec<&SceneBuffers> = Vec::new();
        if let Some(w) = &self.wires {
            scenes.push(w);
        }
        if settings.pattern_on_model {
            let pkey = pattern_key(data, settings, dark);
            if pkey != self.overlay_key {
                self.overlay = data
                    .shown_point()
                    .and_then(|p| p.pattern.as_ref())
                    .map(|grid| {
                        let s = scene::pattern_scene(
                            grid,
                            settings.component(),
                            gain_scale(grid, settings),
                            Vec3::from(centre),
                            radius * 1.2,
                            110,
                        );
                        gpu.upload(&s)
                    });
                self.overlay_key = pkey;
            }
            if let Some(o) = &self.overlay {
                scenes.push(o);
            }
        }

        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let proj = Projector::new(camera, rect);
        let hover = response.hover_pos();
        let alt = ui.input(|i| i.modifiers.alt);
        let snap_on = settings.snap && !alt;

        // Wire ends that can be grabbed (wires moved by GM aren't drawn
        // where their numbers say, so they're left to the table).
        let ends: Vec<(usize, usize, [f64; 3], Pos2)> = model
            .wires
            .iter()
            .filter(|w| !w.moved)
            .flat_map(|w| (0..2).map(move |e| (w.line, e, w.end(e))))
            .filter_map(|(l, e, p)| proj.screen(p).map(|s| (l, e, p, s)))
            .collect();
        let nearest_end = |pos: Pos2, skip: &[(usize, usize)]| {
            ends.iter()
                .filter(|(l, e, _, _)| !skip.contains(&(*l, *e)))
                .map(|&(l, e, p, s)| (l, e, p, s.distance(pos)))
                .filter(|x| x.3 < GRAB_PX)
                .min_by(|a, b| a.3.total_cmp(&b.3))
        };
        // Curve ends snap too (they can't be dragged: change their numbers).
        // Exactly where the curve's own numbers put them (the engine's
        // segment ends are printed to 5 figures, too rough to snap to).
        let curve_ends: Vec<([f64; 3], Pos2)> = {
            let mut out = Vec::new();
            if let Ok(n) = &model.numeric {
                let lift = model.lift();
                for c in curves(&model.deck) {
                    let Some(spec) = model.deck.curve_spec(&c, &n.symbols) else {
                        continue;
                    };
                    let at = |deg: f64| {
                        let a = deg.to_radians();
                        spec.place([spec.radius * a.cos(), spec.radius * a.sin(), 0.0])
                    };
                    let ends = match c.kind {
                        CurveKind::Arc => vec![at(spec.from), at(spec.to)],
                        CurveKind::Circle => Vec::new(),
                        _ => vec![spec.place([spec.radius, 0.0, 0.0])],
                    };
                    for p in ends {
                        // Floating-point dust (1e-17) off, and the height lift on.
                        let p = [0, 1, 2].map(|k| {
                            let v = (p[k] * 1e9).round() / 1e9;
                            if k == 2 { v + lift } else { v }
                        });
                        if let Some(sp) = proj.screen(p) {
                            out.push((p, sp));
                        }
                    }
                }
            }
            // The middle of each face of a patch box (a patch centre when its
            // counts are odd: where a wire can join).
            for p in crate::views::patches::face_centres(model) {
                if let Some(sp) = proj.screen(p) {
                    out.push((p, sp));
                }
            }
            out
        };
        // The nearest wire or curve end to the mouse on the plane where
        // coordinate `axis` is `value` (ends elsewhere only look close).
        let nearest_on_plane = |pos: Pos2, axis: usize, value: f64, skip: &[(usize, usize)]| {
            let tol = 1e-6 * value.abs().max(1.0);
            ends.iter()
                .filter(|(l, e, _, _)| !skip.contains(&(*l, *e)))
                .map(|&(_, _, p, s)| (p, s))
                .chain(curve_ends.iter().copied())
                .filter(|(p, _)| (p[axis] - value).abs() <= tol)
                .map(|(p, s)| (p, s.distance(pos)))
                .filter(|x| x.1 < GRAB_PX)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|x| x.0)
        };
        // Any end under the mouse (it decides the plane of a first click).
        let any_end = |pos: Pos2| {
            ends.iter()
                .map(|&(_, _, p, s)| (p, s))
                .chain(curve_ends.iter().copied())
                .map(|(p, s)| (p, s.distance(pos)))
                .filter(|x| x.1 < GRAB_PX)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|x| x.0)
        };
        // A point on a plane, snapped to wire ends or the grid.
        let snap = |raw: [f64; 3], axis: usize, pos: Pos2, skip: &[(usize, usize)]| {
            if snap_on {
                if let Some(p) = nearest_on_plane(pos, axis, raw[axis], skip) {
                    return (p, true);
                }
                let g = grid.max(1e-12);
                let p = [0, 1, 2].map(|k| {
                    if k == axis {
                        raw[k]
                    } else {
                        (raw[k] / g).round() * g
                    }
                });
                (p, false)
            } else {
                (raw, false)
            }
        };

        let default_plane = {
            let axis = self.curve_axis.unwrap_or_else(|| proj.facing_axis());
            self.last_point.map_or_else(
                || {
                    let g = grid.max(1e-12);
                    (f64::from(centre[axis]) / g).round() * g
                },
                |p| p[axis],
            )
        };
        // The plane for a first click: the current one if an end on it is
        // under the mouse (ends at other depths can look just as close), else
        // the plane through another end under the mouse, else the current one.
        let plane_at = |hover: Option<Pos2>| {
            let axis = self.curve_axis.unwrap_or_else(|| proj.facing_axis());
            hover.map_or(default_plane, |pos| {
                if nearest_on_plane(pos, axis, default_plane, &[]).is_some() {
                    default_plane
                } else {
                    any_end(pos).map_or(default_plane, |e| e[axis])
                }
            })
        };
        let mut camera_free = true;
        let mut cursor: Option<[f64; 3]> = None;
        let mut edited: Option<bool> = None;
        let mut new_wire: Option<([f64; 3], [f64; 3])> = None;
        let mut new_curve: Option<CurveSpec> = None;
        let mut new_radials: Option<[f64; 3]> = None;
        let mut new_grid: Option<[f64; 3]> = None;
        let mut new_plate: Option<(straws_core::deck::plates::PlateKind, [f64; 3])> = None;
        let mut new_box: Option<[f64; 3]> = None;
        let mut new_dish: Option<[f64; 3]> = None;
        match self.tool {
            Tool::Select => {
                let press = ui.input(|i| i.pointer.press_origin());
                if response.drag_started_by(egui::PointerButton::Primary)
                    && let Some(at) = press
                    && let Some((_, _, p, _)) = nearest_end(at, &[])
                {
                    // Every end at this point moves together.
                    let joined: Vec<(usize, usize)> = model
                        .wires
                        .iter()
                        .filter(|w| !w.moved)
                        .flat_map(|w| (0..2).map(move |e| (w.line, e, w.end(e))))
                        .filter(|(_, _, q)| (0..3).all(|k| (q[k] - p[k]).abs() < 1e-9))
                        .map(|(l, e, _)| (l, e))
                        .collect();
                    let axis = proj.facing_axis();
                    self.drag = Some(EndDrag {
                        ends: joined,
                        axis,
                        value: p[axis],
                    });
                }
                if let Some(drag) = &self.drag {
                    camera_free = false;
                    if let Some(pos) = response.interact_pointer_pos().or(hover)
                        && let Some(raw) = proj.on_plane(pos, drag.axis, drag.value)
                    {
                        let (p, _) = snap(raw, drag.axis, pos, &drag.ends);
                        cursor = Some(p);
                        if let Some(m) = &mut data.model {
                            let before = m.deck.clone();
                            let mut moved = false;
                            for &(line, end) in &drag.ends {
                                if let Some(w) = m.wires.iter().find(|w| w.line == line) {
                                    let fields = moved_end(w, end, p);
                                    if fields != w.fields {
                                        m.deck.set_wire_fields(line, &fields);
                                        moved = true;
                                    }
                                }
                            }
                            // A position that would break the deck (e.g. both
                            // ends of a wire together) is skipped.
                            if moved && m.keep_if_valid(before, &data.overrides).is_ok() {
                                edited = Some(false);
                            }
                        }
                    }
                    if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                        self.drag = None;
                        edited = Some(true);
                    }
                } else if (response.clicked() || response.secondary_clicked())
                    && let Some(pos) = hover
                {
                    // Select the wire under the mouse. A right-click keeps
                    // the selection if it's on a selected wire (for the menu).
                    let menu = response.secondary_clicked();
                    let picked = model
                        .wires
                        .iter()
                        .filter(|w| !w.moved)
                        .filter_map(|w| {
                            let (a, b) = (proj.screen(w.a)?, proj.screen(w.b)?);
                            Some((w.line, distance_to_segment(pos, a, b)))
                        })
                        .filter(|x| x.1 < GRAB_PX)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|x| x.0);
                    let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                    // Not a straight wire: perhaps a curve.
                    let curve = if picked.is_none() {
                        let tags: Vec<(i64, String)> = curves(&model.deck)
                            .into_iter()
                            .map(|c| (c.tag, c.name))
                            .collect();
                        model
                            .visible_segments()
                            .filter_map(|s| {
                                let name = &tags.iter().find(|t| t.0 == i64::from(s.tag))?.1;
                                let (a, b) = s.ends();
                                let (a, b) = (proj.screen(a)?, proj.screen(b)?);
                                Some((name.clone(), distance_to_segment(pos, a, b)))
                            })
                            .filter(|x| x.1 < GRAB_PX)
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|x| x.0)
                    } else {
                        None
                    };
                    // A radial selects its whole set (and opens its panel).
                    let set = picked.and_then(|l| {
                        radial_sets(&model.deck)
                            .into_iter()
                            .find(|s| s.lines.contains(&l))
                            .map(|s| (s.name, s.lines))
                            .or_else(|| {
                                straws_core::deck::grids::grid_sets(&model.deck)
                                    .into_iter()
                                    .find(|g| g.lines.contains(&l))
                                    .map(|g| (g.name, g.lines))
                            })
                            .or_else(|| {
                                straws_core::deck::plates::plate_sets(&model.deck)
                                    .into_iter()
                                    .find(|p| p.lines.contains(&l))
                                    .map(|p| (p.name, p.lines))
                            })
                            .or_else(|| {
                                straws_core::deck::reflectors::reflector_sets(&model.deck)
                                    .into_iter()
                                    .find(|r| r.lines.contains(&l))
                                    .map(|r| (r.name, r.lines))
                            })
                    });
                    // Radials on or under the ground are drawn, not wires.
                    let estimated = if picked.is_none() && curve.is_none() {
                        buried_lines(model)
                            .into_iter()
                            .filter_map(|(name, a, b)| {
                                let (a, b) = (proj.screen(a)?, proj.screen(b)?);
                                Some((name, distance_to_segment(pos, a, b)))
                            })
                            .filter(|x| x.1 < GRAB_PX)
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|x| x.0)
                    } else {
                        None
                    };
                    // A surface patch selects its box (nearest patch centre
                    // whose square is under the mouse).
                    let patch_box = if picked.is_none() && curve.is_none() && estimated.is_none() {
                        model
                            .patches
                            .iter()
                            .filter_map(|p| {
                                let corners: Vec<Pos2> = p
                                    .corners()
                                    .iter()
                                    .map(|c| proj.screen(*c))
                                    .collect::<Option<_>>()?;
                                inside(pos, &corners).then(|| (p, proj.depth(p.centre)))
                            })
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .and_then(|(p, _)| crate::views::patches::box_of(model, p))
                    } else {
                        None
                    };
                    if !menu {
                        data.selected_curve = curve
                            .clone()
                            .or(set.as_ref().map(|s| s.0.clone()))
                            .or(estimated.clone())
                            .or(patch_box.clone());
                    }
                    if let Some(name) = &patch_box
                        && !menu
                    {
                        data.selection = straws_core::deck::patches::box_sets(&model.deck)
                            .into_iter()
                            .find(|b| &b.name == name)
                            .map(|b| b.lines)
                            .unwrap_or_default();
                    }
                    if estimated.is_some() && !menu {
                        data.selection.clear();
                    }
                    if let Some(s) = &set
                        && !menu
                        && !add
                    {
                        data.selection = s.1.clone();
                    } else {
                        match (picked, add) {
                            (None, _) if curve.is_some() => data.selection.clear(),
                            (Some(l), _) if menu && data.selection.contains(&l) => {}
                            (None, _) if menu => {}
                            (Some(l), true) => {
                                if let Some(k) = data.selection.iter().position(|&s| s == l) {
                                    data.selection.remove(k);
                                } else {
                                    data.selection.push(l);
                                }
                            }
                            (Some(l), false) => data.selection = vec![l],
                            (None, false) => data.selection.clear(),
                            (None, true) => {}
                        }
                    }
                }
            }
            Tool::Draw if self.shape != Shape::Wire => {
                let axis = match (self.curve_points.first(), self.curve_axis) {
                    (Some(_), Some(a)) => a,
                    _ => proj.facing_axis(),
                };
                let value = match self.curve_points.first() {
                    Some(p) => p[axis],
                    None => plane_at(hover),
                };
                self.plane_value = Some(value);
                let raw = hover.and_then(|pos| proj.on_plane(pos, axis, value));
                if let (Some(pos), Some(raw)) = (hover, raw) {
                    cursor = Some(snap(raw, axis, pos, &[]).0);
                }
                // An arc's third click only sets the bulge: the sweep snaps.
                let bulge = self.shape.is_arc() && self.curve_points.len() == 2;
                let point = if bulge { raw } else { cursor };
                if response.clicked()
                    && let Some(p) = point
                    && (bulge || self.curve_points.last() != Some(&p))
                {
                    if self.curve_points.is_empty() {
                        self.curve_axis = Some(axis);
                    }
                    self.curve_points.push(p);
                    if !bulge {
                        self.last_point = Some(p);
                    }
                    if matches!(
                        self.shape,
                        Shape::Radials
                            | Shape::Grid
                            | Shape::Plate
                            | Shape::Corner
                            | Shape::Dish
                            | Shape::PatchBox
                    ) {
                        use straws_core::deck::plates::PlateKind;
                        match self.shape {
                            Shape::Grid => new_grid = Some(p),
                            Shape::Plate => new_plate = Some((PlateKind::Flat, p)),
                            Shape::Corner => new_plate = Some((PlateKind::Corner, p)),
                            Shape::Dish => new_dish = Some(p),
                            Shape::PatchBox => new_box = Some(p),
                            _ => new_radials = Some(p),
                        }
                        self.curve_points.clear();
                        self.curve_axis = None;
                    } else if self.curve_points.len() == self.shape.clicks() {
                        if let Some(mut spec) = self.shape.curve(&self.curve_points, axis, snap_on)
                        {
                            spec.name = model.deck.curve_name(spec.kind);
                            spec.wire_radius = data
                                .selection
                                .iter()
                                .find_map(|l| model.wires.iter().find(|w| w.line == *l))
                                .or(model.wires.last())
                                .map_or(0.001, |w| w.radius);
                            spec.segments = matched_segments(&spec, model, data.max_mhz);
                            new_curve = Some(spec);
                        }
                        self.curve_points.clear();
                        self.curve_axis = None;
                    }
                }
                if response.secondary_clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.curve_points.clear();
                    self.curve_axis = None;
                }
            }
            Tool::Draw => {
                let axis = proj.facing_axis();
                let value = match self.draw_from {
                    Some(p) => p[axis],
                    None => plane_at(hover),
                };
                self.plane_value = Some(value);
                if let Some(pos) = hover
                    && let Some(raw) = proj.on_plane(pos, axis, value)
                {
                    let (p, on_end) = snap(raw, axis, pos, &[]);
                    let shift = ui.input(|i| i.modifiers.shift);
                    cursor = Some(match self.draw_from {
                        Some(a) if shift && !on_end => {
                            angle_step(a, raw, axis, snap_on.then_some(grid))
                        }
                        _ => p,
                    });
                }
                if response.clicked()
                    && let Some(p) = cursor
                {
                    self.last_point = Some(p);
                    match self.draw_from {
                        Some(a) if a != p => {
                            new_wire = Some((a, p));
                            self.draw_from = Some(p);
                        }
                        Some(_) => {}
                        None => self.draw_from = Some(p),
                    }
                }
                if response.secondary_clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.draw_from = None;
                }
            }
        }
        if camera_free {
            camera.handle_input(ui, &response);
            if response.double_clicked() && self.tool == Tool::Select {
                *camera = home;
            }
        }
        render_3d(ui, gpu, &mut self.viewport, camera, rect, &scenes);

        if let Some((a, b)) = new_wire
            && let Some(m) = &mut data.model
        {
            let length = (0..3).map(|k| (b[k] - a[k]).powi(2)).sum::<f64>().sqrt();
            let radius = data
                .selection
                .iter()
                .find_map(|l| m.wires.iter().find(|w| w.line == *l))
                .or(m.wires.last())
                .map_or(0.001, |w| w.radius);
            let tag = m.next_tag();
            // Points on screen include the height card's lift.
            let lift = m.lift();
            let (a, b) = ([a[0], a[1], a[2] - lift], [b[0], b[1], b[2] - lift]);
            let line = m
                .deck
                .add_wire(tag, auto_segments(length, data.max_mhz), a, b, radius);
            m.refresh(&data.overrides);
            data.selection = vec![line];
            edited = Some(true);
        }
        if let Some(spec) = new_curve
            && let Err(why) = crate::views::curves::add(data, &spec)
        {
            data.message(
                crate::app::Level::Warning,
                format!("That curve wasn't added: {why}."),
            );
        }
        if let Some(hub) = new_radials
            && let Err(why) = crate::views::radials::add(data, hub)
        {
            data.message(
                crate::app::Level::Warning,
                format!("The radials weren't added: {why}."),
            );
        }
        if let Some(at) = new_dish
            && let Err(why) = crate::views::reflectors::add(data, at)
        {
            data.message(
                crate::app::Level::Warning,
                format!("The dish wasn't added: {why}."),
            );
        }
        if let Some(at) = new_box
            && let Err(why) = crate::views::patches::add(data, at)
        {
            data.message(
                crate::app::Level::Warning,
                format!("The patch box wasn't added: {why}."),
            );
        }
        if let Some((kind, at)) = new_plate
            && let Err(why) = crate::views::plates::add(data, kind, at)
        {
            data.message(
                crate::app::Level::Warning,
                format!("The reflector wasn't added: {why}."),
            );
        }
        if let Some(centre) = new_grid
            && let Err(why) = crate::views::grids::add(data, centre)
        {
            data.message(
                crate::app::Level::Warning,
                format!("The grid wasn't added: {why}."),
            );
        }
        if let Some(finished) = edited {
            data.edit_event = Some(data.edit_event.unwrap_or(false) || finished);
        }

        if self.tool == Tool::Select
            && let Some(m) = &data.model
        {
            let mut request = None;
            let (mut feed, mut delete, mut add_source) = (None, false, None);
            response.context_menu(|ui| {
                let one = m
                    .wires
                    .iter()
                    .find(|w| data.selection.len() == 1 && w.line == data.selection[0]);
                if ui
                    .add_enabled(one.is_some(), egui::Button::new("Feed this wire"))
                    .on_disabled_hover_text("Select one wire")
                    .on_hover_text("Put the source on its centre segment")
                    .clicked()
                {
                    feed = one.map(|w| (w.tag, w.segments));
                    ui.close();
                }
                if ui
                    .add_enabled(one.is_some(), egui::Button::new("Add another source here"))
                    .on_disabled_hover_text("Select one wire")
                    .on_hover_text(
                        "A second (third...) source on its centre segment, keeping the others: \
                         set its amplitude and phase in the Feeds view",
                    )
                    .clicked()
                {
                    add_source = one.map(|w| (w.tag, w.segments));
                    ui.close();
                }
                if ui
                    .add_enabled(!data.selection.is_empty(), egui::Button::new("Delete"))
                    .on_hover_text("Delete the selected wires (Del)")
                    .clicked()
                {
                    delete = true;
                    ui.close();
                }
                ui.separator();
                ui.label(RichText::new("Make a variable").strong());
                request = newvar::selection_menu(ui, m, &data.selection, data.mhz, settings);
            });
            if request.is_some() {
                data.new_variable = request;
            }
            if let Some((tag, segments)) = feed
                && let Some(m) = &mut data.model
            {
                let before = m.deck.clone();
                m.deck.feed_wire(tag, segments);
                if m.keep_if_valid(before, &data.overrides).is_ok() {
                    data.edit_event = Some(true);
                }
            }
            if let Some((tag, segments)) = add_source
                && let Some(m) = &mut data.model
            {
                let before = m.deck.clone();
                m.deck.add_source(
                    tag,
                    (segments + 1) / 2,
                    straws_core::deck::sources::SourceKind::Voltage,
                    straws_core::Complex64::new(1.0, 0.0),
                );
                if m.keep_if_valid(before, &data.overrides).is_ok() {
                    data.edit_event = Some(true);
                    data.request = Some(crate::app::Request::ShowTab(crate::app::Tab::Feeds));
                }
            }
            if delete && let Some(m) = &mut data.model {
                let before = m.deck.clone();
                m.deck.remove_lines(&data.selection);
                if m.keep_if_valid(before, &data.overrides).is_ok() {
                    data.selection.clear();
                    data.edit_event = Some(true);
                }
            }
        }

        // Overlays: selected wires, grabbable ends, the wire being drawn.
        let Some(model) = &data.model else { return };
        let painter = ui.painter_at(rect);
        let accent = palette.selected;
        for w in model
            .wires
            .iter()
            .filter(|w| data.selection.contains(&w.line))
        {
            if let (Some(a), Some(b)) = (proj.screen(w.a), proj.screen(w.b)) {
                painter.line_segment([a, b], Stroke::new(4.0, accent.gamma_multiply(0.8)));
                for p in [a, b] {
                    painter.circle_filled(p, 4.5, accent);
                }
            }
        }
        if self.tool == Tool::Select
            && self.drag.is_none()
            && let Some(pos) = hover
            && let Some((_, _, _, _)) = nearest_end(pos, &[])
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            if let Some((_, _, p, _)) = nearest_end(pos, &[])
                && let Some(s) = proj.screen(p)
            {
                painter.circle_stroke(s, 7.0, Stroke::new(2.0, accent));
            }
        }
        // With several sources, each one's amplitude and phase by its feed.
        if let Ok(n) = &model.numeric {
            let list = straws_core::deck::sources::sources(&model.deck, n);
            if list.len() > 1 {
                for s in &list {
                    let (Some(v), Some(w)) = (s.value, model.wires.iter().find(|w| w.tag == s.tag))
                    else {
                        continue;
                    };
                    let f = (s.segment as f64 - 0.5) / w.segments.max(1) as f64;
                    let p = [0, 1, 2].map(|k| w.a[k] + (w.b[k] - w.a[k]) * f);
                    if let Some(at) = proj.screen(p) {
                        painter.text(
                            at + vec2(8.0, -8.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!(
                                "{:.2} {}, {:.0}°",
                                v.norm(),
                                s.kind.unit(),
                                v.arg().to_degrees()
                            ),
                            egui::FontId::proportional(12.0),
                            palette.feed,
                        );
                    }
                }
            }
        }
        // The selected patch box's patches.
        if let Some(name) = &data.selected_curve
            && crate::views::patches::is_box(data, name)
        {
            for p in &model.patches {
                if crate::views::patches::box_of(model, p).as_deref() != Some(name.as_str()) {
                    continue;
                }
                let corners: Option<Vec<Pos2>> =
                    p.corners().iter().map(|c| proj.screen(*c)).collect();
                if let Some(mut c) = corners {
                    c.push(c[0]);
                    painter.add(egui::Shape::line(c, Stroke::new(1.5, accent)));
                }
            }
        }
        // A selected reflector's focus.
        if let Some(name) = &data.selected_curve
            && let Some(f) = crate::views::reflectors::focus(model, name)
            && let Some(s) = proj.screen(f)
        {
            painter.circle_stroke(s, 6.0, Stroke::new(2.0, accent));
            painter.line_segment(
                [s - vec2(9.0, 0.0), s + vec2(9.0, 0.0)],
                Stroke::new(1.5, accent),
            );
            painter.line_segment(
                [s - vec2(0.0, 9.0), s + vec2(0.0, 9.0)],
                Stroke::new(1.5, accent),
            );
        }
        // Radials on or under the ground: dashed, where they lie.
        for (name, a, b) in buried_lines(model) {
            let selected = data.selected_curve.as_deref() == Some(name.as_str());
            let colour = if selected {
                accent
            } else {
                Color32::from_rgb(176, 132, 84)
            };
            if let (Some(a), Some(b)) = (proj.screen(a), proj.screen(b)) {
                painter.extend(egui::Shape::dashed_line(
                    &[a, b],
                    Stroke::new(if selected { 3.0 } else { 1.5 }, colour),
                    6.0,
                    4.0,
                ));
            }
        }
        if let Some(name) = &data.selected_curve
            && let Some(c) = curves(&model.deck).into_iter().find(|c| &c.name == name)
        {
            for s in model
                .visible_segments()
                .filter(|s| i64::from(s.tag) == c.tag)
            {
                let (a, b) = s.ends();
                if let (Some(a), Some(b)) = (proj.screen(a), proj.screen(b)) {
                    painter.line_segment([a, b], Stroke::new(4.0, accent.gamma_multiply(0.8)));
                }
            }
        }
        if self.tool == Tool::Draw
            && self.shape != Shape::Wire
            && let (Some(&first), Some(c)) = (self.curve_points.first(), cursor)
        {
            let axis = self.curve_axis.unwrap_or_else(|| proj.facing_axis());
            let mut pts = self.curve_points.clone();
            // The bulge follows the mouse freely; the ends snap.
            let bulge = self.shape.is_arc() && pts.len() == 2;
            let mouse = if bulge {
                hover
                    .and_then(|pos| proj.on_plane(pos, axis, first[axis]))
                    .unwrap_or(c)
            } else {
                c
            };
            pts.push(mouse);
            let spec = if self.shape.is_arc() && pts.len() == 2 {
                None
            } else {
                self.shape.curve(&pts, axis, snap_on)
            };
            match spec {
                Some(spec) => {
                    let sample = spec.preview(96);
                    let screen: Vec<Pos2> = sample.iter().filter_map(|&p| proj.screen(p)).collect();
                    for w in screen.windows(2) {
                        painter.line_segment([w[0], w[1]], Stroke::new(2.0, accent));
                    }
                    let mut text = format!(
                        "radius {} {}",
                        crate::views::wires::tidy(
                            unit.from_metres(spec.radius, mhz),
                            unit.decimals()
                        ),
                        unit.symbol()
                    );
                    if spec.kind == CurveKind::Arc {
                        text.push_str(&format!(" · {:.0}°", spec.to - spec.from));
                    }
                    if let Some(sp) = proj.screen(mouse) {
                        painter.text(
                            sp + vec2(14.0, 10.0),
                            egui::Align2::LEFT_TOP,
                            text,
                            egui::FontId::proportional(13.0),
                            ui.visuals().strong_text_color(),
                        );
                    }
                }
                None => {
                    // An arc's chord while its second end is placed.
                    if let (Some(a), Some(b)) = (proj.screen(first), proj.screen(c)) {
                        painter.line_segment([a, b], Stroke::new(1.0, accent.gamma_multiply(0.6)));
                    }
                }
            }
            for p in &self.curve_points {
                if let Some(sp) = proj.screen(*p) {
                    painter.circle_filled(sp, 4.0, accent);
                }
            }
        }
        if let (Some(a), Some(c)) = (self.draw_from, cursor)
            && let (Some(sa), Some(sc)) = (proj.screen(a), proj.screen(c))
        {
            painter.line_segment([sa, sc], Stroke::new(2.5, accent));
            painter.circle_filled(sa, 4.0, accent);
        }
        if let Some(c) = cursor
            && let Some(s) = proj.screen(c)
        {
            painter.circle_stroke(s, 5.0, Stroke::new(1.5, palette.feed));
        }

        let mut lines = vec![format!(
            "{} · {} segments · {}",
            model.name,
            model.visible_segments().count(),
            if data.over_ground {
                "over ground"
            } else {
                "free space"
            }
        )];
        if let Some(p) = data.shown_point()
            && let Some(input) = p.feed(data.feed)
        {
            let z = input.impedance;
            lines.push(format!(
                "{:.4} MHz · Z = {:.2} {} j{:.2} Ω{}",
                p.mhz,
                z.re,
                if z.im < 0.0 { "−" } else { "+" },
                z.im.abs(),
                if p.estimated {
                    " (estimate: counterpoise on or under the ground)"
                } else {
                    ""
                }
            ));
        }
        let show = |v: f64| show_length(v, unit, mhz);
        if let Some(c) = cursor {
            let mut text = format!(
                "x {} · y {} · z {} {}",
                show(c[0]),
                show(c[1]),
                show(c[2]),
                unit.symbol()
            );
            if let Some(a) = self.draw_from {
                let l = (0..3).map(|k| (c[k] - a[k]).powi(2)).sum::<f64>().sqrt();
                text.push_str(&format!(" · length {} {}", show(l), unit.symbol()));
            }
            lines.push(text);
        }
        if let Some(c) = data
            .wire_colours
            .as_ref()
            .filter(|c| c.generation == data.model_generation)
        {
            lines.push(format!("Colours: {} (yellow most; Checks view)", c.label));
        } else if settings.show_currents
            && let Some(p) = data.shown_point()
            && !p.currents.is_empty()
        {
            let scale = p.current_scale(100.0);
            let largest = p
                .currents
                .iter()
                .map(|c| c.current.norm())
                .fold(0.0, f64::max);
            lines.push(format!(
                "Currents: yellow is strongest ({:.3} A RMS with 100 W in)",
                largest * scale / std::f64::consts::SQRT_2
            ));
            // The segment under the mouse.
            if let Some(pos) = hover
                && let Some((s, _)) = model
                    .visible_segments()
                    .filter_map(|s| proj.screen(s.centre).map(|q| (s, q.distance(pos))))
                    .filter(|x| x.1 < 12.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                && let Some(c) = p.currents.iter().find(|c| c.segment == s.number)
            {
                lines.push(format!(
                    "Segment {} (wire {}): {:.3} A RMS, phase {:.0}°",
                    s.number,
                    s.tag,
                    c.current.norm() * scale / std::f64::consts::SQRT_2,
                    c.current.arg().to_degrees()
                ));
            }
        }
        if let Some(drag) = &self.drag {
            let axis = drag.axis;
            let locked = drag.ends.iter().any(|&(line, end)| {
                model.wires.iter().any(|w| {
                    w.line == line && (0..3).any(|k| k != axis && !w.is_number(2 + 3 * end + k))
                })
            });
            if locked {
                lines.push(
                    "Coordinates written as formulas stay put: change their variables, or type numbers in the Wires view.".into(),
                );
            }
        }
        if self.tool == Tool::Draw {
            let plane = ["YZ", "XZ", "XY"][proj.facing_axis()];
            let what = match (self.shape, self.curve_points.len()) {
                (Shape::Wire, _) => {
                    "Click to place ends (Shift: 15° steps); Esc or right-click to stop."
                }
                (Shape::Arc | Shape::Semicircle, 0) => "Click where it starts (wire ends snap).",
                (Shape::Arc | Shape::Semicircle, 1) => "Click where it ends.",
                (Shape::Arc, _) => {
                    "Move the mouse to bend it (snaps to 15° steps), then click; Esc cancels."
                }
                (Shape::Semicircle, _) => "Click the side it bulges to; Esc cancels.",
                (Shape::Dish, _) => {
                    "Click the dish's vertex (the middle of its back); it faces the fed wire. Then pick its outline and profile in its panel."
                }
                (Shape::PatchBox, _) => {
                    "Click the middle of the box's bottom face. Then adjust its panel."
                }
                (Shape::Plate, _) => {
                    "Click the plate's centre; it faces the fed wire. Then adjust its panel."
                }
                (Shape::Corner, _) => {
                    "Click the middle of the corner's fold (its apex); it opens towards the fed wire."
                }
                (Shape::Grid, _) => {
                    "Click the middle of the grid, e.g. a vertical's base (wire ends snap); then adjust its panel."
                }
                (Shape::Radials, _) => {
                    "Click the hub, e.g. a vertical's base (wire ends snap); then adjust the set's panel."
                }
                (_, 0) => "Click the centre.",
                (_, _) => "Click to set the radius; Esc cancels.",
            };
            let at = match (self.plane_value, proj.facing_axis()) {
                (Some(v), k) => format!(
                    " at {} = {} {}",
                    ["x", "y", "z"][k],
                    show_length(v, unit, mhz),
                    unit.symbol()
                ),
                (None, _) => String::new(),
            };
            lines.push(format!(
                "Drawing on the {plane} plane{at} (through the last point placed, or a wire end under the mouse). {what}"
            ));
        }
        if let Err(e) = &model.numeric {
            lines.push(format!(
                "The deck can't be read ({e}); showing the last good wires. Ctrl+Z undoes."
            ));
        } else if model.geometry_error.is_some() {
            lines.push(
                "The engine couldn't lay out this geometry: see Messages. Ctrl+Z undoes.".into(),
            );
        }
        if model.wires.iter().any(|w| w.moved) && self.tool == Tool::Select {
            lines.push("Some wires are moved by GM cards: edit those in the Wires view.".into());
        }
        corner_text(ui, rect, &lines);

        // The selected curve's panel, while selecting (not over a drawing).
        if data.selected_curve.is_some() && self.tool == Tool::Select {
            if self.tool == Tool::Select && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                data.selected_curve = None;
            }
            egui::Area::new(egui::Id::new("curve panel"))
                .fixed_pos(rect.right_top() + vec2(-290.0, 8.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(270.0);
                        let name = data.selected_curve.clone().unwrap_or_default();
                        if crate::views::radials::is_set(data, &name) {
                            crate::views::radials::panel(ui, data, settings);
                        } else if crate::views::grids::is_grid(data, &name) {
                            crate::views::grids::panel(ui, data, settings);
                        } else if crate::views::plates::is_plate(data, &name) {
                            crate::views::plates::panel(ui, data, settings);
                        } else if crate::views::patches::is_box(data, &name) {
                            crate::views::patches::panel(ui, data, settings);
                        } else if crate::views::reflectors::is_reflector(data, &name) {
                            crate::views::reflectors::panel(ui, data, settings, &mut self.formula);
                        } else {
                            crate::views::curves::panel(ui, data, settings);
                        }
                    });
                });
        }
    }
}

/// Segments for a new curve: the usual count, but where it joins wires,
/// close to the length of their segments there (NEC wants segments either
/// side of a junction within about 2× of each other).
fn matched_segments(spec: &CurveSpec, model: &crate::model::LoadedModel, max_mhz: f64) -> i64 {
    let usual = curve_segments(spec, max_mhz);
    let ends = spec.preview(2);
    let (a, b) = (ends[0], ends[2]);
    // The engine prints segment ends to 5 figures: compare loosely.
    let tol = 0.02 * spec.radius;
    let near = |p: [f64; 3], q: [f64; 3]| (0..3).all(|k| (p[k] - q[k]).abs() < tol);
    let joined: Vec<f64> = model
        .visible_segments()
        .filter(|s| {
            let (p, q) = s.ends();
            near(p, a) || near(q, a) || near(p, b) || near(q, b)
        })
        .map(|s| s.length)
        .collect();
    if joined.is_empty() {
        return usual;
    }
    let mean = joined.iter().sum::<f64>() / joined.len() as f64;
    let n = (spec.wire_length() / mean).round() as i64;
    // At least two, so it's still a bend; no more than the usual count.
    // A curve, not a corner: at least one segment per 60° of bend.
    let fewest = if spec.kind == CurveKind::Arc {
        ((spec.to - spec.from).abs() / 60.0).ceil().max(2.0) as i64
    } else {
        2
    };
    n.clamp(fewest, usual.max(fewest))
}

/// `to` moved round `from` to the nearest 15° step on the plane facing along
/// `axis`, its length rounded to the grid (if any).
fn angle_step(from: [f64; 3], to: [f64; 3], axis: usize, grid: Option<f64>) -> [f64; 3] {
    let (i, j) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let (di, dj) = (to[i] - from[i], to[j] - from[j]);
    let angle = (dj.atan2(di).to_degrees() / 15.0).round() * 15.0;
    let mut len = di.hypot(dj);
    if let Some(g) = grid.filter(|g| *g > 0.0) {
        len = (len / g).round() * g;
    }
    let mut p = from;
    p[i] += len * angle.to_radians().cos();
    p[j] += len * angle.to_radians().sin();
    p
}

/// What the model view shows with nothing open: ways to start.
fn welcome(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    use crate::app::{Request, Tab};
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.25).max(20.0));
        ui.heading("Start an antenna");
        ui.add_space(12.0);
        let size = egui::vec2(260.0, 32.0);
        if ui
            .add_sized(size, egui::Button::new("Start from a template…"))
            .on_hover_text("Dipole, Yagi, Moxon, quad, LPDA, verticals, helix, J-pole and more")
            .clicked()
        {
            data.request = Some(Request::ShowTab(Tab::Templates));
        }
        if ui
            .add_sized(size, egui::Button::new("Draw a new antenna"))
            .on_hover_text("An empty model: choose Draw and click to place wire ends")
            .clicked()
        {
            data.request = Some(Request::New);
        }
        if ui
            .add_sized(size, egui::Button::new("Open a .nec deck…"))
            .on_hover_text("Including 4nec2's decks with SY variables")
            .clicked()
        {
            data.request = Some(Request::OpenDialog);
        }
        if !settings.recent.is_empty() {
            ui.add_space(14.0);
            ui.label(egui::RichText::new("Recent").weak());
            for p in settings.recent.iter().take(6) {
                let name = p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                if ui
                    .link(name)
                    .on_hover_text(p.display().to_string())
                    .clicked()
                {
                    data.request = Some(Request::Open(p.clone()));
                }
            }
        }
    });
}

fn gain_scale(grid: &PatternGrid, settings: &Settings) -> GainScale {
    GainScale {
        max_dbi: grid.max(settings.component()).0,
        range_db: settings.range_db,
    }
}

#[derive(Default)]
pub struct PatternView {
    camera: Option<OrbitCamera>,
    viewport: Option<Viewport>,
    mesh: Option<(Scene, SceneBuffers)>,
    axes: Option<SceneBuffers>,
    key: Option<PatternKey>,
}

impl PatternView {
    pub fn invalidate(&mut self) {
        self.key = None;
        self.axes = None;
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        gpu: Option<&Gpu>,
        data: &Data,
        settings: &mut Settings,
    ) {
        let Some(gpu) = gpu else {
            return empty_note(ui, "3D views need a GPU renderer, which isn't available.");
        };
        let Some(point) = data.shown_point() else {
            let note = data.waiting_note(
                "Open a .nec deck to see its radiation pattern.",
                "Calculating the pattern…",
            );
            return empty_note(ui, note);
        };
        let Some(grid) = &point.pattern else {
            return empty_note(ui, "No pattern at this frequency.");
        };
        let dark = ui.visuals().dark_mode;
        let key = pattern_key(data, settings, dark);
        if key != self.key {
            let scene = scene::pattern_scene(
                grid,
                settings.component(),
                gain_scale(grid, settings),
                Vec3::ZERO,
                1.0,
                255,
            );
            let buffers = gpu.upload(&scene);
            self.mesh = Some((scene, buffers));
            self.key = key;
        }
        let axes = self.axes.get_or_insert_with(|| {
            let palette = Palette::for_theme(dark);
            let mut s = Scene::default();
            for (k, dir) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
                s.lines.push(crate::render3d::LineInstance::new(
                    Vec3::ZERO,
                    dir * 1.25,
                    2.0,
                    palette.axes[k],
                ));
            }
            gpu.upload(&s)
        });
        let home = OrbitCamera::looking_at([0.0, 0.0, 0.0], 1.15);
        let camera = self.camera.get_or_insert(home);

        ui.horizontal(|ui| {
            view_buttons(ui, camera, home);
            ui.separator();
            ui.label("Range");
            ui.add(
                crate::views::num(&mut settings.range_db)
                    .range(10.0..=80.0)
                    .suffix(" dB"),
            )
            .on_hover_text("How far below the peak the pattern reaches the centre");
        });

        let mut scenes: Vec<&SceneBuffers> = vec![axes];
        if let Some((_, b)) = &self.mesh {
            scenes.push(b);
        }
        let cam_copy = *camera;
        let (rect, response) = show_3d(ui, gpu, &mut self.viewport, camera, home, &scenes);

        let component = settings.component();
        let (max, theta, phi) = grid.max(component);
        let mut lines = vec![
            format!("{:.4} MHz · {}", point.mhz, component.label()),
            format!(
                "Peak {max:.2} dBi at elevation {:.0}°, azimuth {phi:.0}°{}",
                90.0 - theta,
                if point.estimated { " (estimate)" } else { "" }
            ),
        ];
        // A component with next to no field (horizontal on a vertical
        // antenna, say) draws a meaningless shape: say so.
        let (total, _, _) = grid.max(straws_core::analysis::pattern::Component::Total);
        if max < total - 30.0 {
            lines.push(format!(
                "Almost no {} field here ({:.0} dB below the total): choose Total, top right, to see the pattern",
                component.label().to_lowercase(),
                total - max
            ));
        }
        // The receiving directivity factor (for receive antennas).
        if let Some(rdf) = straws_core::analysis::checks::rdf(grid) {
            lines.push(format!(
                "RDF {rdf:.2} dB (peak against the average over {})",
                if (grid.thetas.last().copied().unwrap_or(0.0) - 90.0).abs() < 1e-6 {
                    "the upper half"
                } else {
                    "all directions"
                }
            ));
        }
        if let Some(pos) = response.hover_pos()
            && let Some((scene, _)) = &self.mesh
        {
            let ndc = vec2(
                (pos.x - rect.left()) / rect.width() * 2.0 - 1.0,
                1.0 - (pos.y - rect.top()) / rect.height() * 2.0,
            );
            let (origin, dir) = cam_copy.ray(ndc, rect.width() / rect.height().max(1.0));
            if let Some((t, p)) = scene::pick_pattern(scene, Vec3::ZERO, origin, dir) {
                let g = grid.sample(component, t, p);
                lines.push(format!(
                    "Cursor: {g:.2} dBi at elevation {:.0}°, azimuth {p:.0}°",
                    90.0 - t
                ));
            }
        }
        corner_text(ui, rect, &lines);
        colour_bar(ui, rect, max, settings.range_db);
    }
}

/// The gain colour key down the right-hand side of the pattern view.
fn colour_bar(ui: &egui::Ui, rect: Rect, max_dbi: f64, range_db: f64) {
    let painter = ui.painter_at(rect);
    let h = (rect.height() * 0.5).clamp(80.0, 320.0);
    let bar = Rect::from_min_size(
        pos2(rect.right() - 26.0, rect.center().y - h / 2.0),
        vec2(12.0, h),
    );
    let steps = 48;
    for k in 0..steps {
        let t0 = k as f32 / steps as f32;
        let t1 = (k + 1) as f32 / steps as f32;
        let r = Rect::from_min_max(
            pos2(bar.left(), bar.bottom() - t1 * h),
            pos2(bar.right(), bar.bottom() - t0 * h + 0.5),
        );
        painter.rect_filled(r, 0.0, viridis((t0 + t1) / 2.0));
    }
    let font = egui::FontId::proportional(11.0);
    let colour = ui.visuals().text_color();
    let step = if range_db > 40.0 { 20.0 } else { 10.0 };
    let mut db = 0.0;
    while db <= range_db + 1e-9 {
        let y = bar.top() + (db / range_db) as f32 * h;
        painter.text(
            pos2(bar.left() - 4.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.1}", max_dbi - db),
            font.clone(),
            colour,
        );
        db += step;
    }
    painter.text(
        pos2(bar.center().x, bar.top() - 6.0),
        egui::Align2::CENTER_BOTTOM,
        "dBi",
        font,
        colour,
    );
}

/// What the Draw tool draws.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Wire,
    Arc,
    Semicircle,
    Circle,
    Helix,
    Spiral,
    Radials,
    Grid,
    Plate,
    Corner,
    Dish,
    PatchBox,
}

impl Shape {
    const ALL: [Shape; 12] = [
        Shape::Wire,
        Shape::Arc,
        Shape::Semicircle,
        Shape::Circle,
        Shape::Helix,
        Shape::Spiral,
        Shape::Radials,
        Shape::Grid,
        Shape::Plate,
        Shape::Corner,
        Shape::Dish,
        Shape::PatchBox,
    ];

    fn label(self) -> &'static str {
        match self {
            Shape::Wire => "Wire",
            Shape::Arc => "Arc",
            Shape::Semicircle => "Semicircle",
            Shape::Circle => "Circle",
            Shape::Helix => "Helix",
            Shape::Spiral => "Spiral",
            Shape::Radials => "Radials",
            Shape::Grid => "Grid",
            Shape::Plate => "Plate",
            Shape::Corner => "Corner reflector",
            Shape::Dish => "Dish / curved reflector",
            Shape::PatchBox => "Patch box (closed surface)",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Shape::Wire => "Click ends one after another (Shift keeps to 15° steps)",
            Shape::Arc => {
                "Click its two ends, then move the mouse to bend it (15° steps) and click"
            }
            Shape::Semicircle => "Click its two ends, then the side it bulges to",
            Shape::Circle => "Click the centre, then a point on it",
            Shape::Helix => {
                "Click the centre of its base, then a point for the radius; it runs towards you"
            }
            Shape::Spiral => "Click the centre, then a point for the outer radius",
            Shape::Radials => {
                "Radials or a counterpoise: click the hub (a vertical's base, say). Four level quarter-wave radials appear; change their number, length, droop and height in their panel"
            }
            Shape::Grid => {
                "A grid counterpoise: click its middle (a vertical's base, say). A square \
                 mesh a quarter wavelength across appears; change its size, cells and \
                 height in its panel"
            }
            Shape::Plate => {
                "A flat reflector plate (a wire mesh): click its centre. It faces the fed \
                 wire, one wavelength square with a tenth-wavelength mesh"
            }
            Shape::Corner => {
                "A corner reflector (two meshes at 90°): click the middle of its fold. It \
                 opens towards the fed wire; for the textbook gain put the driven element \
                 about half a wavelength in front of the fold"
            }
            Shape::Dish => {
                "A dish or curved reflector (a wire mesh): click its vertex. A paraboloid \
                 two wavelengths across with f/D 0.4 appears, facing the fed wire; pick \
                 another outline or profile (cylinder, sphere, V, your own formula) in its \
                 panel, and put a dipole at the focus"
            }
            Shape::PatchBox => {
                "A closed metal box of surface patches (a vehicle body, an equipment box): click the middle of its bottom. Half a wavelength long and deep, a quarter high; a wire can stand on the middle of a face"
            }
        }
    }

    fn is_arc(self) -> bool {
        matches!(self, Shape::Arc | Shape::Semicircle)
    }

    /// Clicks needed.
    fn clicks(self) -> usize {
        match self {
            Shape::Radials
            | Shape::Grid
            | Shape::Plate
            | Shape::Corner
            | Shape::Dish
            | Shape::PatchBox => 1,
            s if s.is_arc() => 3,
            _ => 2,
        }
    }

    /// The curve these clicked points make on the plane facing along
    /// `axis` (an arc's sweep snaps to 15° steps when snapping is on).
    fn curve(self, pts: &[[f64; 3]], axis: usize, snap: bool) -> Option<CurveSpec> {
        if self.is_arc() {
            let [s, e, p] = [*pts.first()?, *pts.get(1)?, *pts.get(2)?];
            let sweep = if self == Shape::Semicircle {
                180.0
            } else {
                let sw = sweep_through(s, e, p, axis);
                if snap {
                    ((sw / 15.0).round() * 15.0).clamp(15.0, 345.0)
                } else {
                    sw
                }
            };
            return arc_between(s, e, p, axis, sweep);
        }
        let (c, q) = (*pts.first()?, *pts.get(1)?);
        let r = (0..3)
            .filter(|&k| k != axis)
            .map(|k| (q[k] - c[k]).powi(2))
            .sum::<f64>()
            .sqrt();
        if r < 1e-9 {
            return None;
        }
        let kind = match self {
            Shape::Helix => CurveKind::Helix,
            Shape::Spiral => CurveKind::Spiral,
            _ => CurveKind::Circle,
        };
        let mut spec = CurveSpec::new(kind, "", c, r);
        if kind == CurveKind::Spiral {
            (spec.radius, spec.radius2) = (r / 5.0, r);
        }
        (spec.tilt, spec.azimuth) = facing(axis);
        Some(spec)
    }
}

/// The radials on or under the ground (not NEC wires, so not in the
/// segments): (set name, hub, tip) for each, in metres as shown.
fn buried_lines(model: &crate::model::LoadedModel) -> Vec<(String, [f64; 3], [f64; 3])> {
    let Ok(n) = &model.numeric else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for g in straws_core::deck::grids::grid_numbers(&model.deck, n) {
        if g.buried {
            for (a, b) in g.sides() {
                out.push((g.name.clone(), a, b));
            }
        }
    }
    for r in straws_core::deck::radials::radial_numbers(&model.deck, n) {
        if !r.buried {
            continue;
        }
        for k in 0..r.count {
            let a = (r.turn + 360.0 * k as f64 / r.count as f64).to_radians();
            let tip = [
                r.hub[0] + r.length * a.cos(),
                r.hub[1] + r.length * a.sin(),
                r.hub[2],
            ];
            out.push((r.name.clone(), r.hub, tip));
        }
    }
    out
}
