//! Receive-antenna figures across the sweep: RDF (the peak gain against
//! the average over the directions noise comes from) and front-to-back,
//! from the sweep's patterns, with the slider frequency marked. Click a
//! plot to move the frequency there.

use eframe::egui::{self, Color32, RichText};
use egui_plot::{Line, Plot, PlotPoints, VLine};
use straws_core::analysis::checks::rdf;
use straws_core::analysis::pattern::{Component, PatternGrid};

use crate::app::{Data, Settings};

use super::empty_note;

/// (RDF, front-to-back, peak gain) of a pattern; RDF and F/B where they
/// can be worked out. F/B is in the azimuth cut at the peak's elevation.
fn figures(g: &PatternGrid) -> (Option<f64>, Option<f64>, f64) {
    let (max, theta, _) = g.max(Component::Total);
    let fb = g
        .azimuth_cut(Component::Total, 90.0 - theta)
        .figures()
        .and_then(|f| f.front_to_back);
    (rdf(g), fb, max)
}

pub fn ui(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    let Some(sweep) = data.display_sweep() else {
        let note = data.waiting_note(
            "Open a .nec deck to see its receive figures across the sweep.",
            "Calculating…",
        );
        return empty_note(ui, note);
    };
    let plan = sweep.plan;
    let rows: Vec<(f64, Option<f64>, Option<f64>)> = sweep
        .points()
        .filter_map(|p| {
            let (r, fb, _) = figures(p.pattern.as_ref()?);
            Some((p.mhz, r, fb))
        })
        .collect();
    let summary = match data
        .shown_point()
        .and_then(|p| Some((p.mhz, p.pattern.as_ref()?)))
    {
        Some((mhz, g)) => {
            let (r, fb, max) = figures(g);
            format!(
                "{mhz:.4} MHz   RDF {}   F/B {}   peak {max:.2} dBi",
                r.map_or("-".into(), |r| format!("{r:.2} dB")),
                fb.map_or("-".into(), |f| format!("{f:.1} dB")),
            )
        }
        None => "No pattern at this frequency.".into(),
    };
    ui.label(RichText::new(summary).monospace());
    ui.label(
        RichText::new(format!(
            "RDF: how much more the antenna hears from its best direction than from {} \
             (where the noise comes from), so higher means better signal-to-noise whatever \
             the efficiency: the figure for Beverages, loops, flags and K9AYs. F/B in the \
             azimuth cut at the peak's elevation.",
            if data.over_ground {
                "the whole sky"
            } else {
                "every direction"
            }
        ))
        .weak()
        .small(),
    );
    if rows.iter().all(|r| r.1.is_none()) {
        return empty_note(
            ui,
            if sweep.done() == 0 {
                "Calculating…"
            } else {
                "The sweep has no full patterns to work these out from."
            },
        );
    }
    let dark = ui.visuals().dark_mode;
    let (rdf_colour, fb_colour) = if dark {
        (
            Color32::from_rgb(253, 200, 60),
            Color32::from_rgb(90, 170, 255),
        )
    } else {
        (
            Color32::from_rgb(200, 90, 20),
            Color32::from_rgb(30, 100, 200),
        )
    };
    let marker = ui.visuals().weak_text_color();
    let span = plan.stop_mhz - plan.start_mhz;
    let bands: Vec<_> = crate::views::bands::chosen(settings)
        .into_iter()
        .filter(|b| b.stop_mhz >= plan.start_mhz - span && b.start_mhz <= plan.stop_mhz + span)
        .collect();
    let shade = crate::views::bands::shade(dark);
    let height = (ui.available_height() - 8.0) / 2.0;
    let link = ui.id().with("receive-link");
    let mut clicked_at = None;
    for (k, (name, colour)) in [("RDF (dB)", rdf_colour), ("F/B (dB)", fb_colour)]
        .into_iter()
        .enumerate()
    {
        let pts: PlotPoints = rows
            .iter()
            .filter_map(|r| (if k == 0 { r.1 } else { r.2 }).map(|v| [r.0, v]))
            .collect();
        let r = Plot::new(("receive", k))
            .height(height.max(80.0))
            .link_axis(link, [true, false])
            .include_x(plan.start_mhz)
            .include_x(plan.stop_mhz)
            .y_axis_label(name)
            .x_axis_label(if k == 1 { "MHz" } else { "" })
            .allow_scroll(false)
            .show(ui, |pui| {
                for b in &bands {
                    pui.span(
                        egui_plot::Span::new(b.name.as_str(), b.start_mhz..=b.stop_mhz)
                            .fill(shade)
                            .border_width(0.0),
                    );
                }
                pui.line(Line::new(name, pts).color(colour).width(2.0));
                pui.vline(VLine::new("frequency", data.mhz).color(marker));
                (pui.response().clicked(), pui.pointer_coordinate())
            });
        if let (true, Some(c)) = r.inner {
            clicked_at = Some(c.x);
        }
    }
    if let Some(f) = clicked_at {
        data.mhz = f.clamp(plan.start_mhz, plan.stop_mhz);
    }
}
