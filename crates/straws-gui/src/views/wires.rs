//! The wire table: every straight wire with exact, editable numbers, and
//! tools for the selected wires.

use eframe::egui::{self, RichText};
use straws_core::analysis::checks::Severity;
use straws_core::deck::edit::{
    Wire, card_line, clean_field, mirrored_copy, moved_copy, plain_number,
};
use straws_core::deck::material::{
    MATERIALS, WireMaterial, material_line, material_of, skin_depth,
};
use straws_core::deck::write::format_real;
use straws_core::units::{Length, WireSize, awg_diameter, awg_gauge, wavelength};

use crate::app::{Data, Level, Request, Settings, Tab};
use crate::views::newvar;

use super::empty_note;

/// AWG sizes 4nec2 knows by name in a deck (`#0` to `#20`).
const DECK_AWG: std::ops::RangeInclusive<i64> = 0..=20;

#[derive(Default)]
pub struct WiresView {
    /// The cell being typed in: (GW line, field, text so far).
    editing: Option<(usize, usize, String)>,
    copies: usize,
    /// Pieces for the split tool.
    pieces: usize,
    /// Step between copies, in the display unit.
    step: [f64; 3],
    /// Why the last edit was refused, shown until the next one.
    refused: Option<String>,
    /// `Data::edits` last seen: a new edit clears the refusal.
    edits_seen: u64,
    /// Material to apply: 0 perfect, 1..=MATERIALS.len() a listed one,
    /// after that a custom conductivity.
    material: usize,
    custom_sigma: f64,
    hollow: bool,
    /// Tube wall thickness, mm.
    wall_mm: f64,
}

impl WiresView {
    fn chosen_material(&self) -> WireMaterial {
        let conductivity = match self.material {
            0 => None,
            k if k <= MATERIALS.len() => Some(MATERIALS[k - 1].1),
            _ => Some(self.custom_sigma.max(1.0)),
        };
        WireMaterial {
            conductivity,
            wall: (self.hollow && conductivity.is_some()).then_some(self.wall_mm.max(1e-6) * 1e-3),
        }
    }

    fn material_label(&self) -> String {
        match self.material {
            0 => "Perfect conductor".into(),
            k if k <= MATERIALS.len() => MATERIALS[k - 1].0.into(),
            _ => "Custom".into(),
        }
    }
}

/// A tidy number for a cell: enough decimals for the unit, no trailing zeros.
pub fn tidy(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    if s == "-0" { "0".into() } else { s }
}

/// A length in metres shown in `unit`.
pub fn show_length(m: f64, unit: Length, mhz: f64) -> String {
    tidy(unit.from_metres(m, mhz), unit.decimals() + 2)
}

/// Segments for a new wire: about 20 per wavelength at the top of the
/// sweep, odd so there's a centre segment to feed.
pub fn auto_segments(length: f64, max_mhz: f64) -> i64 {
    let n = (length / (wavelength(max_mhz.max(1e-3)) / 20.0)).ceil() as i64;
    let n = n.max(1);
    if n % 2 == 0 { n + 1 } else { n }
}

fn show_size(radius: f64, unit: WireSize) -> String {
    let d = 2.0 * radius;
    match unit {
        WireSize::Millimetres => tidy(d * 1e3, 4),
        WireSize::Inches => tidy(d / 0.0254, 5),
        WireSize::Awg => {
            let g = awg_gauge(d);
            if (g - g.round()).abs() < 0.05 {
                format!("#{}", g.round())
            } else {
                tidy(g, 1)
            }
        }
    }
}

impl WiresView {
    /// What a cell shows.
    fn display(wire: &Wire, field: usize, settings: &Settings, mhz: f64) -> String {
        let text = &wire.fields[field];
        match field {
            2..=7 => match plain_number(text) {
                Some(v) => show_length(
                    v * wire.scale + wire.offset[(field - 2) % 3],
                    settings.length(),
                    mhz,
                ),
                None => text.clone(),
            },
            8 => match plain_number(text) {
                Some(_) => show_size(wire.radius, settings.wire_size()),
                None => text.clone(),
            },
            _ => text.clone(),
        }
    }

    /// The deck field for what was typed in a cell.
    fn to_field(wire: &Wire, field: usize, typed: &str, settings: &Settings, mhz: f64) -> String {
        let typed = clean_field(typed);
        let number = plain_number(&typed);
        match (field, number) {
            (2..=7, Some(v)) => format_real(
                (settings.length().to_metres(v, mhz) - wire.offset[(field - 2) % 3]) / wire.scale,
            ),
            (8, _) => {
                let gauge = typed.trim_start_matches('#');
                let radius = match settings.wire_size() {
                    WireSize::Awg => match gauge.parse::<f64>() {
                        Ok(g) if g.fract() == 0.0 && DECK_AWG.contains(&(g as i64)) => {
                            return format!("#{}", g as i64);
                        }
                        Ok(g) => Some(awg_diameter(g) / 2.0),
                        Err(_) => None,
                    },
                    WireSize::Millimetres => number.map(|d| d / 2.0 * 1e-3),
                    WireSize::Inches => number.map(|d| d / 2.0 * 0.0254),
                };
                match radius {
                    Some(r) => format_real(r / wire.scale),
                    // A formula or 4nec2 size like #12: the radius as written.
                    None => typed,
                }
            }
            _ => typed,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &mut Settings) {
        if data.edits != self.edits_seen {
            self.edits_seen = data.edits;
            self.refused = None;
        }
        if data.model.is_none() {
            return empty_note(
                ui,
                "Open a deck, start from a template, or draw a new antenna (File menu).",
            );
        }
        let mhz = data.mhz.max(1e-3);
        self.tools(ui, data, settings);
        mechanics_line(ui, data);
        ui.separator();
        let Some(model) = &data.model else { return };
        if let Err(e) = &model.numeric {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("The deck can't be read, so its wires can't be listed: {e}"),
            );
            if ui
                .button("Undo the last change")
                .on_hover_text("Or fix the marked line in the Deck view")
                .clicked()
            {
                data.request_undo = true;
            }
            return;
        }
        if let Some(why) = &self.refused {
            ui.colored_label(ui.visuals().warn_fg_color, why);
        }
        let unit = settings.length().symbol();
        let columns: [(&str, f32, String); 12] = [
            (
                "",
                30.0,
                "Click to select, Ctrl-click to add to the selection".into(),
            ),
            ("Tag", 44.0, "Wire number (tag)".into()),
            ("Segs", 44.0, "Number of segments".into()),
            ("X1", 74.0, format!("Start point, {unit}")),
            ("Y1", 74.0, format!("Start point, {unit}")),
            ("Z1", 74.0, format!("Start point, {unit}")),
            ("X2", 74.0, format!("End point, {unit}")),
            ("Y2", 74.0, format!("End point, {unit}")),
            ("Z2", 74.0, format!("End point, {unit}")),
            (
                "Size",
                66.0,
                format!("Wire {}", settings.wire_size().label()),
            ),
            ("Length", 74.0, format!("Wire length, {unit}")),
            (
                "Material",
                150.0,
                "What the wire is made of (set below the tools)".into(),
            ),
        ];
        let row_height = ui.spacing().interact_size.y + 2.0;
        ui.horizontal(|ui| {
            for (name, w, tip) in &columns {
                ui.add_sized(
                    [*w, row_height],
                    egui::Label::new(RichText::new(*name).strong()),
                )
                .on_hover_text(tip.as_str());
            }
        });
        let wires = model.wires.clone();
        let issues: Vec<(i64, Severity, String)> = data
            .checks
            .iter()
            .flat_map(|i| {
                i.tags
                    .iter()
                    .map(|&t| (i64::from(t), i.severity, i.message.clone()))
            })
            .collect();
        let mut commit: Option<(usize, usize, String)> = None;
        let mut clicked: Option<(usize, bool)> = None;
        let mut new_variable = None;
        let mut show_variables = false;
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        egui::ScrollArea::both()
            .auto_shrink(false)
            .show_rows(ui, row_height, wires.len(), |ui, rows| {
                for row in rows {
                    let wire = &wires[row];
                    let selected = data.selection.contains(&wire.line);
                    ui.horizontal(|ui| {
                        let r = ui.add_sized(
                            [columns[0].1, row_height],
                            egui::Button::selectable(selected, format!("{}", row + 1)),
                        );
                        if r.clicked() {
                            let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                            clicked = Some((wire.line, add));
                        }
                        for field in 0..9 {
                            let shown = Self::display(wire, field, settings, mhz);
                            let mine = matches!(&self.editing, Some((l, f, _)) if *l == wire.line && *f == field);
                            let mut text = match &self.editing {
                                Some((_, _, t)) if mine => t.clone(),
                                _ => shown.clone(),
                            };
                            let formula = field >= 2 && !wire.is_number(field);
                            // A fixed id per cell: clicking the last visible row
                            // scrolls it into view, and an id from the layout
                            // would then belong to the row above.
                            let mut edit = egui::TextEdit::singleline(&mut text)
                                .id(egui::Id::new(("wire cell", row, field)));
                            if formula {
                                edit = edit.text_color(ui.visuals().hyperlink_color);
                            }
                            let r = ui.add_sized([columns[field + 1].1, row_height], edit);
                            if field >= 1 {
                                r.context_menu(|ui| {
                                    if wire.is_number(field) {
                                        if ui
                                            .button("Make this a variable…")
                                            .on_hover_text("Its own slider in the Variables view, and a name you can use in other numbers")
                                            .clicked()
                                        {
                                            new_variable = Some(newvar::field_request(&model.deck, wire.line, field, Some(wire)));
                                            ui.close();
                                        }
                                    } else {
                                        let names = straws_core::deck::expr::names_in(&wire.fields[field]);
                                        ui.label(format!("Set by {}", names.join(", ")));
                                        if ui.button("Show the variables").clicked() {
                                            show_variables = true;
                                            ui.close();
                                        }
                                    }
                                });
                            }
                            if r.gained_focus() {
                                self.editing = Some((wire.line, field, shown.clone()));
                            } else if r.has_focus() {
                                self.editing = Some((wire.line, field, text.clone()));
                            }
                            if r.lost_focus() {
                                if !escape && text.trim() != shown && !text.trim().is_empty() {
                                    let value = Self::to_field(wire, field, &text, settings, mhz);
                                    commit = Some((wire.line, field, value));
                                }
                                self.editing = None;
                            }
                            if formula {
                                let value = match field {
                                    8 => show_size(wire.radius, settings.wire_size()),
                                    f => {
                                        let (end, k) = ((f - 2) / 3, (f - 2) % 3);
                                        format!(
                                            "{} {}",
                                            show_length(wire.end(end)[k], settings.length(), mhz),
                                            settings.length().symbol()
                                        )
                                    }
                                };
                                r.on_hover_text(format!(
                                    "Formula = {value}. It's in the deck's own units; type a number to replace it."
                                ));
                            }
                        }
                        ui.add_sized(
                            [columns[10].1, row_height],
                            egui::Label::new(show_length(wire.length(), settings.length(), mhz)),
                        );
                        if let Ok(n) = &model.numeric {
                            let m = material_of(n, wire.tag);
                            let text = match m.wall {
                                Some(t) => format!("{}, tube {} mm wall", m.name(), tidy(t * 1e3, 3)),
                                None => m.name(),
                            };
                            let r = ui.add_sized(
                                [columns[11].1, row_height],
                                egui::Label::new(text).truncate().sense(egui::Sense::click()),
                            );
                            match material_line(n, wire.tag) {
                                Some(ld) => {
                                    let all = model
                                        .deck
                                        .lines
                                        .get(ld)
                                        .is_some_and(|l| matches!(&l.kind, straws_core::deck::LineKind::Card { fields, .. } if fields.get(1).is_some_and(|t| t == "0")));
                                    let whose = if all {
                                        "every wire's".to_string()
                                    } else {
                                        format!("wire {}'s", wire.tag)
                                    };
                                    r.on_hover_text("Right-click to make the conductivity or wall thickness a variable")
                                        .context_menu(|ui| {
                                            if ui.button("Make the conductivity a variable…").clicked() {
                                                new_variable = Some(newvar::Request::new(
                                                    &model.deck,
                                                    newvar::Kind::Field { line: ld, field: 4 },
                                                    "sigma",
                                                    "S/m",
                                                    format!("The conductivity of {whose} metal becomes a variable."),
                                                ));
                                                ui.close();
                                            }
                                            if m.wall.is_some()
                                                && ui.button("Make the wall thickness a variable…").clicked()
                                            {
                                                new_variable = Some(newvar::Request::new(
                                                    &model.deck,
                                                    newvar::Kind::Wall { line: ld },
                                                    "wall_thickness",
                                                    "m",
                                                    format!("The wall thickness of {whose} tube becomes a variable."),
                                                ));
                                                ui.close();
                                            }
                                        });
                                }
                                None => {
                                    r.on_hover_text("A perfect conductor. Choose a material below the tools to give it a conductivity (which can then be a variable).");
                                }
                            }
                        }
                        if wire.moved {
                            ui.label("↻").on_hover_text(
                                "A GM card moves this wire after it's defined; the numbers here are before the move.",
                            );
                        }
                        let worst = issues
                            .iter()
                            .filter(|(t, _, _)| *t == wire.tag)
                            .max_by_key(|(_, s, _)| *s);
                        if let Some((_, s, msg)) = worst {
                            let (mark, colour) = match s {
                                Severity::Error => ("✖", ui.visuals().error_fg_color),
                                Severity::Warning => ("⚠", ui.visuals().warn_fg_color),
                            };
                            ui.colored_label(colour, mark).on_hover_text(msg.as_str());
                        }
                    });
                }
                let others = model.numeric.as_ref().map_or(0, |n| {
                    n.cards[..n.control_start()]
                        .iter()
                        .filter(|c| matches!(c.mnemonic.as_str(), "GA" | "GH" | "SP" | "SM" | "GM" | "GX" | "GR" | "GF"))
                        .count()
                });
                if others > 0 {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "The deck also has {others} other geometry card{} (arcs, helices, patches, copies or moves). Curves and patch boxes drawn in the 3D view are changed there (click one); edit the rest in the Deck view.",
                            if others == 1 { "" } else { "s" }
                        ))
                        .weak(),
                    );
                }
                if wires.is_empty() {
                    ui.label(RichText::new("No straight wires yet. Click Add wire, or draw one in the 3D model view.").weak());
                }
            });
        if new_variable.is_some() {
            data.new_variable = new_variable;
        }
        if show_variables {
            data.request = Some(Request::ShowTab(Tab::Variables));
        }
        if let Some((line, add)) = clicked {
            if add {
                if let Some(k) = data.selection.iter().position(|&l| l == line) {
                    data.selection.remove(k);
                } else {
                    data.selection.push(line);
                }
            } else {
                data.selection = vec![line];
            }
        }
        if let Some((line, field, value)) = commit
            && let Some(m) = &mut data.model
        {
            let before = m.deck.clone();
            m.deck.set_field(line, field, &value);
            match m.keep_if_valid(before, &data.overrides) {
                Ok(()) => data.edit_event = Some(true),
                Err(why) => {
                    let text = format!("That change wasn't made: {why}.");
                    data.message(Level::Warning, text.clone());
                    self.refused = Some(text);
                }
            }
        }
    }

    /// Buttons for adding wires and working on the selected ones.
    fn tools(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &mut Settings) {
        let Some(model) = &mut data.model else { return };
        let mhz = data.mhz.max(1e-3);
        let selected: Vec<Wire> = model
            .wires
            .iter()
            .filter(|w| data.selection.contains(&w.line))
            .cloned()
            .collect();
        let before = model.deck.clone();
        let mut changed = false;
        let mut new_tags: Vec<i64> = Vec::new();
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("length unit")
                .selected_text(settings.length().symbol())
                .width(48.0)
                .show_ui(ui, |ui| {
                    for (i, u) in Length::ALL.iter().enumerate() {
                        ui.selectable_value(&mut settings.length_unit, i, u.symbol());
                    }
                })
                .response
                .on_hover_text("Length unit for the table and the 3D view (λ: wavelengths at the slider frequency)");
            egui::ComboBox::from_id_salt("wire unit")
                .selected_text(settings.wire_size().label())
                .show_ui(ui, |ui| {
                    for (i, u) in WireSize::ALL.iter().enumerate() {
                        ui.selectable_value(&mut settings.wire_unit, i, u.label());
                    }
                })
                .response
                .on_hover_text("How wire sizes are shown and typed");
            ui.separator();
            if ui
                .button("Add wire")
                .on_hover_text("Add a quarter-wave vertical wire at the origin, then type its numbers")
                .clicked()
            {
                let len = wavelength(mhz) / 4.0;
                let tag = model.next_tag();
                let radius = model.wires.first().map_or(0.001, |w| w.radius);
                let lift = model.lift();
                let line = model.deck.add_wire(
                    tag,
                    auto_segments(len, data.max_mhz),
                    [0.0, 0.0, -lift],
                    [0.0, 0.0, (len * 1000.0).round() / 1000.0 - lift],
                    radius,
                );
                data.selection = vec![line];
                changed = true;
            }
            let one = selected.len() == 1;
            if ui
                .add_enabled(one, egui::Button::new("Feed this wire"))
                .on_hover_text("Put the source on the centre segment of the selected wire")
                .clicked()
            {
                let w = &selected[0];
                model.deck.feed_wire(w.tag, w.segments);
                changed = true;
            }
            if ui
                .add_enabled(!selected.is_empty(), egui::Button::new("Delete"))
                .on_hover_text("Delete the selected wires (Del)")
                .clicked()
            {
                model.deck.remove_lines(&data.selection);
                data.selection.clear();
                changed = true;
            }
            self.pieces = self.pieces.max(2);
            if ui
                .add_enabled(one, egui::Button::new("Split"))
                .on_hover_text("Split the selected wire into equal pieces end to end (sources and loads stay on the same segment)")
                .on_disabled_hover_text("Select one wire")
                .clicked()
            {
                let w = &selected[0];
                let tag = model.next_tag();
                match model.deck.split_wire(w, self.pieces, tag) {
                    Ok(()) => {
                        new_tags = std::iter::once(w.tag)
                            .chain(tag..tag + self.pieces as i64 - 1)
                            .collect();
                        changed = true;
                    }
                    Err(why) => self.refused = Some(format!("Not split: {why}.")),
                }
            }
            ui.add(crate::views::num(&mut self.pieces).range(2..=100).suffix(" pieces"))
                .on_hover_text("How many pieces Split makes");
            ui.separator();
            let mut request = None;
            ui.menu_button("Make a variable", |ui| {
                request = newvar::selection_menu(ui, model, &data.selection, mhz, settings);
            })
            .response
            .on_hover_text("Length, spacing, height or angle of the selected wires, driven by a new variable. Or right-click any number in the table.");
            if request.is_some() {
                data.new_variable = request;
            }
        });
        ui.horizontal_wrapped(|ui| {
            let unit = settings.length();
            let any = !selected.is_empty();
            ui.label("Copies");
            ui.add(crate::views::num(&mut self.copies).range(1..=200))
                .on_hover_text("How many copies to make (an array)");
            self.copies = self.copies.max(1);
            for (k, axis) in ["x", "y", "z"].iter().enumerate() {
                ui.label(*axis);
                ui.add(
                    crate::views::num(&mut self.step[k])
                        .speed(0.01)
                        .max_decimals(6)
                        .suffix(format!(" {}", unit.symbol())),
                )
                .on_hover_text("Step between copies");
            }
            if ui
                .add_enabled(any, egui::Button::new("Make copies"))
                .on_hover_text("Copy the selected wires, each copy moved one more step")
                .clicked()
            {
                let step = self.step.map(|v| unit.to_metres(v, mhz));
                if step.iter().all(|s| s.abs() < 1e-12) {
                    self.refused = Some(
                        "Set a step first (x, y or z): with no step every copy would sit on top of its original.".into(),
                    );
                    return;
                }
                let mut tag = model.next_tag();
                // Each copy goes just after its wire, so GS/GM cards treat
                // it the same way. Work from the bottom so lines don't shift.
                let mut by_line = selected.clone();
                by_line.sort_by_key(|w| std::cmp::Reverse(w.line));
                let mut plans = Vec::new();
                for w in &by_line {
                    let mut lines = Vec::new();
                    for n in 1..=self.copies {
                        let d = step.map(|s| s * n as f64);
                        lines.push(card_line("GW", &moved_copy(w, d, tag), None));
                        new_tags.push(tag);
                        tag += 1;
                    }
                    plans.push((w.line + 1, lines));
                }
                for (at, lines) in plans {
                    model.deck.insert_lines(at, &lines);
                }
                changed = true;
            }
            ui.separator();
            ui.label("Mirror in");
            for (k, plane) in ["x = 0", "y = 0", "z = 0"].iter().enumerate() {
                if ui
                    .add_enabled(any, egui::Button::new(*plane))
                    .on_hover_text("Add a mirror-image copy of the selected wires")
                    .clicked()
                {
                    // A wire that's its own mirror image would only be
                    // duplicated; leave those out.
                    let mut by_line: Vec<Wire> = selected
                        .iter()
                        .filter(|w| {
                            let flip = |p: [f64; 3]| {
                                let mut q = p;
                                q[k] = -q[k];
                                q
                            };
                            let same = |p: [f64; 3], q: [f64; 3]| (0..3).all(|i| (p[i] - q[i]).abs() < 1e-9);
                            !((same(flip(w.a), w.a) && same(flip(w.b), w.b))
                                || (same(flip(w.a), w.b) && same(flip(w.b), w.a)))
                        })
                        .cloned()
                        .collect();
                    if by_line.is_empty() {
                        self.refused = Some(format!(
                            "The selected wires are already symmetrical about {plane}: a mirror image would land on top of them."
                        ));
                        return;
                    }
                    by_line.sort_by_key(|w| std::cmp::Reverse(w.line));
                    for (tag, w) in (model.next_tag()..).zip(&by_line) {
                        let line = card_line("GW", &mirrored_copy(w, k, tag), None);
                        model.deck.insert_lines(w.line + 1, &[line]);
                        new_tags.push(tag);
                    }
                    changed = true;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Material");
            egui::ComboBox::from_id_salt("material")
                .selected_text(self.material_label())
                .width(170.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.material, 0, "Perfect conductor")
                        .on_hover_text("No loss at all (NEC's default)");
                    for (k, (name, sigma)) in MATERIALS.iter().enumerate() {
                        ui.selectable_value(&mut self.material, k + 1, *name)
                            .on_hover_text(format!("σ = {sigma:.3e} S/m"));
                    }
                    ui.selectable_value(&mut self.material, MATERIALS.len() + 1, "Custom");
                });
            if self.material > MATERIALS.len() {
                if self.custom_sigma <= 0.0 {
                    self.custom_sigma = 1e7;
                }
                ui.add(
                    crate::views::num(&mut self.custom_sigma)
                        .range(1.0..=1e9)
                        .speed(1e5)
                        .suffix(" S/m"),
                )
                .on_hover_text("Conductivity, siemens per metre");
            }
            let lossy = self.material != 0;
            ui.add_enabled(lossy, egui::Checkbox::new(&mut self.hollow, "Hollow tube"))
                .on_hover_text(
                    "A tube rather than a solid rod; its outside diameter is the wire size",
                );
            if lossy && self.hollow {
                if self.wall_mm <= 0.0 {
                    self.wall_mm = 1.0;
                }
                ui.label("wall");
                ui.add(
                    crate::views::num(&mut self.wall_mm)
                        .range(0.001..=100.0)
                        .speed(0.01)
                        .max_decimals(4)
                        .suffix(" mm"),
                )
                .on_hover_text("Wall thickness");
            }
            let target = if selected.is_empty() {
                "all wires"
            } else {
                "selected wires"
            };
            if ui
                .button(format!("Apply to {target}"))
                .on_hover_text("Writes LD 5 (conductivity) cards for these wires")
                .clicked()
            {
                // Curves are wires too.
                let all: Vec<i64> = model
                    .wires
                    .iter()
                    .map(|w| w.tag)
                    .chain(
                        straws_core::deck::curves::curves(&model.deck)
                            .iter()
                            .map(|c| c.tag),
                    )
                    .collect();
                let tags: Vec<i64> = if selected.is_empty() {
                    all.clone()
                } else {
                    selected.iter().map(|w| w.tag).collect()
                };
                model.deck.set_material(&tags, &all, self.chosen_material());
                changed = true;
            }
            if let Some(sigma) = self.chosen_material().conductivity {
                let delta = skin_depth(mhz, sigma);
                let mut text = format!("Skin depth {} µm at {:.3} MHz", tidy(delta * 1e6, 1), mhz);
                if self.hollow {
                    let wall = self.wall_mm * 1e-3;
                    text.push_str(if wall >= 3.0 * delta {
                        ": the wall is thick enough to act like solid wire."
                    } else {
                        ": the wall is thin enough to matter, and is modelled as a tube."
                    });
                }
                ui.label(RichText::new(text).weak());
            }
        });
        if changed {
            if let Err(why) = model.keep_if_valid(before, &data.overrides) {
                let text = format!("That change wasn't made: {why}.");
                data.message(Level::Warning, text.clone());
                self.refused = Some(text);
                return;
            }
            if !new_tags.is_empty() {
                data.selection = model
                    .wires
                    .iter()
                    .filter(|w| new_tags.contains(&w.tag))
                    .map(|w| w.line)
                    .collect();
            }
            data.edit_event = Some(true);
        }
    }
}

/// The wires' weight and wind load, in one line (wind speed set here).
fn mechanics_line(ui: &mut egui::Ui, data: &Data) {
    use straws_core::analysis::mechanics::{mechanics, wind_force};
    let Some(model) = &data.model else { return };
    let Ok(n) = &model.numeric else { return };
    if model.wires.is_empty() {
        return;
    }
    let shield: Vec<i64> = straws_core::deck::shield::shield_tag(&model.deck)
        .into_iter()
        .collect();
    let m = mechanics(&model.wires, n, &shield);
    let id = ui.id().with("wind speed");
    let mut kmh = ui.data(|d| d.get_temp::<f64>(id)).unwrap_or(100.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "Weight about {:.2} kg{}; wind area {:.3} m²; at",
            m.mass,
            if m.unknown > 0 {
                format!(" ({} wire{} with no material (perfect conductor) or an unusual one left out)", m.unknown, if m.unknown == 1 { "" } else { "s" })
            } else {
                String::new()
            },
            m.area
        ))
        .on_hover_text(
            "Straight wires only (not curves or meshes' extra cards), from each wire's material density, \
             solid or a tube with its wall. Wind area: length x diameter, every wire broadside (the worst case)",
        );
        ui.add(crate::views::num(&mut kmh).range(1.0..=400.0).speed(1.0).suffix(" km/h"));
        ui.label(format!("the wind pushes about {:.0} N ({:.1} kgf)", wind_force(m.area, kmh / 3.6), wind_force(m.area, kmh / 3.6) / 9.80665))
            .on_hover_text("½ ρ v² Cd A with air at 1.225 kg/m³ and Cd 1.2 for round tubes; add the mast, boom clamps and safety margins yourself");
    });
    ui.data_mut(|d| d.insert_temp(id, kmh));
}
