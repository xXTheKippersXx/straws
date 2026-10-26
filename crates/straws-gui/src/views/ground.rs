//! The ground under the antenna and the antenna's height above it.

use eframe::egui::{self, RichText};
use straws_core::deck::expr::names_in;
use straws_core::deck::ground::{Ground, Method, PRESETS, RadialScreen, ground, radial_screen};
use straws_core::deck::{Deck, LineKind};
use straws_core::units::wavelength;

use crate::views::newvar::{self, Kind, Request};

use crate::app::{Data, Level, Settings};
use crate::views::wires::show_length;

use super::empty_note;

pub fn ui(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    let Some(model) = &data.model else {
        return empty_note(ui, "Open or start a model to choose its ground.");
    };
    let Ok(numeric) = &model.numeric else {
        return empty_note(ui, "The deck can't be read: fix it in the Deck view first.");
    };
    let current = ground(numeric);
    let screen_now = radial_screen(numeric);
    let mut screen_change: Option<Option<RadialScreen>> = None;
    let mut chosen = current;
    let mut dragging = false;
    let unit = settings.length();
    let mhz = data.mhz.max(1e-3);
    let mut warning = None;
    let deck = model.deck.clone();
    let gn = deck
        .lines
        .iter()
        .rposition(|l| matches!(&l.kind, LineKind::Card { mnemonic, .. } if mnemonic == "GN"));
    // Ground values already set by variables: dragging moves the variable.
    let e_link = gn.map_or(Link::Number, |l| link(&deck, l, 4));
    let s_link = gn.map_or(Link::Number, |l| link(&deck, l, 5));
    let mut new_variable = None;
    let mut var_changes: Vec<(String, f64)> = Vec::new();

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.label(RichText::new("Ground").strong());
        ui.horizontal(|ui| {
            if ui
                .radio(current == Ground::FreeSpace, "Free space")
                .on_hover_text("No ground at all, as if the antenna were far out in space")
                .clicked()
            {
                chosen = Ground::FreeSpace;
            }
            if ui
                .radio(current == Ground::Perfect, "Perfect ground")
                .on_hover_text("A perfectly conducting ground: no ground loss (like a huge metal sheet)")
                .clicked()
            {
                chosen = Ground::Perfect;
            }
            let real = matches!(current, Ground::Real { .. });
            if ui
                .radio(real, "Real ground")
                .on_hover_text("Earth or water with losses, set by its permittivity and conductivity")
                .clicked()
                && !real
            {
                chosen = Ground::Real {
                    method: Method::Sommerfeld,
                    permittivity: 13.0,
                    conductivity: 0.005,
                };
            }
        });
        if let Ground::Real {
            method,
            permittivity,
            conductivity,
        } = current
        {
            let (mut m, mut e, mut s) = (method, permittivity, conductivity);
            ui.add_space(4.0);
            egui::Grid::new("real ground")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Type");
                    let preset = PRESETS
                        .iter()
                        .find(|(_, pe, ps)| (pe - e).abs() < 1e-9 && (ps - s).abs() < 1e-12)
                        .map_or("Custom", |p| p.0);
                    egui::ComboBox::from_id_salt("ground type")
                        .selected_text(preset)
                        .width(300.0)
                        .show_ui(ui, |ui| {
                            for (name, pe, ps) in PRESETS {
                                if ui
                                    .selectable_label(preset == *name, format!("{name}  (εr {pe}, σ {ps} S/m)"))
                                    .clicked()
                                {
                                    (e, s) = (*pe, *ps);
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Permittivity εr");
                    ui.horizontal(|ui| {
                        let r = ui
                            .add_enabled(
                                e_link.editable(),
                                crate::views::num(&mut e).range(1.0..=100.0).speed(0.1).max_decimals(3),
                            )
                            .on_hover_text("Relative dielectric constant of the ground");
                        dragging |= r.dragged();
                        if let Some(l) = gn
                            && e_link.button(ui)
                        {
                            new_variable = Some(newvar::field_request(&deck, l, 4, None));
                        }
                    });
                    ui.end_row();
                    ui.label("Conductivity σ");
                    ui.horizontal(|ui| {
                        let r = ui
                            .add_enabled(
                                s_link.editable(),
                                crate::views::num(&mut s)
                                    .range(1e-6..=100.0)
                                    .speed(0.0005)
                                    .max_decimals(6)
                                    .suffix(" S/m"),
                            )
                            .on_hover_text("How well the ground conducts (siemens per metre)");
                        dragging |= r.dragged();
                        if let Some(l) = gn
                            && s_link.button(ui)
                        {
                            new_variable = Some(newvar::field_request(&deck, l, 5, None));
                        }
                    });
                    ui.end_row();
                    ui.label("Method");
                    ui.vertical(|ui| {
                        ui.radio_value(&mut m, Method::Sommerfeld, "Sommerfeld-Norton (accurate)")
                            .on_hover_text("Exact for wires near or touching the ground. A little slower to start each frequency.");
                        ui.radio_value(&mut m, Method::Reflection, "Reflection coefficient (quick)")
                            .on_hover_text("Fine for antennas more than about 0.1 λ above the ground; wrong for wires on or very near it.");
                    });
                    ui.end_row();
                });
            // Values set by a variable change the variable, not the card.
            if let Link::Variable(name) = &e_link
                && e != permittivity
            {
                var_changes.push((name.clone(), e));
                e = permittivity;
            }
            if let Link::Variable(name) = &s_link
                && s != conductivity
            {
                var_changes.push((name.clone(), s));
                s = conductivity;
            }
            chosen = Ground::Real {
                method: m,
                permittivity: e,
                conductivity: s,
            };
        }

        if let Ground::Real { method, .. } = current {
            ui.add_space(8.0);
            if method == Method::Reflection {
                let mut on = screen_now.is_some();
                let mut s = screen_now.unwrap_or(RadialScreen {
                    count: 32,
                    length: wavelength(mhz) / 4.0,
                    wire_radius: 0.001,
                });
                ui.checkbox(&mut on, "Buried radial screen").on_hover_text(
                    "NEC-2's own rough stand-in for buried radials centred under the origin: a \
                     vertical at the centre sees perfect ground right under it, so the number and \
                     length of radials change little (4nec2's engine agrees). For a realistic \
                     result, draw radials with Draw > Radials in the 3D model view and put them on \
                     or under the ground: STRAWS then estimates them (checked against the 1937 \
                     Brown, Lewis and Epstein measurements).",
                );
                if on {
                    egui::Grid::new("radial screen")
                        .num_columns(2)
                        .spacing([8.0, 3.0])
                        .show(ui, |ui| {
                            ui.label("Radials");
                            let r = ui.add(crate::views::num(&mut s.count).range(1..=360));
                            dragging |= r.dragged();
                            ui.end_row();
                            ui.label("Length of each");
                            let mut l = unit.from_metres(s.length, mhz);
                            let speed = (l.abs() * 0.005).max(0.001);
                            let r = ui.add(
                                crate::views::num(&mut l)
                                    .speed(speed)
                                    .max_decimals(6)
                                    .suffix(format!(" {}", unit.symbol())),
                            );
                            dragging |= r.dragged();
                            if r.changed() {
                                s.length = unit.to_metres(l, mhz).max(1e-3);
                            }
                            ui.end_row();
                            ui.label("Wire diameter");
                            let mut d = s.wire_radius * 2000.0;
                            let r = ui.add(
                                crate::views::num(&mut d)
                                    .speed(0.01)
                                    .range(0.01..=100.0)
                                    .max_decimals(3)
                                    .suffix(" mm"),
                            );
                            dragging |= r.dragged();
                            if r.changed() {
                                s.wire_radius = d / 2000.0;
                            }
                            ui.end_row();
                        });
                }
                let want = on.then_some(s);
                if want != screen_now {
                    screen_change = Some(want);
                }
            } else {
                ui.label(
                    RichText::new(
                        "A buried radial screen (NEC's stand-in for buried radials) needs the \
                         reflection-coefficient method.",
                    )
                    .weak(),
                );
            }
        }

        ui.add_space(12.0);
        ui.label(RichText::new("Height").strong());
        let lowest = lowest_point(data);
        match lowest {
            None => {
                ui.label(RichText::new("Add some wires to set their height.").weak());
            }
            Some((low, feed)) => {
                let mut h = unit.from_metres(low, mhz);
                let speed = (h.abs() * 0.005).max(0.001);
                let h_link = deck.height_line().map_or(Link::Number, |l| link(&deck, l, 7));
                ui.horizontal(|ui| {
                    ui.label("Lowest point above ground");
                    let r = ui
                        .add_enabled(
                            h_link.editable(),
                            crate::views::num(&mut h)
                                .speed(speed)
                                .max_decimals(6)
                                .suffix(format!(" {}", unit.symbol())),
                        )
                        .on_hover_text("Moves the whole antenna up or down (one GM card in the deck)");
                    dragging |= r.dragged();
                    if h_link.button(ui) {
                        new_variable = Some(Request::new(
                            &deck,
                            Kind::WholeHeight { lowest: low },
                            "height",
                            "m",
                            "The height of the whole antenna's lowest point becomes a variable.".into(),
                        ));
                    }
                    let dz = unit.to_metres(h, mhz) - low;
                    if r.changed()
                        && let Link::Variable(name) = &h_link
                    {
                        if let Some(now) = value_now(data, &deck, name) {
                            var_changes.push((name.clone(), now + dz));
                        }
                    } else if r.changed()
                        && let Some(m) = &mut data.model
                    {
                        let before = m.deck.clone();
                        let shift = m.deck.height_shift() + dz;
                        m.deck.set_height_shift(shift);
                        match m.keep_if_valid(before, &data.overrides) {
                            Err(why) => warning = Some(format!("Height not changed: {why}.")),
                            Ok(()) => data.edit_event = Some(!dragging),
                        }
                    }
                });
                if let Some(f) = feed {
                    ui.label(format!(
                        "Feed point at {} {} ({:.3} λ at {:.3} MHz).",
                        show_length(f, unit, mhz),
                        unit.symbol(),
                        f / wavelength(mhz),
                        mhz
                    ));
                }
                if current == Ground::FreeSpace {
                    ui.label(RichText::new("In free space the height makes no difference.").weak());
                }
            }
        }

        ui.add_space(12.0);
        ui.label(
            RichText::new(
                "Radials and counterpoises: Draw > Radials in the 3D model view (click the hub, \
                 then set the number, length, droop and height). Raised ones are NEC wires; ones on \
                 or under the ground are estimated, as NEC-2 can't model wires there.",
            )
            .weak(),
        );
    });

    if let Some(w) = warning {
        data.message(Level::Warning, w);
    }
    if new_variable.is_some() {
        data.new_variable = new_variable;
    }
    for (name, value) in var_changes {
        data.overrides
            .retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
        data.overrides.push((name, value));
        data.edit_event = Some(data.edit_event.unwrap_or(false) || !dragging);
    }
    if chosen != current
        && let Some(m) = &mut data.model
    {
        let before = m.deck.clone();
        m.deck.set_ground(chosen);
        // Keep the variables that set the ground's values.
        if matches!(chosen, Ground::Real { .. })
            && let Some(line) = m.deck.lines.iter().rposition(
                |l| matches!(&l.kind, LineKind::Card { mnemonic, .. } if mnemonic == "GN"),
            )
        {
            for (field, l) in [(4, &e_link), (5, &s_link)] {
                if let Link::Variable(text) | Link::Formula(text) = l {
                    m.deck.set_field(line, field, text);
                }
            }
        }
        // The radial screen stays while the ground is still GN 0.
        if let (
            Ground::Real {
                method: Method::Reflection,
                ..
            },
            Some(s),
        ) = (chosen, screen_now)
        {
            m.deck.set_radial_screen(Some(s));
        }
        match m.keep_if_valid(before, &data.overrides) {
            Ok(()) => data.edit_event = Some(data.edit_event.unwrap_or(false) || !dragging),
            Err(why) => data.message(Level::Warning, format!("Ground not changed: {why}.")),
        }
    }
    if let Some(screen) = screen_change
        && let Some(m) = &mut data.model
    {
        let before = m.deck.clone();
        m.deck.set_radial_screen(screen);
        match m.keep_if_valid(before, &data.overrides) {
            Ok(()) => data.edit_event = Some(data.edit_event.unwrap_or(false) || !dragging),
            Err(why) => data.message(Level::Warning, format!("Screen not changed: {why}.")),
        }
    }
}

/// The lowest point of the antenna and the feed height, in metres.
fn lowest_point(data: &Data) -> Option<(f64, Option<f64>)> {
    let m = data.model.as_ref()?;
    let feeds = m.feed_segments();
    let mut low = f64::MAX;
    let mut feed = None;
    // The deck's own wires move at once while dragging; the engine's
    // segments (which also cover arcs and helices) follow a moment later.
    for w in &m.wires {
        low = low.min(w.a[2]).min(w.b[2]);
    }
    for s in m.visible_segments() {
        if m.wires.is_empty() {
            let (a, b) = s.ends();
            low = low.min(a[2]).min(b[2]);
        }
        if feeds.first() == Some(&s.number) {
            feed = Some(s.centre[2]);
        }
    }
    (low < f64::MAX).then_some((low, feed))
}

/// How a ground value is set in the deck.
enum Link {
    /// A plain number: it can be changed here, or made a variable.
    Number,
    /// Just a variable's name: changing it here moves the variable.
    Variable(String),
    /// Some other formula: change its variables instead.
    Formula(String),
}

/// How field `field` of line `line` is set.
fn link(deck: &Deck, line: usize, field: usize) -> Link {
    let Some(LineKind::Card { fields, .. }) = deck.lines.get(line).map(|l| &l.kind) else {
        return Link::Number;
    };
    let Some(text) = fields.get(field) else {
        return Link::Number;
    };
    let names = names_in(text);
    let adjustable = |n: &str| {
        deck.variables()
            .iter()
            .any(|v| v.name.eq_ignore_ascii_case(n) && v.literal.is_some())
    };
    match names.as_slice() {
        [] => Link::Number,
        // One variable, used as itself or plus a constant (as the height
        // variable is): moving it moves the value one for one.
        [n] if adjustable(n)
            && (deck.field_variable(line, field).is_some() || line_is_height(deck, line)) =>
        {
            Link::Variable(n.clone())
        }
        _ => Link::Formula(text.clone()),
    }
}

/// Whether `line` is the GM card that sets the antenna's height.
fn line_is_height(deck: &Deck, line: usize) -> bool {
    deck.height_line() == Some(line)
}

impl Link {
    fn editable(&self) -> bool {
        !matches!(self, Link::Formula(_))
    }

    /// The ƒ button (or a note saying which variable sets it). Returns
    /// true if the button was clicked.
    fn button(&self, ui: &mut egui::Ui) -> bool {
        match self {
            Link::Number => ui
                .small_button("ƒ")
                .on_hover_text("Make this a variable (with its own slider)")
                .clicked(),
            Link::Variable(name) => {
                ui.label(
                    egui::RichText::new(format!("= {name}")).color(ui.visuals().hyperlink_color),
                )
                .on_hover_text("Set by this variable: changing it here moves its slider");
                false
            }
            Link::Formula(text) => {
                ui.label(
                    egui::RichText::new(format!("= {text}")).color(ui.visuals().hyperlink_color),
                )
                .on_hover_text("Set by a formula: change its variables in the Variables view");
                false
            }
        }
    }
}

/// A variable's value now (its slider, or the deck's number).
fn value_now(data: &Data, deck: &Deck, name: &str) -> Option<f64> {
    data.overrides
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, x)| *x)
        .or_else(|| {
            deck.variables()
                .into_iter()
                .find(|v| v.name.eq_ignore_ascii_case(name))
                .and_then(|v| v.literal)
        })
}
