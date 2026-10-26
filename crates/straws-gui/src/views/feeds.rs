//! The Feeds view: every source's amplitude and phase, with sliders and
//! exact entry, presets for phased arrays, and what each feed point does
//! (its active impedance, power and the current it gets).

use eframe::egui::{self, RichText};
use straws_core::Complex64;
use straws_core::analysis::feed::swr;
use straws_core::analysis::feedline::CABLES;
use straws_core::analysis::phasing;
use straws_core::deck::sources::{Source, SourceKind, sources};

use crate::app::{Data, Level, Settings};
use crate::views::variables::set_override;

use super::empty_note;

/// A change to the feed network, asked for in the phasing section.
enum PhasingAction {
    Write {
        design: phasing::Design,
        cable: usize,
        at: [f64; 3],
    },
    Remove,
}

/// Where a phasing junction goes: in the middle of the feed points (where
/// a phasing box would sit), a hundredth of a wavelength above their
/// average height to keep its tiny wire clear of the ground.
fn junction_at(positions: &[[f64; 3]], mhz: f64) -> [f64; 3] {
    let n = positions.len().max(1) as f64;
    let mid = [0, 1, 2].map(|k| positions.iter().map(|p| p[k]).sum::<f64>() / n);
    [
        mid[0],
        mid[1],
        mid[2] + straws_core::units::wavelength(mhz) / 100.0,
    ]
}

/// NEC's number for segment `seg` (from 1) of wire `tag`.
fn segment_number(model: &crate::model::LoadedModel, tag: i64, seg: i64) -> Option<u32> {
    Some(
        model
            .segments
            .iter()
            .filter(|s| i64::from(s.tag) == tag)
            .nth(usize::try_from(seg - 1).ok()?)?
            .number,
    )
}

/// The current NEC gives segment `seg` of wire `tag` at `point`.
fn current_on(
    model: &crate::model::LoadedModel,
    point: &straws_core::analysis::sweep::FreqPoint,
    tag: i64,
    seg: i64,
) -> Option<Complex64> {
    let number = segment_number(model, tag, seg)?;
    point
        .currents
        .iter()
        .find(|c| c.segment == number)
        .map(|c| c.current)
}

/// "1.000 A at -90.0°"
fn amps(c: Complex64) -> String {
    format!("{:.3} A at {:.1}°", c.norm(), c.arg().to_degrees())
}

/// The hybrid-coupler designer: each element on output A or B through its
/// own line, the lines tuned for the wanted currents (at the slider
/// frequency or over the sweep), and what the radio and the dump load get.
#[allow(clippy::too_many_arguments)]
fn hybrid_section(
    ui: &mut egui::Ui,
    model: &crate::model::LoadedModel,
    list: &[Source],
    z: &[Complex64],
    want: &[Complex64],
    min_len: &[f64],
    band: &[phasing::BandPoint],
    cable: &straws_core::analysis::feedline::Cable,
    mhz: f64,
    tune: bool,
    settings: &Settings,
) {
    use straws_core::analysis::hybrid;
    let unit = settings.length();
    let len = |m: f64| format!("{:.3} {}", unit.from_metres(m, mhz), unit.symbol());
    let n = z.len();
    let reference = (0..n)
        .max_by(|&a, &b| want[a].norm().total_cmp(&want[b].norm()))
        .unwrap_or(0);
    let groups_id = egui::Id::new("hybrid groups");
    let mut groups = ui
        .data(|d| d.get_temp::<Vec<bool>>(groups_id))
        .filter(|g| g.len() == n)
        .unwrap_or_else(|| hybrid::default_groups(want, reference));
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (cable.name, mhz.to_bits(), tune, &groups).hash(&mut h);
        for c in z.iter().chain(want) {
            (c.re.to_bits(), c.im.to_bits()).hash(&mut h);
        }
        for m in min_len {
            m.to_bits().hash(&mut h);
        }
        if tune {
            for (f, zs) in band {
                f.to_bits().hash(&mut h);
                for c in zs {
                    (c.re.to_bits(), c.im.to_bits()).hash(&mut h);
                }
            }
        }
        h.finish()
    };
    let feed_id = egui::Id::new("hybrid feed");
    // While something is being dragged, keep the last design (redesigning
    // can take a few hundred ms) and redo it once the mouse is let go.
    let dragging = ui.input(|i| i.pointer.any_down());
    let cached = ui.data(|d| d.get_temp::<(u64, hybrid::HybridFeed)>(feed_id));
    let stale = cached
        .as_ref()
        .is_some_and(|(k, f)| *k != key && f.lengths.len() == n)
        && dragging;
    let feed = match cached {
        Some((k, f)) if k == key || (dragging && f.lengths.len() == n) => f,
        _ => {
            let f = hybrid::design(
                z,
                want,
                &groups,
                min_len,
                cable,
                mhz,
                if tune { band } else { &[] },
            );
            ui.data_mut(|d| d.insert_temp(feed_id, (key, f.clone())));
            f
        }
    };
    if stale {
        ui.label(RichText::new("(updating when you let go)").weak().small());
    }
    let r = hybrid::evaluate(z, &feed, cable, mhz);
    let scale = if r.currents[reference].norm() > 0.0 {
        want[reference] / r.currents[reference]
    } else {
        Complex64::new(1.0, 0.0)
    };
    let lam_line = straws_core::units::wavelength(mhz) * cable.velocity_factor;
    egui::Grid::new("hybrid grid")
        .num_columns(6)
        .striped(true)
        .spacing([10.0, 3.0])
        .show(ui, |ui| {
            for h in [
                "Element",
                "Output",
                "Line",
                "Electrical",
                "Wanted",
                "Hybrid gives",
            ] {
                ui.label(RichText::new(h).weak());
            }
            ui.end_row();
            for (k, s) in list.iter().enumerate() {
                ui.label(format!(
                    "{}{}",
                    source_name(model, s),
                    if k == reference { " (ref)" } else { "" }
                ));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut groups[k], false, "A (0°)");
                    ui.selectable_value(&mut groups[k], true, "B (-90°)");
                });
                ui.label(len(feed.lengths[k]));
                ui.label(format!("{:.1}°", 360.0 * feed.lengths[k] / lam_line));
                ui.label(amps(want[k]));
                ui.label(amps(r.currents[k] * scale));
                ui.end_row();
            }
        });
    ui.data_mut(|d| d.insert_temp(groups_id, groups));
    let (db, deg, vector) = hybrid::errors(&[(mhz, z.to_vec())], want, &feed, cable, reference);
    ui.label(format!(
        "Worst {:.0}% of the wanted current wrong ({db:.2} dB, {deg:.1}°). The radio sees {:.1} {} j{:.1} Ω (SWR {:.2} on {} Ω); {:.1}% of its power goes into the hybrid's dump load.",
        100.0 * vector,
        r.input.re,
        if r.input.im < 0.0 { "-" } else { "+" },
        r.input.im.abs(),
        swr(r.input, cable.z0),
        cable.z0,
        100.0 * r.dumped
    ))
    .on_hover_text("A hybrid fixes the waves it sends out, not the currents: elements with very different impedances can't all get exactly what's wanted (the lines are the best possible). What they reflect goes to the dump load, so the radio sees a good match whatever they do. The hybrid here is ideal and the same at every frequency; a real branch-line one is narrowband too");
    if band.len() > 1 {
        let (db, deg, vector) = hybrid::errors(band, want, &feed, cable, reference);
        ui.label(format!(
            "Across the sweep ({:.3} to {:.3} MHz): worst {:.0}% wrong ({db:.2} dB, {deg:.1}° at worst).",
            band[0].0,
            band[band.len() - 1].0,
            100.0 * vector
        ));
    }
    ui.label(
        RichText::new(
            "The hybrid is worked out by STRAWS from the elements' impedances; it isn't written into the NEC deck.",
        )
        .weak()
        .small(),
    );
}

/// The phasing section: the live table of a network in the model, or the
/// designer when the elements are fed separately.
#[allow(clippy::too_many_arguments)]
fn phasing_ui(
    ui: &mut egui::Ui,
    model: &crate::model::LoadedModel,
    list: &[Source],
    positions: &[[f64; 3]],
    results: &[Option<straws_core::output::InputParams>],
    point: Option<&straws_core::analysis::sweep::FreqPoint>,
    band: &[phasing::BandPoint],
    settings: &Settings,
    mhz: f64,
    action: &mut Option<PhasingAction>,
) {
    let unit = settings.length();
    let len = |m: f64| format!("{:.3} {}", unit.from_metres(m, mhz), unit.symbol());
    let net = phasing::network(&model.deck);
    if !net.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new("Phasing network").strong());
        ui.label(
            RichText::new(
                "The elements are fed from one junction through these lines. Currents against \
                 the first element, at the slider frequency: move it to see how the phasing \
                 holds across the band. NEC's lines have no loss, so the real cable's currents \
                 are estimated from NEC's element impedances.",
            )
            .weak()
            .small(),
        );
        // Per line: NEC's current (lossless lines), and an estimate with
        // the cable's loss: the junction's voltage through the real cable
        // into the element's impedance as NEC has it.
        let rows: Vec<(Option<Complex64>, Option<Complex64>)> = net
            .iter()
            .map(|l| {
                let Some(p) = point else { return (None, None) };
                let got = current_on(model, p, l.tag, l.segment);
                let z = segment_number(model, l.tag, l.segment)
                    .and_then(|n| p.network.iter().find(|i| i.segment == n))
                    .map(|i| i.impedance);
                let v = p.inputs.first().map(|i| i.voltage);
                let cable = CABLES.iter().find(|c| c.name == l.cable);
                let est = match (z, v, cable, l.length_m) {
                    (Some(z), Some(v), Some(c), Some(len)) => {
                        let across = phasing::ElementNet {
                            shunt: l.net.shunt,
                            series: None,
                        };
                        Some(v / phasing::branch(c, p.mhz, z, len, &across).0)
                    }
                    _ => None,
                };
                (got, est)
            })
            .collect();
        // Line k's current against the first line's: NEC's, or the estimate.
        let rel = |k: usize, estimate: bool| {
            let pick =
                |r: &(Option<Complex64>, Option<Complex64>)| if estimate { r.1 } else { r.0 };
            pick(&rows[k])
                .zip(pick(&rows[0]))
                .filter(|(_, f)| f.norm() > 0.0)
                .map(|(g, f)| g / f)
        };
        let ratio = |c: Option<Complex64>| {
            c.map_or("-".into(), |g| {
                format!("{:.3} at {:.1}°", g.norm(), g.arg().to_degrees())
            })
        };
        let first_want = net[0].wanted;
        egui::Grid::new("phasing network")
            .num_columns(6)
            .striped(true)
            .spacing([10.0, 3.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Element").weak());
                ui.label(RichText::new("Line").weak());
                ui.label(RichText::new("Wanted").weak());
                ui.label(RichText::new("NEC (lossless lines)").weak()).on_hover_text(
                    "NEC's own result: its transmission lines (TL cards) have no loss",
                );
                ui.label(RichText::new("With the cable's loss").weak()).on_hover_text(
                    "An estimate of what the real (lossy) cable gives: the junction voltage through \
                     the cable into each element's impedance as NEC has it. The lines were designed \
                     for this.",
                );
                ui.label(RichText::new("Off by").weak())
                    .on_hover_text("The estimate with the cable's loss against what's wanted");
                ui.end_row();
                for (k, l) in net.iter().enumerate() {
                    let name = model
                        .wires
                        .iter()
                        .find(|w| w.tag == l.tag)
                        .and_then(|w| model.deck.lines.get(w.line)?.comment.clone())
                        .map(|c| c.trim().chars().take(24).collect::<String>())
                        .filter(|c| !c.is_empty())
                        .unwrap_or_else(|| format!("wire {}", l.tag));
                    ui.label(format!("{name} (seg {})", l.segment));
                    ui.label(format!(
                        "{} {}",
                        l.cable,
                        l.length_m.map_or("?".to_string(), len)
                    ));
                    let want = l.wanted.zip(first_want).map(|(w, f)| w / f);
                    let est = rel(k, true);
                    ui.label(ratio(want));
                    ui.label(ratio(rel(k, false)));
                    ui.label(ratio(est));
                    match (want, est) {
                        (Some(w), Some(g)) if w.norm() > 0.0 => {
                            let r = g / w;
                            ui.label(format!(
                                "{:+.2} dB, {:+.1}°",
                                20.0 * r.norm().log10(),
                                r.arg().to_degrees()
                            ));
                        }
                        _ => {
                            ui.label("");
                        }
                    }
                    ui.end_row();
                }
            });
        if ui
            .button("Back to separate sources")
            .on_hover_text("Take the lines and the junction out, and feed each element with a current source of the current its line was meant to give")
            .clicked()
        {
            *action = Some(PhasingAction::Remove);
        }
        return;
    }
    if list.len() < 2 {
        return;
    }
    ui.add_space(6.0);
    egui::CollapsingHeader::new("Feed network: phasing lines from one junction")
        .id_salt("phasing lines")
        .show(ui, |ui| {
            ui.label(
                RichText::new(
                    "Cuts a line for each element so the currents set above come from one feed \
                     point. The lines are worked out from the elements' active impedances (with \
                     coupling) at the slider frequency, the cable's loss included.",
                )
                .weak()
                .small(),
            );
            let cable_id = egui::Id::new("phasing cable");
            let mut ci = ui.data(|d| d.get_temp::<usize>(cable_id)).unwrap_or(2).min(CABLES.len() - 1);
            ui.horizontal(|ui| {
                ui.label("Cable");
                egui::ComboBox::from_id_salt("phasing cable box")
                    .selected_text(CABLES[ci].name)
                    .show_ui(ui, |ui| {
                        for (k, c) in CABLES.iter().enumerate() {
                            ui.selectable_value(&mut ci, k, c.name);
                        }
                    });
            });
            let slack_id = egui::Id::new("phasing slack");
            let mut slack = ui.data(|d| d.get_temp::<f64>(slack_id)).unwrap_or(0.25);
            ui.horizontal(|ui| {
                ui.label("Shortest lines within");
                ui.add(crate::views::num(&mut slack).range(0.0..=6.0).speed(0.05).max_decimals(2).suffix(" dB"))
                    .on_hover_text("How much worse than the best possible amplitude match you'll accept for shorter lines: 0 gives the closest match whatever the length");
                ui.label("of the best amplitude match");
            });
            let tune_id = egui::Id::new("phasing over the band");
            let mut tune = ui.data(|d| d.get_temp::<bool>(tune_id)).unwrap_or(false);
            let span = band.first().zip(band.last()).map(|(a, b)| (a.0, b.0));
            if let Some((lo, hi)) = span.filter(|(lo, hi)| hi > lo) {
                ui.checkbox(&mut tune, format!("Tune the lines over the sweep ({lo:.3} to {hi:.3} MHz)"))
                    .on_hover_text("Adjust the lengths together so the currents stay closest to what's wanted across the whole sweep (using the elements' impedances at every sweep frequency), not just at the slider frequency");
            } else {
                tune = false;
            }
            let exact_id = egui::Id::new("phasing exact");
            let mut exact = ui.data(|d| d.get_temp::<bool>(exact_id)).unwrap_or(false);
            ui.checkbox(&mut exact, "Make the currents exact with an L network at each element")
                .on_hover_text("Lines alone set the phases but leave the amplitudes as close as the lengths allow. A part across each line's end and one in series with the element (worked out at the slider frequency) make every current exactly what's wanted there; the reference element needs none");
            ui.data_mut(|d| {
                d.insert_temp(cable_id, ci);
                d.insert_temp(slack_id, slack);
                d.insert_temp(tune_id, tune);
                d.insert_temp(exact_id, exact);
            });
            let cable = CABLES[ci];
            let Some(pairs) = results
                .iter()
                .map(|r| r.map(|r| (r.impedance, r.current)))
                .collect::<Option<Vec<_>>>()
            else {
                ui.label("Waiting for the results at this frequency.");
                return;
            };
            let (z, want): (Vec<Complex64>, Vec<Complex64>) = pairs.into_iter().unzip();
            let at = junction_at(positions, mhz);
            let min_len: Vec<f64> = positions
                .iter()
                .map(|p| (0..3).map(|k| (p[k] - at[k]).powi(2)).sum::<f64>().sqrt())
                .collect();
            // The feed: one junction with phasing lines, or a 90° hybrid.
            let hybrid_id = egui::Id::new("phasing through a hybrid");
            let mut hybrid = ui.data(|d| d.get_temp::<bool>(hybrid_id)).unwrap_or(false);
            ui.horizontal(|ui| {
                ui.label("Feed from");
                ui.selectable_value(&mut hybrid, false, "one junction")
                    .on_hover_text("Every element through its own line from one point");
                ui.selectable_value(&mut hybrid, true, "a 90° hybrid coupler")
                    .on_hover_text("An ideal quadrature hybrid: its two outputs a quarter cycle apart, each feeding its elements through their own lines; what the elements reflect goes to its dump load instead of upsetting the phasing");
            });
            ui.data_mut(|d| d.insert_temp(hybrid_id, hybrid));
            if hybrid {
                hybrid_section(ui, model, list, &z, &want, &min_len, band, &cable, mhz, tune, settings);
                return;
            }
            // Worked out again only when something it uses changes.
            let key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (ci, mhz.to_bits(), slack.to_bits(), tune, exact).hash(&mut h);
                for (f, zs) in band.iter() {
                    f.to_bits().hash(&mut h);
                    for c in zs {
                        (c.re.to_bits(), c.im.to_bits()).hash(&mut h);
                    }
                }
                for c in z.iter().chain(&want) {
                    (c.re.to_bits(), c.im.to_bits()).hash(&mut h);
                }
                for m in &min_len {
                    m.to_bits().hash(&mut h);
                }
                h.finish()
            };
            let design_id = egui::Id::new("phasing design");
            let cached = ui.data(|d| d.get_temp::<(u64, Option<phasing::Design>)>(design_id));
            // While something is being dragged, keep the last design and
            // redo it once the mouse is let go.
            let dragging = ui.input(|i| i.pointer.any_down());
            let fits = |d: &Option<phasing::Design>| d.as_ref().is_some_and(|d| d.lengths.len() == z.len());
            if dragging && cached.as_ref().is_some_and(|(k, d)| *k != key && fits(d)) {
                ui.label(RichText::new("(updating when you let go)").weak().small());
            }
            let design = match cached {
                Some((k, d)) if k == key || (dragging && fits(&d)) => d,
                _ => {
                    let d = phasing::design(&z, &want, &min_len, &cable, mhz, slack)
                        .map(|d| {
                            if tune {
                                phasing::design_band(band, &z, &min_len, &cable, mhz, &d)
                            } else {
                                d
                            }
                        })
                        .map(|d| {
                            if exact {
                                d.with_networks(&z, &cable, mhz).unwrap_or(d)
                            } else {
                                d
                            }
                        });
                    ui.data_mut(|m| m.insert_temp(design_id, (key, d.clone())));
                    d
                }
            };
            let Some(d) = design else {
                ui.label("No lines found (an element with no current?).");
                return;
            };
            let lam_line = straws_core::units::wavelength(mhz) * cable.velocity_factor;
            let has_nets = d.nets.iter().any(|n| n.shunt.is_some() || n.series.is_some());
            if exact && !has_nets && d.worst_db > 1e-6 {
                ui.colored_label(ui.visuals().warn_fg_color, "No L networks solve these lines (a line a whole number of half waves long?): the lines alone are shown.");
            }
            egui::Grid::new("phasing design grid").num_columns(6).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                for h in ["Element", "Line", "Electrical", "Wanted", "Feed gives", if has_nets { "Network at the element" } else { "" }] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                for (k, s) in list.iter().enumerate() {
                    ui.label(format!("{}{}", source_name(model, s), if k == d.reference { " (ref)" } else { "" }));
                    ui.label(len(d.lengths[k])).on_hover_text(format!(
                        "Physical length of {}; it must reach from the junction: at least {}",
                        cable.name,
                        len(min_len[k])
                    ));
                    ui.label(format!("{:.1}°", 360.0 * d.lengths[k] / lam_line));
                    ui.label(amps(want[k]));
                    ui.label(amps(d.currents[k]));
                    if has_nets {
                        let net = d.nets[k];
                        // Parts that hardly matter: a series reactance of a
                        // couple of ohms, or one across the line of
                        // kilohms.
                        let tiny = |p: Option<straws_core::analysis::matching::Part>, series: bool| {
                            p.is_some_and(|p| {
                                let x = p.reactance(mhz).abs();
                                if series { x < 2.0 } else { x > 5000.0 }
                            })
                        };
                        let note = match (tiny(net.shunt, false), tiny(net.series, true)) {
                            (true, true) => " (both parts tiny: can be left out)",
                            (true, false) => " (the part across is tiny: can be left out)",
                            (false, true) => " (the series part is tiny: can be left out)",
                            _ => "",
                        };
                        ui.label(format!("{}{note}", net.describe())).on_hover_text(format!(
                            "With E12 parts: {}",
                            net.snapped(12).describe()
                        ));
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
            ui.label(format!(
                "Worst error {:.2} dB, {:.2}°. At the junction Z = {:.1} {} j{:.1} Ω (SWR {:.2} on {} Ω), fed with {:.1} V.",
                d.worst_db,
                d.worst_deg,
                d.junction.re,
                if d.junction.im < 0.0 { "-" } else { "+" },
                d.junction.im.abs(),
                swr(d.junction, settings.z0),
                settings.z0,
                d.junction_voltage.norm()
            ))
            .on_hover_text(
                "A line's length sets the phase it gives; the amplitude then follows, so it's \
                 only as close as the lengths allow (the shortest lines within the dB you set \
                 of the best are chosen). Under about 0.5 dB rarely matters. Match the junction to \
                 your feedline in the Matching view once the lines are written.",
            );
            if band.len() > 1 {
                let (db, deg, vector) = phasing::band_errors(band, &d.wanted, &d.lengths, &d.nets, &cable, d.reference);
                ui.label(format!(
                    "Across the sweep ({:.3} to {:.3} MHz): worst {:.0}% of the wanted current wrong ({db:.2} dB, {deg:.1}° at worst).",
                    band[0].0,
                    band[band.len() - 1].0,
                    100.0 * vector
                ))
                .on_hover_text("The share of the wanted current that's wrong, |got/wanted - 1|, at the worst element and frequency, from the elements' impedances across the sweep: amplitude and phase errors together (what tuning over the sweep makes as small as it can). The dB and degree figures are each the worst, maybe at different places.");
            }
            if ui
                .button("Write the lines into the model")
                .on_hover_text(
                    "Replaces the separate sources with a tiny junction wire above the array, fed \
                     with one source, and a NEC transmission line (TL card) from it to each \
                     element, so the whole network runs in NEC. NEC's lines are lossless. Undo \
                     with Ctrl+Z, or Back to separate sources.",
                )
                .clicked()
            {
                *action = Some(PhasingAction::Write { design: d, cable: ci, at });
            }
        });
}

/// A preset for the sources, in the deck's order.
#[derive(Clone, Copy)]
enum Preset {
    /// Equal amplitudes, all in phase (broadside).
    InPhase,
    /// Equal amplitudes, each 90° behind the one before (end-fire for
    /// elements a quarter wave apart, towards the last).
    EndFire,
    /// Four elements on a square, the beam towards element `k`: it gets
    /// -180°, the one diagonally opposite 0° and the other two -90°
    /// (worked out from where they are, whatever their order).
    FourSquare(usize),
    /// Binomial amplitudes (1, 2, 1...; no side lobes), phases kept.
    Binomial,
    /// Equal amplitudes, phases kept.
    Uniform,
    /// Four elements on a square, the beam broadside to the side between
    /// elements `a` and `b`: those two -90°, the other two 0° (a pair of
    /// end-fire pairs).
    FourSquareSide(usize, usize),
    /// Progressive phase for a beam towards `azimuth` (degrees) at
    /// wavelength `lambda` (metres): each element lags by its distance
    /// along that direction, the furthest back at 0°. Amplitudes kept.
    Steer { azimuth: f64, lambda: f64 },
    /// Dolph-Chebyshev amplitudes for side lobes `sll` dB down (the
    /// narrowest beam for that side-lobe level), in the deck's order along
    /// the row; phases kept.
    Chebyshev(f64),
}

/// Dolph-Chebyshev weights for `n` elements with side lobes `sll` dB below
/// the main lobe, the largest 1: the array factor T(n-1)(x0 cos(ψ/2))
/// sampled at n points and transformed back to element weights.
fn chebyshev(n: usize, sll: f64) -> Vec<f64> {
    if n < 3 {
        return vec![1.0; n];
    }
    let r = 10f64.powf(sll / 20.0);
    let m = (n - 1) as f64;
    let x0 = (r.acosh() / m).cosh();
    // Chebyshev polynomial of degree n-1, any real x.
    let t = |x: f64| {
        if x.abs() <= 1.0 {
            (m * x.acos()).cos()
        } else {
            (m * x.abs().acosh()).cosh()
                * if x < 0.0 && (n - 1) % 2 == 1 {
                    -1.0
                } else {
                    1.0
                }
        }
    };
    let centre = m / 2.0;
    let w: Vec<f64> = (0..n)
        .map(|k| {
            (0..n)
                .map(|j| {
                    let psi = 2.0 * std::f64::consts::PI * j as f64 / n as f64;
                    t(x0 * (psi / 2.0).cos()) * ((k as f64 - centre) * psi).cos()
                })
                .sum::<f64>()
                / n as f64
        })
        .collect();
    let top = w.iter().copied().fold(0.0, f64::max);
    w.iter().map(|v| v / top).collect()
}

/// What each source should become under `preset`, or why it can't.
fn apply(preset: Preset, now: &[Complex64], at: &[[f64; 3]]) -> Result<Vec<Complex64>, String> {
    let n = now.len();
    let first = now.first().map_or(1.0, |v| v.norm()).max(1e-12);
    let polar = |a: f64, p: f64| Complex64::from_polar(a, p.to_radians());
    Ok(match preset {
        Preset::InPhase => (0..n).map(|_| polar(first, 0.0)).collect(),
        Preset::EndFire => (0..n).map(|k| polar(first, -90.0 * k as f64)).collect(),
        Preset::FourSquare(front) => {
            if n != 4 || at.len() != 4 {
                return Err("a four-square needs four sources".into());
            }
            let dist =
                |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>();
            let back = (0..4)
                .max_by(|&i, &j| dist(at[i], at[front]).total_cmp(&dist(at[j], at[front])))
                .unwrap_or(0);
            (0..4)
                .map(|k| {
                    let phase = if k == front {
                        -180.0
                    } else if k == back {
                        0.0
                    } else {
                        -90.0
                    };
                    polar(first, phase)
                })
                .collect()
        }
        Preset::Binomial => {
            // Row n-1 of Pascal's triangle, the middle (largest) at `first`.
            let mut row = vec![1.0f64];
            for _ in 1..n {
                let mut next = vec![1.0; row.len() + 1];
                for k in 1..row.len() {
                    next[k] = row[k - 1] + row[k];
                }
                row = next;
            }
            let top = row.iter().copied().fold(0.0, f64::max);
            now.iter()
                .zip(row)
                .map(|(v, r)| polar(first * r / top, v.arg().to_degrees()))
                .collect()
        }
        Preset::Uniform => now
            .iter()
            .map(|v| polar(first, v.arg().to_degrees()))
            .collect(),
        Preset::FourSquareSide(a, b) => {
            if n != 4 || at.len() != 4 {
                return Err("a four-square needs four sources".into());
            }
            let centre = [0, 1, 2].map(|k| at.iter().map(|p| p[k]).sum::<f64>() / 4.0);
            let dir = [0, 1, 2].map(|k| (at[a][k] + at[b][k]) / 2.0 - centre[k]);
            (0..4)
                .map(|i| {
                    let along: f64 = (0..3).map(|k| (at[i][k] - centre[k]) * dir[k]).sum();
                    polar(first, if along > 0.0 { -90.0 } else { 0.0 })
                })
                .collect()
        }
        Preset::Steer { azimuth, lambda } => {
            let (c, si) = (azimuth.to_radians().cos(), azimuth.to_radians().sin());
            let along: Vec<f64> = at.iter().map(|p| p[0] * c + p[1] * si).collect();
            let back = along.iter().copied().fold(f64::MAX, f64::min);
            now.iter()
                .zip(&along)
                .map(|(v, d)| polar(v.norm(), -360.0 * (d - back) / lambda.max(1e-9)))
                .collect()
        }
        Preset::Chebyshev(sll) => {
            let w = chebyshev(n, sll);
            let top = now.iter().map(|v| v.norm()).fold(0.0, f64::max).max(1e-12);
            now.iter()
                .zip(w)
                .map(|(v, w)| polar(top * w, v.arg().to_degrees()))
                .collect()
        }
    })
}

/// Sets source `s` to `value`: its variables if it has them (like a
/// slider), else its card.
fn set_value(data: &mut Data, s: &Source, value: Complex64) {
    if let Some((amp, phase)) = &s.variables {
        let base = |data: &Data, name: &str| {
            data.model
                .as_ref()
                .and_then(|m| {
                    m.deck
                        .variables()
                        .into_iter()
                        .find(|v| v.name.eq_ignore_ascii_case(name))
                })
                .and_then(|v| v.literal)
                .unwrap_or(0.0)
        };
        let (ba, bp) = (base(data, amp), base(data, phase));
        set_override(data, amp, value.norm(), ba);
        set_override(data, phase, value.arg().to_degrees(), bp);
        return;
    }
    let Some(m) = &mut data.model else { return };
    let before = m.deck.clone();
    if let Err(why) = m.deck.set_source(s.line, value) {
        m.deck = before;
        data.message(Level::Warning, format!("Not changed: {why}."));
        return;
    }
    if let Err(why) = m.keep_if_valid(before, &data.overrides) {
        data.message(Level::Warning, format!("Not changed: {why}."));
    }
}

/// A source's name, from its wire's comment.
fn source_name(model: &crate::model::LoadedModel, s: &Source) -> String {
    model
        .wires
        .iter()
        .find(|w| w.tag == s.tag)
        .and_then(|w| model.deck.lines.get(w.line)?.comment.clone())
        .map(|c| c.trim().chars().take(24).collect::<String>())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| format!("wire {}", s.tag))
}

/// The "Feed point" choice for the views that show one feed point (SWR,
/// Smith chart, matching, feedline), when the model has two or more.
pub fn feed_picker(ui: &mut egui::Ui, data: &mut Data) {
    let Some(model) = &data.model else { return };
    let Ok(n) = &model.numeric else { return };
    let list = sources(&model.deck, n);
    if list.len() < 2 {
        return;
    }
    let names: Vec<String> = list
        .iter()
        .map(|s| format!("{} (seg {})", source_name(model, s), s.segment))
        .collect();
    let mut k = data.feed.min(names.len() - 1);
    ui.horizontal(|ui| {
        ui.label("Feed point");
        egui::ComboBox::from_id_salt("feed point")
            .selected_text(&names[k])
            .show_ui(ui, |ui| {
                for (i, name) in names.iter().enumerate() {
                    ui.selectable_value(&mut k, i, name);
                }
            })
            .response
            .on_hover_text(
                "Which source's impedance these views show (SWR, Smith chart, matching, \
                 feedline and their exports): the active impedance, with all the others \
                 driven as set in the Feeds view",
            );
    });
    data.feed = k;
}

pub fn ui(ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
    let Some(model) = &data.model else {
        return empty_note(ui, "Open a model to set its feeds.");
    };
    let Ok(numeric) = &model.numeric else {
        return empty_note(ui, "The deck can't be read: see the Deck view.");
    };
    let list = sources(&model.deck, numeric);
    if list.is_empty() {
        return empty_note(
            ui,
            "Nothing feeds this model yet. Right-click a wire in the 3D model view and choose \
             Feed this wire (or Add another source here).",
        );
    }
    let name_of = |s: &Source| source_name(model, s);
    // Where each source is: the middle of its segment.
    let positions: Vec<[f64; 3]> = list
        .iter()
        .map(|s| {
            model
                .wires
                .iter()
                .find(|w| w.tag == s.tag)
                .map_or([0.0; 3], |w| {
                    let f = (s.segment as f64 - 0.5) / w.segments.max(1) as f64;
                    [0, 1, 2].map(|k| w.a[k] + (w.b[k] - w.a[k]) * f)
                })
        })
        .collect();
    let point = data.shown_point();
    // Every sweep frequency with a result for each source: the band the
    // phasing lines can be tuned over.
    let band: Vec<phasing::BandPoint> = data
        .display_sweep()
        .map(|s| {
            s.points()
                .filter(|p| p.inputs.len() == list.len())
                .map(|p| (p.mhz, p.inputs.iter().map(|i| i.impedance).collect()))
                .collect()
        })
        .unwrap_or_default();
    let results: Vec<Option<straws_core::output::InputParams>> = list
        .iter()
        .enumerate()
        .map(|(k, _)| point.and_then(|p| p.inputs.get(k).copied()))
        .collect();
    let total: f64 = results.iter().flatten().map(|r| r.power).sum();
    let z0 = settings.z0;

    let mut change: Option<(usize, Complex64, bool)> = None;
    let mut kind_change: Option<(usize, SourceKind)> = None;
    let mut make_vars: Option<usize> = None;
    let mut remove: Option<usize> = None;
    let mut preset: Option<Preset> = None;
    let mut scale_to: Option<f64> = None;
    // (source, wanted share 0-1, its power now)
    let mut share_change: Option<(usize, f64, f64)> = None;
    // (source, wanted power, its power now): from a share in dB.
    let mut power_change: Option<(usize, f64, f64)> = None;
    let mut phasing_action: Option<PhasingAction> = None;
    // Shares as % of the total, or dB against a reference element.
    let (db_id, ref_id) = (
        egui::Id::new("feeds shares in dB"),
        egui::Id::new("feeds share reference"),
    );
    let mut in_db = ui.data(|d| d.get_temp::<bool>(db_id)).unwrap_or(false);
    let mut reference = ui
        .data(|d| d.get_temp::<usize>(ref_id))
        .unwrap_or(0)
        .min(list.len().saturating_sub(1));
    let ref_power = results.get(reference).copied().flatten().map(|r| r.power);

    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("Presets", |ui| {
                for (label, p, tip) in [
                    ("All in phase (broadside)", Preset::InPhase, "Equal amplitudes, all at 0°: the beam is broadside to the row"),
                    ("End-fire (90° steps)", Preset::EndFire, "Equal amplitudes, each 90° behind the one before: for elements a quarter wave apart the beam points along the row, towards the last"),
                    ("Uniform amplitudes", Preset::Uniform, "Every amplitude like the first one's; phases kept"),
                    ("Binomial taper", Preset::Binomial, "Amplitudes 1, 2, 1 (1, 3, 3, 1...): no side lobes, a wider beam; phases kept"),
                ] {
                    if ui.button(label).on_hover_text(tip).clicked() {
                        preset = Some(p);
                        ui.close();
                    }
                }
                ui.separator();
                ui.label(RichText::new("Four-square, beam towards:").weak());
                if list.len() == 4 {
                    for (k, s) in list.iter().enumerate() {
                        if ui
                            .button(name_of(s))
                            .on_hover_text("This element -180°, the one diagonally opposite 0°, the other two -90°: the beam points over this element")
                            .clicked()
                        {
                            preset = Some(Preset::FourSquare(k));
                            ui.close();
                        }
                    }
                    // Broadside to a side: two elements next to each other.
                    let far = |i: usize, j: usize| {
                        (0..3).map(|k| (positions[i][k] - positions[j][k]).powi(2)).sum::<f64>()
                    };
                    let longest = (0..4)
                        .flat_map(|i| (0..4).map(move |j| (i, j)))
                        .map(|(i, j)| far(i, j))
                        .fold(0.0, f64::max);
                    for i in 0..4 {
                        for j in i + 1..4 {
                            if far(i, j) < 0.75 * longest
                                && ui
                                    .button(format!("between {} and {}", name_of(&list[i]), name_of(&list[j])))
                                    .on_hover_text("Those two -90°, the other two 0°: the beam points broadside over that side")
                                    .clicked()
                            {
                                preset = Some(Preset::FourSquareSide(i, j));
                                ui.close();
                            }
                        }
                    }
                } else {
                    ui.label(RichText::new("(needs four sources)").weak());
                }
                ui.separator();
                let lam = straws_core::units::wavelength(data.mhz.max(1e-3));
                let az_id = ui.id().with("feeds steer azimuth");
                let mut az = ui.data(|d| d.get_temp::<f64>(az_id)).unwrap_or(0.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("Steer towards")
                        .on_hover_text(format!(
                            "Each element lags by how far it is along that direction (360° per wavelength, {:.3} m at the slider frequency): the beam points that way. Amplitudes kept",
                            lam
                        ))
                        .clicked()
                    {
                        preset = Some(Preset::Steer { azimuth: az, lambda: lam });
                        ui.close();
                    }
                    ui.add(crate::views::num(&mut az).speed(1.0).suffix("° azimuth"));
                });
                ui.data_mut(|d| d.insert_temp(az_id, az));
                let sll_id = ui.id().with("feeds chebyshev sll");
                let mut sll = ui.data(|d| d.get_temp::<f64>(sll_id)).unwrap_or(25.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("Dolph-Chebyshev taper")
                        .on_hover_text("Amplitudes for a row (in the deck's order) whose side lobes are all this far down: the narrowest beam for that level. Phases kept")
                        .clicked()
                    {
                        preset = Some(Preset::Chebyshev(sll));
                        ui.close();
                    }
                    ui.add(crate::views::num(&mut sll).range(5.0..=60.0).speed(0.5).suffix(" dB side lobes"));
                });
                ui.data_mut(|d| d.insert_temp(sll_id, sll));
            });
            if total > 0.0 {
                ui.label(format!("{total:.3} W in at {:.4} MHz.", data.mhz));
                let id = ui.id().with("feeds total power");
                let mut want = ui.data(|d| d.get_temp::<f64>(id)).unwrap_or(100.0);
                if ui
                    .small_button("Set the total to")
                    .on_hover_text("Every amplitude times the same factor, so the whole array takes this much")
                    .clicked()
                {
                    scale_to = Some(want);
                }
                ui.add(crate::views::num(&mut want).range(1e-6..=1e9).speed(1.0).suffix(" W"));
                ui.data_mut(|d| d.insert_temp(id, want));
                ui.separator();
                ui.label("Shares in");
                ui.selectable_value(&mut in_db, false, "%")
                    .on_hover_text("Each element's share of the array's total power");
                ui.selectable_value(&mut in_db, true, "dB")
                    .on_hover_text("Each element's power in dB against a reference element");
                if in_db {
                    egui::ComboBox::from_id_salt("feeds share reference")
                        .selected_text(format!("against {}", list.get(reference).map(name_of).unwrap_or_default()))
                        .show_ui(ui, |ui| {
                            for (k, s) in list.iter().enumerate() {
                                ui.selectable_value(&mut reference, k, name_of(s));
                            }
                        });
                }
            }
        });
        ui.add_space(4.0);
        egui::Grid::new("feeds")
            .num_columns(9)
            .spacing([10.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                for h in ["Source", "Kind", "Amplitude", "Phase", "Feed impedance", "SWR", "Power", "Current", ""] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                for (k, s) in list.iter().enumerate() {
                    ui.label(format!("{} (seg {})", name_of(s), s.segment))
                        .on_hover_text(format!("EX card on deck line {}", s.line + 1));
                    let mut kind = s.kind;
                    egui::ComboBox::from_id_salt(("feed kind", k))
                        .selected_text(kind.label())
                        .width(70.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut kind, SourceKind::Voltage, "voltage")
                                .on_hover_text("A voltage across the feed gap (EX 0)");
                            ui.selectable_value(&mut kind, SourceKind::Current, "current")
                                .on_hover_text("A current into the feed point (4nec2's EX 6): what phased arrays usually want, as coupling then can't skew the currents");
                        });
                    if kind != s.kind {
                        kind_change = Some((s.line, kind));
                    }
                    match s.value {
                        Some(v) => {
                            let (mut a, mut p) = (v.norm(), v.arg().to_degrees());
                            let unit = s.kind.unit();
                            ui.horizontal(|ui| {
                                // The range stays put while dragging (twice the
                                // value it had when the drag started).
                                let id = ui.id().with(("feed amplitude range", k));
                                let top = ui
                                    .data(|d| d.get_temp::<f64>(id))
                                    .unwrap_or((a * 2.0).max(1.0));
                                let r1 = ui.add(egui::Slider::new(&mut a, 0.0..=top).show_value(false));
                                if !r1.dragged() {
                                    ui.data_mut(|d| d.insert_temp(id, (a * 2.0).max(1.0)));
                                }
                                let r2 = ui.add(crate::views::num(&mut a).speed(top / 200.0).range(0.0..=f64::MAX).suffix(format!(" {unit}")));
                                if r1.changed() || r2.changed() {
                                    change = Some((k, Complex64::from_polar(a, p.to_radians()), r1.dragged()));
                                }
                                if r1.drag_stopped() {
                                    change = Some((k, Complex64::from_polar(a, p.to_radians()), false));
                                }
                            });
                            ui.horizontal(|ui| {
                                let r1 = ui.add(egui::Slider::new(&mut p, -180.0..=180.0).show_value(false));
                                let r2 = ui.add(crate::views::num(&mut p).speed(1.0).suffix("°"));
                                if r1.changed() || r2.changed() {
                                    change = Some((k, Complex64::from_polar(a, p.to_radians()), r1.dragged()));
                                }
                                if r1.drag_stopped() {
                                    change = Some((k, Complex64::from_polar(a, p.to_radians()), false));
                                }
                            });
                        }
                        None => {
                            ui.label(RichText::new("(a formula)").weak());
                            ui.label("");
                        }
                    }
                    match results.get(k).copied().flatten() {
                        Some(r) => {
                            let z = r.impedance;
                            ui.label(format!("{:.1} {} j{:.1} Ω", z.re, if z.im < 0.0 { "-" } else { "+" }, z.im.abs()))
                                .on_hover_text("The active impedance: what this feed point sees with all the others driven too (coupling included)");
                            ui.label(format!("{:.2}", swr(z, z0)))
                                .on_hover_text(format!("On its own {z0} Ω feedline"));
                            let share = if total > 0.0 { 100.0 * r.power / total } else { 0.0 };
                            ui.horizontal(|ui| {
                                ui.label(format!("{:.3} W", r.power)).on_hover_text(
                                    "Power this feed point takes, for the amplitudes as set (negative: it gives power back)",
                                );
                                if !in_db {
                                    let mut want = share;
                                    if ui
                                        .add(crate::views::num(&mut want).range(0.1..=99.9).speed(0.2).max_decimals(1).suffix(" %"))
                                        .on_hover_text("Its share of the array's power: type a share to set its amplitude for it (coupling moves it a little, so set it again to fine-tune)")
                                        .changed()
                                    {
                                        share_change = Some((k, want / 100.0, r.power));
                                    }
                                } else if k == reference {
                                    ui.label("0 dB (reference)");
                                } else if let Some(pr) = ref_power.filter(|p| *p > 0.0)
                                    && r.power > 0.0
                                {
                                    let mut db = 10.0 * (r.power / pr).log10();
                                    if ui
                                        .add(crate::views::num(&mut db).range(-60.0..=60.0).speed(0.1).max_decimals(2).suffix(" dB"))
                                        .on_hover_text("Its power against the reference element's: type a level to set its amplitude for it, the reference unchanged (coupling moves it a little, so set it again to fine-tune)")
                                        .changed()
                                    {
                                        power_change = Some((k, pr * 10f64.powf(db / 10.0), r.power));
                                    }
                                } else {
                                    ui.label(RichText::new("- dB").weak())
                                        .on_hover_text("No dB figure where either power is zero or flowing back");
                                }
                            });
                            ui.label(format!("{:.3} A at {:.0}°", r.current.norm(), r.current.arg().to_degrees()))
                                .on_hover_text("The current the feed point actually gets");
                        }
                        None => {
                            for _ in 0..4 {
                                ui.label(RichText::new("-").weak());
                            }
                        }
                    }
                    ui.horizontal(|ui| {
                        if s.variables.is_none()
                            && ui
                                .small_button("ƒ")
                                .on_hover_text("Make its amplitude and phase variables (sliders in the Variables view, and the optimiser can change them)")
                                .clicked()
                        {
                            make_vars = Some(k);
                        }
                        if let Some((a, p)) = &s.variables {
                            ui.label(RichText::new("= var").weak())
                                .on_hover_text(format!("Set by variables {a} and {p}"));
                        }
                        if ui.small_button("✖").on_hover_text("Remove this source").clicked() {
                            remove = Some(k);
                        }
                    });
                    ui.end_row();
                }
            });
        phasing_ui(
            ui,
            model,
            &list,
            &positions,
            &results,
            point,
            &band,
            settings,
            data.mhz,
            &mut phasing_action,
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Amplitude is volts for a voltage source and amps for a current source; phase in \
                 degrees. Add another source by right-clicking a wire in the 3D model view. The \
                 pattern follows as you drag.",
            )
            .weak()
            .small(),
        );
    });

    ui.data_mut(|d| {
        d.insert_temp(db_id, in_db);
        d.insert_temp(ref_id, reference);
    });
    // Apply what was asked, after drawing.
    let values: Vec<Complex64> = list.iter().map(|s| s.value.unwrap_or_default()).collect();
    if let Some((k, v, dragging)) = change {
        set_value(data, &list[k], v);
        data.edit_event = Some(data.edit_event.unwrap_or(false) || !dragging);
    }
    if let Some(p) = preset {
        match apply(p, &values, &positions) {
            Ok(new) => {
                for (s, v) in list.iter().zip(new) {
                    set_value(data, s, v);
                }
                data.edit_event = Some(true);
            }
            Err(why) => data.message(Level::Warning, format!("Not changed: {why}.")),
        }
    }
    if let Some((k, want, now)) = share_change
        && now > 0.0
        && total > now
    {
        // Its power for that share with the others' power unchanged.
        let others = total - now;
        let f = (want / (1.0 - want) * others / now).sqrt();
        set_value(data, &list[k], values[k] * f);
        data.edit_event = Some(true);
    }
    if let Some((k, want, now)) = power_change
        && now > 0.0
        && want > 0.0
    {
        set_value(data, &list[k], values[k] * (want / now).sqrt());
        data.edit_event = Some(true);
    }
    if let Some(target) = scale_to
        && total > 0.0
    {
        let f = (target / total).sqrt();
        for (s, v) in list.iter().zip(&values) {
            set_value(data, s, v * f);
        }
        data.edit_event = Some(true);
    }
    let mut warning = None;
    let mhz = data.mhz;
    let Some(m) = &mut data.model else { return };
    if let Some(a) = phasing_action
        && let Ok(n) = m.numeric.clone()
    {
        let before = m.deck.clone();
        match a {
            PhasingAction::Write { design, cable, at } => {
                // A tag clear of the deck's own (not the 4nec2 helpers').
                let tag = m
                    .wires
                    .iter()
                    .map(|w| w.tag)
                    .chain(straws_core::deck::radials::kept_tags(&m.deck))
                    .max()
                    .unwrap_or(0)
                    + 1;
                let srcs = sources(&m.deck, &n);
                phasing::write_lines(&mut m.deck, &srcs, &design, &CABLES[cable], mhz, tag, at);
            }
            PhasingAction::Remove => phasing::remove_network(&mut m.deck),
        }
        match m.keep_if_valid(before.clone(), &data.overrides) {
            Ok(()) => data.edit_event = Some(true),
            Err(why) => {
                m.deck = before;
                warning = Some(format!("Not changed: {why}."));
            }
        }
    }
    if let Some((line, kind)) = kind_change {
        let before = m.deck.clone();
        if m.deck.set_source_kind(line, kind).is_ok()
            && m.keep_if_valid(before, &data.overrides).is_ok()
        {
            data.edit_event = Some(true);
        }
    }
    if let Some(k) = make_vars
        && let Ok(n) = m.numeric.clone()
    {
        let before = m.deck.clone();
        let name = m.deck.unique_name(&format!("feed{}", k + 1));
        let made = m.deck.make_source_variables(list[k].line, &name, &n);
        match made
            .map_err(|e| e.to_string())
            .and_then(|_| m.keep_if_valid(before.clone(), &data.overrides))
        {
            Ok(()) => data.edit_event = Some(true),
            Err(why) => {
                m.deck = before;
                warning = Some(format!("Not changed: {why}."));
            }
        }
    }
    if let Some(k) = remove {
        let before = m.deck.clone();
        m.deck.remove_source(list[k].line);
        if m.keep_if_valid(before, &data.overrides).is_ok() {
            data.edit_event = Some(true);
        }
    }
    if let Some(w) = warning {
        data.message(Level::Warning, w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets() {
        let now = vec![Complex64::new(2.0, 0.0); 4];
        // Listed 1, 2, 4, 3 round the square, as in the test deck: the beam
        // over the first one puts 0° on the diagonally opposite (fourth).
        let at = [
            [-1.0, -1.0, 0.0],
            [-1.0, 1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let fs = apply(Preset::FourSquare(0), &now, &at).unwrap();
        let deg: Vec<f64> = fs.iter().map(|v| v.arg().to_degrees().round()).collect();
        assert_eq!(deg, [-180.0, -90.0, -90.0, 0.0]);
        let b = apply(Preset::Binomial, &now, &at).unwrap();
        let amps: Vec<f64> = b.iter().map(|v| (v.norm() * 3.0 / 2.0).round()).collect();
        assert_eq!(amps, [1.0, 3.0, 3.0, 1.0]);
        assert!(apply(Preset::FourSquare(0), &now[..3], &at[..3]).is_err());
        // Broadside over the side between the first two: they lag 90°.
        let side = apply(Preset::FourSquareSide(0, 1), &now, &at).unwrap();
        let deg: Vec<f64> = side.iter().map(|v| v.arg().to_degrees().round()).collect();
        assert_eq!(deg, [-90.0, -90.0, 0.0, 0.0]);
        // Steering a row a quarter wave apart along +x: 90° steps.
        let row = [[0.0, 0.0, 0.0], [0.25, 0.0, 0.0], [0.5, 0.0, 0.0]];
        let st = apply(
            Preset::Steer {
                azimuth: 0.0,
                lambda: 1.0,
            },
            &now[..3],
            &row,
        )
        .unwrap();
        let deg: Vec<f64> = st.iter().map(|v| v.arg().to_degrees().round()).collect();
        assert_eq!(deg, [0.0, -90.0, -180.0]);
        // Dolph-Chebyshev: symmetric, the middle largest, and every side
        // lobe of its array factor exactly the level asked for.
        let w = chebyshev(5, 20.0);
        assert!((w[0] - w[4]).abs() < 1e-12 && (w[2] - 1.0).abs() < 1e-12 && w[1] < 1.0);
        let af = |psi: f64| {
            w.iter()
                .enumerate()
                .map(|(k, a)| Complex64::from_polar(*a, (k as f64 - 2.0) * psi))
                .sum::<Complex64>()
                .norm()
        };
        let peak = af(0.0);
        let mut lobe = 0.0f64;
        let mut falling = true;
        let mut last = peak;
        for i in 1..=2000 {
            let v = af(std::f64::consts::PI * i as f64 / 2000.0);
            if falling && v > last {
                falling = false;
            }
            if !falling {
                lobe = lobe.max(v);
            }
            last = v;
        }
        assert!((20.0 * (peak / lobe).log10() - 20.0).abs() < 0.05, "{w:?}");
        let e = apply(Preset::EndFire, &now[..2], &at[..2]).unwrap();
        assert!((e[1].arg().to_degrees() + 90.0).abs() < 1e-9);
    }
}
