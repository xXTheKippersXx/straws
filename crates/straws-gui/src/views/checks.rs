//! Geometry checks: what might make the results wrong, and how to fix it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, RichText};
use straws_core::analysis::checks::{Severity, average_gain};
use straws_core::analysis::convergence::{Comparison, Figures, Verdict, compare};
use straws_core::analysis::plan::FrequencyPlan;
use straws_core::analysis::sensitivity::{Trims, trims};
use straws_core::analysis::tolerance::{Spread, percentiles, spread};
use straws_core::engine::{CancelToken, Engine};

use crate::app::{Data, Settings};

use super::empty_note;

/// Most wires the sensitivity check runs (one engine run each).
const MAX_TRIM_WIRES: usize = 300;

/// The Checks view's state: the convergence and tolerance checks' runs and
/// results.
pub struct ChecksView {
    job: Option<(CancelToken, Receiver<Result<Comparison, String>>)>,
    /// The result, and the model (as run) it belongs to.
    result: Option<(Result<Comparison, String>, String)>,
    /// Build accuracy (metres, either way) and number of builds.
    accuracy: f64,
    builds: usize,
    tolerance_job: Option<ToleranceJob>,
    tolerance: Option<(Result<Spread, String>, String)>,
    /// How much longer each wire is made (metres; 0 until first shown).
    trim: f64,
    trim_job: Option<TrimJob>,
    trims: Option<(Result<Trims, String>, String)>,
    /// What the 3D view's wire colours show.
    colour_by: ColourBy,
}

struct TrimJob {
    cancel: CancelToken,
    rx: Receiver<Result<Trims, String>>,
    done: Arc<AtomicUsize>,
    total: usize,
}

/// What the sensitivity check colours the wires by.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ColourBy {
    #[default]
    Off,
    Resonance,
    Swr,
    Gain,
    FrontToBack,
}

impl ColourBy {
    fn label(self) -> &'static str {
        match self {
            ColourBy::Off => "Off",
            ColourBy::Resonance => "Resonance shift",
            ColourBy::Swr => "SWR change",
            ColourBy::Gain => "Gain change",
            ColourBy::FrontToBack => "F/B change",
        }
    }
}

struct ToleranceJob {
    cancel: CancelToken,
    rx: Receiver<Result<Spread, String>>,
    done: Arc<AtomicUsize>,
}

impl Default for ChecksView {
    fn default() -> Self {
        ChecksView {
            job: None,
            result: None,
            accuracy: 0.002,
            builds: 40,
            tolerance_job: None,
            tolerance: None,
            trim: 0.0,
            trim_job: None,
            trims: None,
            colour_by: ColourBy::Off,
        }
    }
}

impl ChecksView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                checks_ui(ui, data);
                if data.model.as_ref().is_some_and(|m| m.numeric.is_ok()) {
                    ui.add_space(8.0);
                    ui.separator();
                    self.convergence(ui, data);
                    ui.add_space(8.0);
                    ui.separator();
                    self.tolerance(ui, data, settings);
                    ui.add_space(8.0);
                    ui.separator();
                    self.sensitivity(ui, data, settings);
                }
            });
    }

    fn convergence(&mut self, ui: &mut egui::Ui, data: &Data) {
        if let Some((_, rx)) = &self.job
            && let Ok(r) = rx.try_recv()
        {
            let run = self.result.take().map(|(_, run)| run).unwrap_or_default();
            self.result = Some((r, run));
            self.job = None;
        }
        let Some(model) = &data.model else { return };
        let Ok(numeric) = &model.numeric else { return };
        let now = numeric.to_nec_text();
        ui.label(RichText::new("Convergence check").strong())
            .on_hover_text(
                "Runs the model again with about twice the segments on every wire and shows how \
             much the results move. Small moves mean the model has enough segments to trust.",
            );
        ui.horizontal(|ui| {
            if let Some((cancel, _)) = &self.job {
                ui.spinner();
                ui.label("Running the model with about twice the segments...");
                if ui.button("Stop").clicked() {
                    cancel.cancel();
                    self.job = None;
                    self.result = None;
                }
            } else if ui
                .button("Run the check")
                .on_hover_text(format!(
                    "Compares the feed impedance, resonance, peak gain and front-to-back at {:.4} MHz",
                    data.mhz
                ))
                .clicked()
            {
                let plan = data.sweep.as_ref().map_or_else(
                    || FrequencyPlan::around(data.mhz),
                    |s| FrequencyPlan {
                        points: s.plan.points.clamp(2, 41),
                        ..s.plan
                    },
                );
                let (deck, mhz) = (numeric.clone(), data.mhz);
                let estimate = match straws_core::analysis::buried::need(&model.deck, numeric) {
                    straws_core::analysis::buried::Need::Estimate(s) => Some(s),
                    _ => None,
                };
                let cancel = CancelToken::new();
                let c = cancel.clone();
                let (tx, rx) = channel();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let r = Engine::self_hosted()
                        .map_err(|e| e.to_string())
                        .and_then(|e| compare(&e, &deck, &plan, mhz, estimate.as_ref(), &c));
                    let _ = tx.send(r);
                    ctx.request_repaint();
                });
                self.job = Some((cancel, rx));
                self.result = Some((Err(String::new()), now.clone()));
            }
        });
        if self.job.is_some() {
            return;
        }
        let Some((result, run)) = &self.result else {
            return;
        };
        let (warn, error, good) = (
            ui.visuals().warn_fg_color,
            ui.visuals().error_fg_color,
            egui::Color32::from_rgb(90, 190, 120),
        );
        let c = match result {
            Ok(c) => c,
            Err(e) if e.is_empty() => return,
            Err(e) => {
                ui.colored_label(error, format!("The check couldn't run: {e}."));
                return;
            }
        };
        if *run != now {
            ui.label(RichText::new("The model has changed since this check: run it again.").weak());
        }
        let (mark, colour, text) = match c.verdict() {
            Verdict::Settled => (
                "✔",
                good,
                "Settled: with about twice the segments the results hardly move, so the model has enough segments.",
            ),
            Verdict::NearlySettled => (
                "⚠",
                warn,
                "Nearly settled: small moves. Fine for most work; for exact figures, add segments and check again.",
            ),
            Verdict::NotSettled => (
                "✖",
                error,
                "Not settled: the results move a lot with more segments. Add segments (try doubling them) until this check says settled.",
            ),
        };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(colour, mark);
            ui.label(text);
        });
        let (a, b) = (&c.before, &c.after);
        egui::Grid::new("convergence")
            .num_columns(4)
            .spacing([14.0, 3.0])
            .show(ui, |ui| {
                ui.label("");
                ui.label(RichText::new(format!("{} segments", a.segments)).weak());
                ui.label(RichText::new(format!("{} segments", b.segments)).weak());
                ui.label(RichText::new("change").weak());
                ui.end_row();
                let z = |f: &straws_core::analysis::convergence::Figures| {
                    let z = f.z;
                    format!(
                        "{:.2} {} j{:.2} Ω",
                        z.re,
                        if z.im < 0.0 { "-" } else { "+" },
                        z.im.abs()
                    )
                };
                ui.label(format!("Feed impedance at {:.4} MHz", c.mhz));
                ui.label(z(a));
                ui.label(z(b));
                ui.label(format!("{:.1}%", 100.0 * c.z_change()));
                ui.end_row();
                if let (Some(ra), Some(rb)) = (a.resonance, b.resonance) {
                    ui.label("Resonance");
                    ui.label(format!("{ra:.4} MHz"));
                    ui.label(format!("{rb:.4} MHz"));
                    ui.label(format!("{:.1} kHz", 1000.0 * (rb - ra)));
                    ui.end_row();
                }
                ui.label("Peak gain");
                ui.label(format!("{:.2} dBi", a.peak_dbi));
                ui.label(format!("{:.2} dBi", b.peak_dbi));
                ui.label(format!("{:+.2} dB", b.peak_dbi - a.peak_dbi));
                ui.end_row();
                if let (Some(fa), Some(fb)) = (a.front_to_back, b.front_to_back) {
                    ui.label("Front-to-back");
                    ui.label(format!("{fa:.1} dB"));
                    ui.label(format!("{fb:.1} dB"));
                    ui.label(format!("{:+.1} dB", fb - fa));
                    ui.end_row();
                }
            });
        if numeric.cards.iter().any(|c| c.is("SP") || c.is("SM")) {
            ui.label(RichText::new("Surface patches aren't refined, only wires.").weak());
        }
    }

    /// The tolerance check: random builds within the build accuracy.
    fn tolerance(&mut self, ui: &mut egui::Ui, data: &Data, settings: &Settings) {
        if let Some(job) = &self.tolerance_job
            && let Ok(r) = job.rx.try_recv()
        {
            let run = self
                .tolerance
                .take()
                .map(|(_, run)| run)
                .unwrap_or_default();
            self.tolerance = Some((r, run));
            self.tolerance_job = None;
        }
        let Some(model) = &data.model else { return };
        let Ok(numeric) = &model.numeric else { return };
        let now = numeric.to_nec_text();
        ui.label(RichText::new("Tolerance check").strong()).on_hover_text(
            "Runs the antenna as many slightly different builds: every wire end moved at random by up \
             to the build accuracy along each axis (ends that meet move together). Shows how much the \
             figures spread, so you know how carefully it needs building.",
        );
        let unit = settings.length();
        let mhz = data.mhz.max(1e-3);
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(self.tolerance_job.is_none(), |ui| {
                ui.label("Build accuracy ±");
                let mut v = unit.from_metres(self.accuracy, mhz);
                if ui
                    .add(
                        crate::views::num(&mut v)
                            .speed(unit.from_metres(0.0005, mhz))
                            .range(0.0..=f64::MAX)
                            .suffix(format!(" {}", unit.symbol())),
                    )
                    .changed()
                {
                    self.accuracy = unit.to_metres(v, mhz).max(0.0);
                }
                ui.add(
                    crate::views::num(&mut self.builds)
                        .range(5..=500)
                        .suffix(" builds"),
                );
            });
            if let Some(job) = &self.tolerance_job {
                ui.spinner();
                ui.label(format!(
                    "{} of {} builds run",
                    job.done.load(Ordering::Relaxed),
                    self.builds
                ));
                if ui.button("Stop").clicked() {
                    job.cancel.cancel();
                    self.tolerance_job = None;
                    self.tolerance = None;
                }
            } else if ui
                .button("Run")
                .on_hover_text(format!(
                    "Feed impedance, resonance, peak gain and F/B at {:.4} MHz for each build",
                    data.mhz
                ))
                .clicked()
            {
                let plan = data.sweep.as_ref().map_or_else(
                    || FrequencyPlan::around(data.mhz),
                    |s| FrequencyPlan {
                        points: s.plan.points.clamp(2, 41),
                        ..s.plan
                    },
                );
                let (deck, accuracy, builds) = (numeric.clone(), self.accuracy, self.builds);
                let over_ground = data.over_ground;
                let estimate = match straws_core::analysis::buried::need(&model.deck, numeric) {
                    straws_core::analysis::buried::Need::Estimate(s) => Some(s),
                    _ => None,
                };
                let cancel = CancelToken::new();
                let c = cancel.clone();
                let done = Arc::new(AtomicUsize::new(0));
                let d = done.clone();
                let (tx, rx) = channel();
                let ctx = ui.ctx().clone();
                let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
                std::thread::spawn(move || {
                    let ctx2 = ctx.clone();
                    let r = Engine::self_hosted()
                        .map_err(|e| e.to_string())
                        .and_then(|e| {
                            spread(
                                &e,
                                &deck,
                                &plan,
                                mhz,
                                accuracy,
                                builds,
                                threads,
                                over_ground,
                                estimate.as_ref(),
                                &c,
                                &|n, _| {
                                    d.store(n, Ordering::Relaxed);
                                    ctx2.request_repaint();
                                },
                            )
                        });
                    let _ = tx.send(r);
                    ctx.request_repaint();
                });
                self.tolerance_job = Some(ToleranceJob { cancel, rx, done });
                self.tolerance = Some((Err(String::new()), now.clone()));
            }
        });
        if self.tolerance_job.is_some() {
            return;
        }
        let Some((result, run)) = &self.tolerance else {
            return;
        };
        let s = match result {
            Ok(s) => s,
            Err(e) if e.is_empty() => return,
            Err(e) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("The check couldn't run: {e}."),
                );
                return;
            }
        };
        if *run != now {
            ui.label(RichText::new("The model has changed since this check: run it again.").weak());
        }
        let z0 = settings.z0;
        let swr = |f: &Figures| straws_core::analysis::feed::swr(f.z, z0);
        let fixed = |d: usize| move |v: f64| format!("{v:.d$}");
        // Name, as drawn, every build, decimals, unit.
        type Row<'a> = (&'a str, Option<f64>, Vec<f64>, usize, &'a str);
        let rows: Vec<Row> = vec![
            (
                "SWR",
                Some(swr(&s.design)),
                s.builds.iter().map(swr).collect(),
                2,
                "",
            ),
            (
                "Feed R",
                Some(s.design.z.re),
                s.builds.iter().map(|b| b.z.re).collect(),
                1,
                " Ω",
            ),
            (
                "Feed X",
                Some(s.design.z.im),
                s.builds.iter().map(|b| b.z.im).collect(),
                1,
                " Ω",
            ),
            (
                "Resonance",
                s.design.resonance,
                s.builds.iter().filter_map(|b| b.resonance).collect(),
                4,
                " MHz",
            ),
            (
                "Peak gain",
                Some(s.design.peak_dbi),
                s.builds.iter().map(|b| b.peak_dbi).collect(),
                2,
                " dBi",
            ),
            (
                "Front-to-back",
                s.design.front_to_back,
                s.builds.iter().filter_map(|b| b.front_to_back).collect(),
                1,
                " dB",
            ),
        ];
        ui.label(
            RichText::new(format!(
                "{} builds at ±{} {}, at {:.4} MHz (SWR against {z0} Ω):",
                s.builds.len(),
                crate::views::wires::show_length(s.accuracy, unit, mhz),
                unit.symbol(),
                s.mhz
            ))
            .weak(),
        );
        egui::Grid::new("tolerance")
            .num_columns(5)
            .spacing([14.0, 3.0])
            .striped(true)
            .show(ui, |ui| {
                for h in ["", "as drawn", "lowest 10%", "middle", "highest 10%"] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                for (name, design, mut values, digits, unit) in rows {
                    let Some(p) = percentiles(&mut values) else {
                        continue;
                    };
                    let text = |v: f64| format!("{}{unit}", fixed(digits)(v));
                    ui.label(name);
                    ui.label(design.map_or(String::new(), text));
                    for v in [p[1], p[2], p[3]] {
                        ui.label(text(v));
                    }
                    ui.end_row();
                }
            });
        ui.label(
            RichText::new(
                "The 10% columns: one build in ten comes out below the lowest, one in ten above the \
                 highest. Straight wires only: arcs, helices and patches are built as drawn.",
            )
            .weak()
            .small(),
        );
    }

    /// The sensitivity check: each wire made a little longer at its tips.
    fn sensitivity(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
        if let Some(job) = &self.trim_job
            && let Ok(r) = job.rx.try_recv()
        {
            let run = self.trims.take().map(|(_, run)| run).unwrap_or_default();
            self.trims = Some((r, run));
            self.trim_job = None;
            self.colours(data, settings);
        }
        let Some(model) = &data.model else { return };
        let Ok(numeric) = &model.numeric else { return };
        let now = numeric.to_nec_text();
        let unit = settings.length();
        let mhz = data.mhz.max(1e-3);
        if self.trim <= 0.0 {
            // About a thousandth of a wavelength, rounded: 2 cm at 14 MHz.
            let lam = straws_core::units::wavelength(mhz);
            self.trim = f64::from(crate::scene::nice((lam / 1000.0) as f32));
        }
        ui.label(RichText::new("Sensitivity: which wire to trim").strong()).on_hover_text(
            "Makes each wire a little longer at its free tips (where you'd trim it) and runs the \
             model again: the table shows how far that moves each figure, so you know which \
             element to trim, and which way. Wires joined at both ends (loops) are left out.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(self.trim_job.is_none(), |ui| {
                ui.label("Each wire longer by");
                let mut v = unit.from_metres(self.trim, mhz);
                if ui
                    .add(
                        crate::views::num(&mut v)
                            .speed(unit.from_metres(0.001, mhz))
                            .range(0.0..=f64::MAX)
                            .suffix(format!(" {}", unit.symbol())),
                    )
                    .changed()
                {
                    self.trim = unit.to_metres(v, mhz).max(1e-6);
                }
            });
            if let Some(job) = &self.trim_job {
                ui.spinner();
                ui.label(format!(
                    "{} of {} wires run",
                    job.done.load(Ordering::Relaxed),
                    job.total
                ));
                if ui.button("Stop").clicked() {
                    job.cancel.cancel();
                    self.trim_job = None;
                    self.trims = None;
                }
            } else if ui
                .button("Run")
                .on_hover_text(format!("At {:.4} MHz, one run per wire", data.mhz))
                .clicked()
            {
                // Mesh lines (plates, dishes, grids) aren't trimmed one by
                // one; nor are hidden helper wires.
                let mut meshes: Vec<i64> = Vec::new();
                for p in straws_core::deck::plates::plate_sets(&model.deck) {
                    meshes.extend(p.tags);
                }
                for r in straws_core::deck::reflectors::reflector_sets(&model.deck) {
                    meshes.extend(r.tags);
                }
                for g in straws_core::deck::grids::grid_sets(&model.deck) {
                    meshes.extend(g.tags);
                }
                let mut tags: Vec<i64> = model
                    .visible_segments()
                    .map(|s| i64::from(s.tag))
                    .filter(|t| !meshes.contains(t))
                    .collect();
                tags.sort_unstable();
                tags.dedup();
                tags.truncate(MAX_TRIM_WIRES);
                let plan = data.sweep.as_ref().map_or_else(
                    || FrequencyPlan::around(data.mhz),
                    |s| FrequencyPlan {
                        points: s.plan.points.clamp(2, 41),
                        ..s.plan
                    },
                );
                let (deck, delta) = (numeric.clone(), self.trim);
                let over_ground = data.over_ground;
                let estimate = match straws_core::analysis::buried::need(&model.deck, numeric) {
                    straws_core::analysis::buried::Need::Estimate(s) => Some(s),
                    _ => None,
                };
                let cancel = CancelToken::new();
                let c = cancel.clone();
                let done = Arc::new(AtomicUsize::new(0));
                let d = done.clone();
                let (tx, rx) = channel();
                let ctx = ui.ctx().clone();
                let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
                let total = tags.len();
                std::thread::spawn(move || {
                    let ctx2 = ctx.clone();
                    let r = Engine::self_hosted()
                        .map_err(|e| e.to_string())
                        .and_then(|e| {
                            trims(
                                &e,
                                &deck,
                                &tags,
                                &plan,
                                mhz,
                                delta,
                                threads,
                                over_ground,
                                estimate.as_ref(),
                                &c,
                                &|n, _| {
                                    d.store(n, Ordering::Relaxed);
                                    ctx2.request_repaint();
                                },
                            )
                        });
                    let _ = tx.send(r);
                    ctx.request_repaint();
                });
                self.trim_job = Some(TrimJob {
                    cancel,
                    rx,
                    done,
                    total,
                });
                self.trims = Some((Err(String::new()), now.clone()));
            }
        });
        if self.trim_job.is_some() {
            return;
        }
        let Some((result, run)) = &self.trims else {
            return;
        };
        let t = match result {
            Ok(t) => t,
            Err(e) if e.is_empty() => return,
            Err(e) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("The check couldn't run: {e}."),
                );
                return;
            }
        };
        if *run != now {
            ui.label(RichText::new("The model has changed since this check: run it again.").weak());
        }
        let before = self.colour_by;
        ui.horizontal(|ui| {
            ui.label("Colour the wires in 3D by");
            egui::ComboBox::from_id_salt("trim colours")
                .selected_text(self.colour_by.label())
                .show_ui(ui, |ui| {
                    for c in [
                        ColourBy::Off,
                        ColourBy::Resonance,
                        ColourBy::Swr,
                        ColourBy::Gain,
                        ColourBy::FrontToBack,
                    ] {
                        ui.selectable_value(&mut self.colour_by, c, c.label());
                    }
                });
        });
        if self.colour_by != before {
            self.colours(data, settings);
        }
        let z0 = settings.z0;
        let swr = |f: &Figures| straws_core::analysis::feed::swr(f.z, z0);
        let d = &t.design;
        // Most effect on the resonance (or SWR, when there's none) first.
        let mut rows: Vec<&(i64, Figures)> = t.wires.iter().collect();
        let effect = |f: &Figures| match (f.resonance, d.resonance) {
            (Some(a), Some(b)) => (a - b).abs(),
            _ => (swr(f) - swr(d)).abs(),
        };
        rows.sort_by(|a, b| effect(&b.1).total_cmp(&effect(&a.1)));
        ui.label(
            RichText::new(format!(
                "Each wire {} {} longer, at {:.4} MHz (SWR against {z0} \u{3a9}); click a row to select the wire:",
                crate::views::wires::show_length(t.delta, unit, mhz),
                unit.symbol(),
                t.mhz
            ))
            .weak(),
        );
        let mut select = None;
        egui::Grid::new("sensitivity")
            .num_columns(5)
            .spacing([14.0, 3.0])
            .striped(true)
            .show(ui, |ui| {
                for h in ["Wire", "Resonance", "SWR", "Peak gain", "F/B"] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                for (tag, f) in rows {
                    if ui.selectable_label(false, format!("{tag}")).clicked() {
                        select = Some(*tag);
                    }
                    ui.label(match (f.resonance, d.resonance) {
                        (Some(a), Some(b)) => format!("{:+.1} kHz", 1000.0 * (a - b)),
                        _ => String::new(),
                    });
                    ui.label(format!("{:+.3}", swr(f) - swr(d)));
                    ui.label(format!("{:+.2} dB", f.peak_dbi - d.peak_dbi));
                    ui.label(match (f.front_to_back, d.front_to_back) {
                        (Some(a), Some(b)) => format!("{:+.1} dB", a - b),
                        _ => String::new(),
                    });
                    ui.end_row();
                }
            });
        ui.label(
            RichText::new(
                "To move a figure the other way, shorten that wire. Wires joined at both ends, and \
                 the mesh lines of plates, dishes and grids, aren't listed.",
            )
            .weak()
            .small(),
        );
        if let Some(tag) = select
            && let Some(m) = &data.model
        {
            data.selection = m
                .wires
                .iter()
                .filter(|w| w.tag == tag)
                .map(|w| w.line)
                .collect();
        }
    }

    /// Colours the 3D view's wires by the chosen effect (or clears them).
    fn colours(&self, data: &mut Data, settings: &Settings) {
        let Some((Ok(t), _)) = &self.trims else {
            data.wire_colours = None;
            return;
        };
        let z0 = settings.z0;
        let swr = |f: &Figures| straws_core::analysis::feed::swr(f.z, z0);
        let d = &t.design;
        let effect = |f: &Figures| -> Option<f64> {
            match self.colour_by {
                ColourBy::Off => None,
                ColourBy::Resonance => Some((f.resonance? - d.resonance?).abs()),
                ColourBy::Swr => Some((swr(f) - swr(d)).abs()),
                ColourBy::Gain => Some((f.peak_dbi - d.peak_dbi).abs()),
                ColourBy::FrontToBack => Some((f.front_to_back? - d.front_to_back?).abs()),
            }
        };
        let values: Vec<(i64, f64)> = t
            .wires
            .iter()
            .filter_map(|(tag, f)| Some((*tag, effect(f)?)))
            .collect();
        let largest = values.iter().map(|v| v.1).fold(0.0, f64::max);
        if self.colour_by == ColourBy::Off || values.is_empty() || largest <= 0.0 {
            data.wire_colours = None;
            return;
        }
        let version = data.wire_colours.as_ref().map_or(0, |c| c.version) + 1;
        data.wire_colours = Some(crate::app::WireColours {
            generation: data.model_generation,
            version,
            label: format!(
                "{} when each wire is {} {} longer",
                self.colour_by.label(),
                crate::views::wires::show_length(t.delta, settings.length(), data.mhz),
                settings.length().symbol()
            ),
            values: values
                .into_iter()
                .filter_map(|(tag, v)| Some((u32::try_from(tag).ok()?, (v / largest) as f32)))
                .collect(),
        });
    }
}

fn checks_ui(ui: &mut egui::Ui, data: &mut Data) {
    let Some(model) = &data.model else {
        return empty_note(ui, "Open or start a model to check it.");
    };
    let (warn, error) = (ui.visuals().warn_fg_color, ui.visuals().error_fg_color);
    let mut select = None;
    if let Err(e) = &model.numeric {
        ui.colored_label(error, format!("✖ The deck can't be read: {e}"));
        ui.label(RichText::new("Fix it in the Deck view; the line is marked there.").weak());
        return;
    }
    let errors = data
        .checks
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warnings = data.checks.len() - errors;
    if data.checks.is_empty() {
        ui.label(format!(
            "✔ No geometry problems found (checked at {:.3} MHz, the top of the sweep).",
            data.max_mhz
        ));
    } else {
        ui.label(format!(
                    "{errors} problem{} and {warnings} warning{} at {:.3} MHz. Click one to select its wires.",
                    if errors == 1 { "" } else { "s" },
                    if warnings == 1 { "" } else { "s" },
                    data.max_mhz
                ));
    }
    ui.add_space(4.0);
    for (k, issue) in data.checks.iter().enumerate() {
        let (mark, colour) = match issue.severity {
            Severity::Error => ("✖", error),
            Severity::Warning => ("⚠", warn),
        };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(colour, mark);
            let r = ui.add(egui::Label::new(&issue.message).sense(egui::Sense::click()));
            if !issue.fix.is_empty() {
                ui.label(RichText::new(&issue.fix).weak());
            }
            if r.clicked() {
                select = Some(k);
            }
            r.on_hover_text("Select these wires");
        });
    }
    ui.add_space(8.0);
    ui.separator();
    average_gain_test(ui, data);
    if let Some(k) = select
        && let Some(m) = &data.model
    {
        let tags = &data.checks[k].tags;
        data.selection = m
            .wires
            .iter()
            .filter(|w| tags.iter().any(|&t| i64::from(t) == w.tag))
            .map(|w| w.line)
            .collect();
    }
}

/// The average gain test on the pattern shown now.
fn average_gain_test(ui: &mut egui::Ui, data: &Data) {
    ui.label(RichText::new("Average gain test").strong())
        .on_hover_text(
            "Adds up the power in the whole pattern. A lossless antenna in free space \
             or over perfect ground should give 1 (0 dB). Far from 1 means the model \
             can't be trusted: look at segment lengths and junctions.",
        );
    let Some(point) = data.shown_point() else {
        ui.label(RichText::new("Waiting for a result.").weak());
        return;
    };
    let Some(grid) = &point.pattern else {
        ui.label(RichText::new("No pattern at this frequency.").weak());
        return;
    };
    let perfect = data
        .model
        .as_ref()
        .and_then(|m| m.numeric.as_ref().ok())
        .and_then(|n| n.cards.iter().rfind(|c| c.is("GN")))
        .is_some_and(|c| c.ints[0] == 1);
    let lossy_ground = data.over_ground && !perfect;
    match average_gain(grid) {
        Some(ag) => {
            let db = 10.0 * ag.log10();
            let good = (0.9..=1.1).contains(&ag);
            let verdict = if lossy_ground {
                "over real ground part of the power is lost in the ground, so less than 1 is normal"
            } else if good {
                "good"
            } else {
                "suspect: the model may not be accurate (losses in loads or wires also lower it)"
            };
            ui.label(format!(
                "{:.4} MHz: {ag:.3} ({db:+.2} dB), {verdict}.",
                point.mhz
            ));
        }
        None => {
            ui.label(RichText::new("The pattern doesn't cover every direction.").weak());
        }
    }
}
