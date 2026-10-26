//! Boxes of surface patches drawn in the 3D view: adding one, finding the
//! box a clicked patch belongs to, and the small panel that adjusts it.

use eframe::egui::{self, RichText};
use straws_core::deck::patches::{BoxSpec, box_label, box_sets, patch_count};
use straws_core::output::Patch;
use straws_core::units::wavelength;

use crate::app::{Data, Level, Settings};
use crate::model::LoadedModel;
use crate::views::curves::variable_rows;
use crate::views::variables::set_override;

/// Patches along a side `size` long: about a tenth of a wavelength each at
/// the top of the sweep, odd so a patch centre sits in the middle of every
/// face (where a wire can join).
fn cells_for(size: f64, mhz: f64) -> usize {
    let n = (size / (wavelength(mhz.max(1e-3)) / 10.0)).ceil() as usize;
    (n | 1).clamp(1, 29)
}

/// Adds a box whose bottom face is centred on `at` (as the screen shows
/// it): half a wavelength long and deep and a quarter high at the slider
/// frequency, standing on the clicked point.
pub fn add(data: &mut Data, at: [f64; 3]) -> Result<(), String> {
    let lam = wavelength(data.mhz.max(1e-3));
    let top = data.max_mhz;
    let Some(m) = &mut data.model else {
        return Err("no model is open".into());
    };
    let size = [lam / 2.0, lam / 2.0, lam / 4.0].map(|s| (s * 1e4).round() / 1e4);
    let spec = BoxSpec {
        name: m.deck.patch_box_name(),
        base: [at[0], at[1], at[2] - m.lift()],
        size,
        facing: 0.0,
        cells: size.map(|s| cells_for(s, top)),
    };
    let before = m.deck.clone();
    if let Err(e) = m.deck.add_patch_box(&spec) {
        m.deck = before;
        return Err(e);
    }
    m.keep_if_valid(before, &data.overrides)?;
    data.selected_curve = Some(spec.name);
    data.selection.clear();
    data.edit_event = Some(true);
    Ok(())
}

/// Deletes box `name` (and the variables only it used).
pub fn delete(data: &mut Data, name: &str) {
    let Some(m) = &mut data.model else { return };
    let before = m.deck.clone();
    m.deck.remove_patch_box(name);
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

/// Whether `name` is a patch box in the open model.
pub fn is_box(data: &Data, name: &str) -> bool {
    data.model
        .as_ref()
        .is_some_and(|m| box_sets(&m.deck).iter().any(|b| b.name == name))
}

/// A variable's worked-out value.
fn value(m: &LoadedModel, name: &str, k: &str) -> Option<f64> {
    m.numeric.as_ref().ok().and_then(|n| {
        n.symbols
            .iter()
            .find(|(s, _)| s.eq_ignore_ascii_case(&format!("{name}_{k}")))
            .map(|(_, v)| *v)
    })
}

/// The box (by name) that patch `p` lies on, if it's one of ours.
pub fn box_of(m: &LoadedModel, p: &Patch) -> Option<String> {
    box_sets(&m.deck).into_iter().map(|b| b.name).find(|name| {
        let v = |k| value(m, name, k);
        let (Some(x), Some(y), Some(z), Some(w), Some(d), Some(h), Some(az)) =
            (v("x"), v("y"), v("z"), v("w"), v("d"), v("h"), v("az"))
        else {
            return false;
        };
        let (dx, dy) = (p.centre[0] - x, p.centre[1] - y);
        let (c, s) = (az.to_radians().cos(), az.to_radians().sin());
        let (a, b) = (dx * c + dy * s, -dx * s + dy * c);
        let dz = p.centre[2] - m.lift() - z;
        let tol = 1e-6 * (w + d + h);
        a.abs() <= w / 2.0 + tol && b.abs() <= d / 2.0 + tol && (-tol..=h + tol).contains(&dz)
    })
}

/// The middle of every face of every box, as the screen shows them.
pub fn face_centres(m: &LoadedModel) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    for b in box_sets(&m.deck) {
        let v = |k| value(m, &b.name, k);
        let (Some(x), Some(y), Some(z), Some(w), Some(d), Some(h), Some(az)) =
            (v("x"), v("y"), v("z"), v("w"), v("d"), v("h"), v("az"))
        else {
            continue;
        };
        let (c, s) = (az.to_radians().cos(), az.to_radians().sin());
        let z = z + m.lift();
        let at = |a: f64, b: f64, up: f64| {
            [x + a * c - b * s, y + a * s + b * c, z + up].map(|v| (v * 1e9).round() / 1e9)
        };
        out.extend([
            at(0.0, 0.0, h),
            at(0.0, 0.0, 0.0),
            at(w / 2.0, 0.0, h / 2.0),
            at(-w / 2.0, 0.0, h / 2.0),
            at(0.0, d / 2.0, h / 2.0),
            at(0.0, -d / 2.0, h / 2.0),
        ]);
    }
    out
}

/// The selected box's numbers, in a small panel.
pub fn panel(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    let Some(name) = data.selected_curve.clone() else {
        return;
    };
    let Some(m) = &data.model else { return };
    let Some(b) = box_sets(&m.deck).into_iter().find(|b| b.name == name) else {
        return;
    };
    let mut changes: Vec<(String, f64, f64)> = Vec::new();
    let mut event = None;
    let (mut remove, mut close) = (false, false);
    let mut cells = b.cells;
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
        ui.label(RichText::new("box of surface patches").weak());
    });
    // The biggest patch, at the top of the sweep.
    let lam = wavelength(data.max_mhz.max(1e-3));
    let sizes = ["w", "d", "h"].map(|k| value(m, &name, k).unwrap_or(0.0));
    let side = |k: usize| sizes[k] / cells[k] as f64;
    let largest = [side(0) * side(1), side(1) * side(2), side(0) * side(2)]
        .into_iter()
        .fold(0.0, f64::max)
        / (lam * lam);
    let text = format!(
        "{} patches; the largest {largest:.3} λ² at the top of the sweep{}",
        patch_count(b.cells),
        if largest > 0.04 {
            " (over 0.04 λ²: add patches)"
        } else {
            ""
        }
    );
    let colour = if largest > 0.04 {
        ui.visuals().warn_fg_color
    } else {
        ui.visuals().weak_text_color()
    };
    ui.label(RichText::new(text).color(colour).small())
        .on_hover_text(
            "Each patch counts as two unknowns: the solve time grows with the cube of the total",
        );
    egui::Grid::new("box numbers")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            for (k, label) in [
                (0, "patches along"),
                (1, "patches across"),
                (2, "patches up"),
            ] {
                ui.label(label);
                ui.add(crate::views::num(&mut cells[k]).range(1..=29).speed(0.1));
                ui.end_row();
            }
            variable_rows(
                ui,
                &m.deck,
                &vars,
                &prefix,
                box_label,
                (&data.overrides, settings, mhz),
                &mut changes,
                &mut event,
            );
        });
    ui.label(
        RichText::new(
            "NEC-2's patches model closed metal surfaces only. A wire joins the box only at a \
             patch's centre, at right angles to it: odd counts put one in the middle of each face.",
        )
        .weak()
        .small(),
    );
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
    for (v, value, base) in changes {
        set_override(data, &v, value, base);
    }
    if let Some(e) = event {
        data.edit_event = Some(data.edit_event.unwrap_or(false) || e);
    }
    if cells != b.cells
        && let Some(m) = &mut data.model
    {
        let before = m.deck.clone();
        let result = m
            .deck
            .set_box_cells(&name, cells)
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
