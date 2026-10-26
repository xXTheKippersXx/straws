//! Curved wires drawn in the 3D view: adding them, and the small panel that
//! adjusts the selected one (its sizes and placement are variables, so they
//! also have sliders and can be optimised).

use eframe::egui::{self, RichText};
use straws_core::deck::curves::{Curve, CurveKind, CurveSpec, curves};
use straws_core::units::wavelength;

use crate::app::{Data, Level, Settings};
use crate::views::variables::{set_override, shown};

/// Segments (or straight pieces, for spirals) for a curve: about 20 per
/// wavelength at the top of the sweep, and enough that each step turns
/// gently.
pub fn auto_segments(spec: &CurveSpec, max_mhz: f64) -> i64 {
    let per_lambda = (spec.wire_length() / (wavelength(max_mhz.max(1e-3)) / 20.0)).ceil();
    let gentle = match spec.kind {
        CurveKind::Circle => 16.0,
        CurveKind::Arc => ((spec.to - spec.from).abs() / 22.5).ceil(),
        _ => 12.0 * spec.turns,
    };
    let mut n = per_lambda.max(gentle).max(3.0);
    // Segments much shorter than about four wire radii upset NEC: on a tight
    // bend in thick wire, use fewer, longer ones.
    let shortest = 4.0 * spec.wire_radius;
    if spec.wire_length() / n < shortest {
        n = (spec.wire_length() / shortest).floor().max(2.0);
    }
    n as i64
}

/// Adds `spec` to the model (its centre as the screen shows it, so the
/// height card's lift is taken off). Returns why not, if it can't.
pub fn add(data: &mut Data, spec: &CurveSpec) -> Result<(), String> {
    let Some(m) = &mut data.model else {
        return Err("no model is open".into());
    };
    let mut spec = spec.clone();
    spec.centre[2] -= m.lift();
    let before = m.deck.clone();
    let tag = m.next_tag();
    if let Err(e) = m.deck.add_curve(&spec, tag) {
        m.deck = before;
        return Err(e);
    }
    // On wire ends written as formulas, it follows them from the start.
    m.deck.link_curves();
    m.keep_if_valid(before, &data.overrides)?;
    data.selected_curve = Some(spec.name.clone());
    data.selection.clear();
    data.edit_event = Some(true);
    Ok(())
}

/// Deletes curve `name` (and the variables only it used).
pub fn delete(data: &mut Data, name: &str) {
    let Some(m) = &mut data.model else { return };
    let before = m.deck.clone();
    m.deck.remove_curve(name);
    match m.keep_if_valid(before, &data.overrides) {
        Ok(()) => {
            let prefix = format!("{}_", name.to_ascii_lowercase());
            data.overrides
                .retain(|(n, _)| !n.to_ascii_lowercase().starts_with(&prefix));
            data.selected_curve = None;
            data.edit_event = Some(true);
        }
        Err(why) => data.message(Level::Warning, format!("Not deleted: {why}.")),
    }
}

/// Segments of a curve (pieces, for a spiral).
fn segments_of(deck: &straws_core::deck::Deck, c: &Curve) -> i64 {
    if matches!(c.kind, CurveKind::Spiral | CurveKind::LogSpiral) {
        return (c.lines.len() - 1) as i64;
    }
    match &deck.lines[c.lines[0]].kind {
        straws_core::deck::LineKind::Card { fields, .. } => fields
            .get(1)
            .and_then(|f| f.parse::<f64>().ok())
            .unwrap_or(1.0) as i64,
        _ => 1,
    }
}

/// What a curve variable is, in a few words.
pub fn label_of(suffix: &str) -> &'static str {
    match suffix {
        "x" => "centre x",
        "y" => "centre y",
        "z" => "centre z",
        "tilt" => "axis tilt",
        "az" => "axis turn",
        "r" => "radius",
        "r2" => "end / outer radius",
        "from" => "start angle",
        "to" => "end angle",
        "len" => "length",
        "turns" => "turns",
        "wire" => "wire radius",
        _ => "",
    }
}

/// The selected curve's numbers, in a small panel. Returns false if no
/// curve is selected (so there's nothing to show).
pub fn panel(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) -> bool {
    let Some(name) = data.selected_curve.clone() else {
        return false;
    };
    let Some(m) = &data.model else { return false };
    let Some(c) = curves(&m.deck).into_iter().find(|c| c.name == name) else {
        data.selected_curve = None;
        return false;
    };
    let mut feed: Option<i64> = None;
    let mut remove = false;
    let mut close = false;
    let mut changes: Vec<(String, f64, f64)> = Vec::new();
    let mut set_segments: Option<i64> = None;
    let mut event = None;
    let mhz = data.mhz.max(1e-3);
    let prefix = format!("{name}_").to_ascii_lowercase();
    let vars: Vec<_> = m
        .deck
        .variables()
        .into_iter()
        .filter(|v| v.name.to_ascii_lowercase().starts_with(&prefix))
        .collect();
    let n_segments = segments_of(&m.deck, &c);
    ui.horizontal(|ui| {
        ui.label(RichText::new(&name).strong());
        ui.label(RichText::new(format!("{}, wire {}", c.kind.label(), c.tag)).weak());
    });
    egui::Grid::new("curve numbers")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            variable_rows(
                ui,
                &m.deck,
                &vars,
                &prefix,
                label_of,
                (&data.overrides, settings, mhz),
                &mut changes,
                &mut event,
            );
            let spiral = matches!(c.kind, CurveKind::Spiral | CurveKind::LogSpiral);
            ui.label(if spiral { "pieces" } else { "segments" });
            let mut n = n_segments;
            if spiral {
                ui.label(n.to_string())
                    .on_hover_text("Draw the spiral again for a different number of pieces");
            } else if ui.add(crate::views::num(&mut n).range(1..=2000)).changed() {
                set_segments = Some(n);
            }
            ui.end_row();
        });
    ui.horizontal(|ui| {
        if ui.small_button("Feed the middle").clicked() {
            feed = Some(n_segments);
        }
        if ui
            .small_button("Feed the start")
            .on_hover_text("The usual place for a helix or spiral")
            .clicked()
        {
            feed = Some(1);
        }
        if ui
            .small_button("Delete")
            .on_hover_text("Or press Del")
            .clicked()
        {
            remove = true;
        }
        if ui
            .small_button("Close")
            .on_hover_text("Or press Esc")
            .clicked()
        {
            close = true;
        }
    });
    let line = c.lines[0];
    for (v, value, base) in changes {
        set_override(data, &v, value, base);
    }
    if let Some(n) = set_segments
        && let Some(m) = &mut data.model
    {
        let before = m.deck.clone();
        m.deck.set_field(line, 1, &n.to_string());
        match m.keep_if_valid(before, &data.overrides) {
            Ok(()) => data.edit_event = Some(true),
            Err(why) => data.message(Level::Warning, format!("Not changed: {why}.")),
        }
    }
    if let Some(e) = event {
        data.edit_event = Some(data.edit_event.unwrap_or(false) || e);
    }
    if let Some(seg) = feed
        && let Some(m) = &mut data.model
    {
        let before = m.deck.clone();
        m.deck.feed_wire(c.tag, seg);
        if m.keep_if_valid(before, &data.overrides).is_ok() {
            data.edit_event = Some(true);
        }
    }
    if remove {
        delete(data, &name);
    }
    if close {
        data.selected_curve = None;
    }
    true
}

/// One row per variable of a curve or radial set (`prefix` is its name and
/// `_`): a drag box for each plain number, the formula for the rest. Changes
/// go into `changes` as (name, new value, deck value), and `event` says
/// whether a full rerun is due (false while dragging).
#[allow(clippy::too_many_arguments)]
pub fn variable_rows(
    ui: &mut egui::Ui,
    deck: &straws_core::deck::Deck,
    vars: &[straws_core::deck::Variable],
    prefix: &str,
    label_of: fn(&str) -> &'static str,
    (overrides, settings, mhz): (&[(String, f64)], &Settings, f64),
    changes: &mut Vec<(String, f64, f64)>,
    event: &mut Option<bool>,
) {
    for v in vars {
        let suffix = &v.name[prefix.len()..];
        let label = label_of(&suffix.to_ascii_lowercase());
        ui.label(if label.is_empty() { suffix } else { label })
            .on_hover_text(format!(
                "The variable {} (also in the Variables view)",
                v.name
            ));
        let notes = deck.notes(&v.name);
        match v.literal {
            Some(base) => {
                let now = overrides
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&v.name))
                    .map_or(base, |(_, x)| *x);
                let (shown_now, unit, per) = shown(now, &notes.unit, settings, mhz);
                let mut x = shown_now;
                let speed = (base.abs() * 0.002).max(0.001) * per;
                let r = ui.add(
                    crate::views::num(&mut x)
                        .speed(speed)
                        .max_decimals(6)
                        .suffix(unit),
                );
                if x != shown_now {
                    changes.push((v.name.clone(), x / per, base));
                    *event = Some(!r.dragged());
                }
                if r.drag_stopped() || r.lost_focus() {
                    *event = Some(true);
                }
            }
            None => {
                ui.label(RichText::new(format!("= {}", v.expr)).weak());
            }
        }
        ui.end_row();
    }
}
