//! The deck's SY variables: a live slider for each number-valued one, and
//! an editor to add, rename, describe, group, re-range or delete them,
//! without touching the deck text.

use eframe::egui::{self, RichText};
use straws_core::deck::vars::VarNotes;
use straws_core::deck::write::format_real;
use straws_core::deck::{Deck, LineKind, Variable};
use straws_core::units::Length;

use crate::app::{Data, Level, Settings};
use crate::views::newvar::{self, Kind, Request, field_label};

use super::empty_note;

/// Units offered in the editor (deck value, label).
const UNITS: &[(&str, &str)] = &[
    ("", "Plain number"),
    ("m", "Length (metres, shown in your unit)"),
    ("mm", "Millimetres"),
    ("deg", "Degrees"),
    ("MHz", "MHz"),
    ("ohm", "Ohms"),
    ("pF", "pF"),
    ("nH", "nH"),
];

#[derive(Default, PartialEq, Clone, Copy)]
enum Order {
    #[default]
    Deck,
    Name,
}

/// The editor's working copy of one variable.
struct Editing {
    /// The variable's name when editing began.
    original: String,
    name: String,
    expr: String,
    notes: VarNotes,
    own_range: bool,
    own_step: bool,
    /// Asked to confirm a delete that writes the value back in.
    confirm_delete: bool,
}

#[derive(Default)]
pub struct VariablesView {
    editing: Option<Editing>,
    order: Order,
    /// Why the last change was refused.
    refused: Option<String>,
}

/// How a variable's value is shown: (value in shown units, suffix,
/// shown units per deck unit).
pub fn shown(value: f64, unit: &str, settings: &Settings, mhz: f64) -> (f64, String, f64) {
    match unit {
        "m" => {
            let u: Length = settings.length();
            let per = u.from_metres(1.0, mhz);
            (value * per, format!(" {}", u.symbol()), per)
        }
        "deg" => (value, "°".into(), 1.0),
        "ohm" => (value, " Ω".into(), 1.0),
        "" => (value, String::new(), 1.0),
        other => (value, format!(" {other}"), 1.0),
    }
}

/// The current value of `name`: an override if there is one, else the deck's.
fn current(data: &Data, v: &Variable) -> Option<f64> {
    data.overrides
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(&v.name))
        .map(|(_, x)| *x)
        .or(v.literal)
}

/// Every variable's worked-out value.
fn values(data: &Data) -> Vec<(String, f64)> {
    data.model
        .as_ref()
        .and_then(|m| m.numeric.as_ref().ok())
        .map_or_else(Vec::new, |n| n.symbols.clone())
}

fn value_of(values: &[(String, f64)], name: &str) -> Option<f64> {
    values
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, x)| *x)
}

/// Selects the wires that depend on `name` (so the 3D view shows them).
fn show_uses(data: &mut Data, name: &str) {
    let Some(m) = &data.model else { return };
    let lines = m.deck.uses_through(name);
    data.selection = m
        .wires
        .iter()
        .filter(|w| lines.contains(&w.line))
        .map(|w| w.line)
        .collect();
}

/// A short "where it's used" summary.
fn uses_text(deck: &Deck, name: &str, data: &Data) -> String {
    let lines = deck.uses_through(name);
    let view = !deck.notes(name).role.is_empty();
    if lines.is_empty() {
        return if view {
            "Used by the Feedline view.".into()
        } else {
            "Not used by any card yet.".into()
        };
    }
    let cards: Vec<i64> = data.model.as_ref().map_or_else(Vec::new, |m| {
        m.wires
            .iter()
            .filter(|w| lines.contains(&w.line))
            .map(|w| w.tag)
            .collect()
    });
    let other = lines.len() - cards.len();
    // Each tag once; a long list (a mesh's hundreds of cards) as a range.
    let mut tags = cards.clone();
    tags.sort_unstable();
    tags.dedup();
    let mut parts = Vec::new();
    if tags.len() > 10 {
        parts.push(format!(
            "{} cards on wires {}-{}",
            cards.len(),
            tags[0],
            tags[tags.len() - 1]
        ));
    } else if !tags.is_empty() {
        parts.push(format!(
            "wire{} {}",
            if tags.len() == 1 { "" } else { "s" },
            tags.iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if other > 0 {
        parts.push(format!(
            "{other} {}card{}",
            if cards.is_empty() { "" } else { "other " },
            if other == 1 { "" } else { "s" }
        ));
    }
    if view {
        parts.push("the Feedline view".into());
    }
    format!("Used by {}.", parts.join(" and "))
}

impl VariablesView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
        let Some(model) = &data.model else {
            return empty_note(ui, "Open a model to see its variables.");
        };
        let deck = model.deck.clone();
        let mut vars = deck.variables();
        if self.order == Order::Name {
            vars.sort_by_key(|v| v.name.to_ascii_lowercase());
        }
        let worked = values(data);
        let mhz = data.mhz.max(1e-3);

        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Add variable")
                .on_hover_text("A new variable with a value you type; then use its name in any number")
                .clicked()
            {
                let r = Request::new(
                    &deck,
                    Kind::Plain {
                        value: "1".into(),
                    },
                    "var",
                    "",
                    "A new variable. Type its name into any number (in the wire table, for example) to use it.".into(),
                );
                data.new_variable = Some(r);
            }
            ui.menu_button("From the selected wires", |ui| {
                if let Some(m) = &data.model
                    && let Some(r) = newvar::selection_menu(ui, m, &data.selection, data.mhz, settings)
                {
                    data.new_variable = Some(r);
                }
            })
            .response
            .on_hover_text("Length, spacing, height or angle of the selected wires, driven by a new variable");
            ui.separator();
            ui.label("Order");
            ui.selectable_value(&mut self.order, Order::Deck, "as in the deck");
            ui.selectable_value(&mut self.order, Order::Name, "by name");
        });
        ui.label(
            RichText::new(
                "Drag a slider and every view follows. Click a name to see where it's used. \
                 Right-click a number in the wire table to make it a variable.",
            )
            .weak(),
        );
        if let Some(why) = &self.refused {
            ui.colored_label(ui.visuals().warn_fg_color, why);
        }
        ui.add_space(4.0);

        let mut event = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                if self.editing.is_some() {
                    self.editor(ui, data, &deck, &worked, settings);
                    ui.separator();
                }
                if vars.is_empty() {
                    ui.label(
                        RichText::new(
                            "No variables yet. Add one above, make one from the selected wires, \
                             or right-click a number in the wire table.",
                        )
                        .weak(),
                    );
                }
                // Groups in the order they first appear; ungrouped first.
                let notes: Vec<VarNotes> = vars.iter().map(|v| deck.notes(&v.name)).collect();
                let mut groups: Vec<String> = vec![String::new()];
                for n in &notes {
                    if !groups.contains(&n.group) {
                        groups.push(n.group.clone());
                    }
                }
                for group in &groups {
                    let members: Vec<usize> = (0..vars.len())
                        .filter(|&i| &notes[i].group == group)
                        .collect();
                    if members.is_empty() {
                        continue;
                    }
                    let mut rows = |ui: &mut egui::Ui| {
                        egui::Grid::new(("variables", group))
                            .num_columns(5)
                            .spacing([10.0, 6.0])
                            .striped(true)
                            .show(ui, |ui| {
                                for &i in &members {
                                    if let Some(e) = self.row(
                                        ui, data, &deck, &vars[i], &notes[i], &worked, settings,
                                        mhz,
                                    ) {
                                        event = Some(event.unwrap_or(false) || e);
                                    }
                                    ui.end_row();
                                }
                            });
                    };
                    if group.is_empty() {
                        rows(ui);
                    } else {
                        egui::CollapsingHeader::new(RichText::new(group).strong())
                            .id_salt(("group", group))
                            .default_open(true)
                            .show(ui, rows);
                    }
                }
                ui.add_space(10.0);
                egui::CollapsingHeader::new("Make a variable from another number in the deck")
                    .id_salt("other numbers")
                    .show(ui, |ui| other_numbers(ui, data, &deck));
            });
        if let Some(final_change) = event {
            data.edit_event = Some(data.edit_event.unwrap_or(false) || final_change);
        }
    }

    /// One variable's row. Returns an edit event, if its value changed.
    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        ui: &mut egui::Ui,
        data: &mut Data,
        deck: &Deck,
        v: &Variable,
        notes: &VarNotes,
        worked: &[(String, f64)],
        settings: &Settings,
        mhz: f64,
    ) -> Option<bool> {
        let mut event = None;
        let mut tip = String::new();
        if !notes.description.is_empty() {
            tip.push_str(&notes.description);
            tip.push('\n');
        }
        tip.push_str(&uses_text(deck, &v.name, data));
        if notes.fixed {
            tip.push_str("\nThe optimiser leaves this one alone.");
        }
        let name_text = if notes.fixed {
            RichText::new(&v.name).strong().italics()
        } else {
            RichText::new(&v.name).strong()
        };
        if ui
            .add(egui::Button::new(name_text).frame(false))
            .on_hover_text(tip)
            .clicked()
        {
            show_uses(data, &v.name);
        }
        match (v.literal, current(data, v)) {
            (Some(base), Some(now)) => {
                let (shown_now, suffix, per) = shown(now, &notes.unit, settings, mhz);
                let (lo, hi) = match notes.range {
                    Some((a, b)) => (a.min(b), a.max(b)),
                    None if base == 0.0 => (-1.0, 1.0),
                    None => {
                        let (a, b) = (base * 0.5, base * 1.5);
                        (a.min(b), a.max(b))
                    }
                };
                let mut value = shown_now;
                let mut slider = egui::Slider::new(
                    &mut value,
                    (lo * per).min(shown_now)..=(hi * per).max(shown_now),
                )
                .show_value(false)
                .clamping(egui::SliderClamping::Never);
                if let Some(step) = notes.step.filter(|s| *s > 0.0) {
                    slider = slider.step_by(step * per);
                }
                let slider = ui.add(slider);
                let speed = notes
                    .step
                    .filter(|s| *s > 0.0)
                    .unwrap_or((base.abs() * 0.001).max(1e-6))
                    * per;
                let exact = ui.add(
                    crate::views::num(&mut value)
                        .speed(speed)
                        .max_decimals(6)
                        .suffix(suffix),
                );
                if value != shown_now {
                    set_override(data, &v.name, value / per, base);
                    // While dragging, show quick results; the full sweep
                    // runs when the slider is let go.
                    event = Some(!(slider.dragged() || exact.dragged()));
                }
                if slider.drag_stopped() || exact.drag_stopped() || exact.lost_focus() {
                    event = Some(true);
                }
                if ui
                    .add_enabled(now != base, egui::Button::new("↺").small())
                    .on_hover_text(format!("Back to the deck's value, {}", v.expr))
                    .clicked()
                {
                    set_override(data, &v.name, base, base);
                    event = Some(true);
                }
            }
            _ => {
                ui.label(RichText::new(format!("= {}", v.expr)).weak())
                    .on_hover_text("Worked out from other variables");
                let (x, suffix, _) = value_of(worked, &v.name)
                    .map_or((f64::NAN, String::new(), 1.0), |x| {
                        shown(x, &notes.unit, settings, mhz)
                    });
                ui.label(if x.is_nan() {
                    "?".to_string()
                } else {
                    format!("{}{suffix}", format_real(x))
                });
                ui.label("");
            }
        }
        if ui
            .small_button("Edit")
            .on_hover_text("Rename, describe, group, set the slider range, or delete")
            .clicked()
        {
            self.start_editing(deck, v);
        }
        event
    }

    fn start_editing(&mut self, deck: &Deck, v: &Variable) {
        let notes = deck.notes(&v.name);
        self.refused = None;
        self.editing = Some(Editing {
            original: v.name.clone(),
            name: v.name.clone(),
            expr: v.expr.clone(),
            own_range: notes.range.is_some(),
            own_step: notes.step.is_some(),
            notes,
            confirm_delete: false,
        });
    }

    /// The editor for one variable, shown above the list.
    fn editor(
        &mut self,
        ui: &mut egui::Ui,
        data: &mut Data,
        deck: &Deck,
        worked: &[(String, f64)],
        settings: &Settings,
    ) {
        let Some(e) = &mut self.editing else { return };
        if !deck.has_variable(&e.original) {
            self.editing = None;
            return;
        }
        let mut close = false;
        let mut apply = false;
        let mut delete = false;
        let base_value = value_of(worked, &e.original).unwrap_or(0.0);
        ui.label(RichText::new(format!("Editing {}", e.original)).strong());
        egui::Grid::new("variable editor")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(200.0));
                    let name = e.name.trim();
                    if !name.eq_ignore_ascii_case(&e.original)
                        && let Err(why) = deck.check_new_name(name)
                    {
                        ui.colored_label(ui.visuals().error_fg_color, why);
                    } else if name != e.original {
                        ui.label(
                            RichText::new("Every formula that uses it is renamed too.").weak(),
                        );
                    }
                });
                ui.end_row();

                ui.label("Value or formula");
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut e.expr).desired_width(200.0))
                        .on_hover_text(
                            "A number (units like 5 ft work), or a formula such as lam/4 or 2*len",
                        );
                    match check_expression(deck, &e.original, &e.expr, &data.overrides) {
                        Ok(x) => {
                            let (x, suffix, _) =
                                shown(x, &e.notes.unit, settings, data.mhz.max(1e-3));
                            ui.label(RichText::new(format!("= {}{suffix}", format_real(x))).weak());
                        }
                        Err(why) => {
                            ui.colored_label(ui.visuals().error_fg_color, why);
                        }
                    }
                });
                ui.end_row();

                ui.label("Description");
                ui.add(
                    egui::TextEdit::singleline(&mut e.notes.description)
                        .hint_text("e.g. Reflector length")
                        .desired_width(260.0),
                );
                ui.end_row();

                ui.label("Group");
                ui.add(
                    egui::TextEdit::singleline(&mut e.notes.group)
                        .hint_text("e.g. Reflector, Boom")
                        .desired_width(200.0),
                )
                .on_hover_text("Variables with the same group are listed together");
                ui.end_row();

                ui.label("Unit");
                let label = UNITS
                    .iter()
                    .find(|(u, _)| *u == e.notes.unit)
                    .map_or(e.notes.unit.as_str(), |(_, l)| l);
                egui::ComboBox::from_id_salt("variable unit")
                    .selected_text(label)
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for (u, l) in UNITS {
                            ui.selectable_value(&mut e.notes.unit, u.to_string(), *l);
                        }
                    })
                    .response
                    .on_hover_text(
                        "Only changes how the value is shown; the deck keeps the plain number",
                    );
                ui.end_row();

                ui.label("Slider range");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut e.own_range, "own range");
                    if e.own_range {
                        let r = e.notes.range.get_or_insert_with(|| {
                            if base_value == 0.0 {
                                (-1.0, 1.0)
                            } else {
                                let (a, b) = (base_value * 0.5, base_value * 1.5);
                                (a.min(b), a.max(b))
                            }
                        });
                        let speed = ((r.1 - r.0).abs() * 0.002).max(1e-6);
                        ui.add(crate::views::num(&mut r.0).speed(speed).max_decimals(6));
                        ui.label("to");
                        ui.add(crate::views::num(&mut r.1).speed(speed).max_decimals(6));
                    } else {
                        ui.label(RichText::new("half to one and a half times the value").weak());
                    }
                })
                .response
                .on_hover_text(
                    "In the deck's own numbers. The optimiser also starts from this range.",
                );
                ui.end_row();

                ui.label("Slider step");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut e.own_step, "own step");
                    if e.own_step {
                        let s = e
                            .notes
                            .step
                            .get_or_insert((base_value.abs() * 0.001).max(1e-6));
                        let speed = *s * 0.05;
                        ui.add(
                            crate::views::num(s)
                                .speed(speed)
                                .range(1e-9..=1e9)
                                .max_decimals(9),
                        );
                    }
                });
                ui.end_row();

                ui.label("Optimiser");
                let mut vary = !e.notes.fixed;
                ui.checkbox(&mut vary, "may vary it").on_hover_text(
                    "Untick to keep the optimiser away from it (it won't be listed there)",
                );
                e.notes.fixed = !vary;
                ui.end_row();

                ui.label("Used by");
                ui.horizontal_wrapped(|ui| {
                    ui.label(uses_text(deck, &e.original, data));
                });
                ui.end_row();
            });
        if !e.own_range {
            e.notes.range = None;
        }
        if !e.own_step {
            e.notes.step = None;
        }
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                apply = true;
            }
            if ui.button("Close").clicked() {
                close = true;
            }
            if ui
                .button("Show in 3D")
                .on_hover_text("Select the wires that use it")
                .clicked()
            {
                show_uses(data, &e.original);
            }
            ui.separator();
            let used = !deck.uses(&e.original).is_empty();
            if e.confirm_delete {
                ui.label(format!(
                    "Write its value ({}) into the cards that use it, then delete it?",
                    format_real(base_value)
                ));
                if ui.button("Yes, delete").clicked() {
                    delete = true;
                }
                if ui.button("No").clicked() {
                    e.confirm_delete = false;
                }
            } else if ui
                .button("Delete")
                .on_hover_text(if used {
                    "Delete it, writing its current value back into the cards that use it"
                } else {
                    "Delete it (nothing uses it)"
                })
                .clicked()
            {
                if used {
                    e.confirm_delete = true;
                } else {
                    delete = true;
                }
            }
        });

        if apply || delete {
            let e = self.editing.as_ref().expect("editing");
            let result = if delete {
                change(data, |d| d.delete_variable(&e.original, Some(base_value)))
            } else {
                let (original, name, expr, notes) = (
                    e.original.clone(),
                    e.name.trim().to_string(),
                    e.expr.clone(),
                    e.notes.clone(),
                );
                let old_expr = deck
                    .variables()
                    .into_iter()
                    .find(|v| v.name.eq_ignore_ascii_case(&original))
                    .map(|v| v.expr)
                    .unwrap_or_default();
                let expr_changed = expr.split_whitespace().collect::<String>()
                    != old_expr.split_whitespace().collect::<String>();
                let r = change(data, |d| {
                    d.set_notes(&original, &notes);
                    if expr_changed {
                        d.set_expression(&original, &expr);
                    }
                    if name != original {
                        d.rename_variable(&original, &name)?;
                    }
                    Ok(())
                });
                if r.is_ok() {
                    // A new value replaces any slider setting; a new name
                    // takes the slider setting with it.
                    if expr_changed {
                        data.overrides
                            .retain(|(n, _)| !n.eq_ignore_ascii_case(&original));
                    }
                    for o in &mut data.overrides {
                        if o.0.eq_ignore_ascii_case(&original) {
                            o.0 = name.clone();
                        }
                    }
                    self.editing.as_mut().expect("editing").original = name;
                }
                r
            };
            match result {
                Ok(()) => {
                    self.refused = None;
                    if delete {
                        let name = &self.editing.as_ref().expect("editing").original;
                        data.overrides
                            .retain(|(n, _)| !n.eq_ignore_ascii_case(name));
                        data.message(Level::Info, format!("Deleted the variable {name}."));
                        close = true;
                    }
                }
                Err(why) => {
                    let text = format!("That change wasn't made: {why}.");
                    data.message(Level::Warning, text.clone());
                    self.refused = Some(text);
                }
            }
        }
        if close {
            self.editing = None;
        }
    }
}

/// Changes the deck with `edit`, keeping the change only if the deck still
/// works afterwards.
fn change(
    data: &mut Data,
    edit: impl FnOnce(&mut Deck) -> Result<(), String>,
) -> Result<(), String> {
    let Some(m) = &mut data.model else {
        return Err("no model is open".into());
    };
    let before = m.deck.clone();
    if let Err(e) = edit(&mut m.deck) {
        m.deck = before;
        return Err(e);
    }
    m.keep_if_valid(before, &data.overrides)?;
    data.edit_event = Some(true);
    Ok(())
}

/// What `name` would be worth if set to `expr`, or why it can't be.
fn check_expression(
    deck: &Deck,
    name: &str,
    expr: &str,
    overrides: &[(String, f64)],
) -> Result<f64, String> {
    if expr.trim().is_empty() {
        return Err("Type a number or a formula.".into());
    }
    let mut trial = deck.clone();
    trial.set_expression(name, expr);
    let others: Vec<(String, f64)> = overrides
        .iter()
        .filter(|(n, _)| !n.eq_ignore_ascii_case(name))
        .cloned()
        .collect();
    match trial.expand_with(&others) {
        Ok(n) => value_of(&n.symbols, name).ok_or_else(|| "?".into()),
        Err(e) => {
            let text = e.message;
            let earlier = straws_core::deck::expr::names_in(expr)
                .into_iter()
                .any(|u| u.eq_ignore_ascii_case(name));
            Err(if earlier {
                format!("{text} (a variable can't be worked out from itself)")
            } else {
                text
            })
        }
    }
}

/// Records a new value for a variable (dropping the override if it's back
/// to the deck's own value).
pub fn set_override(data: &mut Data, name: &str, value: f64, base: f64) {
    data.overrides
        .retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    if value != base {
        data.overrides.push((name.to_string(), value));
    }
}

/// Every number on the deck's other cards (sources, loads, lines, arcs,
/// helices, moves, frequency, ground), each one click from a variable.
fn other_numbers(ui: &mut egui::Ui, data: &mut Data, deck: &Deck) {
    ui.label(
        RichText::new("Click a number to make it a variable. Wires are in the wire table (right-click a cell).")
            .weak(),
    );
    let mut request = None;
    egui::Grid::new("other numbers")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (i, line) in deck.lines.iter().enumerate() {
                let LineKind::Card { mnemonic, fields } = &line.kind else {
                    continue;
                };
                if matches!(
                    mnemonic.as_str(),
                    "GW" | "GE" | "EN" | "PT" | "XQ" | "NX" | "GS"
                ) {
                    continue;
                }
                ui.label(RichText::new(mnemonic).monospace().strong())
                    .on_hover_text(line.comment.as_deref().unwrap_or(""));
                ui.horizontal_wrapped(|ui| {
                    for (k, f) in fields.iter().enumerate() {
                        let (_, words) = field_label(mnemonic, k);
                        let names = straws_core::deck::expr::names_in(f);
                        let tip = if words.is_empty() {
                            format!("Field {}", k + 1)
                        } else {
                            words.to_string()
                        };
                        if names.is_empty() {
                            if ui
                                .small_button(f)
                                .on_hover_text(format!("{tip}: make it a variable"))
                                .clicked()
                            {
                                request = Some(newvar::field_request(deck, i, k, None));
                            }
                        } else {
                            ui.label(RichText::new(f).color(ui.visuals().hyperlink_color))
                                .on_hover_text(format!("{tip}: already uses {}", names.join(", ")));
                        }
                    }
                });
                ui.end_row();
            }
        });
    if request.is_some() {
        data.new_variable = request;
    }
}
