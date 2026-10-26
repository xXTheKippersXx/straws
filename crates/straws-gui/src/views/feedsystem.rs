//! The Feed system view: the Feedline view's chain (balun, connectors,
//! cable) with a matching network, tuned over the sweep or chosen bands for
//! the lowest worst-case SWR at the radio (with a weighting against loss),
//! and shown with standard part values too.

use eframe::egui::{self, Color32, RichText};
use egui_plot::{Legend, Line, LineStyle, Plot, PlotPoints, VLine};
use straws_core::Complex64;
use straws_core::analysis::feedline::Feed;
use straws_core::analysis::feedsystem::{
    Free, Stage, evaluate, l_starts, optimise, pi_t_starts, worst,
};
use straws_core::analysis::matching::{Element, stub_matches};

use crate::app::{Data, Settings};

use super::empty_note;

/// The matching network to add.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Network {
    #[default]
    None,
    LAtAntenna,
    LAtRadio,
    PiAtAntenna,
    TAtAntenna,
    StubShorted,
    StubOpen,
}

impl Network {
    const ALL: [Network; 7] = [
        Network::None,
        Network::LAtAntenna,
        Network::LAtRadio,
        Network::PiAtAntenna,
        Network::TAtAntenna,
        Network::StubShorted,
        Network::StubOpen,
    ];

    fn label(self) -> &'static str {
        match self {
            Network::None => "none (tune the line and balun only)",
            Network::LAtAntenna => "L network at the antenna",
            Network::LAtRadio => "L network at the radio (a tuner)",
            Network::PiAtAntenna => "pi network at the antenna",
            Network::TAtAntenna => "T network at the antenna",
            Network::StubShorted => "shorted stub near the antenna",
            Network::StubOpen => "open stub near the antenna",
        }
    }
}

/// A tuned feed system and how it does.
#[derive(Clone)]
struct Tuned {
    stages: Vec<Stage>,
    snapped: Vec<Stage>,
    /// Which stage is the feedline and the balun (for "use in the Feedline
    /// view").
    line: usize,
    balun: usize,
}

pub struct FeedSystemView {
    network: Network,
    q: f64,
    loss_weight: f64,
    tune_line: bool,
    line_min: f64,
    line_max: f64,
    tune_balun: bool,
    /// E12 or E24.
    series: usize,
    /// Band names to tune over (empty: the whole sweep).
    bands: Vec<String>,
    result: Option<Tuned>,
    note: Option<String>,
}

impl Default for FeedSystemView {
    fn default() -> Self {
        FeedSystemView {
            network: Network::LAtAntenna,
            q: 150.0,
            loss_weight: 1.0,
            tune_line: false,
            line_min: 1.0,
            line_max: 50.0,
            tune_balun: false,
            series: 12,
            bands: Vec::new(),
            result: None,
            note: None,
        }
    }
}

/// The chain from the antenna: balun, antenna connectors, cable, radio
/// connectors (connector losses at `mhz`), and where the cable and balun
/// are in it.
fn base_chain(feed: &Feed, mhz: f64) -> (Vec<Stage>, Vec<Stage>, Stage) {
    let (stages, k) = straws_core::analysis::feedsystem::chain_from_feed(feed, mhz);
    (stages[..k].to_vec(), stages[k + 1..].to_vec(), stages[k])
}

impl FeedSystemView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &mut Settings) {
        egui::ScrollArea::both()
            .auto_shrink(false)
            .show(ui, |ui| self.body(ui, data, settings));
    }

    fn body(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &mut Settings) {
        crate::views::feeds::feed_picker(ui, data);
        let Some(sweep) = data.display_sweep() else {
            let note = data.waiting_note(
                "Open a model to tune its feed system over the sweep.",
                "Calculating…",
            );
            return empty_note(ui, note);
        };
        let plan = sweep.plan;
        let all: Vec<(f64, Complex64)> = sweep
            .points()
            .filter_map(|p| p.feed(data.feed).map(|i| (p.mhz, i.impedance)))
            .collect();
        if all.is_empty() {
            return empty_note(ui, "No feed-point results yet.");
        }
        let feed = crate::views::feedline::current_feed(data, settings);
        let z0 = settings.z0;
        let bands: Vec<_> = crate::views::bands::chosen(settings)
            .into_iter()
            .filter(|b| b.stop_mhz >= plan.start_mhz && b.start_mhz <= plan.stop_mhz)
            .collect();
        self.bands.retain(|n| bands.iter().any(|b| &b.name == n));
        let points: Vec<(f64, Complex64)> = all
            .iter()
            .copied()
            .filter(|(f, _)| {
                self.bands.is_empty()
                    || bands
                        .iter()
                        .any(|b| self.bands.contains(&b.name) && b.contains(*f))
            })
            .collect();

        ui.label(
            RichText::new(format!(
                "Starts from the Feedline view's balun ({:.2}:1), connectors and {:.2} m of {}, \
                 adds a matching network, and tunes it over the sweep (or the bands you tick) \
                 for the lowest worst SWR at the radio ({z0:.0} Ω), with a weighting against \
                 loss. Parts have the Q you give (inductors' loss dominates). No engine runs: \
                 it works on the sweep's feed impedances.",
                feed.balun_ratio, feed.length_m, feed.cable.name
            ))
            .weak()
            .small(),
        );
        egui::Grid::new("feed system settings")
            .num_columns(2)
            .spacing([12.0, 5.0])
            .show(ui, |ui| {
                ui.label("Matching network");
                egui::ComboBox::from_id_salt("feed system network")
                    .selected_text(self.network.label())
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for n in Network::ALL {
                            ui.selectable_value(&mut self.network, n, n.label());
                        }
                    });
                ui.end_row();
                ui.label("Part Q");
                ui.add(crate::views::num(&mut self.q).range(10.0..=5000.0).speed(5.0))
                    .on_hover_text("Each part's reactance over its loss resistance: about 100-300 for air-wound inductors, more for capacitors. Lower Q means more loss in sharp networks");
                ui.end_row();
                ui.label("Also tune");
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.tune_line, "the cable length, from");
                    ui.add(crate::views::num(&mut self.line_min).range(0.0..=1e5).speed(0.1).suffix(" m"));
                    ui.label("to");
                    ui.add(crate::views::num(&mut self.line_max).range(0.0..=1e5).speed(0.1).suffix(" m"));
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut self.tune_balun, "the balun ratio (1 to 16)");
                ui.end_row();
                ui.label("Loss weighting");
                ui.add(crate::views::num(&mut self.loss_weight).range(0.0..=20.0).speed(0.05).suffix(" per dB"))
                    .on_hover_text("How much a dB of loss counts against a unit of SWR: 0 tunes for SWR alone");
                ui.end_row();
                ui.label("Over");
                ui.horizontal_wrapped(|ui| {
                    if bands.is_empty() {
                        ui.label("the whole sweep (no band of the Band menu's list is in it)");
                    } else {
                        ui.label(if self.bands.is_empty() { "the whole sweep, or only:" } else { "only:" });
                        for b in &bands {
                            let mut on = self.bands.contains(&b.name);
                            if ui.checkbox(&mut on, b.name.as_str()).changed() {
                                if on {
                                    self.bands.push(b.name.clone());
                                } else {
                                    self.bands.retain(|n| n != &b.name);
                                }
                            }
                        }
                    }
                });
                ui.end_row();
                ui.label("Standard parts");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.series, 12, "E12");
                    ui.selectable_value(&mut self.series, 24, "E24");
                });
                ui.end_row();
            });
        if ui
            .button("Tune")
            .on_hover_text("Works out a starting network at the middle of the band, then tunes everything ticked for the band")
            .clicked()
        {
            self.tune(&feed, &points, z0);
        }
        if let Some(n) = &self.note {
            ui.colored_label(ui.visuals().warn_fg_color, n);
        }

        // The Feedline view's chain as it is, for comparison.
        let mid = points.get(points.len() / 2).map_or(data.mhz, |p| p.0);
        let (before, after, line) = base_chain(&feed, mid);
        let mut current = before.clone();
        current.push(line);
        current.extend(after);
        let (cs, cl) = worst(&current, &points, z0);
        ui.label(format!("As the Feedline view has it: worst SWR {cs:.2}, worst loss {cl:.2} dB over the chosen frequencies."));
        let Some(t) = self.result.clone() else {
            return;
        };
        let (os, ol) = worst(&t.stages, &points, z0);
        let (ss, sl) = worst(&t.snapped, &points, z0);
        egui::Grid::new("feed system result")
            .num_columns(2)
            .striped(true)
            .spacing([16.0, 3.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Tuned (from the antenna)").weak());
                ui.label(RichText::new(format!("With E{} parts", self.series)).weak());
                ui.end_row();
                for (a, b) in t.stages.iter().zip(&t.snapped) {
                    ui.label(a.describe());
                    ui.label(b.describe());
                    ui.end_row();
                }
                ui.label(RichText::new(format!("worst SWR {os:.2}, loss {ol:.2} dB")).strong());
                ui.label(RichText::new(format!("worst SWR {ss:.2}, loss {sl:.2} dB")).strong());
                ui.end_row();
            });
        if (self.tune_line || self.tune_balun)
            && ui
                .button("Use this cable length and balun ratio in the Feedline view")
                .clicked()
        {
            if let Some(v) = t.stages[t.line].value() {
                crate::views::feedline::set(data, "feed_length", &mut settings.feed_length, v);
            }
            if let Some(v) = t.stages[t.balun].value() {
                crate::views::feedline::set(data, "balun_ratio", &mut settings.balun_ratio, v);
            }
        }

        // SWR at the radio across the sweep.
        let curve = |chain: &[Stage]| -> PlotPoints {
            all.iter()
                .map(|(f, z)| [*f, evaluate(chain, *z, *f, z0).1.min(10.0)])
                .collect()
        };
        let dark = ui.visuals().dark_mode;
        let shade = crate::views::bands::shade(dark);
        let marker = ui.visuals().weak_text_color();
        let (c_now, c_tuned, c_snap) = if dark {
            (
                Color32::from_gray(150),
                Color32::from_rgb(253, 200, 60),
                Color32::from_rgb(90, 170, 255),
            )
        } else {
            (
                Color32::from_gray(110),
                Color32::from_rgb(200, 90, 20),
                Color32::from_rgb(30, 100, 200),
            )
        };
        let r = Plot::new("feed system swr")
            .height(240.0)
            .include_x(plan.start_mhz)
            .include_x(plan.stop_mhz)
            .include_y(1.0)
            .include_y(3.0)
            .y_axis_label(format!("SWR at the radio ({z0:.0} Ω)"))
            .x_axis_label("MHz")
            .allow_scroll(false)
            .legend(Legend::default())
            .show(ui, |pui| {
                for b in &bands {
                    if self.bands.is_empty() || self.bands.contains(&b.name) {
                        pui.span(
                            egui_plot::Span::new(b.name.as_str(), b.start_mhz..=b.stop_mhz)
                                .fill(shade)
                                .border_width(0.0),
                        );
                    }
                }
                pui.line(
                    Line::new("as the Feedline view has it", curve(&current))
                        .color(c_now)
                        .width(1.5),
                );
                pui.line(
                    Line::new("tuned", curve(&t.stages))
                        .color(c_tuned)
                        .width(2.0),
                );
                pui.line(
                    Line::new(format!("with E{} parts", self.series), curve(&t.snapped))
                        .color(c_snap)
                        .width(2.0)
                        .style(LineStyle::dashed_dense()),
                );
                pui.vline(VLine::new("frequency", data.mhz).color(marker));
                (pui.response().clicked(), pui.pointer_coordinate())
            });
        if let (true, Some(c)) = r.inner {
            data.mhz = c.x.clamp(plan.start_mhz, plan.stop_mhz);
        }
    }

    /// Works out starting networks and tunes each; keeps the best.
    fn tune(&mut self, feed: &Feed, points: &[(f64, Complex64)], z0: f64) {
        self.note = None;
        let Some(&(mid, z_mid)) = points.get(points.len() / 2) else {
            self.note = Some("No frequencies to tune over.".into());
            return;
        };
        let (before, after, line) = base_chain(feed, mid);
        let mut line_after = vec![line];
        line_after.extend(after.iter().copied());
        // Starting chains, each with its tunable network stages.
        let starts: Vec<(Vec<Stage>, Vec<usize>)> = match self.network {
            Network::None => {
                let mut c = before.clone();
                c.extend(line_after.iter().copied());
                vec![(c, Vec::new())]
            }
            Network::LAtAntenna => {
                l_starts(&before, &line_after, z_mid, mid, feed.cable.z0, self.q)
                    .into_iter()
                    .map(|(c, k)| (c, k.to_vec()))
                    .collect()
            }
            Network::LAtRadio => {
                let mut b = before.clone();
                b.push(line);
                l_starts(&b, &after, z_mid, mid, z0, self.q)
                    .into_iter()
                    .map(|(c, k)| (c, k.to_vec()))
                    .collect()
            }
            Network::PiAtAntenna | Network::TAtAntenna => pi_t_starts(
                &before,
                &line_after,
                z_mid,
                mid,
                feed.cable.z0,
                self.q,
                self.network == Network::PiAtAntenna,
            ),
            Network::StubShorted | Network::StubOpen => {
                let shorted = self.network == Network::StubShorted;
                let (seen, _, _) = evaluate(&before, z_mid, mid, feed.cable.z0);
                stub_matches(
                    seen,
                    feed.cable.z0,
                    feed.cable.velocity_factor,
                    mid,
                    shorted,
                )
                .into_iter()
                .filter_map(|net| {
                    let (d, l) = match net.as_slice() {
                        [
                            Element::Line { length: d, .. },
                            Element::Stub { length: l, .. },
                        ] => (*d, *l),
                        _ => return None,
                    };
                    let k = before.len();
                    let mut c = before.clone();
                    c.push(Stage::Line {
                        cable: feed.cable,
                        length: d.max(0.001),
                    });
                    c.push(Stage::Stub {
                        cable: feed.cable,
                        length: l.max(0.001),
                        shorted,
                    });
                    c.extend(line_after.iter().copied());
                    Some((c, vec![k, k + 1]))
                })
                .collect()
            }
        };
        if starts.is_empty() {
            self.note = Some(
                "No starting network for this load at the band's middle (try another kind).".into(),
            );
            return;
        }
        let lam = straws_core::units::wavelength(mid) * feed.cable.velocity_factor;
        let mut best: Option<(f64, Tuned)> = None;
        for (chain, net) in starts {
            let balun = 0;
            let line_idx = chain
                .iter()
                .rposition(|s| matches!(s, Stage::Line { length, .. } if (*length - feed.length_m).abs() < 1e-12))
                .unwrap_or(0);
            let mut free: Vec<Free> = net
                .iter()
                .map(|&k| {
                    let v = chain[k].value().unwrap_or(1.0);
                    match chain[k] {
                        Stage::Line { .. } | Stage::Stub { .. } => Free {
                            stage: k,
                            min: 0.001,
                            max: lam * 0.75,
                        },
                        _ => Free {
                            stage: k,
                            min: v / 30.0,
                            max: v * 30.0,
                        },
                    }
                })
                .collect();
            if self.tune_line {
                let (lo, hi) = (
                    self.line_min.min(self.line_max),
                    self.line_min.max(self.line_max),
                );
                free.push(Free {
                    stage: line_idx,
                    min: lo.max(0.001),
                    max: hi.max(0.001),
                });
            }
            if self.tune_balun {
                free.push(Free {
                    stage: balun,
                    min: 1.0,
                    max: 16.0,
                });
            }
            let tuned = optimise(&chain, &free, points, z0, self.loss_weight);
            let snapped: Vec<Stage> = tuned.iter().map(|s| s.snapped(self.series)).collect();
            let (s, l) = worst(&tuned, points, z0);
            let score = s.min(50.0) + self.loss_weight * l;
            if best.as_ref().is_none_or(|(b, _)| score < *b) {
                best = Some((
                    score,
                    Tuned {
                        stages: tuned,
                        snapped,
                        line: line_idx,
                        balun,
                    },
                ));
            }
        }
        self.result = best.map(|b| b.1);
    }
}
