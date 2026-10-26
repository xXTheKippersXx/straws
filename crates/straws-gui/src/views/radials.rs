//! Radials and counterpoises drawn in the 3D view: adding a set at a hub
//! point, and the small panel that adjusts the selected set (its sizes are
//! variables, so they also have sliders and can be optimised).

use eframe::egui::{self, RichText};
use straws_core::analysis::buried::skin_depth;
use straws_core::deck::ground::{Ground, ground};
use straws_core::deck::radials::{
    Place, RadialSpec, place_for, radial_label, radial_numbers, radial_sets,
};
use straws_core::units::wavelength;

use crate::app::{Data, Level, Settings};
use crate::views::curves::variable_rows;
use crate::views::variables::set_override;

/// Adds four level quarter-wave radials (at the slider frequency) meeting at
/// `hub` (as the screen shows it), with the radius of the selected or last
/// wire. Returns why not, if it can't.
pub fn add(data: &mut Data, hub: [f64; 3]) -> Result<(), String> {
    let mhz = data.mhz.max(1e-3);
    let Some(m) = &mut data.model else {
        return Err("no model is open".into());
    };
    let length = wavelength(mhz) / 4.0;
    let wire_radius = data
        .selection
        .iter()
        .find_map(|l| m.wires.iter().find(|w| w.line == *l))
        .or(m.wires.last())
        .map_or(0.001, |w| w.radius);
    let spec = RadialSpec {
        name: m.deck.radials_name(),
        count: 4,
        hub: [hub[0], hub[1], hub[2] - m.lift()],
        length,
        droop: 0.0,
        turn: 0.0,
        segments: crate::views::wires::auto_segments(length, data.max_mhz),
        wire_radius,
    };
    let before = m.deck.clone();
    let tag = m.next_tag();
    if let Err(e) = m.deck.add_radials(&spec, tag) {
        m.deck = before;
        return Err(e);
    }
    m.keep_if_valid(before, &data.overrides)?;
    data.selected_curve = Some(spec.name);
    data.selection.clear();
    data.edit_event = Some(true);
    Ok(())
}

/// Deletes radial set `name` (and the variables only it used).
pub fn delete(data: &mut Data, name: &str) {
    let Some(m) = &mut data.model else { return };
    let before = m.deck.clone();
    m.deck.remove_radials(name);
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

/// Whether `name` is a radial set in the open model.
pub fn is_set(data: &Data, name: &str) -> bool {
    data.model
        .as_ref()
        .is_some_and(|m| radial_sets(&m.deck).iter().any(|s| s.name == name))
}

/// The selected radial set's numbers, in a small panel.
pub fn panel(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    let Some(name) = data.selected_curve.clone() else {
        return;
    };
    let Some(m) = &data.model else { return };
    let Some(set) = radial_sets(&m.deck).into_iter().find(|s| s.name == name) else {
        return;
    };
    let mut changes: Vec<(String, f64, f64)> = Vec::new();
    let mut event = None;
    let (mut remove, mut close) = (false, false);
    let mut count = set.tags.len();
    let had_ring = set.ring.is_some();
    let mut ring = had_ring;
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
        let first = set.tags.first().copied().unwrap_or(0);
        let last = set.tags.last().copied().unwrap_or(0);
        ui.label(RichText::new(format!("radials, wires {first}-{last}")).weak());
    });
    // Where they are, and so how they're worked out.
    let numbers = m.numeric.as_ref().ok().and_then(|n| {
        radial_numbers(&m.deck, n)
            .into_iter()
            .find(|r| r.name == name)
    });
    let soil = m.numeric.as_ref().ok().map(ground);
    let z_var = vars
        .iter()
        .find(|v| v.name.eq_ignore_ascii_case(&format!("{name}_z")))
        .cloned();
    // A place chosen with the buttons: (place, hub height above ground).
    let mut new_place: Option<(Place, f64)> = None;
    let mut where_now: Option<([f64; 3], f64)> = None;
    if let Some(r) = &numbers
        && !matches!(soil, Some(Ground::FreeSpace) | None)
    {
        let top = data.max_mhz.max(1e-3);
        let place = place_for(r.hub[2], r.wire_radius, top);
        let lift = m
            .numeric
            .as_ref()
            .map_or(0.0, |n| straws_core::deck::ground::height_now(n, &m.deck));
        where_now = Some((r.hub, lift));
        let editable = z_var.as_ref().is_some_and(|v| v.literal.is_some());
        new_place = place_row(ui, place, r.wire_radius, top, editable);
        let text = if r.buried {
            let depth = -r.hub[2].min(0.0);
            let mut t = format!(
                "{}. Estimated: NEC-2 can't model wires on or under lossy ground, so their effect on \
                 the feed impedance and gain is worked out separately (checked against the 1937 \
                 Brown, Lewis and Epstein measurements). Results say \"estimate\".{}",
                if depth > 0.0 {
                    format!("Buried {depth:.3} m deep")
                } else {
                    "On the ground".to_string()
                },
                if r.count <= 8 {
                    " Rougher with this few radials."
                } else {
                    ""
                }
            );
            if let Some(Ground::Real {
                permittivity,
                conductivity,
                ..
            }) = soil
            {
                let delta = skin_depth(permittivity, conductivity, top);
                if depth > 0.2 * delta {
                    t += &format!(
                        " At {:.2} m deep they're a fair part of the soil's skin depth ({delta:.1} m), \
                         which the estimate ignores: it will flatter them.",
                        depth
                    );
                }
            }
            if matches!(soil, Some(Ground::Perfect)) {
                t = "Over perfect ground, radials on or under it make no difference.".into();
            }
            t
        } else {
            format!(
                "Modelled as NEC wires; the hub is {:.3} m above the ground (the Ground view's \
                 height lifts the whole antenna).",
                r.hub[2]
            )
        };
        ui.label(RichText::new(text).weak().small());
    }
    egui::Grid::new("radial numbers")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            ui.label("radials");
            ui.add(crate::views::num(&mut count).range(1..=120))
                .on_hover_text("How many, spread evenly round the hub");
            ui.end_row();
            ui.label("ring");
            ui.add_enabled(
                count >= 3,
                egui::Checkbox::new(&mut ring, "joining the tips"),
            )
            .on_hover_text(
                "A wire from each radial's tip to the next: a counterpoise that's also a \
                     loop (needs 3 or more radials)",
            );
            ui.end_row();
            variable_rows(
                ui,
                &m.deck,
                &vars,
                &prefix,
                radial_label,
                (&data.overrides, settings, mhz),
                &mut changes,
                &mut event,
            );
        });
    ui.label(
        RichText::new(
            "Droop slopes them down from the hub (about 45° brings a ground plane near 50 Ω).",
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
    let changed_count = count != set.tags.len();
    if ring != had_ring
        && let Some(m) = &mut data.model
    {
        // Sides about as long as the gaps between the tips.
        let side = numbers.as_ref().map_or(1.0, |r| {
            2.0 * r.length * (std::f64::consts::PI / r.count.max(3) as f64).sin()
        });
        let segments = crate::views::wires::auto_segments(side, data.max_mhz);
        let before = m.deck.clone();
        let result = m
            .deck
            .set_radial_ring(&name, ring.then_some(segments))
            .and_then(|()| m.keep_if_valid(before.clone(), &data.overrides));
        match result {
            Ok(()) => data.edit_event = Some(true),
            Err(why) => {
                m.deck = before;
                data.message(Level::Warning, format!("Not changed: {why}."));
            }
        }
    }
    if let (Some((place, z)), Some((hub, lift)), Some(v)) = (new_place, where_now, &z_var)
        && let Some(base) = v.literal
    {
        move_to(data, &set.lines, &v.name, base, hub, lift, place, z);
    }
    for (v, value, base) in changes {
        set_override(data, &v, value, base);
    }
    if let Some(e) = event {
        data.edit_event = Some(data.edit_event.unwrap_or(false) || e);
    }
    if changed_count && let Some(m) = &mut data.model {
        let before = m.deck.clone();
        let result = m
            .deck
            .set_radial_count(&name, count)
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

/// Puts radial set (cards `lines`, hub height variable `z_name`) where
/// `place` says, `z` metres above the ground (hub at `hub`, the antenna
/// lifted `lift` by the Ground view). Radials meeting the antenna's base go
/// with it: raised or on the ground, the whole antenna moves (the Ground
/// view's height) so its base stays on the hub; buried, the antenna stands
/// on the ground and only the radials go under it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn move_to(
    data: &mut Data,
    lines: &[usize],
    z_name: &str,
    base: f64,
    hub: [f64; 3],
    lift: f64,
    place: Place,
    z: f64,
) {
    let Some(m) = &mut data.model else { return };
    let near = |p: [f64; 3]| {
        let tol = 1e-6 * hub[0].abs().max(hub[1].abs()).max(1.0);
        (p[0] - hub[0]).abs() <= tol && (p[1] - hub[1]).abs() <= tol
    };
    // The lowest wire end standing over the hub: the antenna's base.
    let anchor = m
        .wires
        .iter()
        .filter(|w| !lines.contains(&w.line))
        .flat_map(|w| [w.a, w.b])
        .filter(|&p| near(p))
        .map(|p| p[2])
        .reduce(f64::min);
    let hub_z = match anchor {
        Some(a) => {
            let level = if place == Place::Raised { z } else { 0.0 };
            let shift = level - a;
            let height_is_a_variable = m.deck.height_line().is_some_and(|l| {
                matches!(&m.deck.lines[l].kind, straws_core::deck::LineKind::Card { fields, .. }
                    if fields.get(7).is_some_and(|f| straws_core::deck::edit::plain_number(f).is_none()))
            });
            if shift.abs() > 1e-9 {
                if height_is_a_variable {
                    data.message(
                        Level::Warning,
                        "The antenna's height is set by a variable: change it in the Ground view.",
                    );
                    return;
                }
                let before = m.deck.clone();
                let now = m.deck.height_shift();
                m.deck.set_height_shift(now + shift);
                if let Err(why) = m.keep_if_valid(before, &data.overrides) {
                    data.message(Level::Warning, format!("Not moved: {why}."));
                    return;
                }
            }
            let a_deck = a - lift;
            if place == Place::Buried {
                a_deck + z
            } else {
                a_deck
            }
        }
        None => z - lift,
    };
    set_override(data, z_name, hub_z, base);
    data.edit_event = Some(true);
}

/// The Raised / On the ground / Buried buttons for a counterpoise now at
/// `place`. Returns the place chosen and its height above the ground.
pub(crate) fn place_row(
    ui: &mut egui::Ui,
    place: Place,
    wire_radius: f64,
    top_mhz: f64,
    editable: bool,
) -> Option<(Place, f64)> {
    let mut chosen = None;
    ui.horizontal(|ui| {
        let raised_at = (20.0 * wire_radius)
            .max(2e-3 * straws_core::units::wavelength(top_mhz))
            .max(0.25);
        for (p, label, z, tip) in [
            (
                Place::Raised,
                "Raised",
                raised_at,
                "Above the ground: modelled as NEC wires",
            ),
            (
                Place::OnGround,
                "On the ground",
                0.0,
                "Lying on the ground: estimated",
            ),
            (
                Place::Buried,
                "Buried",
                -0.1,
                "10 cm under the ground (change the height for another depth): estimated",
            ),
        ] {
            let r = ui
                .add_enabled(editable, egui::Button::selectable(place == p, label))
                .on_hover_text(tip);
            if r.clicked() && place != p {
                chosen = Some((p, z));
            }
        }
    });
    chosen
}
