//! Dishes and curved reflectors drawn in the 3D view: adding one, its panel
//! (outline, profile, cells, sizes, the focus and a dipole there), and the
//! focus marker.

use eframe::egui::{self, RichText};
use straws_core::deck::reflectors::{
    Outline, Profile, ReflectorSpec, focus_point, reflector_label, reflector_sets,
};
use straws_core::units::wavelength;

use crate::app::{Data, Level, Settings};
use crate::views::curves::variable_rows;
use crate::views::variables::set_override;

/// Adds a dish at `vertex` (as the screen shows it): a paraboloid two
/// wavelengths across (at the slider frequency), f/D 0.4, facing the fed
/// wire (or +x), with a mesh of a thirteenth of a wavelength at the top of
/// the sweep (the rim's sloping sides are then still under 0.1 λ).
pub fn add(data: &mut Data, vertex: [f64; 3]) -> Result<(), String> {
    let lam = wavelength(data.mhz.max(1e-3));
    let mesh = wavelength(data.max_mhz.max(1e-3)) / 13.0;
    let Some(m) = &mut data.model else {
        return Err("no model is open".into());
    };
    let feed = m.numeric.as_ref().ok().and_then(|n| {
        let ex = n.cards.iter().find(|c| c.is("EX"))?;
        let w = m.wires.iter().find(|w| w.tag == ex.ints[1])?;
        Some([0, 1, 2].map(|k| (w.a[k] + w.b[k]) / 2.0))
    });
    let facing = feed
        .map(|f| {
            let (dx, dy, dz) = (f[0] - vertex[0], f[1] - vertex[1], f[2] - vertex[2]);
            [
                dy.atan2(dx).to_degrees(),
                dz.atan2(dx.hypot(dy)).to_degrees(),
            ]
        })
        .filter(|a| a.iter().all(|v| v.is_finite()))
        .map_or([0.0, 0.0], |a| a.map(|v| (v * 1e6).round() / 1e6));
    let size = 2.0 * lam;
    let cells = ((size / mesh).ceil() as usize).clamp(2, 80);
    let wire_radius = m.wires.last().map_or(0.001, |w| w.radius);
    let spec = ReflectorSpec {
        name: m.deck.reflector_name(),
        outline: Outline::Ellipse,
        profile: Profile::Paraboloid,
        vertex: [vertex[0], vertex[1], vertex[2] - m.lift()],
        facing,
        size: [size, size],
        offset: [0.0, 0.0],
        focal: [0.4 * size, 0.4 * size],
        radius: size,
        angle: 90.0,
        cells: [cells, cells],
        segments: 1,
        wire_radius,
    };
    let before = m.deck.clone();
    let tag = m.next_tag();
    if let Err(e) = m.deck.add_reflector(&spec, tag) {
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
    m.deck.remove_reflector(name);
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

/// Whether `name` is a dish or curved reflector in the open model.
pub fn is_reflector(data: &Data, name: &str) -> bool {
    data.model
        .as_ref()
        .is_some_and(|m| reflector_sets(&m.deck).iter().any(|r| r.name == name))
}

/// A reflector variable's value now (slider values included).
fn value(model: &crate::model::LoadedModel, name: &str, k: &str) -> Option<f64> {
    let n = model.numeric.as_ref().ok()?;
    n.symbols
        .iter()
        .find(|(s, _)| s.eq_ignore_ascii_case(&format!("{name}_{k}")))
        .map(|(_, v)| *v)
}

/// The selected reflector's focus, as the screen shows it.
pub fn focus(model: &crate::model::LoadedModel, name: &str) -> Option<[f64; 3]> {
    let r = reflector_sets(&model.deck)
        .into_iter()
        .find(|r| r.name == name)?;
    let mut p = focus_point(&r.profile, |k| value(model, name, k))?;
    p[2] += model.lift();
    Some(p)
}

/// The panel's text box for a z = f(x, y) formula, kept between frames.
#[derive(Default)]
pub struct FormulaEdit {
    pub name: String,
    pub text: String,
}

/// The selected reflector's numbers, in a small panel.
pub fn panel(ui: &mut egui::Ui, data: &mut Data, settings: &Settings, edit: &mut FormulaEdit) {
    let Some(name) = data.selected_curve.clone() else {
        return;
    };
    let Some(m) = &data.model else { return };
    let Some(r) = reflector_sets(&m.deck).into_iter().find(|r| r.name == name) else {
        return;
    };
    let mut changes: Vec<(String, f64, f64)> = Vec::new();
    let mut event = None;
    let (mut remove, mut close, mut feed) = (false, false, false);
    let mut cells = r.cells;
    let mut outline = r.outline;
    let mut profile_index = r.profile.index();
    if edit.name != name {
        edit.name = name.clone();
        edit.text = match &r.profile {
            Profile::Formula(f) => f.clone(),
            _ => "0.1*x^2".into(),
        };
    }
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
        let first = r.tags.iter().min().copied().unwrap_or(0);
        let last = r.tags.iter().max().copied().unwrap_or(0);
        ui.label(RichText::new(format!("reflector, wires {first}-{last}")).weak());
    });
    // Mesh size at the top of the sweep.
    let lam_top = wavelength(data.max_mhz.max(1e-3));
    let spacing = [("w", 0), ("l", 1)]
        .into_iter()
        .filter_map(|(k, i)| value(m, &name, k).map(|s| s / r.cells[i] as f64))
        .fold(0.0, f64::max)
        / lam_top;
    let count = r.lines.len() as i64
        * match &m.deck.lines[r.lines[0]].kind {
            straws_core::deck::LineKind::Card { fields, .. } => fields
                .get(1)
                .and_then(|f| f.parse::<f64>().ok())
                .map_or(1, |s| s as i64),
            _ => 1,
        };
    let colour = if spacing > 0.1 {
        ui.visuals().warn_fg_color
    } else {
        ui.visuals().weak_text_color()
    };
    ui.label(
        RichText::new(format!(
            "{count} segments; mesh {spacing:.3} λ at the top of the sweep{}",
            if spacing > 0.1 {
                " (over 0.1 λ: add cells so it acts as a solid sheet)"
            } else {
                ""
            }
        ))
        .color(colour)
        .small(),
    )
    .on_hover_text("Big meshes take longer: the solve time grows with the cube of the segments");
    if let Some(f) = focus(m, &name) {
        ui.label(
            RichText::new(format!(
                "Focus (marked in 3D): x {:.3}, y {:.3}, z {:.3} m",
                f[0], f[1], f[2]
            ))
            .small(),
        );
    }
    egui::Grid::new("reflector numbers")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            ui.label("outline");
            egui::ComboBox::from_id_salt("reflector outline")
                .selected_text(outline.label())
                .show_ui(ui, |ui| {
                    for o in Outline::CHOICES {
                        ui.selectable_value(&mut outline, o, o.label());
                    }
                });
            ui.end_row();
            ui.label("profile");
            egui::ComboBox::from_id_salt("reflector profile")
                .selected_text(Profile::CHOICES[profile_index])
                .show_ui(ui, |ui| {
                    for (k, label) in Profile::CHOICES.iter().enumerate() {
                        ui.selectable_value(&mut profile_index, k, *label);
                    }
                });
            ui.end_row();
            if profile_index == 7 {
                ui.label("z =");
                ui.add(egui::TextEdit::singleline(&mut edit.text).desired_width(140.0))
                    .on_hover_text(
                        "Depth towards the front, in metres, from x (across) and y (up) in \
                         metres, with any variables, e.g. 0.1*x^2 or x^2/(4*f). Applied when \
                         you leave the box.",
                    );
                ui.end_row();
            }
            for (k, label) in [(0, "cells across"), (1, "cells up")] {
                ui.label(label);
                ui.add(crate::views::num(&mut cells[k]).range(2..=80).speed(0.1));
                ui.end_row();
            }
            variable_rows(
                ui,
                &m.deck,
                &vars,
                &prefix,
                reflector_label,
                (&data.overrides, settings, mhz),
                &mut changes,
                &mut event,
            );
        });
    ui.horizontal(|ui| {
        let has_focus = focus(m, &name).is_some();
        if ui
            .add_enabled(has_focus, egui::Button::new("Dipole at the focus").small())
            .on_hover_text(
                "Adds a half-wave dipole at the focus, across the aperture, and moves the feed to \
                 it. It stays at the focus as the focal length changes. (A bare dipole sends half \
                 its power away from the dish: a reflector element a quarter wave behind it helps.)",
            )
            .clicked()
        {
            feed = true;
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
    let profile = Profile::from_index(profile_index, edit.text.trim());
    let reshape = outline != r.outline || profile != r.profile || cells != r.cells;
    // A formula applies once typed (Enter or leaving the box), not per key.
    let typing =
        profile_index == 7 && r.profile.index() == 7 && ui.memory(|mem| mem.focused().is_some());
    for (v, value, base) in changes {
        set_override(data, &v, value, base);
    }
    if let Some(e) = event {
        data.edit_event = Some(data.edit_event.unwrap_or(false) || e);
    }
    if reshape
        && !typing
        && let Some(m) = &mut data.model
    {
        let size = [value(m, &name, "w"), value(m, &name, "l")].map(|v| v.unwrap_or(1.0));
        let spec = ReflectorSpec {
            name: name.clone(),
            outline,
            profile,
            vertex: [0.0; 3],
            facing: [0.0, 0.0],
            size,
            offset: [0.0, 0.0],
            focal: [0.4 * size[0], 0.4 * size[1]],
            radius: size[0].max(size[1]),
            angle: 90.0,
            cells,
            segments: 1,
            wire_radius: 0.001,
        };
        let before = m.deck.clone();
        let result = m
            .deck
            .reshape_reflector(&name, &spec)
            .and_then(|()| m.keep_if_valid(before.clone(), &data.overrides));
        match result {
            Ok(()) => data.edit_event = Some(true),
            Err(why) => {
                m.deck = before;
                data.message(Level::Warning, format!("Not changed: {why}."));
            }
        }
    }
    if feed && let Some(m) = &mut data.model {
        let half = 0.24 * wavelength(mhz);
        let segments = crate::views::wires::auto_segments(2.0 * half, data.max_mhz);
        let radius = value(m, &name, "wire").unwrap_or(0.001);
        let before = m.deck.clone();
        let tag = m.next_tag();
        let result = m
            .deck
            .dipole_at_focus(&name, half, segments, radius, tag)
            .map(|_| ())
            .and_then(|()| m.keep_if_valid(before.clone(), &data.overrides));
        match result {
            Ok(()) => data.edit_event = Some(true),
            Err(why) => {
                m.deck = before;
                data.message(Level::Warning, format!("No dipole added: {why}."));
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
