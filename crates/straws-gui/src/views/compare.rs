//! Comparison snapshots: keep the results of a design, change it, and see
//! both on the SWR plot, Smith chart and pattern cuts. Also measured sweeps
//! from a VNA, shown on the SWR plot and Smith chart.

use eframe::egui::{self, Color32, RichText};
use straws_core::Complex64;
use straws_core::analysis::{feed, feedsystem, measured};

use crate::app::{Comparison, Data, Request, Settings};

use super::empty_note;

/// Colours for snapshots, from the Okabe-Ito colour-blind-safe set (the
/// current design keeps its own yellow/orange).
pub const COLOURS: [Color32; 6] = [
    Color32::from_rgb(86, 180, 233),
    Color32::from_rgb(0, 158, 115),
    Color32::from_rgb(204, 121, 167),
    Color32::from_rgb(213, 94, 0),
    Color32::from_rgb(0, 114, 178),
    Color32::from_rgb(240, 228, 66),
];

/// Colours for measurements: Okabe-Ito colours that stay clear of the
/// current design's orange SWR, blue R and green X in both themes.
pub const MEASURED_COLOURS: [Color32; 3] = [
    Color32::from_rgb(204, 121, 167),
    Color32::from_rgb(86, 180, 233),
    Color32::from_rgb(213, 94, 0),
];

#[derive(Default)]
pub struct CompareView {
    name: String,
}

impl CompareView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
        let z0 = settings.z0;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                self.snapshots(ui, data, z0);
                ui.add_space(8.0);
                measurements(ui, data, settings);
            });
    }

    fn snapshots(&mut self, ui: &mut egui::Ui, data: &mut Data, z0: f64) {
        let Some(model) = &data.model else {
            return empty_note(
                ui,
                "Open a model, then take snapshots here to compare designs.",
            );
        };
        let complete = data.sweep.as_ref().is_some_and(|s| s.is_complete());
        ui.label(
            RichText::new(
                "A snapshot keeps this design and its results. Change the design and both \
                 show on the SWR plot, Smith chart and pattern cuts.",
            )
            .weak(),
        );
        ui.add_space(4.0);
        let mut take = false;
        ui.horizontal(|ui| {
            // The default name is a hint: a cleared box stays clear to type in.
            let default = format!("Snapshot {}", data.snapshot_count + 1);
            ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .hint_text(default.as_str())
                    .desired_width(200.0),
            )
            .on_hover_text("A name for the snapshot (or leave it for the one shown)");
            take = ui
                .add_enabled(complete, egui::Button::new("Take snapshot"))
                .on_disabled_hover_text("Wait for the sweep to finish")
                .clicked();
        });
        if take && let Some(sweep) = &data.sweep {
            let mut deck = model.deck.clone();
            for (n, v) in &data.overrides {
                deck.set_variable(n, *v);
            }
            let colour = COLOURS[data.snapshot_count % COLOURS.len()];
            data.comparisons.push(Comparison {
                name: match std::mem::take(&mut self.name).trim() {
                    "" => format!("Snapshot {}", data.snapshot_count + 1),
                    typed => typed.to_string(),
                },
                colour,
                visible: true,
                sweep: sweep.clone(),
                deck: deck.to_text(),
            });
            data.snapshot_count += 1;
        }
        ui.separator();
        if data.comparisons.is_empty() {
            ui.label(RichText::new("No snapshots yet.").weak());
            return;
        }
        let mut remove = None;
        let mut restore = None;
        {
            {
                for (k, c) in data.comparisons.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let (r, painter) =
                            ui.allocate_painter(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        painter.rect_filled(r.rect, 3.0, c.colour);
                        ui.checkbox(&mut c.visible, "")
                            .on_hover_text("Show on the plots");
                        ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(160.0));
                        let best = c
                            .sweep
                            .points()
                            .filter_map(|p| {
                                p.feed(data.feed).map(|i| {
                                    (p.mhz, straws_core::analysis::feed::swr(i.impedance, z0))
                                })
                            })
                            .min_by(|a, b| a.1.total_cmp(&b.1));
                        if let Some((f, s)) = best {
                            ui.label(
                                RichText::new(format!("lowest SWR {s:.2} at {f:.3} MHz")).weak(),
                            );
                        }
                        if ui
                            .button("Restore")
                            .on_hover_text("Make this design the current one again (Ctrl+Z undoes)")
                            .clicked()
                        {
                            restore = Some(k);
                        }
                        if ui.button("Delete").clicked() {
                            remove = Some(k);
                        }
                    });
                }
            }
        }
        if let Some(k) = restore
            && let Some(m) = &mut data.model
        {
            m.deck = straws_core::deck::Deck::parse(&data.comparisons[k].deck);
            data.overrides.clear();
            data.edit_event = Some(true);
        }
        if let Some(k) = remove {
            data.comparisons.remove(k);
        }
    }
}

/// The Measurements part: import VNA sweeps and choose how they're shown.
fn measurements(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Measurements").strong());
        if ui
            .button("Import a VNA sweep…")
            .on_hover_text(
                "A Touchstone file (.s1p or .s2p) saved by a NanoVNA or another analyser",
            )
            .clicked()
        {
            data.request = Some(Request::ImportMeasurement);
        }
    });
    if data.measurements.is_empty() {
        ui.label(
            RichText::new(
                "Import a measured sweep to see it on the SWR plot and Smith chart beside the model.",
            )
            .weak(),
        );
        return;
    }
    let feed = crate::views::feedline::current_feed(data, settings);
    let mut remove = None;
    let mut sweep = None;
    for (k, m) in data.measurements.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let (r, painter) = ui.allocate_painter(egui::vec2(14.0, 14.0), egui::Sense::hover());
            painter.rect_filled(r.rect, 3.0, m.colour);
            ui.checkbox(&mut m.visible, "")
                .on_hover_text("Show on the SWR plot and Smith chart");
            ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(160.0));
            if ui
                .button("Sweep this range")
                .on_hover_text("Set the model's sweep to the measured frequencies")
                .clicked()
            {
                sweep = Some(k);
            }
            if ui.button("Delete").clicked() {
                remove = Some(k);
            }
        });
        ui.indent(("measurement", k), |ui| {
            ui.checkbox(&mut m.through_feed, "Measured through the feedline")
                .on_hover_text(
                    "Measured at the radio end of the cable set in the Feedline view: take the \
                     cable, connectors and balun off again to compare with the antenna itself",
                );
            let (first, last) = (m.points[0].mhz, m.points[m.points.len() - 1].mhz);
            let best = m
                .points
                .iter()
                .map(|p| {
                    (
                        p.mhz,
                        feed::swr(at_antenna(m, &feed, p.mhz, p.z), settings.z0),
                    )
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let mut text = format!("{} points, {first:.3} to {last:.3} MHz", m.points.len());
            if let Some((f, s)) = best {
                text += &format!(", lowest SWR {s:.2} at {f:.3} MHz");
            }
            ui.label(RichText::new(text).weak());
        });
    }
    if let Some(k) = sweep {
        let m = &data.measurements[k];
        data.request = Some(Request::Sweep(
            m.points[0].mhz,
            m.points[m.points.len() - 1].mhz,
        ));
    }
    if let Some(k) = remove {
        data.measurements.remove(k);
    }
}

/// Measurement `k`'s points as the antenna itself would show them (the
/// feedline taken off when it was measured through it).
pub fn antenna_points(data: &Data, settings: &Settings, k: usize) -> Vec<measured::MeasuredPoint> {
    let Some(m) = data.measurements.get(k) else {
        return Vec::new();
    };
    let feed = crate::views::feedline::current_feed(data, settings);
    m.points
        .iter()
        .map(|p| measured::MeasuredPoint {
            mhz: p.mhz,
            z: at_antenna(m, &feed, p.mhz, p.z),
        })
        .collect()
}

/// A measured impedance as the antenna itself would show it.
fn at_antenna(
    m: &crate::app::Measurement,
    feed: &straws_core::analysis::feedline::Feed,
    mhz: f64,
    z: Complex64,
) -> Complex64 {
    if m.through_feed {
        let (stages, _) = feedsystem::chain_from_feed(feed, mhz);
        measured::at_antenna(&stages, z, mhz)
    } else {
        z
    }
}

/// A measured trace to draw: name, colour, (MHz, impedance at the antenna).
pub type Trace = (String, Color32, Vec<(f64, Complex64)>);

/// The measurements to draw: (name, colour, (MHz, impedance at the
/// antenna)) for each shown one, only within `range` (MHz) so a wide VNA
/// sweep doesn't squash the model's plots.
pub fn measured(data: &Data, settings: &Settings, range: (f64, f64)) -> Vec<Trace> {
    if data.measurements.is_empty() {
        return Vec::new();
    }
    let feed = crate::views::feedline::current_feed(data, settings);
    data.measurements
        .iter()
        .filter(|m| m.visible)
        .map(|m| {
            let pts = m
                .points
                .iter()
                .filter(|p| p.mhz >= range.0 && p.mhz <= range.1)
                .map(|p| (p.mhz, at_antenna(m, &feed, p.mhz, p.z)))
                .collect();
            (format!("{} (VNA)", m.name), m.colour, pts)
        })
        .collect()
}
