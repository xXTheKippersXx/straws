//! Comparison snapshots: keep the results of a design, change it, and see
//! both on the SWR plot, Smith chart and pattern cuts.

use eframe::egui::{self, Color32, RichText};

use crate::app::{Comparison, Data};

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

#[derive(Default)]
pub struct CompareView {
    name: String,
}

impl CompareView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, z0: f64) {
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
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
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
            });
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
