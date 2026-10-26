//! Band plans: the toolbar's Band menu, the editor for the user's own
//! bands, and the bands shaded on plots.

use eframe::egui;
use straws_core::bands::{self, Band};

use crate::app::Settings;

/// The band lists to choose from (`Settings::band_plan` indexes this).
pub const PLANS: [&str; 4] = [
    "ITU Region 1: Europe, Africa, the Middle East, northern Asia",
    "ITU Region 2: the Americas",
    "ITU Region 3: the rest of Asia and the Pacific",
    "My own bands",
];

/// The bands of the chosen plan.
pub fn chosen(settings: &Settings) -> Vec<Band> {
    match settings.band_plan {
        k @ 0..=2 => bands::itu_region(k as u8 + 1),
        _ => bands::tidy(&own(settings)),
    }
}

fn own(settings: &Settings) -> Vec<Band> {
    settings
        .my_bands
        .iter()
        .map(|(n, a, b)| Band::new(n, *a, *b))
        .collect()
}

/// A short name for the plan, for menus.
fn short(plan: usize) -> &'static str {
    match plan {
        0 => "ITU Region 1",
        1 => "ITU Region 2",
        2 => "ITU Region 3",
        _ => "my own bands",
    }
}

/// A Band menu: returns the band picked. `range` is the current sweep
/// (shown as the band's name when it matches one). `edit` is set when the
/// user asks to edit their own bands.
pub fn menu(
    ui: &mut egui::Ui,
    id: &str,
    settings: &mut Settings,
    range: (f64, f64),
    edit: &mut bool,
) -> Option<Band> {
    let list = chosen(settings);
    let near = |a: f64, b: f64| (a - b).abs() <= 1e-6 * b.abs().max(1.0);
    let current = list
        .iter()
        .find(|b| near(b.start_mhz, range.0) && near(b.stop_mhz, range.1));
    let mut picked = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(current.map_or("Band", |b| b.name.as_str()))
        .width(70.0)
        .height(700.0)
        .show_ui(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Bands from");
                for (k, name) in PLANS.iter().enumerate() {
                    let label = if k < 3 { short(k) } else { "My own" };
                    ui.selectable_value(&mut settings.band_plan, k, label)
                        .on_hover_text(*name);
                }
                if ui.button("Edit my own…").clicked() {
                    *edit = true;
                }
            });
            ui.separator();
            if list.is_empty() {
                ui.weak("No bands in your own list yet");
            }
            for b in &list {
                if ui
                    .selectable_label(
                        current == Some(b),
                        format!("{}   {}-{} MHz", b.name, b.start_mhz, b.stop_mhz),
                    )
                    .clicked()
                {
                    picked = Some(b.clone());
                }
            }
        })
        .response
        .on_hover_text(
            "Pick a band to sweep across it. The ITU lists are each region's amateur \
             allocations; your own licence may allow less (or more), so you can keep your \
             own list too.",
        );
    picked
}

/// The editor for the user's own bands (a modal while `open`).
pub fn editor(ctx: &egui::Context, settings: &mut Settings, open: &mut bool) {
    if !*open {
        return;
    }
    egui::Modal::new(egui::Id::new("my bands")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.heading("My own bands");
        ui.label(
            "The bands your licence allows. They show in the Band menus and are shaded on \
             the SWR plot when \"My own bands\" is chosen.",
        );
        ui.add_space(6.0);
        let mut remove = None;
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                egui::Grid::new("my bands grid")
                    .num_columns(4)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.strong("Name");
                        ui.strong("From (MHz)");
                        ui.strong("To (MHz)");
                        ui.end_row();
                        for (k, (name, a, b)) in settings.my_bands.iter_mut().enumerate() {
                            ui.add_sized([110.0, 20.0], egui::TextEdit::singleline(name));
                            ui.add(crate::views::num(a).speed(0.001).max_decimals(4));
                            ui.add(crate::views::num(b).speed(0.001).max_decimals(4));
                            if ui
                                .small_button("✖")
                                .on_hover_text("Remove this band")
                                .clicked()
                            {
                                remove = Some(k);
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(k) = remove {
            settings.my_bands.remove(k);
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add a band").clicked() {
                let (a, b) = settings
                    .my_bands
                    .last()
                    .map_or((14.0, 14.35), |(_, _, b)| (*b, *b + 1.0));
                settings.my_bands.push((String::new(), a, b));
            }
            ui.label("Start from:");
            for r in 1..=3u8 {
                if ui
                    .button(format!("Region {r}"))
                    .on_hover_text(
                        "Replace the list with this ITU region's bands, to trim to your licence",
                    )
                    .clicked()
                {
                    settings.my_bands = bands::itu_region(r)
                        .into_iter()
                        .map(|b| (b.name, b.start_mhz, b.stop_mhz))
                        .collect();
                }
            }
        });
        ui.add_space(6.0);
        if ui.button("Done").clicked() {
            settings.my_bands = bands::tidy(&own(settings))
                .into_iter()
                .map(|b| (b.name, b.start_mhz, b.stop_mhz))
                .collect();
            settings.band_plan = 3;
            *open = false;
        }
    });
}

/// The shading for a band on a plot (a light wash that suits either theme).
pub fn shade(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgba_unmultiplied(86, 180, 233, 26)
    } else {
        egui::Color32::from_rgba_unmultiplied(0, 114, 178, 22)
    }
}
