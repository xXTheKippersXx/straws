//! The Smith chart: the feed impedance across the sweep, normalised to Z₀,
//! with the slider frequency marked and any comparison snapshots.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, vec2};
use straws_core::Complex64;
use straws_core::analysis::feed;

use crate::app::{Data, Settings};

use super::empty_note;

/// Reflection coefficient of `z` against `z0`.
pub fn gamma(z: Complex64, z0: f64) -> Complex64 {
    (z - z0) / (z + z0)
}

/// Where a reflection coefficient sits on a chart at `centre`, `radius`.
pub fn place(g: Complex64, centre: Pos2, radius: f32) -> Pos2 {
    centre + vec2(g.re as f32, -g.im as f32) * radius
}

/// The main trace's colour.
pub fn trace_colour(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(253, 200, 60)
    } else {
        Color32::from_rgb(200, 90, 20)
    }
}

/// Draws the chart's grid in `rect`; returns its centre and radius.
pub fn grid(ui: &egui::Ui, rect: Rect) -> (Pos2, f32) {
    let painter = ui.painter_at(rect);
    let v = ui.visuals();
    let line = v.widgets.noninteractive.bg_stroke.color;
    let weak = v.weak_text_color();
    let font = egui::FontId::proportional(10.5);
    let radius = (rect.width().min(rect.height()) / 2.0 - 22.0).max(20.0);
    let centre = rect.center();
    let at = |z: Complex64| place(gamma(z, 1.0), centre, radius);
    let stroke = Stroke::new(1.0, line);
    // Constant-resistance circles.
    for r in [0.0, 0.2, 0.5, 1.0, 2.0, 5.0] {
        let pts: Vec<Pos2> = (0..=200)
            .map(|k| {
                let t = -std::f64::consts::FRAC_PI_2 * 0.999
                    + std::f64::consts::PI * 0.999 * k as f64 / 200.0;
                at(Complex64::new(r, t.tan()))
            })
            .collect();
        painter.add(egui::Shape::line(pts, stroke));
        if r > 0.0 {
            painter.text(
                at(Complex64::new(r, 0.0)) + vec2(2.0, -2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{r}"),
                font.clone(),
                weak,
            );
        }
    }
    // Constant-reactance arcs.
    for x in [0.2, 0.5, 1.0, 2.0, 5.0] {
        for sign in [1.0, -1.0] {
            let pts: Vec<Pos2> = (0..=160)
                .map(|k| {
                    let s = 0.995 * k as f64 / 160.0;
                    at(Complex64::new(s / (1.0 - s), sign * x))
                })
                .collect();
            painter.add(egui::Shape::line(pts, stroke));
            let edge = at(Complex64::new(0.0, sign * x));
            let out = (edge - centre).normalized() * 11.0;
            painter.text(
                edge + out,
                egui::Align2::CENTER_CENTER,
                format!("{}{x}", if sign > 0.0 { "+j" } else { "−j" }),
                font.clone(),
                weak,
            );
        }
    }
    painter.line_segment(
        [centre - vec2(radius, 0.0), centre + vec2(radius, 0.0)],
        stroke,
    );
    (centre, radius)
}

pub fn ui(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    crate::views::feeds::feed_picker(ui, data);
    let Some(sweep) = data.display_sweep() else {
        let note = data.waiting_note("Open a .nec deck to see its Smith chart.", "Calculating…");
        return empty_note(ui, note);
    };
    let z0 = settings.z0;
    let points: Vec<(f64, Complex64)> = sweep
        .points()
        .filter_map(|p| p.feed(data.feed).map(|i| (p.mhz, i.impedance)))
        .collect();
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click());
    let (centre, radius) = grid(ui, rect);
    let painter = ui.painter_at(rect);
    let dark = ui.visuals().dark_mode;

    for c in data.comparisons.iter().filter(|c| c.visible) {
        let pts: Vec<Pos2> = c
            .sweep
            .points()
            .filter_map(|p| p.feed(data.feed))
            .map(|i| place(gamma(i.impedance, z0), centre, radius))
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.5, c.colour)));
    }
    if let (Some(a), Some(b)) = (points.first(), points.last()) {
        for (_, colour, pts) in crate::views::compare::measured(data, settings, (a.0, b.0)) {
            let pts: Vec<Pos2> = pts
                .iter()
                .map(|&(_, z)| place(gamma(z, z0), centre, radius))
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(2.0, colour)));
        }
    }
    let trace: Vec<Pos2> = points
        .iter()
        .map(|&(_, z)| place(gamma(z, z0), centre, radius))
        .collect();
    let colour = trace_colour(dark);
    painter.add(egui::Shape::line(trace.clone(), Stroke::new(2.0, colour)));
    if let (Some(first), Some(last)) = (trace.first(), trace.last()) {
        let font = egui::FontId::proportional(10.5);
        let text = ui.visuals().weak_text_color();
        painter.circle_stroke(*first, 3.0, Stroke::new(1.5, colour));
        painter.text(
            *first + vec2(5.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{:.3}", points[0].0),
            font.clone(),
            text,
        );
        painter.text(
            *last + vec2(5.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{:.3}", points[points.len() - 1].0),
            font,
            text,
        );
    }

    // The slider frequency, with its SWR circle.
    let mut lines = Vec::new();
    if let Some(p) = data.shown_point()
        && let Some(i) = p.feed(data.feed)
    {
        let g = gamma(i.impedance, z0);
        let at = place(g, centre, radius);
        painter.circle_stroke(
            centre,
            radius * g.norm() as f32,
            Stroke::new(1.0, colour.gamma_multiply(0.45)),
        );
        painter.circle_filled(at, 5.0, colour);
        lines.push(describe(p.mhz, i.impedance, z0));
    }
    // The nearest sweep point under the mouse; click to go there.
    if let Some(pos) = response.hover_pos()
        && let Some((k, d)) = trace
            .iter()
            .enumerate()
            .map(|(k, q)| (k, q.distance(pos)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
        && d < 12.0
    {
        let (mhz, z) = points[k];
        painter.circle_stroke(trace[k], 6.0, Stroke::new(1.5, ui.visuals().text_color()));
        lines.push(format!("Cursor: {}", describe(mhz, z, z0)));
        if response.clicked() {
            data.mhz = mhz;
        }
    }
    let font = egui::FontId::proportional(12.5);
    for (n, l) in lines.iter().enumerate() {
        painter.text(
            rect.left_top() + vec2(8.0, 6.0 + 18.0 * n as f32),
            egui::Align2::LEFT_TOP,
            l,
            font.clone(),
            ui.visuals().text_color(),
        );
    }
    painter.text(
        rect.left_bottom() + vec2(8.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        format!("Normalised to Z₀ = {z0:.0} Ω. Click the trace to go to that frequency."),
        egui::FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
    );
}

fn describe(mhz: f64, z: Complex64, z0: f64) -> String {
    let g = gamma(z, z0);
    format!(
        "{mhz:.4} MHz · Z = {:.2} {} j{:.2} Ω · |Γ| {:.3} at {:.1}° · SWR {:.2}",
        z.re,
        if z.im < 0.0 { "−" } else { "+" },
        z.im.abs(),
        g.norm(),
        g.arg().to_degrees(),
        feed::swr(z, z0)
    )
}
