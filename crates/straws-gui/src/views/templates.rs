//! Templates: pick an antenna, set a few numbers, and get an editable,
//! variable-driven model. And the design library: proven designs from
//! their designers' dimensions, made for any band.

use eframe::egui::{self, RichText};
use straws_core::library::{LIBRARY, Make};
use straws_core::templates::{Common, Kind, TEMPLATES, TemplateGround};
use straws_core::units::wavelength;

use crate::app::{Data, Level, Settings};

pub struct TemplatesView {
    chosen: usize,
    mhz: f64,
    diameter_mm: f64,
    /// Per template: its settings and ground/height choices.
    values: Vec<Vec<f64>>,
    grounds: Vec<TemplateGround>,
    heights: Vec<f64>,
    /// Showing the design library rather than the templates.
    library: bool,
    design: usize,
    /// Per design: the frequency to make it for, and the wire diameter
    /// (for designs worked out from formulas).
    design_mhz: Vec<f64>,
    design_wire: Vec<f64>,
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
            library: false,
            design: 0,
            design_mhz: LIBRARY.iter().map(|d| d.mhz()).collect(),
            design_wire: LIBRARY
                .iter()
                .map(|d| match d.make {
                    Make::Scaled { diameter_mm, .. } | Make::Formula { diameter_mm, .. } => {
                        diameter_mm
                    }
                })
                .collect(),
        }
    }
}

impl TemplatesView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &mut Settings) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.library, false, "Templates")
                .on_hover_text("Antennas built from a few settings of your own");
            ui.selectable_value(&mut self.library, true, "Design library")
                .on_hover_text(
                    "Proven designs from their designers' published dimensions, made for your band",
                );
        });
        ui.separator();
        if self.library {
            self.library_ui(ui, data, settings);
            return;
        }
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

    fn library_ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &mut Settings) {
        let mut create = false;
        ui.horizontal_top(|ui| {
            // The designs, grouped by kind.
            ui.vertical(|ui| {
                ui.set_width(210.0);
                let mut kinds: Vec<&str> = LIBRARY.iter().map(|d| d.kind).collect();
                kinds.dedup();
                for kind in kinds {
                    ui.label(RichText::new(kind).weak().small());
                    for (k, d) in LIBRARY.iter().enumerate().filter(|(_, d)| d.kind == kind) {
                        ui.selectable_value(&mut self.design, k, d.name);
                    }
                    ui.add_space(4.0);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("design").show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                let k = self.design;
                let d = &LIBRARY[k];
                ui.heading(d.name);
                ui.label(format!("Designed by {}", d.credit));
                ui.label(RichText::new(d.summary).weak());
                ui.add_space(6.0);
                ui.label(RichText::new("Published").strong());
                ui.label(d.published);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Dimensions from");
                    if d.source.starts_with("http") {
                        ui.hyperlink_to(d.source.trim_start_matches("https://"), d.source);
                    } else {
                        ui.label(d.source);
                    }
                });
                ui.add_space(6.0);
                ui.label(RichText::new("Building it").strong());
                for note in d.build {
                    ui.horizontal_top(|ui| {
                        ui.label("•");
                        ui.add(egui::Label::new(*note).wrap());
                    });
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Make it for").strong());
                ui.horizontal(|ui| {
                    ui.add(
                        crate::views::num(&mut self.design_mhz[k])
                            .range(0.01..=100_000.0)
                            .speed(0.05)
                            .max_decimals(4)
                            .suffix(" MHz"),
                    );
                    let bands = crate::views::bands::chosen(settings);
                    egui::ComboBox::from_id_salt("design band")
                        .selected_text(
                            bands
                                .iter()
                                .find(|b| b.contains(self.design_mhz[k]))
                                .map_or("Band".to_string(), |b| b.name.clone()),
                        )
                        .show_ui(ui, |ui| {
                            for b in &bands {
                                if ui.selectable_label(b.contains(self.design_mhz[k]), &b.name).clicked() {
                                    // Its own frequency when the band holds it,
                                    // else the band's middle.
                                    self.design_mhz[k] = if b.contains(d.mhz()) {
                                        d.mhz()
                                    } else {
                                        b.centre_mhz()
                                    };
                                }
                            }
                        });
                    if ui.button("As published").clicked() {
                        self.design_mhz[k] = d.mhz();
                    }
                });
                let mhz = self.design_mhz[k];
                match d.make {
                    Make::Scaled { mhz: f0, diameter_mm, .. } => {
                        let s = f0 / mhz;
                        ui.label(
                            RichText::new(if (s - 1.0).abs() < 1e-9 {
                                format!("As published: {diameter_mm} mm elements.")
                            } else {
                                format!(
                                    "Every dimension scaled by {s:.4} from the {f0} MHz original, the elements too: {:.2} mm across. Building with other tube? Make its diameter and element lengths variables and let the optimiser retune it.",
                                    d.diameter_at(mhz, 0.0)
                                )
                            })
                            .weak(),
                        );
                    }
                    Make::Formula { .. } => {
                        ui.horizontal(|ui| {
                            ui.label("Wire diameter");
                            ui.add(
                                crate::views::num(&mut self.design_wire[k])
                                    .range(0.01..=500.0)
                                    .speed(0.05)
                                    .max_decimals(3)
                                    .suffix(" mm"),
                            );
                        });
                        ui.label(
                            RichText::new("Worked out afresh for this frequency and wire size.")
                                .weak(),
                        );
                    }
                }
                ui.add_space(6.0);
                create = ui
                    .button("Create this antenna")
                    .on_hover_text("Opens it as a new, unsaved model; every dimension is a named variable")
                    .clicked();
                });
            });
        });
        if create {
            let k = self.design;
            let d = &LIBRARY[k];
            let text = d.deck(self.design_mhz[k], self.design_wire[k]);
            data.new_deck = Some((d.name.to_string(), text));
            if (settings.z0 - d.z0).abs() > 1e-9 {
                settings.z0 = d.z0;
                data.message(
                    Level::Info,
                    format!(
                        "SWR is now shown on {} Ω, the impedance the {} is built to be fed at (change it on the toolbar).",
                        d.z0, d.name
                    ),
                );
            }
        }
    }
}
