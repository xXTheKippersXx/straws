//! Near fields: the electric field on a plane near the antenna as a heat
//! map, scaled to the transmitter power, with a rough RF exposure guide.

use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};
use straws_core::analysis::nearfield::{Grid, Plane, icnirp_public_e, near_field_deck};
use straws_core::engine::{CancelToken, Engine};
use straws_core::output::NearField;

use crate::app::{Data, Settings};
use crate::colormap::viridis;
use crate::views::wires::show_length;

use super::empty_note;

struct Result {
    grid: Grid,
    mhz: f64,
    points: Vec<NearField>,
    /// Input power of the run (W), to scale to the chosen power.
    input_power: f64,
}

pub struct NearFieldView {
    plane: Plane,
    offset: f64,
    size: f64,
    n: usize,
    power: f64,
    range_db: f64,
    job: Option<(CancelToken, Receiver<std::result::Result<Result, String>>)>,
    result: Option<Result>,
    error: Option<String>,
    /// Model generation the defaults were set for.
    set_for: Option<u64>,
}

impl Default for NearFieldView {
    fn default() -> Self {
        NearFieldView {
            plane: Plane::Xy,
            offset: 2.0,
            size: 20.0,
            n: 41,
            power: 100.0,
            range_db: 30.0,
            job: None,
            result: None,
            error: None,
            set_for: None,
        }
    }
}

impl NearFieldView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
        // Pick up a finished calculation.
        if let Some((_, rx)) = &self.job
            && let Ok(r) = rx.try_recv()
        {
            match r {
                Ok(r) => {
                    self.result = Some(r);
                    self.error = None;
                }
                Err(e) => self.error = Some(e),
            }
            self.job = None;
        }
        let Some(model) = &data.model else {
            return empty_note(ui, "Open a model to see the field around it.");
        };
        let Ok(numeric) = &model.numeric else {
            return empty_note(ui, "The deck can't be read: fix it in the Deck view first.");
        };
        let (centre, radius) = model.bounds();
        // Sensible starting plane: head height over ground, the antenna's
        // middle in free space; a little wider than the antenna.
        if self.set_for != Some(data.model_generation) && self.set_for.is_none() {
            self.offset = if data.over_ground {
                2.0
            } else {
                f64::from(centre[2])
            };
            self.size = (f64::from(radius) * 3.0).max(1.0);
            self.set_for = Some(data.model_generation);
        }
        let unit = settings.length();
        let mhz = data.mhz.max(1e-3);
        let mut run = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Plane");
            egui::ComboBox::from_id_salt("near plane")
                .selected_text(match self.plane {
                    Plane::Xy => "Horizontal (at a height)",
                    Plane::Xz => "Vertical, along X",
                    Plane::Yz => "Vertical, along Y",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.plane, Plane::Xy, "Horizontal (at a height)");
                    ui.selectable_value(&mut self.plane, Plane::Xz, "Vertical, along X");
                    ui.selectable_value(&mut self.plane, Plane::Yz, "Vertical, along Y");
                });
            let axis = ["x", "y", "z"][self.plane.axes().2];
            ui.label(format!("at {axis} ="));
            let mut v = unit.from_metres(self.offset, mhz);
            if ui
                .add(
                    crate::views::num(&mut v)
                        .speed(0.05)
                        .max_decimals(3)
                        .suffix(format!(" {}", unit.symbol())),
                )
                .changed()
            {
                self.offset = unit.to_metres(v, mhz);
            }
            ui.label("size");
            let mut v = unit.from_metres(self.size, mhz);
            if ui
                .add(
                    crate::views::num(&mut v)
                        .speed(0.1)
                        .range(0.001..=1e6)
                        .max_decimals(3)
                        .suffix(format!(" {}", unit.symbol())),
                )
                .changed()
            {
                self.size = unit.to_metres(v, mhz).max(1e-3);
            }
            ui.add(
                crate::views::num(&mut self.n)
                    .range(5..=121)
                    .suffix(" points"),
            )
            .on_hover_text("Points along each side (more is finer and slower)");
            ui.label("power");
            ui.add(
                crate::views::num(&mut self.power)
                    .range(0.001..=1e6)
                    .speed(1.0)
                    .suffix(" W"),
            );
            ui.label("range");
            ui.add(
                crate::views::num(&mut self.range_db)
                    .range(10.0..=80.0)
                    .suffix(" dB"),
            );
            if self.job.is_some() {
                ui.add(egui::Spinner::new());
                if ui.button("Stop").clicked()
                    && let Some((c, _)) = &self.job
                {
                    c.cancel();
                    self.job = None;
                }
            } else {
                run = ui
                    .button("Calculate")
                    .on_hover_text(format!("Work out the field at {mhz:.4} MHz"))
                    .clicked();
            }
        });
        if run {
            let (u, v, _) = self.plane.axes();
            let grid = Grid {
                plane: self.plane,
                centre: [f64::from(centre[u]), f64::from(centre[v])],
                size: self.size,
                offset: self.offset,
                n: self.n,
            };
            let deck = near_field_deck(numeric, mhz, &grid);
            let cancel = CancelToken::new();
            let c = cancel.clone();
            let (tx, rx) = channel();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let r = Engine::self_hosted()
                    .map_err(|e| e.to_string())
                    .and_then(|e| e.run(&deck, &c).map_err(|e| e.to_string()))
                    .and_then(|out| {
                        let f = out.frequencies.into_iter().next().ok_or("no result")?;
                        let input_power = f.inputs.iter().map(|i| i.power).sum();
                        Ok(Result {
                            grid,
                            mhz,
                            points: f.near_e,
                            input_power,
                        })
                    });
                let _ = tx.send(r);
                ctx.request_repaint();
            });
            self.job = Some((cancel, rx));
        }
        if let Some(e) = &self.error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Couldn't work out the field: {e}"),
            );
        }
        let Some(r) = &self.result else {
            ui.label(
                RichText::new("Choose a plane and press Calculate. Over ground, a horizontal plane at about 2 m shows the field where people stand.")
                    .weak(),
            );
            return;
        };
        if (r.mhz - mhz).abs() > 1e-9 {
            ui.label(
                RichText::new(format!(
                    "Showing {:.4} MHz: press Calculate for the slider frequency.",
                    r.mhz
                ))
                .weak(),
            );
        }
        // V/m RMS for the chosen power.
        let scale = if r.input_power > 0.0 {
            (self.power / r.input_power).sqrt() / std::f64::consts::SQRT_2
        } else {
            1.0
        };
        let values: Vec<f64> = r.points.iter().map(|p| p.magnitude() * scale).collect();
        let max = values.iter().copied().fold(0.0, f64::max);
        let limit = icnirp_public_e(r.mhz);
        let over = values.iter().filter(|v| **v > limit).count();
        ui.label(format!(
            "Strongest {max:.1} V/m RMS for {} W. ICNIRP general-public reference level at {:.3} MHz: {limit:.1} V/m; {} of {} points are above it (outlined in red).",
            self.power,
            r.mhz,
            over,
            values.len()
        ));
        ui.label(
            RichText::new("A rough guide only: a real exposure assessment averages over time and the body, and counts the magnetic field too.")
                .weak(),
        );

        // The heat map, square, with the antenna drawn over it.
        let avail = ui.available_rect_before_wrap();
        let side = (avail.width() - 70.0).min(avail.height() - 10.0).max(50.0);
        let rect = Rect::from_min_size(avail.min + vec2(10.0, 4.0), vec2(side, side));
        let response = ui.allocate_rect(avail, Sense::hover());
        let painter = ui.painter_at(avail);
        let g = r.grid;
        let (u, v, w) = g.plane.axes();
        let n = g.n.max(2);
        let cell = side / n as f32;
        let floor = max * 10f64.powf(-self.range_db / 20.0);
        let to_unit = |e: f64| -> f32 {
            if e <= floor || max <= 0.0 {
                0.0
            } else {
                ((20.0 * (e / max).log10() + self.range_db) / self.range_db) as f32
            }
        };
        // Grid index of a point along u and v.
        let index = |p: &NearField| -> (usize, usize) {
            let i = ((p.position[u] - (g.centre[0] - g.size / 2.0)) / g.step()).round() as usize;
            let j = ((p.position[v] - (g.centre[1] - g.size / 2.0)) / g.step()).round() as usize;
            (i.min(n - 1), j.min(n - 1))
        };
        let cell_rect = |i: usize, j: usize| {
            Rect::from_min_size(
                pos2(
                    rect.left() + i as f32 * cell,
                    rect.bottom() - (j + 1) as f32 * cell,
                ),
                vec2(cell + 0.5, cell + 0.5),
            )
        };
        for (p, e) in r.points.iter().zip(&values) {
            let (i, j) = index(p);
            painter.rect_filled(cell_rect(i, j), 0.0, viridis(to_unit(*e)));
        }
        let red = Color32::from_rgb(230, 60, 60);
        for (p, e) in r.points.iter().zip(&values) {
            if *e > limit {
                let (i, j) = index(p);
                painter.rect_stroke(
                    cell_rect(i, j),
                    0.0,
                    Stroke::new(1.5, red),
                    egui::StrokeKind::Inside,
                );
            }
        }
        // Wires, flattened onto the plane.
        let to_screen = |q: [f64; 3]| -> Pos2 {
            let x = (q[u] - (g.centre[0] - g.size / 2.0)) / g.size;
            let y = (q[v] - (g.centre[1] - g.size / 2.0)) / g.size;
            pos2(
                rect.left() + x as f32 * side,
                rect.bottom() - y as f32 * side,
            )
        };
        let wire_colour = Color32::from_rgba_unmultiplied(255, 255, 255, 170);
        for s in model.visible_segments() {
            let (a, b) = s.ends();
            painter.line_segment([to_screen(a), to_screen(b)], Stroke::new(1.5, wire_colour));
        }
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, ui.visuals().weak_text_color()),
            egui::StrokeKind::Outside,
        );
        let axis = ["x", "y", "z"];
        let font = egui::FontId::proportional(11.0);
        let weak = ui.visuals().weak_text_color();
        painter.text(
            rect.left_bottom() + vec2(0.0, 2.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{} from {} {}",
                axis[u],
                show_length(g.centre[0] - g.size / 2.0, unit, mhz),
                unit.symbol()
            ),
            font.clone(),
            weak,
        );
        painter.text(
            rect.right_bottom() + vec2(0.0, 2.0),
            egui::Align2::RIGHT_TOP,
            format!(
                "to {} {}",
                show_length(g.centre[0] + g.size / 2.0, unit, mhz),
                unit.symbol()
            ),
            font.clone(),
            weak,
        );
        // Colour key.
        let bar = Rect::from_min_size(pos2(rect.right() + 12.0, rect.top()), vec2(12.0, side));
        for k in 0..48 {
            let t0 = k as f32 / 48.0;
            let t1 = (k + 1) as f32 / 48.0;
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(bar.left(), bar.bottom() - t1 * side),
                    pos2(bar.right(), bar.bottom() - t0 * side + 0.5),
                ),
                0.0,
                viridis((t0 + t1) / 2.0),
            );
        }
        painter.text(
            bar.right_top() + vec2(4.0, 0.0),
            egui::Align2::LEFT_TOP,
            format!("{max:.1}"),
            font.clone(),
            weak,
        );
        painter.text(
            bar.right_bottom() + vec2(4.0, 0.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{floor:.2}"),
            font.clone(),
            weak,
        );
        painter.text(
            bar.right_center() + vec2(4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "V/m",
            font,
            weak,
        );
        // Readout under the mouse.
        if let Some(pos) = response.hover_pos()
            && rect.contains(pos)
        {
            let i = (((pos.x - rect.left()) / cell) as usize).min(n - 1);
            let j = (((rect.bottom() - pos.y) / cell) as usize).min(n - 1);
            if let Some((p, e)) = r
                .points
                .iter()
                .zip(&values)
                .find(|(p, _)| index(p) == (i, j))
            {
                let text = format!(
                    "{} {} {}, {} {} {}, {} {} {}: {e:.2} V/m RMS",
                    axis[u],
                    show_length(p.position[u], unit, mhz),
                    unit.symbol(),
                    axis[v],
                    show_length(p.position[v], unit, mhz),
                    unit.symbol(),
                    axis[w],
                    show_length(p.position[w], unit, mhz),
                    unit.symbol()
                );
                painter.text(
                    rect.left_top() + vec2(6.0, 6.0),
                    egui::Align2::LEFT_TOP,
                    text,
                    egui::FontId::proportional(12.5),
                    Color32::WHITE,
                );
            }
        }
    }
}
