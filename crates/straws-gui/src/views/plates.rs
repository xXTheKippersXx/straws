//! Reflector plates (flat and corner) drawn in the 3D view: adding one, and
//! the small panel that adjusts the selected one.

use eframe::egui::{self, RichText};
use straws_core::deck::plates::{PlateKind, PlateSpec, plate_label, plate_sets, segment_count};
use straws_core::units::wavelength;

use crate::app::{Data, Level, Settings};
use crate::views::curves::variable_rows;
use crate::views::variables::set_override;

/// Adds a reflector of `kind` at `at` (its centre, or a corner's apex, as
/// the screen shows it), facing the fed wire: one wavelength wide and high
/// (at the slider frequency), a 90° corner, with a mesh of a tenth of a
/// wavelength at the top of the sweep.
pub fn add(data: &mut Data, kind: PlateKind, at: [f64; 3]) -> Result<(), String> {
    let lam = wavelength(data.mhz.max(1e-3));
    let mesh = wavelength(data.max_mhz.max(1e-3)) / 10.0;
    let Some(m) = &mut data.model else {
        return Err("no model is open".into());
    };
    // Face the fed wire, if there is one.
    let feed = m.numeric.as_ref().ok().and_then(|n| {
        let ex = n.cards.iter().find(|c| c.is("EX"))?;
        let w = m.wires.iter().find(|w| w.tag == ex.ints[1])?;
        Some([(w.a[0] + w.b[0]) / 2.0, (w.a[1] + w.b[1]) / 2.0])
    });
    let facing = feed
        .map(|f| (f[1] - at[1]).atan2(f[0] - at[0]))
        .filter(|a| a.is_finite())
        .map_or(0.0, f64::to_degrees);
    let facing = (facing * 1e6).round() / 1e6;
    let cells = |size: f64| ((size / mesh).ceil() as usize).clamp(1, 60);
    let (width, cells) = match kind {
        PlateKind::Flat => (lam, [cells(lam), cells(lam)]),
        PlateKind::Corner => (lam, [(2 * cells(lam)).min(60), cells(lam)]),
    };
    let wire_radius = data
        .selection
        .iter()
        .find_map(|l| m.wires.iter().find(|w| w.line == *l))
        .or(m.wires.last())
        .map_or(0.001, |w| w.radius);
    let spec = PlateSpec {
        name: m.deck.plate_name(kind),
        kind,
        centre: [at[0], at[1], at[2] - m.lift()],
        width,
        height: lam,
        facing,
        angle: if kind == PlateKind::Corner { 90.0 } else { 0.0 },
        cells,
        segments: 1,
        wire_radius,
    };
    let before = m.deck.clone();
    let tag = m.next_tag();
    if let Err(e) = m.deck.add_plate(&spec, tag) {
        m.deck = before;
        return Err(e);
    }
    m.keep_if_valid(before, &data.overrides)?;
    data.selected_curve = Some(spec.name);
    data.selection.clear();
    data.edit_event = Some(true);
    Ok(())
}

/// Deletes reflector `name` (and the variables only it used).
pub fn delete(data: &mut Data, name: &str) {
    let Some(m) = &mut data.model else { return };
    let before = m.deck.clone();
    m.deck.remove_plate(name);
    match m.keep_if_valid(before, &data.overrides) {
        Ok(()) => {
            let prefix = format!("{}_", name.to_ascii_lowercase());
            data.overrides
                .retain(|(n, _)| !n.to_ascii_lowercase().starts_with(&prefix));
            data.selected_curve = None;
            data.selection.clear();
            data.edit_event = Some(true);
        }
        Err(why) => data.message(Level::Warning, format!("Not deleted: {why}.")),
    }
}

/// Whether `name` is a reflector in the open model.
pub fn is_plate(data: &Data, name: &str) -> bool {
    data.model
        .as_ref()
        .is_some_and(|m| plate_sets(&m.deck).iter().any(|p| p.name == name))
}

/// The selected reflector's numbers, in a small panel.
pub fn panel(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    let Some(name) = data.selected_curve.clone() else {
        return;
    };
    let Some(m) = &data.model else { return };
    let Some(p) = plate_sets(&m.deck).into_iter().find(|p| p.name == name) else {
        return;
    };
    let mut changes: Vec<(String, f64, f64)> = Vec::new();
    let mut event = None;
    let (mut remove, mut close) = (false, false);
    let mut cells = p.cells;
    let mhz = data.mhz.max(1e-3);
    let prefix = format!("{name}_").to_ascii_lowercase();
    let vars: Vec<_> = m
        .deck
        .variables()
        .into_iter()
        .filter(|v| v.name.to_ascii_lowercase().starts_with(&prefix))
        .collect();
    let value = |k: &str| {
        m.numeric.as_ref().ok().and_then(|n| {
            n.symbols
                .iter()
                .find(|(s, _)| s.eq_ignore_ascii_case(&format!("{name}_{k}")))
                .map(|(_, v)| *v)
        })
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(&name).strong());
        let first = p.tags.iter().min().copied().unwrap_or(0);
        let last = p.tags.iter().max().copied().unwrap_or(0);
        ui.label(RichText::new(format!("{}, wires {first}-{last}", p.kind.label())).weak());
    });
    // How fine the mesh is, at the top of the sweep.
    let segments = m
        .deck
        .lines
        .get(p.lines[0])
        .and_then(|l| match &l.kind {
            straws_core::deck::LineKind::Card { fields, .. } => fields.get(1)?.parse::<f64>().ok(),
            _ => None,
        })
        .map_or(1, |s| s as i64);
    let across = match p.kind {
        PlateKind::Flat => value("w").map(|w| w / p.cells[0] as f64),
        PlateKind::Corner => value("side").map(|s| 2.0 * s / p.cells[0] as f64),
    };
    let up = value("h").map(|h| h / p.cells[1] as f64);
    let lam_top = wavelength(data.max_mhz.max(1e-3));
    let spacing = across.into_iter().chain(up).fold(0.0, f64::max) / lam_top;
    let count = segment_count(p.cells, segments);
    let text = format!(
        "{count} segments; mesh {spacing:.3} λ at the top of the sweep{}",
        if spacing > 0.1 {
            " (over 0.1 λ: add cells so it acts as a solid sheet)"
        } else {
            ""
        }
    );
    let colour = if spacing > 0.1 {
        ui.visuals().warn_fg_color
    } else {
        ui.visuals().weak_text_color()
    };
    ui.label(RichText::new(text).color(colour).small())
        .on_hover_text(
            "Big meshes take longer: the solve time grows with the cube of the segments",
        );
    egui::Grid::new("plate numbers")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            let across_label = match p.kind {
                PlateKind::Flat => "cells across",
                PlateKind::Corner => "cells across (both sides)",
            };
            for (k, label) in [(0, across_label), (1, "cells up")] {
                ui.label(label);
                ui.add(crate::views::num(&mut cells[k]).range(1..=60).speed(0.1));
                ui.end_row();
            }
            variable_rows(
                ui,
                &m.deck,
                &vars,
                &prefix,
                plate_label,
                (&data.overrides, settings, mhz),
                &mut changes,
                &mut event,
            );
        });
    ui.horizontal(|ui| {
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
    // A corner has as many cells on each side: step past odd counts.
    if p.kind == PlateKind::Corner && cells[0] % 2 == 1 {
        cells[0] = if cells[0] > p.cells[0] {
            cells[0] + 1
        } else {
            cells[0].saturating_sub(1).max(2)
        };
    }
    for (v, value, base) in changes {
        set_override(data, &v, value, base);
    }
    if let Some(e) = event {
        data.edit_event = Some(data.edit_event.unwrap_or(false) || e);
    }
    if cells != p.cells
        && let Some(m) = &mut data.model
    {
        let before = m.deck.clone();
        let result = m
            .deck
            .set_plate_cells(&name, cells)
            .and_then(|()| m.keep_if_valid(before.clone(), &data.overrides));
        match result {
            Ok(()) => data.edit_event = Some(true),
            Err(why) => {
                m.deck = before;
                data.message(Level::Warning, format!("Not changed: {why}."));
            }
        }
    }
    if remove {
        delete(data, &name);
    }
    if close {
        data.selected_curve = None;
    }
}
