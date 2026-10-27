//! Exporting the model as STEP for CAD: the choice of which wire meshes
//! (plates, corner reflectors, dishes, grids) go out as solid sheets.

use eframe::egui::{self, RichText};
use straws_core::deck::Deck;
use straws_core::deck::edit::Wire;
use straws_core::deck::reflectors::Profile;
use straws_core::step;

/// One of the model's meshes.
pub struct MeshChoice {
    pub name: String,
    kind: &'static str,
    pub tags: Vec<i64>,
    /// Its wires' diameter (m).
    diameter: f64,
    /// Write it as a solid sheet rather than as tubes.
    pub solid: bool,
    /// Why it can't be a sheet, if it can't.
    problem: Option<String>,
}

/// The export being set up (the window is open while this exists).
pub struct StepExport {
    pub meshes: Vec<MeshChoice>,
    /// One thickness for every sheet, instead of each mesh's wire diameter.
    own_thickness: bool,
    thickness_mm: f64,
}

pub enum Outcome {
    Open,
    Cancel,
    Export,
}

/// The model's meshes that could become sheets: plates and corner
/// reflectors, dishes and curved reflectors, and grids above the ground.
pub fn meshes(deck: &Deck) -> Vec<(String, &'static str, Vec<i64>)> {
    let mut out = Vec::new();
    for p in straws_core::deck::plates::plate_sets(deck) {
        out.push((p.name, p.kind.label(), p.tags));
    }
    for r in straws_core::deck::reflectors::reflector_sets(deck) {
        let kind = match r.profile {
            Profile::Paraboloid | Profile::Sphere => "dish",
            Profile::Flat => "flat reflector",
            _ => "curved reflector",
        };
        out.push((r.name, kind, r.tags));
    }
    for g in straws_core::deck::grids::grid_sets(deck) {
        if !g.buried {
            out.push((g.name, "grid", g.tags));
        }
    }
    out
}

/// A mesh's wires, ready to write.
pub fn mesh(name: &str, tags: &[i64], wires: &[Wire], thickness: f64) -> step::Mesh {
    let mine: Vec<&Wire> = wires.iter().filter(|w| tags.contains(&w.tag)).collect();
    step::Mesh {
        name: name.to_string(),
        wires: mine.iter().map(|w| (w.a, w.b)).collect(),
        thickness,
        radius: mine.first().map_or(0.001, |w| w.radius),
    }
}

impl StepExport {
    pub fn new(deck: &Deck, wires: &[Wire]) -> StepExport {
        let meshes = meshes(deck)
            .into_iter()
            .map(|(name, kind, tags)| {
                let diameter = wires
                    .iter()
                    .find(|w| tags.contains(&w.tag))
                    .map_or(0.002, |w| 2.0 * w.radius);
                let problem =
                    step::sheet_solid(&mesh(&name, &tags, wires, diameter.max(1e-6))).err();
                MeshChoice {
                    name,
                    kind,
                    tags,
                    diameter,
                    solid: problem.is_none(),
                    problem,
                }
            })
            .collect();
        StepExport {
            meshes,
            own_thickness: false,
            thickness_mm: 2.0,
        }
    }

    /// A sheet's thickness (m).
    pub fn thickness(&self, m: &MeshChoice) -> f64 {
        if self.own_thickness {
            self.thickness_mm / 1000.0
        } else {
            m.diameter
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::Open;
        egui::Modal::new(egui::Id::new("export step")).show(ctx, |ui| {
            ui.set_width(500.0);
            ui.heading("Export as STEP");
            ui.label(
                RichText::new(
                    "Every wire becomes a solid tube at its real diameter, in place. Wires joined \
                     to each other are one body, so each part of the antenna (an element, a \
                     reflector) stays a separate object in your CAD program.",
                )
                .weak(),
            );
            ui.add_space(6.0);
            ui.label("Meshes to write as solid sheets instead of tubes:");
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    for m in &mut self.meshes {
                        let label = format!("{} ({})", m.name, m.kind);
                        match &m.problem {
                            None => {
                                ui.checkbox(&mut m.solid, label);
                            }
                            Some(why) => {
                                ui.add_enabled(false, egui::Checkbox::new(&mut m.solid, label))
                                    .on_disabled_hover_text(format!(
                                        "Can't be a solid sheet: {why}"
                                    ));
                            }
                        }
                    }
                });
            ui.label(
                RichText::new(
                    "Only meshes made with Draw > Plate, Corner reflector, Dish or Grid can be \
                     sheets; other wires are always tubes.",
                )
                .weak(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Sheet thickness");
                ui.radio_value(&mut self.own_thickness, false, "the mesh's wire diameter");
                ui.radio_value(&mut self.own_thickness, true, "");
                ui.add_enabled(
                    self.own_thickness,
                    crate::views::num(&mut self.thickness_mm)
                        .speed(0.05)
                        .range(0.01..=1000.0)
                        .max_decimals(3)
                        .suffix(" mm"),
                );
            });
            if self.meshes.iter().any(|m| m.solid) {
                let list: Vec<String> = self
                    .meshes
                    .iter()
                    .filter(|m| m.solid)
                    .map(|m| format!("{} {:.2} mm", m.name, self.thickness(m) * 1000.0))
                    .collect();
                ui.label(
                    RichText::new(format!(
                        "Thickness: {}. Each sheet is centred on its wires, so nothing moves; \
                         any wires sticking out past a mesh's last whole cell stay tubes.",
                        list.join(", ")
                    ))
                    .weak(),
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Export…").clicked() {
                    outcome = Outcome::Export;
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
        });
        outcome
    }
}
