//! Making new variables from anywhere in the window: the menus that offer
//! it, and the prompt that asks for a name.

use eframe::egui::{self, RichText};
use straws_core::deck::edit::Wire;
use straws_core::deck::vars::{Anchor, VarNotes};
use straws_core::deck::{Deck, LineKind};

use crate::app::{Data, Level, Settings};
use crate::model::LoadedModel;
use crate::views::wires::show_length;

/// What a new variable stands for.
#[derive(Clone)]
pub enum Kind {
    /// A number on a card line: (line index, field).
    Field { line: usize, field: usize },
    /// The length of the first wire, shared by all of them.
    Length { wires: Vec<Wire>, anchor: Anchor },
    /// Equal spacing along an axis, or the distance between two wires.
    Spacing {
        wires: Vec<Wire>,
        axis: Option<usize>,
    },
    /// The height of these wires' lowest point.
    Height { wires: Vec<Wire> },
    /// The size of these wires together (a scale factor about their middle).
    Scale { wires: Vec<Wire> },
    /// The height of the whole antenna's lowest point (metres now).
    WholeHeight { lowest: f64 },
    /// A wire's angle above the horizontal, pivoting about one end.
    Angle { wire: Wire, pivot: usize },
    /// The wall thickness of the tube set by an LD line.
    Wall { line: usize },
    /// A variable of its own, with a value typed in.
    Plain { value: String },
}

/// A new variable being named in the prompt.
#[derive(Clone)]
pub struct Request {
    pub kind: Kind,
    pub name: String,
    pub notes: VarNotes,
    /// What the variable will do, in words.
    pub what: String,
    pub error: Option<String>,
    /// Ends of other wires at a point that moves follow it.
    pub joined: bool,
    focused: bool,
}

impl Request {
    pub fn new(deck: &Deck, kind: Kind, name: &str, unit: &str, what: String) -> Request {
        Request {
            kind,
            name: deck.unique_name(name),
            notes: VarNotes {
                unit: unit.to_string(),
                ..VarNotes::default()
            },
            what,
            error: None,
            joined: true,
            focused: false,
        }
    }
}

/// A short name for a wire, from its comment if it has one ("reflector").
pub fn wire_name(deck: &Deck, wire: &Wire) -> String {
    let comment = deck
        .lines
        .get(wire.line)
        .and_then(|l| l.comment.as_deref())
        .unwrap_or("");
    let word: String = comment
        .split_whitespace()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if word.len() >= 3 && word.starts_with(|c: char| c.is_ascii_alphabetic()) {
        word
    } else {
        format!("wire{}", wire.tag)
    }
}

/// What each field of a card is, as (short name for a variable, words).
pub fn field_label(mnemonic: &str, field: usize) -> (&'static str, &'static str) {
    let table: &[(&str, &str)] = match mnemonic {
        "GW" => &[
            ("tag", "wire number"),
            ("segs", "segments"),
            ("x1", "start x"),
            ("y1", "start y"),
            ("z1", "start z"),
            ("x2", "end x"),
            ("y2", "end y"),
            ("z2", "end z"),
            ("radius", "wire radius"),
        ],
        "GA" => &[
            ("tag", "wire number"),
            ("segs", "segments"),
            ("arc_r", "arc radius"),
            ("arc_from", "start angle (°)"),
            ("arc_to", "end angle (°)"),
            ("radius", "wire radius"),
        ],
        "GH" => &[
            ("tag", "wire number"),
            ("segs", "segments"),
            ("pitch", "turn spacing"),
            ("helix_len", "total length"),
            ("rx1", "start radius x"),
            ("ry1", "start radius y"),
            ("rx2", "end radius x"),
            ("ry2", "end radius y"),
            ("radius", "wire radius"),
        ],
        "GM" => &[
            ("tag_step", "tag increment"),
            ("copies", "copies"),
            ("rot_x", "rotation about x (°)"),
            ("rot_y", "rotation about y (°)"),
            ("rot_z", "rotation about z (°)"),
            ("dx", "move x"),
            ("dy", "move y"),
            ("dz", "move z"),
            ("from_tag", "first tag moved"),
        ],
        "EX" => &[
            ("ex_type", "source type"),
            ("ex_tag", "wire"),
            ("ex_seg", "segment"),
            ("ex_flags", "options"),
            ("ex_re", "real part (V or A)"),
            ("ex_im", "imaginary part"),
        ],
        "LD" => &[
            ("ld_type", "load type"),
            ("ld_tag", "wire"),
            ("ld_from", "first segment"),
            ("ld_to", "last segment"),
            ("ld_r", "R (Ω), or conductivity"),
            ("ld_l", "L (H) or X (Ω)"),
            ("ld_c", "C (F)"),
        ],
        "TL" => &[
            ("tl_tag1", "wire 1"),
            ("tl_seg1", "segment 1"),
            ("tl_tag2", "wire 2"),
            ("tl_seg2", "segment 2"),
            ("tl_z0", "impedance (Ω)"),
            ("tl_len", "length (m)"),
        ],
        "FR" => &[
            ("fr_type", "step type"),
            ("fr_count", "number of frequencies"),
            ("", ""),
            ("", ""),
            ("freq", "frequency (MHz)"),
            ("fr_step", "step (MHz)"),
        ],
        "GN" => &[
            ("gn_type", "ground type"),
            ("radials", "radials"),
            ("", ""),
            ("", ""),
            ("ground_er", "permittivity εr"),
            ("ground_sigma", "conductivity (S/m)"),
        ],
        _ => &[],
    };
    match table.get(field) {
        Some(&(short, words)) if !short.is_empty() => (short, words),
        _ => ("value", ""),
    }
}

/// The unit for a variable made from a card field.
fn field_unit(mnemonic: &str, field: usize, scale: f64) -> &'static str {
    let length = match mnemonic {
        "GW" => (2..=8).contains(&field),
        "GA" => field == 2 || field == 5,
        "GH" => field >= 2,
        "GM" => (5..=7).contains(&field),
        "TL" => field == 5,
        _ => false,
    };
    match mnemonic {
        _ if length && scale == 1.0 => "m",
        "GA" if field == 3 || field == 4 => "deg",
        "GM" if (2..=4).contains(&field) => "deg",
        "FR" if field >= 4 => "MHz",
        _ => "",
    }
}

/// A request to make card field `field` of line `line` a variable.
pub fn field_request(deck: &Deck, line: usize, field: usize, wire: Option<&Wire>) -> Request {
    let mnemonic = match deck.lines.get(line).map(|l| &l.kind) {
        Some(LineKind::Card { mnemonic, .. }) => mnemonic.clone(),
        _ => String::new(),
    };
    let (short, words) = field_label(&mnemonic, field);
    let (name, scale) = match wire {
        Some(w) => (format!("{}_{short}", wire_name(deck, w)), w.scale),
        None => (short.to_string(), 1.0),
    };
    let what = match wire {
        Some(w) => format!("Wire {}'s {words} becomes a variable.", w.tag),
        None if words.is_empty() => format!("This {mnemonic} number becomes a variable."),
        None => format!("The {mnemonic} card's {words} becomes a variable."),
    };
    let unit = field_unit(&mnemonic, field, scale);
    Request::new(deck, Kind::Field { line, field }, &name, unit, what)
}

/// Menu items for making variables from the selected wires (used by the
/// wire table, the 3D view and the Variables view). Returns the request
/// for the item clicked.
pub fn selection_menu(
    ui: &mut egui::Ui,
    m: &LoadedModel,
    selection: &[usize],
    mhz: f64,
    settings: &Settings,
) -> Option<Request> {
    // In the order they were picked: the first is the reference.
    let wires: Vec<Wire> = selection
        .iter()
        .filter_map(|l| m.wires.iter().find(|w| w.line == *l).cloned())
        .collect();
    let deck = &m.deck;
    let n = wires.len();
    let unit = settings.length();
    let mhz = mhz.max(1e-3);
    let len = |w: &Wire| format!("{} {}", show_length(w.length(), unit, mhz), unit.symbol());
    let mut request = None;
    if n == 0 {
        ui.label(RichText::new("Select wires first (click, Ctrl-click for more).").weak());
        return None;
    }
    let first = &wires[0];
    let label = if n == 1 {
        format!("Length of wire {} ({})", first.tag, len(first))
    } else {
        format!("Same length for all {n} wires ({})", len(first))
    };
    if ui
        .button(label)
        .on_hover_text("A variable for the length; you choose which end (or the middle) stays put")
        .clicked()
    {
        let what = if n == 1 {
            format!("Wire {}'s length becomes a variable.", first.tag)
        } else {
            format!(
                "All {n} wires get wire {}'s length, set by one variable.",
                first.tag
            )
        };
        let name = format!("{}_len", wire_name(deck, first));
        let kind = Kind::Length {
            wires: wires.clone(),
            anchor: Anchor::Centre,
        };
        request = Some(Request::new(deck, kind, &name, "m", what));
    }
    let two = n == 2;
    if ui
        .add_enabled(two, egui::Button::new("Distance between the two wires"))
        .on_hover_text("Centre to centre. The second wire you picked moves; the first stays.")
        .on_disabled_hover_text("Select exactly two wires")
        .clicked()
    {
        let what = format!(
            "The distance from wire {} to wire {} becomes a variable; wire {} moves.",
            first.tag, wires[1].tag, wires[1].tag
        );
        let kind = Kind::Spacing {
            wires: wires.clone(),
            axis: None,
        };
        request = Some(Request::new(deck, kind, "spacing", "m", what));
    }
    for (k, axis) in ["x", "y", "z"].iter().enumerate() {
        let text = if n > 2 {
            format!("Equal spacing along {axis}")
        } else {
            format!("Spacing along {axis}")
        };
        if ui
            .add_enabled(n >= 2, egui::Button::new(text))
            .on_hover_text(
                "The wire lowest along this axis stays put; each other wire is one more step along",
            )
            .on_disabled_hover_text("Select two or more wires")
            .clicked()
        {
            let what = if n > 2 {
                format!(
                    "The {n} wires are spaced equally along {axis}, by one variable (e.g. a boom pitch)."
                )
            } else {
                format!("The spacing along {axis} becomes a variable.")
            };
            let kind = Kind::Spacing {
                wires: wires.clone(),
                axis: Some(k),
            };
            let name = if n > 2 { "pitch" } else { "spacing" };
            request = Some(Request::new(deck, kind, name, "m", what));
        }
    }
    if ui
        .button("Height of the lowest point")
        .on_hover_text(
            "These wires move up and down together. For the whole antenna, use the Ground view.",
        )
        .clicked()
    {
        let what = if n == 1 {
            format!(
                "Wire {}'s height above ground becomes a variable.",
                first.tag
            )
        } else {
            format!("The height of these {n} wires (their lowest point) becomes a variable.")
        };
        let kind = Kind::Height {
            wires: wires.clone(),
        };
        request = Some(Request::new(deck, kind, "height", "m", what));
    }
    if ui
        .button("Size (scale about their middle)")
        .on_hover_text(
            "The selected wires grow or shrink together: a scale factor (1 now) the optimiser can \
             vary, e.g. an element's size",
        )
        .clicked()
    {
        let what = format!(
            "The size of these {n} wire{} becomes a scale factor (1 now) about their middle.",
            if n == 1 { "" } else { "s" }
        );
        let kind = Kind::Scale {
            wires: wires.clone(),
        };
        request = Some(Request::new(deck, kind, "scale", "", what));
    }
    if ui
        .add_enabled(n == 1, egui::Button::new("Angle above horizontal"))
        .on_hover_text("Tilts the wire about one end, keeping its length (e.g. inverted-V legs)")
        .on_disabled_hover_text("Select one wire")
        .clicked()
    {
        let what = format!(
            "Wire {}'s angle above the horizontal becomes a variable (degrees).",
            first.tag
        );
        let name = format!("{}_angle", wire_name(deck, first));
        let kind = Kind::Angle {
            wire: first.clone(),
            pivot: 0,
        };
        request = Some(Request::new(deck, kind, &name, "deg", what));
    }
    if request.is_some() {
        ui.close();
    }
    request
}

/// The prompt that names a new variable, drawn over the window.
pub fn prompt(ctx: &egui::Context, data: &mut Data) {
    let Some(mut r) = data.new_variable.take() else {
        return;
    };
    let mut done = false;
    let mut create = false;
    egui::Modal::new(egui::Id::new("new variable")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.heading("New variable");
        ui.label(&r.what);
        ui.add_space(6.0);
        egui::Grid::new("new variable grid")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Name");
                let name = ui.add(egui::TextEdit::singleline(&mut r.name).desired_width(220.0));
                if !r.focused {
                    name.request_focus();
                    r.focused = true;
                }
                if name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    create = true;
                }
                ui.end_row();
                match &mut r.kind {
                    Kind::Plain { value } => {
                        ui.label("Value");
                        ui.add(egui::TextEdit::singleline(value).desired_width(220.0))
                            .on_hover_text("A number (units like 5 ft or 30 mm work), or a formula of other variables");
                        ui.end_row();
                    }
                    Kind::Length { anchor, .. } => {
                        ui.label("Keep in place");
                        ui.vertical(|ui| {
                            ui.radio_value(anchor, Anchor::Centre, "The middle (both ends move)");
                            ui.radio_value(anchor, Anchor::Start, "The start (X1, Y1, Z1)");
                            ui.radio_value(anchor, Anchor::End, "The end (X2, Y2, Z2)");
                        });
                        ui.end_row();
                    }
                    Kind::Angle { pivot, .. } => {
                        ui.label("Pivot about");
                        ui.vertical(|ui| {
                            ui.radio_value(pivot, 0, "The start (X1, Y1, Z1)");
                            ui.radio_value(pivot, 1, "The end (X2, Y2, Z2)");
                        });
                        ui.end_row();
                    }
                    _ => {}
                }
                ui.label("Description");
                ui.add(
                    egui::TextEdit::singleline(&mut r.notes.description)
                        .hint_text("optional, e.g. Reflector length")
                        .desired_width(220.0),
                );
                ui.end_row();
                ui.label("Group");
                ui.add(
                    egui::TextEdit::singleline(&mut r.notes.group)
                        .hint_text("optional, e.g. Reflector")
                        .desired_width(220.0),
                );
                ui.end_row();
            });
        let moves_ends = match &r.kind {
            Kind::Length { .. }
            | Kind::Spacing { .. }
            | Kind::Height { .. }
            | Kind::Scale { .. }
            | Kind::Angle { .. } => true,
            Kind::Field { line, field } => {
                (2..=7).contains(field)
                    && data
                        .model
                        .as_ref()
                        .is_some_and(|m| m.wires.iter().any(|w| w.line == *line))
            }
            _ => false,
        };
        if moves_ends {
            ui.checkbox(&mut r.joined, "Move joined wire ends too")
                .on_hover_text("Other wires whose ends meet the ends that move follow them, so junctions stay joined");
        }
        if let Some(e) = &r.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Create").clicked() {
                create = true;
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = true;
            }
        });
    });
    if create {
        match apply(data, &r) {
            Ok(()) => done = true,
            Err(e) => r.error = Some(format!("Not made: {e}.")),
        }
    }
    if !done {
        data.new_variable = Some(r);
    }
}

/// Makes the variable.
fn apply(data: &mut Data, r: &Request) -> Result<(), String> {
    let Some(m) = &mut data.model else {
        return Err("no model is open".into());
    };
    let name = r.name.trim();
    // Use the wires as they are now, in case the deck changed meanwhile.
    let fresh = |ws: &[Wire]| -> Vec<Wire> {
        ws.iter()
            .filter_map(|w| m.wires.iter().find(|x| x.line == w.line && x.tag == w.tag))
            .cloned()
            .collect()
    };
    let tags: Vec<i64> = m
        .wires
        .iter()
        .filter(|w| data.selection.contains(&w.line))
        .map(|w| w.tag)
        .collect();
    let before = m.deck.clone();
    let notes = &r.notes;
    let joined: Vec<Wire> = if r.joined {
        m.wires.clone()
    } else {
        Vec::new()
    };
    let coord = |line: usize, field: usize| {
        m.wires
            .iter()
            .find(|w| w.line == line)
            .filter(|_| (2..=7).contains(&field))
            .cloned()
    };
    let deck = &mut m.deck;
    let result = match &r.kind {
        Kind::Field { line, field } => match coord(*line, *field) {
            Some(w) => deck.make_coord_variable(&w, *field, &joined, name, notes),
            None => deck.make_field_variable(*line, *field, name, notes),
        },
        Kind::Length { wires, anchor } => {
            deck.drive_length(&fresh(wires), &joined, *anchor, name, notes)
        }
        Kind::Spacing { wires, axis } => {
            deck.drive_spacing(&fresh(wires), &joined, *axis, name, notes)
        }
        Kind::Height { wires } => deck.drive_height(&fresh(wires), &joined, name, notes),
        Kind::Scale { wires } => deck.drive_scale(&fresh(wires), &joined, name, notes),
        Kind::WholeHeight { lowest } => deck.drive_height_shift(*lowest, name, notes),
        Kind::Angle { wire, pivot } => match fresh(std::slice::from_ref(wire)).first() {
            Some(w) => deck.drive_angle(w, &joined, *pivot, name, notes),
            None => Err("that wire has gone".into()),
        },
        Kind::Wall { line } => deck.make_wall_variable(*line, name, notes),
        Kind::Plain { value } => deck.add_variable(name, value, notes).map(|_| {
            // A choke variable sets the choke card, if the shield is there.
            if notes.role == "choke"
                && let Some(l) = straws_core::deck::shield::choke_line(deck)
            {
                deck.set_field(l, 4, name);
            }
        }),
    };
    if let Err(e) = result {
        m.deck = before;
        return Err(e);
    }
    m.keep_if_valid(before, &data.overrides)?;
    data.selection = m
        .wires
        .iter()
        .filter(|w| tags.contains(&w.tag))
        .map(|w| w.line)
        .collect();
    data.edit_event = Some(true);
    data.message(
        Level::Info,
        format!("New variable {name}: it has a slider in the Variables view."),
    );
    Ok(())
}
