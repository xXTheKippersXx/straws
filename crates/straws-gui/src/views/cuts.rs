//! 2D pattern cuts: an elevation plot and an azimuth plot, linked to the
//! 3D pattern (both follow the frequency slider and field component).

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use straws_core::analysis::pattern::{Cut, CutFigures};

use crate::app::{Data, Settings};
use crate::scene::GainScale;

use super::empty_note;

#[derive(Default)]
pub struct CutsView {
    /// Azimuth (degrees) of the elevation cut; `None` follows the peak.
    azimuth: Option<f64>,
    /// Elevation (degrees) of the azimuth cut; `None` follows the peak.
    elevation: Option<f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Elevation angle: 0° = horizon ahead (right), 90° = up.
    Elevation { upper_half: bool },
    /// Azimuth: 0° = +X (right), counter-clockwise.
    Azimuth,
}

impl CutsView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &Data, settings: &Settings) {
        let Some(point) = data.shown_point() else {
            let note = data.waiting_note(
                "Open a .nec deck to see its pattern cuts.",
                "Calculating the pattern…",
            );
            return empty_note(ui, note);
        };
        let Some(grid) = &point.pattern else {
            return empty_note(ui, "No pattern at this frequency.");
        };
        let component = settings.component();
        let (max, peak_theta, peak_phi) = grid.max(component);
        let azimuth = self.azimuth.unwrap_or(peak_phi);
        // With the peak overhead, a cut at the peak's elevation is a flat
        // circle that shows nothing; use 45° instead.
        let overhead = 90.0 - peak_theta > 75.0;
        let elevation = self
            .elevation
            .unwrap_or(if overhead { 45.0 } else { 90.0 - peak_theta });

        ui.horizontal(|ui| {
            follow_control(
                ui,
                "Elevation cut at azimuth",
                &mut self.azimuth,
                azimuth,
                0.0..=360.0,
            );
            ui.separator();
            follow_control(
                ui,
                "Azimuth cut at elevation",
                &mut self.elevation,
                elevation,
                -90.0..=90.0,
            );
            if overhead && self.elevation.is_none() {
                ui.label(egui::RichText::new("(the peak is overhead, so 45° is shown)").weak());
            }
        });

        let scale = GainScale {
            max_dbi: max,
            range_db: settings.range_db,
        };
        let el_cut = grid.elevation_cut(component, azimuth);
        let az_cut = grid.azimuth_cut(component, elevation);
        // Snapshots at the nearest frequency they have, same angles.
        let (mut el_others, mut az_others) = (Vec::new(), Vec::new());
        for c in data.comparisons.iter().filter(|c| c.visible) {
            if let Some(g) = c.sweep.nearest(data.mhz).and_then(|p| p.pattern.as_ref()) {
                el_others.push((g.elevation_cut(component, azimuth), c.colour));
                az_others.push((g.azimuth_cut(component, elevation), c.colour));
            }
        }
        let avail = ui.available_rect_before_wrap();
        // Side by side when there's room, otherwise one above the other.
        let wide = avail.width() > avail.height() * 1.2;
        let (r1, r2) = if wide {
            let w = avail.width() / 2.0;
            (
                Rect::from_min_size(avail.min, vec2(w, avail.height())),
                Rect::from_min_size(avail.min + vec2(w, 0.0), vec2(w, avail.height())),
            )
        } else {
            let h = avail.height() / 2.0;
            (
                Rect::from_min_size(avail.min, vec2(avail.width(), h)),
                Rect::from_min_size(avail.min + vec2(0.0, h), vec2(avail.width(), h)),
            )
        };
        ui.allocate_rect(avail, Sense::hover());
        let upper_half = grid.upper_half_only();
        polar(
            ui,
            r1,
            &format!("Elevation, azimuth {azimuth:.0}°"),
            &el_cut,
            &el_others,
            scale,
            Kind::Elevation { upper_half },
        );
        polar(
            ui,
            r2,
            &format!("Azimuth, elevation {elevation:.0}°"),
            &az_cut,
            &az_others,
            scale,
            Kind::Azimuth,
        );
    }
}

/// "Follow peak" checkbox and a value to set the angle by hand.
fn follow_control(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<f64>,
    current: f64,
    range: std::ops::RangeInclusive<f64>,
) {
    ui.label(label);
    let mut follow = value.is_none();
    if ui
        .checkbox(&mut follow, "peak")
        .on_hover_text("Follow the direction of maximum gain")
        .changed()
    {
        *value = if follow { None } else { Some(current) };
    }
    let mut v = current;
    if ui
        .add_enabled(
            !follow,
            crate::views::num(&mut v)
                .range(range)
                .speed(1.0)
                .suffix("°"),
        )
        .changed()
    {
        *value = Some(v);
    }
}

/// How an elevation-cut angle (0-90 ahead, 90-270 over the top and behind,
/// 270-360 below the horizon ahead) reads as an elevation above the horizon.
fn elevation_label(angle: f64) -> f64 {
    let a = angle.rem_euclid(360.0);
    if a <= 90.0 {
        a
    } else if a <= 270.0 {
        180.0 - a
    } else {
        a - 360.0
    }
}

/// Screen angle (radians, counter-clockwise from the right) for a cut
/// angle. Both plots put 0° on the right and count anticlockwise.
fn screen_angle(angle_deg: f64) -> f32 {
    angle_deg.to_radians() as f32
}

fn polar(
    ui: &mut egui::Ui,
    rect: Rect,
    title: &str,
    cut: &Cut,
    others: &[(Cut, Color32)],
    scale: GainScale,
    kind: Kind,
) {
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    let text_colour = visuals.text_color();
    let weak = visuals.weak_text_color();
    let grid_colour = visuals.widgets.noninteractive.bg_stroke.color;
    let trace = if visuals.dark_mode {
        Color32::from_rgb(253, 200, 60)
    } else {
        Color32::from_rgb(200, 90, 20)
    };
    let font = egui::FontId::proportional(11.0);

    let upper = matches!(kind, Kind::Elevation { upper_half: true });
    // Room for the title and the 90° label above the circle.
    let top = rect.top() + 44.0;
    let bottom_text = 44.0;
    let usable = Rect::from_min_max(
        pos2(rect.left() + 28.0, top),
        pos2(rect.right() - 28.0, rect.bottom() - bottom_text),
    );
    let (centre, radius) = if upper {
        let r = (usable.width() / 2.0).min(usable.height() - 12.0).max(10.0);
        (pos2(usable.center().x, usable.top() + 12.0 + r), r)
    } else {
        let r = (usable.width().min(usable.height()) / 2.0).max(10.0);
        (usable.center(), r)
    };
    let at = |angle: f32, r: f32| -> Pos2 { centre + vec2(angle.cos(), -angle.sin()) * r };

    painter.text(
        pos2(rect.center().x, rect.top() + 6.0),
        egui::Align2::CENTER_TOP,
        title,
        egui::FontId::proportional(13.0),
        text_colour,
    );

    // Rings every 10 dB (20 dB for wide ranges) below the peak.
    let ring_step = if scale.range_db > 40.0 { 20.0 } else { 10.0 };
    let (a0, a1) = if upper {
        (0.0, std::f32::consts::PI)
    } else {
        (0.0, std::f32::consts::TAU)
    };
    let mut db = 0.0;
    while db < scale.range_db - 1e-9 {
        let r = radius * (1.0 - (db / scale.range_db) as f32);
        let pts: Vec<Pos2> = (0..=96)
            .map(|k| at(a0 + (a1 - a0) * k as f32 / 96.0, r))
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.0, grid_colour)));
        painter.text(
            at(1.4, r) + vec2(2.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{:.0}", -db),
            font.clone(),
            weak,
        );
        db += ring_step;
    }
    // Spokes every 30 degrees.
    for k in 0..12 {
        let deg = k as f64 * 30.0;
        if upper && deg > 180.0 {
            break;
        }
        let a = screen_angle(deg);
        painter.line_segment([centre, at(a, radius)], Stroke::new(1.0, grid_colour));
        let label = match kind {
            Kind::Elevation { .. } => format!("{:.0}°", elevation_label(deg)),
            Kind::Azimuth => format!("{deg:.0}°"),
        };
        painter.text(
            at(a, radius + 12.0),
            egui::Align2::CENTER_CENTER,
            label,
            font.clone(),
            weak,
        );
    }

    // Snapshots first, then the current design on top.
    for (other, colour) in others {
        let pts: Vec<Pos2> = other
            .points
            .iter()
            .map(|&(angle, g)| at(screen_angle(angle), radius * scale.unit(g)))
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.5, *colour)));
    }
    // The trace.
    let pts: Vec<Pos2> = cut
        .points
        .iter()
        .map(|&(angle, g)| at(screen_angle(angle), radius * scale.unit(g)))
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(2.0, trace)));

    // Figures under the plot.
    let figures = cut.figures();
    let summary = figures.map_or_else(
        || "No field in this plane".to_string(),
        |f| describe(kind, f),
    );
    painter.text(
        pos2(rect.center().x, rect.bottom() - 30.0),
        egui::Align2::CENTER_TOP,
        summary,
        font.clone(),
        text_colour,
    );

    // Readout under the mouse: the cut value in the pointed-at direction.
    let response = ui.interact(rect, ui.id().with(title), Sense::hover());
    if let Some(pos) = response.hover_pos() {
        let d = pos - centre;
        if d.length() <= radius * 1.1 {
            let angle = f64::from((-d.y).atan2(d.x).to_degrees());
            if let Some(&(a, g)) = cut
                .points
                .iter()
                .min_by(|p, q| angle_gap(p.0, angle).total_cmp(&angle_gap(q.0, angle)))
            {
                let shown = match kind {
                    Kind::Elevation { .. } => elevation_label(a),
                    Kind::Azimuth => a.rem_euclid(360.0),
                };
                let tip = pos2(rect.center().x, rect.bottom() - 16.0);
                painter.text(
                    tip,
                    egui::Align2::CENTER_TOP,
                    format!("Cursor: {shown:.0}° · {g:.2} dBi"),
                    font,
                    weak,
                );
                painter.circle_filled(at(screen_angle(a), radius * scale.unit(g)), 3.5, trace);
            }
        }
    }
}

fn angle_gap(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn describe(kind: Kind, f: CutFigures) -> String {
    let angle = match kind {
        Kind::Elevation { .. } => elevation_label(f.max_angle),
        Kind::Azimuth => f.max_angle.rem_euclid(360.0),
    };
    let mut s = format!("Max {:.2} dBi at {angle:.0}°", f.max_dbi);
    if let Some(bw) = f.beamwidth {
        s += &format!(" · beamwidth {bw:.0}°");
    }
    if matches!(kind, Kind::Azimuth)
        && let Some(fb) = f.front_to_back
    {
        s += &format!(" · F/B {fb:.1} dB");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::elevation_label;

    #[test]
    fn elevation_labels_read_as_angles_above_the_horizon() {
        assert_eq!(elevation_label(30.0), 30.0);
        assert_eq!(elevation_label(150.0), 30.0);
        assert_eq!(elevation_label(210.0), -30.0);
        assert_eq!(elevation_label(300.0), -60.0);
        assert_eq!(elevation_label(-60.0), -60.0);
    }
}
