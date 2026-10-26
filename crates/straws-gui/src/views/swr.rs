//! SWR and impedance against frequency, with the slider frequency marked.
//! Click a plot to move the frequency there.

use eframe::egui::{self, Color32};
use egui_plot::{Legend, Line, Plot, PlotPoints, VLine};
use straws_core::analysis::feed;

use crate::app::{Data, Settings};

use super::empty_note;

pub fn ui(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    crate::views::feeds::feed_picker(ui, data);
    let Some(sweep) = data.display_sweep() else {
        let note = data.waiting_note("Open a .nec deck to see SWR and impedance.", "Calculating…");
        return empty_note(ui, note);
    };
    let z0 = settings.z0;
    let points: Vec<_> = sweep
        .points()
        .filter_map(|p| p.feed(data.feed).map(|i| (p.mhz, i.impedance)))
        .collect();
    let plan = sweep.plan;

    // Figures at the slider frequency.
    let summary = match data.shown_point() {
        Some(p) => match p.feed(data.feed) {
            Some(i) => {
                let z = i.impedance;
                let mut s = format!(
                    "{:.4} MHz   Z = {:.2} {} j{:.2} Ω   SWR {:.2} ({z0:.0} Ω)   return loss {:.1} dB",
                    p.mhz,
                    z.re,
                    if z.im < 0.0 { "−" } else { "+" },
                    z.im.abs(),
                    feed::swr(z, z0),
                    feed::return_loss_db(z, z0),
                );
                if let Some(pb) = p.power {
                    s += &format!("   efficiency {:.1}%", pb.efficiency_percent);
                }
                if p.estimated {
                    s += "   (estimate: includes a counterpoise on or under the ground)";
                }
                s
            }
            None => "No voltage source to show: this deck is fed by current sources (4nec2's EX 6), which set the current, not a feed impedance.".to_string(),
        },
        None => "Calculating…".to_string(),
    };
    ui.label(egui::RichText::new(summary).monospace());

    if points.is_empty() {
        return empty_note(
            ui,
            if sweep.done() == 0 {
                "Calculating…"
            } else {
                "No feed-point results."
            },
        );
    }

    let dark = ui.visuals().dark_mode;
    let swr_colour = if dark {
        Color32::from_rgb(253, 200, 60)
    } else {
        Color32::from_rgb(200, 90, 20)
    };
    let r_colour = if dark {
        Color32::from_rgb(90, 170, 255)
    } else {
        Color32::from_rgb(30, 100, 200)
    };
    let x_colour = if dark {
        Color32::from_rgb(120, 220, 140)
    } else {
        Color32::from_rgb(30, 140, 60)
    };
    let marker = ui.visuals().weak_text_color();

    let height = (ui.available_height() - 8.0) / 2.0;
    let mut clicked_at = None;
    let link = ui.id().with("swr-link");

    // SWR, capped so a big mismatch doesn't flatten the interesting part.
    let swr_points: PlotPoints = points
        .iter()
        .map(|&(f, z)| [f, feed::swr(z, z0).min(20.0)])
        .collect();
    let others: Vec<(String, Color32, Vec<[f64; 3]>)> = data
        .comparisons
        .iter()
        .filter(|c| c.visible)
        .map(|c| {
            let pts = c
                .sweep
                .points()
                .filter_map(|p| {
                    p.feed(data.feed)
                        .map(|i| [p.mhz, i.impedance.re, i.impedance.im])
                })
                .collect();
            (c.name.clone(), c.colour, pts)
        })
        .collect();
    // The chosen plan's bands within (or near) the sweep, shaded.
    let span = plan.stop_mhz - plan.start_mhz;
    let bands: Vec<_> = crate::views::bands::chosen(settings)
        .into_iter()
        .filter(|b| b.stop_mhz >= plan.start_mhz - span && b.start_mhz <= plan.stop_mhz + span)
        .collect();
    let shade = crate::views::bands::shade(ui.visuals().dark_mode);
    let r = Plot::new("swr")
        .height(height.max(80.0))
        .link_axis(link, [true, false])
        .include_x(plan.start_mhz)
        .include_x(plan.stop_mhz)
        .include_y(1.0)
        .include_y(3.0)
        .y_axis_label("SWR")
        .allow_scroll(false)
        .show(ui, |pui| {
            for b in &bands {
                pui.span(
                    egui_plot::Span::new(b.name.as_str(), b.start_mhz..=b.stop_mhz)
                        .fill(shade)
                        .border_width(0.0),
                );
            }
            for (name, colour, pts) in &others {
                let p: PlotPoints = pts
                    .iter()
                    .map(|v| {
                        [
                            v[0],
                            feed::swr(straws_core::Complex64::new(v[1], v[2]), z0).min(20.0),
                        ]
                    })
                    .collect();
                pui.line(Line::new(name.as_str(), p).color(*colour).width(1.5));
            }
            pui.line(Line::new("SWR", swr_points).color(swr_colour).width(2.0));
            pui.vline(VLine::new("frequency", data.mhz).color(marker));
            (pui.response().clicked(), pui.pointer_coordinate())
        });
    if let (true, Some(c)) = r.inner {
        clicked_at = Some(c.x);
    }

    let r_points: PlotPoints = points.iter().map(|&(f, z)| [f, z.re]).collect();
    let x_points: PlotPoints = points.iter().map(|&(f, z)| [f, z.im]).collect();
    let r = Plot::new("impedance")
        .height(height.max(80.0))
        .link_axis(link, [true, false])
        .include_x(plan.start_mhz)
        .include_x(plan.stop_mhz)
        .include_y(0.0)
        .y_axis_label("Ω")
        .x_axis_label("MHz")
        .allow_scroll(false)
        .legend(Legend::default())
        .show(ui, |pui| {
            for (name, colour, pts) in &others {
                let r: PlotPoints = pts.iter().map(|v| [v[0], v[1]]).collect();
                let x: PlotPoints = pts.iter().map(|v| [v[0], v[2]]).collect();
                pui.line(Line::new(format!("{name} R"), r).color(*colour).width(1.2));
                pui.line(
                    Line::new(format!("{name} X"), x)
                        .color(colour.gamma_multiply(0.7))
                        .width(1.2)
                        .style(egui_plot::LineStyle::dashed_dense()),
                );
            }
            pui.line(
                Line::new("R (resistance)", r_points)
                    .color(r_colour)
                    .width(2.0),
            );
            pui.line(
                Line::new("X (reactance)", x_points)
                    .color(x_colour)
                    .width(2.0),
            );
            pui.vline(VLine::new("frequency", data.mhz).color(marker));
            (pui.response().clicked(), pui.pointer_coordinate())
        });
    if let (true, Some(c)) = r.inner {
        clicked_at = Some(c.x);
    }

    if let Some(f) = clicked_at {
        data.mhz = f.clamp(plan.start_mhz, plan.stop_mhz);
    }
}
