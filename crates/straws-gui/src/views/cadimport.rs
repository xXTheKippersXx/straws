//! Importing a wire-frame drawing (DXF) or a solid model's tubes (STEP or
//! STL): the choices before the wires go in.

use eframe::egui::{self, RichText};
use straws_core::deck::reflectors::ReflectorSpec;
use straws_core::dxf::{self, Drawing, DrawnWire, WireOptions};
use straws_core::sheetfit::{Shape, SheetFit};
use straws_core::{step, stl};

/// Units a drawing may be in: name and metres per unit.
const UNITS: [(&str, f64); 6] = [
    ("metres", 1.0),
    ("centimetres", 0.01),
    ("millimetres", 0.001),
    ("inches", 0.0254),
    ("feet", 0.3048),
    ("yards", 0.9144),
];

/// The import being set up (the dialog is open while this exists).
pub struct CadImport {
    pub file: String,
    text: String,
    drawing: Drawing,
    /// A solid model's parts (STEP or STL), instead of a drawing.
    solid: Option<stl::Parts>,
    /// Each tube keeps its own diameter (solid models).
    own_diameter: bool,
    /// Index into `UNITS`.
    units: usize,
    /// The file said which units it's in.
    units_from_file: bool,
    /// (layer, shapes, import it).
    layers: Vec<(String, usize, bool)>,
    diameter_mm: f64,
    join_mm: f64,
    /// Largest turn (degrees) of one straight piece of a curve.
    step_deg: f64,
    /// Highest frequency the model will be used at (for segment lengths).
    pub mhz: f64,
    /// Start a new model rather than adding to the open one.
    pub new_model: bool,
    /// Wires as the choices stand, and what they were worked out from.
    preview: Vec<DrawnWire>,
    preview_key: Option<(usize, u64, u64, Vec<bool>)>,
    error: Option<String>,
    /// Thin solid sheets in a STEP model (plates, corner reflectors,
    /// dishes), which can come in as reflector meshes.
    sheets: Vec<SheetChoice>,
}

/// A solid sheet and what to do with it.
struct SheetChoice {
    /// In the file's units.
    fit: SheetFit,
    /// Bring it in as a reflector mesh.
    import: bool,
    /// Cells across and up, once changed by hand.
    cells: Option<[usize; 2]>,
}

/// What the dialog wants done.
pub enum Outcome {
    Open,
    Cancel,
    /// Add these wires (metres), with this radius (m) where a wire has no
    /// radius of its own, and this frequency (MHz).
    Import {
        wires: Vec<DrawnWire>,
        radius: f64,
        mhz: f64,
        new_model: bool,
        /// Reflector meshes for the solid sheets (names to be given).
        reflectors: Vec<ReflectorSpec>,
    },
}

impl CadImport {
    pub fn new(file: String, bytes: &[u8], mhz: f64, has_model: bool) -> Result<CadImport, String> {
        let step_deg = 10.0;
        let lower = file.to_ascii_lowercase();
        let is_step = lower.ends_with(".step") || lower.ends_with(".stp");
        if is_step || lower.ends_with(".stl") {
            let (mut parts, mut unit, mut sheets) = if is_step {
                let read = step::read_step(&String::from_utf8_lossy(bytes))?;
                (read.parts, read.metres_per_unit, read.sheets)
            } else {
                (stl::tubes(&stl::read_stl(bytes)?), None, Vec::new())
            };
            if parts.tubes.is_empty() && sheets.is_empty() {
                return Err(format!(
                    "no straight round tubes or thin sheets in it ({} flat pieces, {} other shapes)",
                    parts.flat, parts.other
                ));
            }
            // A unit not on the list (kilometres, mils): the tubes in metres.
            if let Some(m) = unit
                && !UNITS.iter().any(|u| (u.1 - m).abs() < 1e-12)
            {
                for t in &mut parts.tubes {
                    t.a = t.a.map(|v| v * m);
                    t.b = t.b.map(|v| v * m);
                    t.radius *= m;
                }
                for s in &mut sheets {
                    *s = s.scaled(m);
                }
                unit = Some(1.0);
            }
            let count = parts.tubes.len();
            let units = unit
                .and_then(|m| UNITS.iter().position(|u| (u.1 - m).abs() < 1e-12))
                // STL files don't say; CAD programs mostly use millimetres.
                .unwrap_or(2);
            return Ok(CadImport {
                file,
                text: String::new(),
                drawing: Drawing::default(),
                solid: Some(parts),
                own_diameter: true,
                units,
                units_from_file: unit.is_some(),
                // One entry a tube (the count is its index), all ticked.
                layers: (0..count)
                    .map(|k| (format!("tube {}", k + 1), k, true))
                    .collect(),
                diameter_mm: 2.0,
                join_mm: 1.0,
                step_deg,
                mhz,
                new_model: !has_model,
                preview: Vec::new(),
                preview_key: None,
                error: None,
                sheets: sheets
                    .into_iter()
                    .map(|fit| SheetChoice {
                        fit,
                        import: true,
                        cells: None,
                    })
                    .collect(),
            });
        }
        let text = String::from_utf8_lossy(bytes).into_owned();
        let drawing = dxf::read_dxf(&text, step_deg)?;
        if drawing.paths.is_empty() {
            let skipped = if drawing.skipped.is_empty() {
                "nothing at all".to_string()
            } else {
                drawing
                    .skipped
                    .iter()
                    .map(|(k, n)| format!("{n} {k}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            return Err(format!(
                "no lines, polylines, arcs, circles, ellipses or splines in it (found {skipped})"
            ));
        }
        let units_from_file = drawing.metres_per_unit.is_some();
        let units = drawing
            .metres_per_unit
            .and_then(|m| UNITS.iter().position(|u| (u.1 - m).abs() < 1e-12))
            .unwrap_or(2); // millimetres: what most CAD drawings use
        let layers = drawing
            .layers()
            .into_iter()
            .map(|(n, c)| (n, c, true))
            .collect();
        Ok(CadImport {
            file,
            text,
            drawing,
            solid: None,
            own_diameter: false,
            units,
            units_from_file,
            layers,
            diameter_mm: 2.0,
            join_mm: 1.0,
            step_deg,
            mhz,
            new_model: !has_model,
            preview: Vec::new(),
            preview_key: None,
            error: None,
            sheets: Vec::new(),
        })
    }

    /// A sheet in metres, and its cells: as chosen, or a mesh of a
    /// thirteenth of a wavelength at the highest frequency (as Draw > Dish
    /// makes).
    fn sheet_in_metres(&self, k: usize) -> (SheetFit, [usize; 2]) {
        let s = &self.sheets[k];
        let fit = s.fit.scaled(UNITS[self.units].1);
        let mesh = 299.792458 / self.mhz.max(1e-3) / 13.0;
        let cells = s.cells.unwrap_or_else(|| fit.cells(mesh));
        (fit, cells)
    }

    fn options(&self) -> WireOptions {
        WireOptions {
            metres_per_unit: UNITS[self.units].1,
            join_m: self.join_mm / 1000.0,
            layers: self
                .layers
                .iter()
                .filter(|l| l.2)
                .map(|l| l.0.clone())
                .collect(),
        }
    }

    /// Works the wires out again when a choice that changes them changed.
    fn refresh(&mut self) {
        let key = (
            self.units,
            self.join_mm.to_bits(),
            self.step_deg.to_bits(),
            self.layers.iter().map(|l| l.2).collect(),
        );
        if self.preview_key.as_ref() == Some(&key) {
            return;
        }
        if self.preview_key.as_ref().is_some_and(|k| k.2 != key.2) {
            match dxf::read_dxf(&self.text, self.step_deg) {
                Ok(d) => self.drawing = d,
                Err(e) => self.error = Some(e),
            }
        }
        if let Some(parts) = &self.solid {
            let mut wires = stl::wires(&parts.tubes, UNITS[self.units].1, self.join_mm / 1000.0);
            wires.retain(|w| self.layers.iter().any(|l| l.2 && l.0 == w.layer));
            self.preview = wires;
            self.preview_key = Some(key);
            return;
        }
        let mut options = self.options();
        if self.layers.iter().all(|l| !l.2) {
            options.layers = vec![String::new()]; // none: nothing matches
        }
        self.preview = dxf::wires(&self.drawing, &options);
        self.preview_key = Some(key);
    }

    pub fn ui(&mut self, ctx: &egui::Context, has_model: bool) -> Outcome {
        self.refresh();
        let mut outcome = Outcome::Open;
        egui::Modal::new(egui::Id::new("import drawing")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading(format!("Import {}", self.file));
            let solid = self.solid.is_some();
            ui.label(
                RichText::new(if solid {
                    "Each straight round tube in the model becomes a wire along its middle, with \
                     the tube's diameter. Ends closer than the join distance become one point, so \
                     tubes that meet end to end are joined."
                } else {
                    "Lines, polylines, arcs, circles, ellipses and splines become wires; curves are \
                     cut into straight pieces. Ends closer than the join distance become one point, \
                     so wires that touch in the drawing are joined."
                })
                .weak(),
            );
            ui.add_space(6.0);
            egui::Grid::new("import grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Drawn in");
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("import units")
                            .selected_text(UNITS[self.units].0)
                            .show_ui(ui, |ui| {
                                for (k, u) in UNITS.iter().enumerate() {
                                    ui.selectable_value(&mut self.units, k, u.0);
                                }
                            });
                        ui.label(
                            RichText::new(if self.units_from_file {
                                "(as the file says)"
                            } else {
                                "(the file doesn't say: check the size below)"
                            })
                            .weak(),
                        );
                    });
                    ui.end_row();
                    ui.label("Wire diameter");
                    ui.horizontal(|ui| {
                        if solid {
                            ui.checkbox(&mut self.own_diameter, "each tube's own");
                        }
                        ui.add_enabled(
                            !(solid && self.own_diameter),
                            crate::views::num(&mut self.diameter_mm)
                                .speed(0.05)
                                .range(0.01..=1000.0)
                                .max_decimals(3)
                                .suffix(" mm"),
                        );
                    });
                    ui.end_row();
                    ui.label("Join ends within");
                    ui.add(
                        crate::views::num(&mut self.join_mm)
                            .speed(0.05)
                            .range(0.0..=1000.0)
                            .max_decimals(3)
                            .suffix(" mm"),
                    )
                    .on_hover_text("Ends this close together become one point, so the wires join");
                    ui.end_row();
                    if !solid {
                        ui.label("Curves in steps of");
                        ui.add(
                            crate::views::num(&mut self.step_deg)
                                .speed(0.5)
                                .range(1.0..=45.0)
                                .max_decimals(1)
                                .suffix("°"),
                        )
                        .on_hover_text(
                            "Each straight piece of an arc or circle turns at most this much",
                        );
                        ui.end_row();
                    }
                    ui.label("Highest frequency");
                    ui.add(
                        crate::views::num(&mut self.mhz)
                            .speed(0.1)
                            .range(0.001..=100_000.0)
                            .max_decimals(4)
                            .suffix(" MHz"),
                    )
                    .on_hover_text("Sets the segment lengths (about 20 per wavelength)");
                    ui.end_row();
                    ui.label("Into");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.new_model, true, "a new model");
                        ui.add_enabled_ui(has_model, |ui| {
                            ui.radio_value(&mut self.new_model, false, "the open model");
                        });
                    });
                    ui.end_row();
                });
            if self.layers.len() > 1 {
                ui.add_space(4.0);
                ui.label(if solid { "Tubes (untick a boom or mast to leave it out)" } else { "Layers" });
                let mm = UNITS[self.units].1 * 1000.0;
                egui::ScrollArea::vertical()
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for (name, count, on) in &mut self.layers {
                            let text = match &self.solid {
                                Some(parts) => {
                                    let t = parts.tubes[*count];
                                    let long: f64 =
                                        (0..3).map(|i| (t.b[i] - t.a[i]).powi(2)).sum::<f64>().sqrt();
                                    format!(
                                        "{name}: {:.1} mm long, {:.2} mm across",
                                        long * mm,
                                        2.0 * t.radius * mm
                                    )
                                }
                                None => format!("{name} ({count})"),
                            };
                            ui.checkbox(on, text);
                        }
                    });
            }
            if !self.sheets.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Solid sheets").strong());
                ui.label(
                    RichText::new(
                        "These are solid plates, corner reflectors or dishes, not tubes. NEC-2 \
                         models wires, so each can come in as a reflector made of a wire mesh, \
                         its shape and size fitted to the sheet and its wires as thick as the \
                         sheet. Afterwards, select it in the 3D view to change its size, shape \
                         and cells.",
                    )
                    .weak(),
                );
                let len = |m: f64| short(m);
                egui::ScrollArea::vertical()
                    .id_salt("import sheets")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for k in 0..self.sheets.len() {
                            let (fit, cells) = self.sheet_in_metres(k);
                            let shape = match fit.shape {
                                Shape::ParabolicCylinder(f) | Shape::Paraboloid(f) => {
                                    format!("{}, focal length {} m", fit.shape.label(), len(f))
                                }
                                Shape::TwoCurvatures(f, g) => format!(
                                    "{}, focal lengths {} and {} m",
                                    fit.shape.label(),
                                    len(f),
                                    len(g)
                                ),
                                Shape::Cylinder(r) | Shape::Sphere(r) => {
                                    format!("{}, radius {} m", fit.shape.label(), len(r))
                                }
                                Shape::Vee(a) => format!("{} of {a:.1}°", fit.shape.label()),
                                _ => fit.shape.label().to_string(),
                            };
                            let choice = &mut self.sheets[k];
                            ui.checkbox(
                                &mut choice.import,
                                format!(
                                    "Sheet {}: {shape}; {} x {} m, {:.2} mm thick, facing azimuth {:.1}°, elevation {:.1}°",
                                    k + 1,
                                    len(fit.size[0]),
                                    len(fit.size[1]),
                                    fit.thickness * 1000.0,
                                    straws_core::sheetfit::snap_angle(fit.facing[0]),
                                    straws_core::sheetfit::snap_angle(fit.facing[1])
                                ),
                            )
                            .on_hover_text("Bring it in as a reflector made of wires");
                            ui.add_enabled_ui(choice.import, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_space(24.0);
                                    ui.label("Cells");
                                    let mut c = cells;
                                    let across = ui.add(
                                        egui::DragValue::new(&mut c[0]).range(2..=80).suffix(" across"),
                                    );
                                    let up = ui.add(
                                        egui::DragValue::new(&mut c[1]).range(2..=80).suffix(" up"),
                                    );
                                    if across.changed() || up.changed() {
                                        choice.cells = Some(c);
                                    }
                                    ui.label(
                                        RichText::new(format!(
                                            "about {:.0} mm each; wire {:.2} mm",
                                            1000.0 * fit.size[0].max(fit.size[1])
                                                / c[0].max(c[1]) as f64,
                                            fit.thickness * 1000.0
                                        ))
                                        .weak(),
                                    );
                                });
                            });
                        }
                    });
            }
            ui.add_space(6.0);
            // What will come in: count and overall size, to catch wrong units.
            let n = self.preview.len();
            let sheets_in = self.sheets.iter().filter(|s| s.import).count();
            if n == 0 && sheets_in > 0 {
                ui.label(format!(
                    "{sheets_in} reflector{} from the sheets.",
                    if sheets_in == 1 { "" } else { "s" }
                ));
            } else if n == 0 {
                ui.colored_label(ui.visuals().warn_fg_color, "Nothing to import with these choices.");
            } else {
                let mut lo = [f64::MAX; 3];
                let mut hi = [f64::MIN; 3];
                let mut shortest = f64::MAX;
                for w in &self.preview {
                    for p in [w.a, w.b] {
                        for i in 0..3 {
                            lo[i] = lo[i].min(p[i]);
                            hi[i] = hi[i].max(p[i]);
                        }
                    }
                    let d: f64 = (0..3).map(|i| (w.b[i] - w.a[i]).powi(2)).sum::<f64>().sqrt();
                    shortest = shortest.min(d);
                }
                ui.label(format!(
                    "{n} wire{}, {} x {} x {} m overall, the shortest {:.1} mm.",
                    if n == 1 { "" } else { "s" },
                    short(hi[0] - lo[0]),
                    short(hi[1] - lo[1]),
                    short(hi[2] - lo[2]),
                    shortest * 1000.0
                ));
                let thickest = if solid && self.own_diameter {
                    self.preview
                        .iter()
                        .filter_map(|w| w.radius)
                        .fold(0.0, f64::max)
                        * 2.0
                } else {
                    self.diameter_mm / 1000.0
                };
                if shortest < 2.0 * thickest {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Some pieces are shorter than twice the wire's thickness: NEC handles them \
                         badly. Larger curve steps, or a larger join distance, make fewer of them.",
                    );
                }
            }
            if let Some(parts) = &self.solid
                && parts.flat + parts.other > 0
            {
                ui.label(
                    RichText::new(if self.sheets.is_empty() {
                        format!(
                            "Left out: {} flat pieces (tube ends, plates, faces of blocks) and {} \
                             other shapes (bent tubes, dishes). Model plates and dishes with Draw > \
                             Plate or Dish instead.",
                            parts.flat, parts.other
                        )
                    } else {
                        format!(
                            "Left out: {} flat pieces (tube ends, faces of blocks) and {} other \
                             shapes (bent tubes, thick parts).",
                            parts.flat, parts.other
                        )
                    })
                    .weak(),
                );
            }
            if !self.drawing.skipped.is_empty() {
                let list: Vec<String> = self
                    .drawing
                    .skipped
                    .iter()
                    .map(|(k, c)| format!("{c} {k}"))
                    .collect();
                ui.label(
                    RichText::new(format!(
                        "Left out (not wires): {}. Block references (INSERT) aren't read: explode them in your CAD program first.",
                        list.join(", ")
                    ))
                    .weak(),
                );
            }
            ui.label(
                RichText::new(if self.sheets.is_empty() {
                    "Only wires: NEC-2 can't model solid parts, dielectrics or plastic. Draw a \
                     solid reflector as a wire grid."
                } else {
                    "Only wires: NEC-2 can't model solid parts, dielectrics or plastic, which is \
                     why sheets come in as wire meshes."
                })
                .weak(),
            );
            if let Some(e) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(n > 0 || sheets_in > 0, egui::Button::new("Import"))
                    .clicked()
                {
                    let reflectors = (0..self.sheets.len())
                        .filter(|k| self.sheets[*k].import)
                        .map(|k| {
                            let (fit, cells) = self.sheet_in_metres(k);
                            fit.reflector("", cells)
                        })
                        .collect();
                    let mut wires = std::mem::take(&mut self.preview);
                    if !(solid && self.own_diameter) {
                        for w in &mut wires {
                            w.radius = None;
                        }
                    }
                    self.preview_key = None;
                    outcome = Outcome::Import {
                        wires,
                        radius: self.diameter_mm / 2000.0,
                        mhz: self.mhz,
                        new_model: self.new_model || !has_model,
                        reflectors,
                    };
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancel;
                }
            });
        });
        outcome
    }
}

/// A size in metres with sensible figures.
fn short(m: f64) -> String {
    if m >= 10.0 {
        format!("{m:.1}")
    } else if m >= 0.1 {
        format!("{m:.3}")
    } else {
        format!("{m:.4}")
    }
}
