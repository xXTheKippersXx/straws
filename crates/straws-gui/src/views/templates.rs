//! Templates: pick an antenna, set a few numbers, and get an editable,
//! variable-driven model.

use eframe::egui::{self, RichText};
use straws_core::templates::{Common, Kind, TEMPLATES, TemplateGround};
use straws_core::units::wavelength;

use crate::app::Data;

pub struct TemplatesView {
    chosen: usize,
    mhz: f64,
    diameter_mm: f64,
    /// Per template: its settings and ground/height choices.
    values: Vec<Vec<f64>>,
    grounds: Vec<TemplateGround>,
    heights: Vec<f64>,
}

impl Default for TemplatesView {
    fn default() -> Self {
        TemplatesView {
            chosen: 0,
            mhz: 14.2,
            diameter_mm: 2.0,
            values: TEMPLATES.iter().map(|t| t.defaults()).collect(),
            grounds: TEMPLATES.iter().map(|t| t.ground).collect(),
            heights: TEMPLATES.iter().map(|t| t.height_wl).collect(),
        }
    }
}

impl TemplatesView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data) {
        let mut create = false;
        ui.horizontal_top(|ui| {
            // The list of templates.
            ui.vertical(|ui| {
                ui.set_width(170.0);
                for (k, t) in TEMPLATES.iter().enumerate() {
                    ui.selectable_value(&mut self.chosen, k, t.name);
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                let k = self.chosen;
                let t = &TEMPLATES[k];
                ui.heading(t.name);
                ui.label(RichText::new(t.description).weak());
                ui.add_space(6.0);
                let lam = wavelength(self.mhz.max(1e-3));
                egui::Grid::new("template settings")
                    .num_columns(3)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        if !t.name.starts_with("Log") {
                            ui.label("Design frequency");
                            ui.add(
                                crate::views::num(&mut self.mhz)
                                    .range(0.01..=100_000.0)
                                    .speed(0.05)
                                    .max_decimals(4)
                                    .suffix(" MHz"),
                            );
                            ui.label(RichText::new(format!("λ = {lam:.3} m")).weak());
                            ui.end_row();
                        }
                        ui.label("Wire diameter");
                        ui.add(
                            crate::views::num(&mut self.diameter_mm)
                                .range(0.01..=500.0)
                                .speed(0.05)
                                .max_decimals(3)
                                .suffix(" mm"),
                        );
                        ui.label("");
                        ui.end_row();
                        ui.label("Ground");
                        ui.horizontal(|ui| {
                            let g = &mut self.grounds[k];
                            ui.radio_value(g, TemplateGround::FreeSpace, "Free space");
                            ui.radio_value(g, TemplateGround::Real, "Average real ground")
                                .on_hover_text("εr 13, σ 0.005 S/m, Sommerfeld-Norton (change it later in the Ground view)");
                            ui.radio_value(g, TemplateGround::Perfect, "Perfect ground");
                        });
                        ui.label("");
                        ui.end_row();
                        if self.grounds[k] != TemplateGround::FreeSpace && t.height_wl > 0.0 {
                            ui.label("Height");
                            ui.add(
                                crate::views::num(&mut self.heights[k])
                                    .range(0.0..=20.0)
                                    .speed(0.01)
                                    .max_decimals(3)
                                    .suffix(" λ"),
                            );
                            ui.label(RichText::new(format!("= {:.3} m", self.heights[k] * lam)).weak());
                            ui.end_row();
                        }
                        for (n, s) in t.settings.iter().enumerate() {
                            let v = &mut self.values[k][n];
                            ui.label(s.label).on_hover_text(s.help);
                            let (suffix, speed) = match s.kind {
                                Kind::Wavelengths => (" λ", 0.001),
                                Kind::Degrees => ("°", 0.5),
                                Kind::Mhz => (" MHz", 0.05),
                                Kind::Ohms => (" Ω", 1.0),
                                Kind::Count => ("", 0.05),
                                Kind::Number => ("", 0.005),
                            };
                            let mut dv = crate::views::num(v).speed(speed).suffix(suffix);
                            dv = if s.kind == Kind::Count {
                                dv.range(1.0..=16.0).max_decimals(0)
                            } else {
                                dv.max_decimals(4)
                            };
                            ui.add(dv).on_hover_text(s.help);
                            if s.kind == Kind::Wavelengths {
                                ui.label(RichText::new(format!("= {:.3} m", *v * lam)).weak());
                            } else {
                                ui.label("");
                            }
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    create = ui
                        .button("Create this antenna")
                        .on_hover_text("Opens it as a new, unsaved model")
                        .clicked();
                    if ui.button("Back to defaults").clicked() {
                        self.values[k] = t.defaults();
                        self.grounds[k] = t.ground;
                        self.heights[k] = t.height_wl;
                    }
                });
                ui.label(
                    RichText::new(
                        "Every dimension becomes a variable: tune it with the sliders in the Variables view or let the Optimiser do it.",
                    )
                    .weak(),
                );
            });
        });
        if create {
            let k = self.chosen;
            let t = &TEMPLATES[k];
            let common = Common {
                mhz: self.mhz,
                diameter_mm: self.diameter_mm,
                ground: self.grounds[k],
                height_wl: self.heights[k],
            };
            data.new_deck = Some((t.name.to_string(), t.deck(&common, &self.values[k])));
        }
    }
}
