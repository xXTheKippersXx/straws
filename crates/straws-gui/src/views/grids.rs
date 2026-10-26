//! Grid counterpoises drawn in the 3D view: adding one at a centre point,
//! and the small panel that adjusts the selected grid.

use eframe::egui::{self, RichText};
use straws_core::deck::grids::{GridSpec, grid_label, grid_numbers, grid_sets};
use straws_core::deck::ground::{Ground, ground};
use straws_core::deck::radials::place_for;
use straws_core::units::wavelength;

use crate::app::{Data, Level, Settings};
use crate::views::curves::variable_rows;
use crate::views::radials::{move_to, place_row};
use crate::views::variables::set_override;

/// Adds a square grid a quarter wavelength across (at the slider
/// frequency), 4 x 4 cells, centred on `centre` (as the screen shows it),
/// with the radius of the selected or last wire.
pub fn add(data: &mut Data, centre: [f64; 3]) -> Result<(), String> {
    let mhz = data.mhz.max(1e-3);
    let Some(m) = &mut data.model else {
        return Err("no model is open".into());
    };
    let side = wavelength(mhz) / 4.0;
    let wire_radius = data
        .selection
        .iter()
        .find_map(|l| m.wires.iter().find(|w| w.line == *l))
        .or(m.wires.last())
        .map_or(0.001, |w| w.radius);
    let spec = GridSpec {
        name: m.deck.grid_name(),
        centre: [centre[0], centre[1], centre[2] - m.lift()],
        width: side,
        length: side,
        turn: 0.0,
        cells: [4, 4],
        segments: crate::views::wires::auto_segments(side / 4.0, data.max_mhz),
        wire_radius,
    };
    let before = m.deck.clone();
    let tag = m.next_tag();
    if let Err(e) = m.deck.add_grid(&spec, tag) {
        m.deck = before;
        return Err(e);
    }
    m.keep_if_valid(before, &data.overrides)?;
    data.selected_curve = Some(spec.name);
    data.selection.clear();
    data.edit_event = Some(true);
    Ok(())
}

/// Deletes grid `name` (and the variables only it used).
pub fn delete(data: &mut Data, name: &str) {
    let Some(m) = &mut data.model else { return };
    let before = m.deck.clone();
    m.deck.remove_grid(name);
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

/// Whether `name` is a grid in the open model.
pub fn is_grid(data: &Data, name: &str) -> bool {
    data.model
        .as_ref()
        .is_some_and(|m| grid_sets(&m.deck).iter().any(|g| g.name == name))
}

/// The selected grid's numbers, in a small panel.
pub fn panel(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    let Some(name) = data.selected_curve.clone() else {
        return;
    };
    let Some(m) = &data.model else { return };
    let Some(g) = grid_sets(&m.deck).into_iter().find(|g| g.name == name) else {
        return;
    };
    let mut changes: Vec<(String, f64, f64)> = Vec::new();
    let mut event = None;
    let (mut remove, mut close) = (false, false);
    let mut cells = g.cells;
    let mhz = data.mhz.max(1e-3);
    let prefix = format!("{name}_").to_ascii_lowercase();
    let vars: Vec<_> = m
        .deck
        .variables()
        .into_iter()
        .filter(|v| v.name.to_ascii_lowercase().starts_with(&prefix))
        .collect();
    ui.horizontal(|ui| {
        ui.label(RichText::new(&name).strong());
        let first = g.tags.iter().min().copied().unwrap_or(0);
        let last = g.tags.iter().max().copied().unwrap_or(0);
        ui.label(RichText::new(format!("grid, wires {first}-{last}")).weak());
    });
    let numbers = m.numeric.as_ref().ok().and_then(|n| {
        grid_numbers(&m.deck, n)
            .into_iter()
            .find(|x| x.name == name)
    });
    let soil = m.numeric.as_ref().ok().map(ground);
    let z_var = vars
        .iter()
        .find(|v| v.name.eq_ignore_ascii_case(&format!("{name}_z")))
        .cloned();
    let mut new_place = None;
    let mut where_now = None;
    if let Some(n) = &numbers
        && !matches!(soil, Some(Ground::FreeSpace) | None)
    {
        let top = data.max_mhz.max(1e-3);
        let place = place_for(n.centre[2], n.wire_radius, top);
        let lift = m
            .numeric
            .as_ref()
            .map_or(0.0, |x| straws_core::deck::ground::height_now(x, &m.deck));
        where_now = Some((n.centre, lift));
        let editable = z_var.as_ref().is_some_and(|v| v.literal.is_some());
        new_place = place_row(ui, place, n.wire_radius, top, editable);
        let text = if matches!(soil, Some(Ground::Perfect)) && n.buried {
            "Over perfect ground, a grid on or under it makes no difference.".to_string()
        } else if n.buried {
            let depth = -n.centre[2].min(0.0);
            format!(
                "{}. Estimated: NEC-2 can't model wires on or under lossy ground (the mesh is \
                 worked out as a surface; not checked against measurements, unlike radials). \
                 Right round a vertical's base, where the return current is strongest, a mesh \
                 screens poorly unless its cells are small: radials do better there.",
                if depth > 0.0 {
                    format!("Buried {depth:.3} m deep")
                } else {
                    "On the ground".to_string()
                }
            )
        } else {
            format!(
                "Modelled as NEC wires; {:.3} m above the ground (the Ground view's height lifts \
                 the whole antenna).",
                n.centre[2]
            )
        };
        ui.label(RichText::new(text).weak().small());
    }
    egui::Grid::new("grid numbers")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            for (k, label) in [(0, "cells across (x)"), (1, "cells along (y)")] {
                ui.label(label);
                ui.add(crate::views::num(&mut cells[k]).range(2..=40).speed(0.1))
                    .on_hover_text("An even number, so there's a node in the middle");
                ui.end_row();
            }
            variable_rows(
                ui,
                &m.deck,
                &vars,
                &prefix,
                grid_label,
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
    // Even counts only: step past odd ones in the direction moved.
    for (c, was) in cells.iter_mut().zip(g.cells) {
        if *c % 2 == 1 {
            *c = if *c > was { *c + 1 } else { *c - 1 };
        }
    }
    if let (Some((place, z)), Some((centre, lift)), Some(v)) = (new_place, where_now, &z_var)
        && let Some(base) = v.literal
    {
        move_to(data, &g.lines, &v.name, base, centre, lift, place, z);
    }
    for (v, value, base) in changes {
        set_override(data, &v, value, base);
    }
    if let Some(e) = event {
        data.edit_event = Some(data.edit_event.unwrap_or(false) || e);
    }
    if cells != g.cells
        && let Some(m) = &mut data.model
    {
        let before = m.deck.clone();
        let result = m
            .deck
            .set_grid_cells(&name, cells)
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
