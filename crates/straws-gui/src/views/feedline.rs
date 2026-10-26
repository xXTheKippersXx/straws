//! The feedline view: what the radio sees through the coax (or ladder
//! line) and balun, and how much power reaches the antenna.

use eframe::egui::{self, Color32, RichText};
use egui_plot::{Legend, Line, Plot, PlotPoints, VLine};
use straws_core::analysis::feed::swr;
use straws_core::analysis::feedline::{CABLES, CONNECTORS, Cable, Connector, Feed, at_radio};

use straws_core::deck::write::format_real;

use crate::app::{Data, Settings};
use crate::views::newvar::{Kind, Request};

use super::empty_note;
use super::smith::trace_colour;

const BALUNS: &[(&str, f64)] = &[
    ("None, or a 1:1 balun", 1.0),
    ("4:1", 4.0),
    ("9:1", 9.0),
    ("16:1", 16.0),
    ("49:1 (end-fed half-wave)", 49.0),
    ("64:1", 64.0),
];

/// Outside radius (m) of the cable's shield, for modelling it as a wire.
fn shield_radius(name: &str) -> f64 {
    match name {
        "RG-58C/U" => 0.0025,
        "RG-8X (mini-8)" => 0.003,
        "RG-6 (75 Ω)" => 0.0035,
        _ => 0.005,
    }
}

/// The coax shield as a wire: add, update or remove it, and how much
/// current flows back down it (common-mode current).
fn shield_ui(ui: &mut egui::Ui, data: &mut Data, settings: &mut Settings) {
    use straws_core::deck::shield::{choke, shield_tag};
    let Some(model) = &data.model else { return };
    let current_tag = shield_tag(&model.deck);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Coax shield").strong()).on_hover_text(
            "Models the outside of the coax as a wire from the feed point, so you can see \
             common-mode current: current that flows back down the shield instead of into \
             the antenna (it makes the coax radiate and brings RF into the shack).",
        );
        ui.label("choke");
        let mut choke_r = get(data, "choke", settings.shield_choke);
        if ui
            .add(crate::views::num(&mut choke_r).range(0.0..=100_000.0).speed(10.0).suffix(" Ω"))
            .on_hover_text("Resistance of a choke (current balun) at the top of the shield; 0 for none. A good choke is a few thousand ohms.")
            .changed()
        {
            set(data, "choke", &mut settings.shield_choke, choke_r);
        }
        f_button(ui, data, "choke", choke_r, "choke", "ohm", "The choke's resistance becomes a variable (it sets the choke card once the shield is modelled).");
        let unit = settings.length();
        let mhz = data.mhz.max(1e-3);
        let shield_m = get(data, "shield_length", settings.shield_length);
        let mut len = unit.from_metres(shield_m, mhz);
        ui.label("length");
        if ui
            .add(crate::views::num(&mut len).range(0.0..=1e5).speed(0.1).max_decimals(3).suffix(format!(" {}", unit.symbol())))
            .on_hover_text("How far the shield runs straight down; 0 means to the ground (or half a wavelength in free space)")
            .changed()
        {
            set(data, "shield_length", &mut settings.shield_length, unit.to_metres(len, mhz).max(0.0));
        }
        f_button(ui, data, "shield_length", shield_m, "shield_len", "m", "The shield's length becomes a variable (used when you model or update the shield).");
    });
    let Some(model) = &data.model else { return };
    // Where the shield starts: one end of the fed segment.
    let feed = model.feed_segments().first().copied();
    let top = feed
        .and_then(|n| model.segments.iter().find(|s| s.number == n))
        .map(|s| s.ends().0);
    let mut change: Option<bool> = None;
    ui.horizontal_wrapped(|ui| {
        let label = if current_tag.is_some() { "Update the shield" } else { "Model the shield" };
        if ui
            .add_enabled(top.is_some(), egui::Button::new(label))
            .on_disabled_hover_text("Needs a model with a voltage source")
            .clicked()
        {
            change = Some(true);
        }
        if current_tag.is_some() && ui.button("Remove the shield").clicked() {
            change = Some(false);
        }
        // How much current is on the shield.
        if let (Some(tag), Some(p)) = (current_tag, data.shown_point())
            && let Some(feed) = p.feed(data.feed)
            && let Some(top_seg) = p
                .currents
                .iter()
                .filter(|c| i64::from(c.tag) == tag)
                .min_by_key(|c| c.segment)
        {
            let ratio = top_seg.current.norm() / feed.current.norm().max(1e-30);
            let db = 20.0 * ratio.max(1e-12).log10();
            let ok = ratio < 0.1;
            let text = format!(
                "Common-mode current at the top of the shield: {:.1}% of the feed current ({db:.1} dB)",
                100.0 * ratio
            );
            if ok {
                ui.label(text);
            } else {
                ui.colored_label(ui.visuals().warn_fg_color, text)
                    .on_hover_text("More than about 10% is worth a choke");
            }
            if let Ok(n) = &model.numeric
                && let Some((r, x)) = choke(n, &model.deck)
            {
                ui.label(RichText::new(format!("(choke {r:.0} {} j{:.0} Ω)", if x < 0.0 { "−" } else { "+" }, x.abs())).weak());
            }
        }
    });
    let Some(add) = change else { return };
    let shield_m = get(data, "shield_length", settings.shield_length);
    let choke_r = get(data, "choke", settings.shield_choke);
    let Some(m) = &mut data.model else { return };
    let before = m.deck.clone();
    if add {
        let Some(top) = top else { return };
        let lam = straws_core::units::wavelength(data.mhz.max(1e-3));
        let length = if shield_m > 0.0 {
            shield_m
        } else if data.over_ground {
            top[2].max(0.01)
        } else {
            lam / 2.0
        };
        let bottom = [top[0], top[1], top[2] - length];
        let tag = shield_tag(&m.deck).unwrap_or_else(|| m.next_tag());
        let segments = crate::views::wires::auto_segments(length, data.max_mhz);
        m.deck.add_shield(
            tag,
            top,
            bottom,
            segments,
            shield_radius(cable(settings).name),
            choke_r,
            0.0,
        );
        // A choke variable sets the choke card.
        if let Some((name, _)) = link(&m.deck, &data.overrides, "choke")
            && let Some(l) = straws_core::deck::shield::choke_line(&m.deck)
        {
            m.deck.set_field(l, 4, &name);
        }
    } else {
        m.deck.remove_shield();
    }
    match m.keep_if_valid(before, &data.overrides) {
        Ok(()) => data.edit_event = Some(true),
        Err(why) => data.message(
            crate::app::Level::Warning,
            format!("Shield not changed: {why}."),
        ),
    }
}

/// The cable in use: a listed one, or the user's own.
pub fn cable(settings: &Settings) -> Cable {
    match CABLES.get(settings.feed_cable) {
        Some(c) => *c,
        None => Cable {
            name: "Custom",
            z0: settings.feed_z0.max(1.0),
            velocity_factor: settings.feed_vf.clamp(0.1, 1.0),
            loss_10: settings.feed_loss10.max(0.0),
            loss_100: settings.feed_loss100.max(0.0),
        },
    }
}

/// The connector chosen (0: none, 1-3: `CONNECTORS`, 4: other with its
/// own loss).
fn connector(choice: usize, other_db: f64) -> Option<Connector> {
    match choice {
        0 => None,
        k if k <= CONNECTORS.len() => Some(CONNECTORS[k - 1]),
        _ => Some(Connector::other(other_db.max(0.0))),
    }
}

/// Connector choices for both ends, with their loss at the slider
/// frequency and a warning above their rating.
fn connectors_ui(ui: &mut egui::Ui, settings: &mut Settings, mhz: f64) {
    let pick = |ui: &mut egui::Ui, id: &str, choice: &mut usize, other: &mut f64| {
        let name = connector(*choice, *other).map_or("none", |c| c.name);
        egui::ComboBox::from_id_salt(id)
            .selected_text(name)
            .width(60.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(choice, 0, "none");
                for (k, c) in CONNECTORS.iter().enumerate() {
                    ui.selectable_value(choice, k + 1, c.name).on_hover_text(c.source);
                }
                ui.selectable_value(choice, CONNECTORS.len() + 1, "Other")
                    .on_hover_text("Type its loss. UHF (PL-259/SO-239) has no dependable published figure (one maker's sheet: 0.5 dB max, frequency not given; rated to 300 MHz), so measure yours or use this.");
            });
        if *choice > CONNECTORS.len() {
            ui.add(
                crate::views::num(other)
                    .range(0.0..=3.0)
                    .speed(0.01)
                    .suffix(" dB"),
            );
        }
        if let Some(c) = connector(*choice, *other) {
            let loss = c.loss_db(mhz);
            let text = format!("{loss:.3} dB");
            if mhz / 1000.0 > c.max_ghz {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("{text}: above its {} GHz rating", c.max_ghz),
                )
                .on_hover_text(c.source);
            } else {
                ui.label(text).on_hover_text(format!(
                    "At the slider frequency; {}. That's a maximum: good parts lose less.",
                    c.source
                ));
            }
        }
    };
    ui.horizontal_wrapped(|ui| {
        ui.label("Connectors: at the antenna");
        pick(ui, "conn antenna", &mut settings.conn_antenna, &mut settings.conn_antenna_db);
        if settings.conn_antenna != 0 {
            ui.add(
                crate::views::num(&mut settings.conn_length_mm)
                    .range(0.0..=500.0)
                    .speed(0.5)
                    .prefix("adds ")
                    .suffix(" mm of line"),
            )
            .on_hover_text("The length of 50 Ω line inside the connectors (and any adapter) between the antenna and the cable, PTFE-filled. It only matters at UHF and up, where a few centimetres turn the impedance. Measure it from the feed point to where the cable starts; 0 to leave it out.");
        }
        ui.label("at the radio");
        pick(ui, "conn radio", &mut settings.conn_radio, &mut settings.conn_radio_db);
    });
}

/// The feed system as the Feedline view has it (values set by variables
/// tagged for it win).
pub fn current_feed(data: &Data, settings: &Settings) -> Feed {
    let mut cable = cable(settings);
    if settings.feed_cable >= CABLES.len() {
        cable.z0 = get(data, "feed_z0", cable.z0).max(1.0);
        cable.velocity_factor = get(data, "feed_vf", cable.velocity_factor).clamp(0.1, 1.0);
        cable.loss_10 = get(data, "feed_loss10", cable.loss_10).max(0.0);
        cable.loss_100 = get(data, "feed_loss100", cable.loss_100).max(0.0);
    }
    Feed {
        cable,
        length_m: get(data, "feed_length", settings.feed_length).max(0.0),
        balun_ratio: get(data, "balun_ratio", settings.balun_ratio).max(1e-6),
        balun_loss_db: get(data, "balun_loss", settings.balun_loss).max(0.0),
        antenna_connector: connector(settings.conn_antenna, settings.conn_antenna_db),
        connector_length_m: settings.conn_length_mm.max(0.0) / 1000.0,
        radio_connector: connector(settings.conn_radio, settings.conn_radio_db),
    }
}

pub fn ui(ui: &mut egui::Ui, data: &mut Data, settings: &mut Settings) {
    crate::views::feeds::feed_picker(ui, data);
    ui.horizontal_wrapped(|ui| {
        ui.label("Cable");
        let name = cable(settings).name;
        egui::ComboBox::from_id_salt("cable")
            .selected_text(name)
            .width(180.0)
            .show_ui(ui, |ui| {
                for (k, c) in CABLES.iter().enumerate() {
                    ui.selectable_value(&mut settings.feed_cable, k, c.name)
                        .on_hover_text(format!(
                            "{} Ω, velocity factor {}, {} dB/100 m at 10 MHz, {} at 100 MHz",
                            c.z0, c.velocity_factor, c.loss_10, c.loss_100
                        ));
                }
                ui.selectable_value(&mut settings.feed_cable, CABLES.len(), "Custom");
            });
        let unit = settings.length();
        let mhz = data.mhz.max(1e-3);
        let feed_m = get(data, "feed_length", settings.feed_length);
        let mut len = unit.from_metres(feed_m, mhz);
        ui.label("length");
        if ui
            .add(
                crate::views::num(&mut len)
                    .range(0.0..=1e6)
                    .speed(0.1)
                    .max_decimals(3)
                    .suffix(format!(" {}", unit.symbol())),
            )
            .changed()
        {
            set(
                data,
                "feed_length",
                &mut settings.feed_length,
                unit.to_metres(len, mhz).max(0.0),
            );
        }
        f_button(
            ui,
            data,
            "feed_length",
            feed_m,
            "feed_len",
            "m",
            "The feedline's length becomes a variable.",
        );
        ui.label("balun/unun");
        let mut ratio = get(data, "balun_ratio", settings.balun_ratio);
        let before = ratio;
        let current = BALUNS
            .iter()
            .find(|b| (b.1 - ratio).abs() < 1e-9)
            .map_or("Custom", |b| b.0);
        egui::ComboBox::from_id_salt("balun")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (name, r) in BALUNS {
                    ui.selectable_value(&mut ratio, *r, *name);
                }
            });
        ui.add(
            crate::views::num(&mut ratio)
                .range(0.01..=100.0)
                .speed(0.05)
                .prefix("ratio ")
                .suffix(":1"),
        )
        .on_hover_text("Impedance ratio: the antenna's impedance is divided by this");
        if ratio != before {
            set(data, "balun_ratio", &mut settings.balun_ratio, ratio);
        }
        f_button(
            ui,
            data,
            "balun_ratio",
            ratio,
            "balun_ratio",
            "",
            "The balun's impedance ratio becomes a variable.",
        );
        let mut loss = get(data, "balun_loss", settings.balun_loss);
        if ui
            .add(
                crate::views::num(&mut loss)
                    .range(0.0..=6.0)
                    .speed(0.01)
                    .prefix("loss ")
                    .suffix(" dB"),
            )
            .on_hover_text("Balun loss (about 0.1-0.3 dB for a good one, more for a big ratio)")
            .changed()
        {
            set(data, "balun_loss", &mut settings.balun_loss, loss);
        }
        f_button(
            ui,
            data,
            "balun_loss",
            loss,
            "balun_loss",
            "dB",
            "The balun's loss becomes a variable.",
        );
    });
    if settings.feed_cable >= CABLES.len() {
        ui.horizontal_wrapped(|ui| {
            let custom = [
                ("Z₀", "feed_z0", "cable_z0", " Ω", 1.0, 1000.0, 1.0, "ohm"),
                (
                    "velocity factor",
                    "feed_vf",
                    "cable_vf",
                    "",
                    0.1,
                    1.0,
                    0.005,
                    "",
                ),
                (
                    "loss at 10 MHz",
                    "feed_loss10",
                    "cable_loss10",
                    " dB/100 m",
                    0.0,
                    100.0,
                    0.05,
                    "dB/100 m",
                ),
                (
                    "at 100 MHz",
                    "feed_loss100",
                    "cable_loss100",
                    " dB/100 m",
                    0.0,
                    300.0,
                    0.05,
                    "dB/100 m",
                ),
            ];
            for (label, role, name, suffix, lo, hi, speed, unit) in custom {
                let setting = match role {
                    "feed_z0" => &mut settings.feed_z0,
                    "feed_vf" => &mut settings.feed_vf,
                    "feed_loss10" => &mut settings.feed_loss10,
                    _ => &mut settings.feed_loss100,
                };
                ui.label(label);
                let mut v = get(data, role, *setting);
                if ui
                    .add(
                        crate::views::num(&mut v)
                            .range(lo..=hi)
                            .speed(speed)
                            .suffix(suffix),
                    )
                    .changed()
                {
                    set(data, role, setting, v);
                }
                f_button(
                    ui,
                    data,
                    role,
                    v,
                    name,
                    unit,
                    &format!("The cable's {label} becomes a variable."),
                );
            }
        });
    }
    let feed = current_feed(data, settings);
    connectors_ui(ui, settings, data.mhz);

    shield_ui(ui, data, settings);
    ui.separator();

    let Some(sweep) = data.display_sweep() else {
        let note = data.waiting_note(
            "Open a model to see what the radio sees through the feedline.",
            "Calculating…",
        );
        return empty_note(ui, note);
    };
    let z0 = feed.cable.z0;
    if let Some(p) = data.shown_point()
        && let Some(i) = p.feed(data.feed)
    {
        let r = at_radio(&feed, p.mhz, i.impedance);
        let z = r.impedance;
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{:.4} MHz: at the radio Z = {:.2} {} j{:.2} Ω, SWR {:.2} ({z0:.0} Ω). Antenna end SWR {:.2}. Loss {:.2} dB ({:.2} dB matched), so {:.0} W of every 100 W reaches the antenna.",
                p.mhz,
                z.re,
                if z.im < 0.0 { "−" } else { "+" },
                z.im.abs(),
                r.swr_line,
                swr(i.impedance / feed.balun_ratio.max(1e-6), z0),
                r.total_loss_db,
                r.matched_loss_db,
                100.0 * r.efficiency
            ))
            .monospace(),
        );
        ui.label(
            RichText::new(
                "Line loss makes the SWR at the radio look better than at the antenna: the difference is power warming the cable.",
            )
            .weak(),
        );
    }
    let antenna: PlotPoints = sweep
        .points()
        .filter_map(|p| {
            p.feed(data.feed).map(|i| {
                [
                    p.mhz,
                    swr(i.impedance / feed.balun_ratio.max(1e-6), z0).min(20.0),
                ]
            })
        })
        .collect();
    let radio: PlotPoints = sweep
        .points()
        .filter_map(|p| {
            p.feed(data.feed).map(|i| {
                [
                    p.mhz,
                    at_radio(&feed, p.mhz, i.impedance).swr_line.min(20.0),
                ]
            })
        })
        .collect();
    let loss: PlotPoints = sweep
        .points()
        .filter_map(|p| {
            p.feed(data.feed).map(|i| {
                [
                    p.mhz,
                    at_radio(&feed, p.mhz, i.impedance).total_loss_db.min(30.0),
                ]
            })
        })
        .collect();
    let dark = ui.visuals().dark_mode;
    let marker = ui.visuals().weak_text_color();
    let blue = Color32::from_rgb(86, 180, 233);
    let h = ((ui.available_height() - 8.0) / 2.0).max(80.0);
    let link = ui.id().with("feed-link");
    Plot::new("feed swr")
        .height(h)
        .link_axis(link, [true, false])
        .include_y(1.0)
        .include_y(3.0)
        .y_axis_label("SWR")
        .allow_scroll(false)
        .legend(Legend::default())
        .show(ui, |pui| {
            pui.line(
                Line::new("at the antenna (after the balun)", antenna)
                    .color(trace_colour(dark))
                    .width(1.5),
            );
            pui.line(Line::new("at the radio", radio).color(blue).width(2.0));
            pui.vline(VLine::new("frequency", data.mhz).color(marker));
        });
    Plot::new("feed loss")
        .height(h)
        .link_axis(link, [true, false])
        .include_y(0.0)
        .y_axis_label("loss, dB")
        .x_axis_label("MHz")
        .allow_scroll(false)
        .show(ui, |pui| {
            pui.line(Line::new("total loss", loss).color(blue).width(2.0));
            pui.vline(VLine::new("frequency", data.mhz).color(marker));
        });
}

/// The deck variable tagged for setting `role` here (`use=role` in its
/// notes), with its value now: (name, value).
fn link(
    deck: &straws_core::deck::Deck,
    overrides: &[(String, f64)],
    role: &str,
) -> Option<(String, f64)> {
    deck.variables().into_iter().find_map(|v| {
        if deck.notes(&v.name).role != role {
            return None;
        }
        let value = overrides
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&v.name))
            .map(|(_, x)| *x)
            .or(v.literal)?;
        Some((v.name, value))
    })
}

/// The Feedline view's chain for an optimiser goal measured at the radio:
/// balun, connectors (their loss at `mhz`), cable; the cable's length and
/// the balun's ratio follow their variables, if variables set them.
pub(crate) fn radio_chain(
    data: &Data,
    settings: &Settings,
    mhz: f64,
) -> straws_core::optimise::RadioChain {
    let feed = current_feed(data, settings);
    let (stages, line) = straws_core::analysis::feedsystem::chain_from_feed(&feed, mhz);
    let mut vars = Vec::new();
    if let Some(m) = &data.model {
        for (k, role) in [(0, "balun_ratio"), (line, "feed_length")] {
            if let Some((name, _)) = link(&m.deck, &[], role) {
                vars.push((k, name));
            }
        }
    }
    straws_core::optimise::RadioChain { stages, vars }
}

/// The value of setting `role`: its variable's, if one sets it.
fn get(data: &Data, role: &str, setting: f64) -> f64 {
    data.model
        .as_ref()
        .and_then(|m| link(&m.deck, &data.overrides, role))
        .map_or(setting, |(_, v)| v)
}

/// Changes setting `role`: its variable (like its slider), if one sets it.
pub(crate) fn set(data: &mut Data, role: &str, setting: &mut f64, value: f64) {
    let linked = data.model.as_ref().and_then(|m| {
        let (name, _) = link(&m.deck, &[], role)?;
        let base = m
            .deck
            .variables()
            .into_iter()
            .find(|v| v.name == name)
            .and_then(|v| v.literal)?;
        Some((name, base))
    });
    match linked {
        Some((name, base)) => {
            data.overrides
                .retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
            if value != base {
                data.overrides.push((name, value));
            }
            // The choke is a card in the deck: rerun the model.
            if role == "choke" {
                data.edit_event = Some(true);
            }
        }
        None => *setting = value,
    }
}

/// The ƒ button that makes setting `role` a variable, or a note naming the
/// variable that sets it.
fn f_button(
    ui: &mut egui::Ui,
    data: &mut Data,
    role: &str,
    value: f64,
    name: &str,
    unit: &str,
    what: &str,
) {
    let Some(m) = &data.model else { return };
    match link(&m.deck, &data.overrides, role) {
        Some((linked, _)) => {
            ui.label(RichText::new(format!("= {linked}")).color(ui.visuals().hyperlink_color))
                .on_hover_text(
                    "Set by this variable: changing it here moves its slider in the Variables view",
                );
        }
        None => {
            if ui
                .small_button("ƒ")
                .on_hover_text("Make this a variable, with its own slider (kept in the deck)")
                .clicked()
            {
                let mut r = Request::new(
                    &m.deck,
                    Kind::Plain {
                        value: format_real(value),
                    },
                    name,
                    unit,
                    what.to_string(),
                );
                r.notes.role = role.to_string();
                r.notes.group = "Feedline".into();
                data.new_variable = Some(r);
            }
        }
    }
}
