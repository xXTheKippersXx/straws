//! The optimiser view: which variables may change and within what limits,
//! the goals, and a live run that shows the best design so far.

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};
use egui_plot::{Line, Plot, PlotPoints};
use straws_core::engine::{CancelToken, Engine};
use straws_core::optimise::{
    FAILED, Goal, Limits, Method, Options, Param, Problem, Trial, optimise,
};

use crate::app::{Data, Level, Settings};

use super::empty_note;

struct VarSetting {
    name: String,
    vary: bool,
    min: f64,
    max: f64,
}

enum Msg {
    Progress {
        evaluations: usize,
        best: Trial,
        stage: &'static str,
        rejected: usize,
        last_rejection: Option<String>,
    },
    Done(Trial),
}

struct Running {
    cancel: CancelToken,
    rx: Receiver<Msg>,
    started: Instant,
    /// Variables being changed, in the order of the trial's values.
    names: Vec<String>,
    /// The deck as it was when the run started.
    deck_text: String,
    last_shown: Instant,
}

pub struct OptimiserView {
    vars: Vec<VarSetting>,
    goals: Vec<Goal>,
    method: Method,
    max_evaluations: usize,
    stop_when_met: bool,
    /// Skip designs whose wires cross, overlap or go underground.
    avoid_clashes: bool,
    /// Largest overall size along x, y and z (metres), when ticked.
    size: [(bool, f64); 3],
    /// Designs skipped as impossible in this run, and the last reason.
    rejected: usize,
    last_rejection: Option<String>,
    /// Limits for "set limits" as a percentage either side.
    spread_percent: f64,
    run: Option<Running>,
    evaluations: usize,
    stage: &'static str,
    best: Option<Trial>,
    /// Names of the variables in `best`.
    best_names: Vec<String>,
    /// Variable values before the last run, to put back.
    before: Option<Vec<(String, f64)>>,
    /// (designs run, best cost) as the run went.
    history: Vec<[f64; 2]>,
    elapsed: Duration,
}

impl Default for OptimiserView {
    fn default() -> Self {
        OptimiserView {
            vars: Vec::new(),
            goals: Vec::new(),
            method: Method::DifferentialEvolution,
            max_evaluations: 400,
            stop_when_met: true,
            avoid_clashes: true,
            size: [(false, 1.0); 3],
            rejected: 0,
            last_rejection: None,
            spread_percent: 20.0,
            run: None,
            evaluations: 0,
            stage: "",
            best: None,
            best_names: Vec::new(),
            before: None,
            history: Vec::new(),
            elapsed: Duration::ZERO,
        }
    }
}

fn num(ui: &mut egui::Ui, v: &mut f64, speed: f64, suffix: &str) -> egui::Response {
    ui.add(
        crate::views::num(v)
            .speed(speed)
            .max_decimals(6)
            .suffix(suffix),
    )
}

/// A goal's figure for the results table.
fn figure(goal: &Goal, value: f64) -> String {
    match goal {
        Goal::Swr { .. } => format!("worst SWR {value:.3}"),
        Goal::Gain { .. } => format!("{value:.2} dBi"),
        Goal::FrontToBack { .. } => format!("{value:.2} dB"),
        Goal::CommonMode { .. } => format!("{value:.2} % on the shield"),
        Goal::Match { .. } => format!("RMS reflection difference {value:.4}"),
    }
}

/// A variable's own slider range (low, high).
type Range = (f64, f64);

impl OptimiserView {
    /// Clears a finished run's results (e.g. when another file opens).
    pub fn forget_results(&mut self) {
        if let Some(r) = &self.run {
            r.cancel.cancel();
        }
        self.run = None;
        self.best = None;
        self.before = None;
        self.history.clear();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
        self.poll(ui.ctx(), data);
        let Some(model) = &data.model else {
            return empty_note(ui, "Open a model to optimise it.");
        };
        // Keep the variable list in step with the deck.
        // Variables marked "fixed" in the Variables view are left alone.
        let values: Vec<(String, f64, Option<Range>)> = model
            .deck
            .variables()
            .into_iter()
            .filter_map(|v| {
                let current = data
                    .overrides
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&v.name))
                    .map(|(_, x)| *x)
                    .or(v.literal)?;
                let notes = model.deck.notes(&v.name);
                (!notes.fixed).then_some((v.name, current, notes.range))
            })
            .collect();
        self.vars
            .retain(|s| values.iter().any(|(n, _, _)| n == &s.name));
        for (name, value, range) in &values {
            if !self.vars.iter().any(|s| &s.name == name) {
                let span = if *value == 0.0 {
                    1.0
                } else {
                    value.abs() * 0.2
                };
                // The variable's own slider range, if it has one.
                let (min, max) = range.unwrap_or((value - span, value + span));
                self.vars.push(VarSetting {
                    name: name.clone(),
                    vary: false,
                    min,
                    max,
                });
            }
        }
        if self.goals.is_empty() {
            let plan = data.sweep.as_ref().map(|s| s.plan);
            let (a, b) = plan.map_or((data.mhz * 0.99, data.mhz * 1.01), |p| {
                (p.start_mhz, p.stop_mhz)
            });
            self.goals.push(Goal::Swr {
                start_mhz: a,
                stop_mhz: b,
                points: 11,
                z0: settings.z0,
                target: 1.5,
                weight: 1.0,
                through: None,
            });
        }
        let running = self.run.is_some();
        let has_shield = straws_core::deck::shield::shield_tag(&model.deck).is_some();
        let readable = model.numeric.is_ok();
        let over_ground = data.over_ground;
        let mut start = false;
        let mut revert = false;
        // Radials on or under the ground: scored with their estimate (slower).
        let buried = model.numeric.as_ref().is_ok_and(|n| {
            straws_core::deck::radials::radial_numbers(&model.deck, n)
                .iter()
                .any(|r| r.buried)
                || straws_core::deck::grids::grid_numbers(&model.deck, n)
                    .iter()
                    .any(|g| g.buried)
        });

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if buried {
                ui.label(
                    RichText::new(
                        "This model has radials or a grid on or under the ground: each design is scored with their estimate, as the views show it, which takes about five engine runs a design, so the optimiser runs slower.",
                    )
                    .weak(),
                );
            }
            if values.is_empty() {
                ui.label(
                    RichText::new(
                        "The optimiser changes variables, and this model has none it may change. \
                         Make some first: right-click a number in the wire table, use Make a \
                         variable for the selected wires (lengths, spacings, heights), or start \
                         from a template, which has them already. Variables marked \"may vary it\" \
                         off in the Variables view are left alone.",
                    )
                    .weak(),
                );
                return;
            }
            ui.label(RichText::new("Variables to change").strong());
            ui.add_enabled_ui(!running, |ui| {
                egui::Grid::new("opt vars")
                    .num_columns(5)
                    .spacing([10.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("");
                        ui.label("Variable");
                        ui.label("Now");
                        ui.label("Lowest");
                        ui.label("Highest");
                        ui.end_row();
                        for s in &mut self.vars {
                            let now = values.iter().find(|(n, _, _)| n == &s.name).map_or(0.0, |v| v.1);
                            ui.checkbox(&mut s.vary, "");
                            ui.label(&s.name);
                            ui.label(format!("{now:.6}"));
                            let speed = (s.max - s.min).abs().max(1e-6) * 0.005;
                            num(ui, &mut s.min, speed, "");
                            num(ui, &mut s.max, speed, "");
                            if s.max < s.min {
                                std::mem::swap(&mut s.min, &mut s.max);
                            }
                            if now < s.min || now > s.max {
                                ui.colored_label(ui.visuals().warn_fg_color, "⚠")
                                    .on_hover_text("The current value is outside these limits; the optimiser starts from the nearest limit.");
                            }
                            ui.end_row();
                        }
                    });
                ui.horizontal(|ui| {
                    if ui.button("Tick all").clicked() {
                        self.vars.iter_mut().for_each(|v| v.vary = true);
                    }
                    if ui.button("Untick all").clicked() {
                        self.vars.iter_mut().for_each(|v| v.vary = false);
                    }
                    if ui
                        .button("Set limits")
                        .on_hover_text("Limits of the ticked variables: their current value plus and minus this percentage")
                        .clicked()
                    {
                        for v in self.vars.iter_mut().filter(|v| v.vary) {
                            let now = values.iter().find(|(n, _, _)| n == &v.name).map_or(0.0, |x| x.1);
                            let span = if now == 0.0 { 1.0 } else { now.abs() * self.spread_percent / 100.0 };
                            (v.min, v.max) = (now - span, now + span);
                        }
                    }
                    ui.label("±");
                    ui.add(crate::views::num(&mut self.spread_percent).range(1.0..=90.0).suffix(" %"));
                });
            });

            ui.add_space(10.0);
            ui.label(RichText::new("Goals").strong()).on_hover_text(
                "Each goal is met once its target is reached; the weight says how much it matters against the others.",
            );
            let mut remove = None;
            let peak = data
                .shown_point()
                .and_then(|p| p.pattern.as_ref())
                .map(|g| g.max(settings.component()));
            // Goals measured at the radio follow the Feedline view.
            let chain_now = crate::views::feedline::radio_chain(data, settings, data.mhz);
            for g in &mut self.goals {
                if let Goal::Swr { through: Some(t), .. } = g {
                    *t = chain_now.clone();
                }
            }
            ui.add_enabled_ui(!running, |ui| {
                for (k, goal) in self.goals.iter_mut().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        match goal {
                            Goal::Swr {
                                start_mhz,
                                stop_mhz,
                                points,
                                z0,
                                target,
                                weight,
                                through,
                            } => {
                                ui.label("SWR at most");
                                num(ui, target, 0.01, "");
                                ui.label("from");
                                num(ui, start_mhz, 0.01, " MHz");
                                ui.label("to");
                                num(ui, stop_mhz, 0.01, " MHz");
                                // Or a whole band from the toolbar's band list.
                                egui::ComboBox::from_id_salt(("goal band", k))
                                    .selected_text("band")
                                    .width(50.0)
                                    .show_ui(ui, |ui| {
                                        for b in crate::views::bands::chosen(settings) {
                                            if ui.selectable_label(false, &b.name).clicked() {
                                                *start_mhz = b.start_mhz;
                                                *stop_mhz = b.stop_mhz;
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text(
                                        "Fill in a band's edges (from the toolbar's Band menu's list)",
                                    );
                                ui.add(crate::views::num(points).range(1..=51).suffix(" pts"))
                                    .on_hover_text("Frequencies checked across the band");
                                ui.label("Z₀");
                                num(ui, z0, 0.5, " Ω");
                                ui.label("weight");
                                num(ui, weight, 0.05, "");
                                let mut at_radio = through.is_some();
                                if ui
                                    .checkbox(&mut at_radio, "at the radio")
                                    .on_hover_text("Measure this SWR at the radio, through the Feedline view's balun, connectors and cable (as it shows them). A variable that sets the cable length or the balun ratio (ƒ in the Feedline view) can then be tuned with the antenna")
                                    .changed()
                                {
                                    *through = at_radio.then(|| chain_now.clone());
                                }
                                *target = target.max(1.0);
                                *z0 = z0.max(1.0);
                                if *stop_mhz < *start_mhz {
                                    *stop_mhz = *start_mhz;
                                }
                            }
                            Goal::Gain {
                                mhz,
                                elevation,
                                azimuth,
                                peak,
                                target,
                                weight,
                            } => {
                                ui.label("Gain at least");
                                num(ui, target, 0.1, " dBi");
                                ui.label("at");
                                num(ui, mhz, 0.01, " MHz");
                                ui.checkbox(peak, "wherever the peak is")
                                    .on_hover_text("Off: the gain in one direction you choose");
                                if !*peak {
                                    ui.label("elevation");
                                    num(ui, elevation, 1.0, "°");
                                    ui.label("azimuth");
                                    num(ui, azimuth, 1.0, "°");
                                }
                                ui.label("weight");
                                num(ui, weight, 0.05, "");
                            }
                            Goal::FrontToBack {
                                mhz,
                                elevation,
                                azimuth,
                                rear,
                                target,
                                weight,
                            } => {
                                egui::ComboBox::from_id_salt(("fb", k))
                                    .selected_text(if *rear { "Front-to-rear" } else { "Front-to-back" })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(rear, false, "Front-to-back")
                                            .on_hover_text("Against the direction straight behind");
                                        ui.selectable_value(rear, true, "Front-to-rear")
                                            .on_hover_text("Against the strongest direction anywhere in the back half");
                                    });
                                ui.label("at least");
                                num(ui, target, 0.5, " dB");
                                ui.label("at");
                                num(ui, mhz, 0.01, " MHz");
                                ui.label("towards elevation");
                                num(ui, elevation, 1.0, "°");
                                ui.label("azimuth");
                                num(ui, azimuth, 1.0, "°");
                                ui.label("weight");
                                num(ui, weight, 0.05, "");
                            }
                            Goal::Match {
                                name,
                                start_mhz,
                                stop_mhz,
                                measured,
                                target,
                                weight,
                                ..
                            } => {
                                ui.label(format!(
                                    "Match {name} from {start_mhz:.3} to {stop_mhz:.3} MHz ({} points): RMS reflection difference at most",
                                    measured.len()
                                ))
                                .on_hover_text(
                                    "How far the model's reflection coefficient is from the measured one, on average over \
                                     the points (0 = identical, 0.05 = close, 2 = opposite). Tick the variables that could \
                                     differ in the real antenna: lengths, height, the ground's permittivity and \
                                     conductivity, wire or load losses.",
                                );
                                num(ui, target, 0.005, "");
                                ui.label("weight");
                                num(ui, weight, 0.05, "");
                                *target = target.max(0.0);
                            }
                            Goal::CommonMode { mhz, target, weight } => {
                                ui.label("Common-mode current at most");
                                num(ui, target, 0.5, " %");
                                ui.label("of the feed current at");
                                num(ui, mhz, 0.01, " MHz");
                                ui.label("weight");
                                num(ui, weight, 0.05, "");
                                *target = target.max(0.01);
                                if !has_shield {
                                    ui.colored_label(ui.visuals().warn_fg_color, "⚠").on_hover_text(
                                        "Model the coax shield first (Feedline view, Model the shield): this goal measures the current on it",
                                    );
                                }
                            }
                        }
                        let low = match goal {
                            Goal::Gain { elevation, peak: false, .. } | Goal::FrontToBack { elevation, .. } => {
                                *elevation < 0.5
                            }
                            _ => false,
                        };
                        if low && over_ground {
                            ui.colored_label(ui.visuals().warn_fg_color, "⚠").on_hover_text(
                                "Over ground there's almost no field right at the horizon, so this goal \
                                 can't be met. Use an elevation a little above it (the take-off angle), \
                                 or peak gain.",
                            );
                        }
                        if ui.small_button("✖").on_hover_text("Remove this goal").clicked() {
                            remove = Some(k);
                        }
                    });
                }
                if let Some(k) = remove {
                    self.goals.remove(k);
                }
                let (el, az) = peak.map_or((0.0, 0.0), |(_, t, p)| ((90.0 - t).round(), p.round()));
                ui.horizontal(|ui| {
                    ui.menu_button("Add goal", |ui| {
                        let plan = data.sweep.as_ref().map(|s| s.plan);
                        if ui.button("Low SWR across a band").clicked() {
                            let (a, b) = plan.map_or((data.mhz, data.mhz), |p| (p.start_mhz, p.stop_mhz));
                            self.goals.push(Goal::Swr {
                                start_mhz: a,
                                stop_mhz: b,
                                points: 11,
                                z0: settings.z0,
                                target: 1.5,
                                weight: 1.0,
                                through: None,
                            });
                            ui.close();
                        }
                        if ui.button("Gain").clicked() {
                            self.goals.push(Goal::Gain {
                                mhz: data.mhz,
                                elevation: el,
                                azimuth: az,
                                peak: false,
                                target: peak.map_or(0.0, |p| (p.0 + 1.0).round()),
                                weight: 1.0,
                            });
                            ui.close();
                        }
                        if ui
                            .button("Low common-mode current")
                            .on_hover_text("Current on the outside of the coax, as a share of the feed current: model the shield in the Feedline view first. Make the choke a variable (ƒ there) to let the optimiser size it")
                            .clicked()
                        {
                            self.goals.push(Goal::CommonMode {
                                mhz: data.mhz,
                                target: 5.0,
                                weight: 1.0,
                            });
                            ui.close();
                        }
                        ui.menu_button("Match a measurement", |ui| {
                            if data.measurements.is_empty() {
                                ui.label(
                                    RichText::new("Import a VNA sweep first (File menu, or the Compare view).").weak(),
                                );
                            }
                            for k in 0..data.measurements.len() {
                                let name = data.measurements[k].name.clone();
                                if ui
                                    .button(&name)
                                    .on_hover_text("Fit the model to this measurement within the sweep (21 points), with the variables ticked above")
                                    .clicked()
                                {
                                    let points = crate::views::compare::antenna_points(data, settings, k);
                                    let (a, b) = data
                                        .sweep
                                        .as_ref()
                                        .map_or((0.0, f64::MAX), |s| (s.plan.start_mhz, s.plan.stop_mhz));
                                    match Goal::fit_to(&name, &points, a, b, 21, settings.z0) {
                                        Some(g) => self.goals.push(g),
                                        None => data.message(
                                            Level::Warning,
                                            format!("{name} doesn't overlap the sweep: use Sweep this range in the Compare view first."),
                                        ),
                                    }
                                    ui.close();
                                }
                            }
                        });
                        if ui.button("Front-to-back or front-to-rear").clicked() {
                            self.goals.push(Goal::FrontToBack {
                                mhz: data.mhz,
                                elevation: el,
                                azimuth: az,
                                rear: false,
                                target: 20.0,
                                weight: 1.0,
                            });
                            ui.close();
                        }
                    });
                    ui.label(RichText::new("(directions start at the current pattern's peak)").weak());
                });
            });

            ui.add_space(10.0);
            ui.add_enabled_ui(!running, |ui| {
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("method")
                        .selected_text(match self.method {
                            Method::NelderMead => "Fine-tune from here",
                            Method::DifferentialEvolution => "Search the whole range, then fine-tune",
                        })
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.method,
                                Method::DifferentialEvolution,
                                "Search the whole range, then fine-tune",
                            )
                            .on_hover_text("Global search (adaptive differential evolution) on every core, then Nelder-Mead polishing. Best for a first run or when the design is far off.");
                            ui.selectable_value(&mut self.method, Method::NelderMead, "Fine-tune from here")
                                .on_hover_text("Local search (Nelder-Mead) from the current values. Quick when the design is already close.");
                        });
                    ui.label("at most");
                    ui.add(crate::views::num(&mut self.max_evaluations).range(10..=100_000))
                        .on_hover_text("Designs to try before stopping");
                    ui.label("designs");
                    ui.checkbox(&mut self.stop_when_met, "Stop once every goal is met");
                });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.avoid_clashes, "Skip designs that can't be built")
                        .on_hover_text(
                            "Designs where wires cross, touch, lie on top of each other or go below the ground are skipped (unless the design you start from has them too)",
                        );
                    ui.label("Keep within").on_hover_text(
                        "The whole antenna's overall size along each axis (feedline shields not counted)",
                    );
                    let unit = settings.length();
                    let mhz = data.mhz.max(1e-3);
                    for (k, axis) in ["x", "y", "z"].iter().enumerate() {
                        let (on, metres) = &mut self.size[k];
                        ui.checkbox(on, *axis);
                        if *on {
                            let mut v = unit.from_metres(*metres, mhz);
                            let step = unit.from_metres(0.01, mhz);
                            if ui
                                .add(
                                    crate::views::num(&mut v)
                                        .speed(step)
                                        .range(0.0..=f64::MAX)
                                        .suffix(format!(" {}", unit.symbol())),
                                )
                                .changed()
                            {
                                *metres = unit.to_metres(v, mhz);
                            }
                        }
                    }
                });
            });
            ui.horizontal(|ui| {
                let can_start = !running && self.vars.iter().any(|v| v.vary) && !self.goals.is_empty() && readable;
                if running {
                    if ui.button("Stop").on_hover_text("Stop and keep the best design").clicked()
                        && let Some(r) = &self.run
                    {
                        r.cancel.cancel();
                    }
                } else {
                    start = ui
                        .add_enabled(can_start, egui::Button::new("Start"))
                        .on_disabled_hover_text("Tick at least one variable and add a goal")
                        .clicked();
                    if self.before.is_some() {
                        revert = ui
                            .button("Put the starting values back")
                            .on_hover_text("Undo the optimiser's change (Ctrl+Z does too)")
                            .clicked();
                    }
                }
            });

            ui.add_space(8.0);
            if let Some(best) = &self.best {
                let secs = self.elapsed.as_secs_f64();
                let rate = if secs > 0.0 { self.evaluations as f64 / secs } else { 0.0 };
                if running {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.label(format!("{}: {} designs in {secs:.1} s ({rate:.1} a second).", self.stage, self.evaluations));
                    });
                } else {
                    ui.label(format!("Finished: {} designs in {secs:.1} s ({rate:.1} a second).", self.evaluations));
                }
                if self.rejected > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} skipped as impossible to build (last: {}).",
                            self.rejected,
                            self.last_rejection.as_deref().unwrap_or("").trim_end_matches('.')
                        ))
                        .weak(),
                    );
                }
                if best.cost >= FAILED {
                    ui.colored_label(ui.visuals().warn_fg_color, "No design that runs has been found yet.");
                } else {
                    egui::Grid::new("opt results").num_columns(3).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
                        ui.label(RichText::new("Goal").weak());
                        ui.label(RichText::new("Best design").weak());
                        ui.label("");
                        ui.end_row();
                        for (goal, r) in self.goals.iter().zip(&best.results) {
                            ui.label(goal.describe());
                            ui.label(figure(goal, r.value));
                            if r.met {
                                ui.colored_label(egui::Color32::from_rgb(0, 158, 115), "met");
                            } else {
                                ui.colored_label(ui.visuals().warn_fg_color, "not yet");
                            }
                            ui.end_row();
                        }
                    });
                    let vals: Vec<String> = self
                        .best_names
                        .iter()
                        .zip(&best.values)
                        .map(|(n, v)| format!("{n} = {v:.6}"))
                        .collect();
                    ui.label(vals.join(",  "));
                    if !running && !best.all_met() {
                        ui.label(RichText::new(
                            "Not every goal was met. Try the whole-range search, more designs, wider limits, more variables, or a gentler target.",
                        ).weak());
                    }
                    // The SWR goal only looks at its own check frequencies:
                    // the full sweep can show a peak in between.
                    if !running && let Some(sweep) = data.display_sweep() {
                        for (goal, r) in self.goals.iter().zip(&best.results) {
                            let Goal::Swr { start_mhz, stop_mhz, target, z0, points, .. } = *goal else {
                                continue;
                            };
                            let worst = sweep
                                .points()
                                .filter(|p| p.mhz >= start_mhz - 1e-9 && p.mhz <= stop_mhz + 1e-9)
                                .filter_map(|p| {
                                    p.inputs
                                        .first()
                                        .map(|i| (p.mhz, straws_core::analysis::feed::swr(i.impedance, z0)))
                                })
                                .max_by(|a, b| a.1.total_cmp(&b.1));
                            if let Some((mhz, swr)) = worst
                                && swr > target * 1.05
                                && swr > r.value * 1.05
                            {
                                ui.colored_label(
                                    ui.visuals().warn_fg_color,
                                    format!(
                                        "Between its {points} check frequencies the SWR reaches {swr:.2} at {mhz:.2} MHz (see the SWR plot). Use more points in the SWR goal and run again."
                                    ),
                                );
                            }
                        }
                    }
                }
                if self.history.len() > 1 {
                    let pts: PlotPoints = self.history.iter().copied().collect();
                    Plot::new("opt progress")
                        .height(140.0)
                        .x_axis_label("designs")
                        .y_axis_label("cost")
                        .allow_scroll(false)
                        .allow_drag(false)
                        .allow_zoom(false)
                        .show(ui, |pui| {
                            pui.line(Line::new("best so far", pts).width(2.0));
                        });
                }
                if !running {
                    ui.label(RichText::new("The best design is now in the Variables view: Ctrl+S saves it into the deck.").weak());
                }
            }
        });
        if start {
            self.start(data, ui.ctx().clone());
        }
        if revert && let Some(before) = self.before.take() {
            data.overrides = before;
            data.edit_event = Some(true);
        }
    }

    fn start(&mut self, data: &mut Data, ctx: egui::Context) {
        let Some(model) = &data.model else { return };
        let engine = match Engine::self_hosted() {
            Ok(e) => e.with_solver_threads(1),
            Err(e) => {
                data.message(Level::Error, format!("Can't start the NEC engine: {e}"));
                return;
            }
        };
        let current = |name: &str| {
            data.overrides
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, x)| *x)
                .or_else(|| {
                    model
                        .deck
                        .variables()
                        .into_iter()
                        .find(|v| v.name.eq_ignore_ascii_case(name))
                        .and_then(|v| v.literal)
                })
                .unwrap_or(0.0)
        };
        let params: Vec<Param> = self
            .vars
            .iter()
            .filter(|v| v.vary)
            .map(|v| Param {
                name: v.name.clone(),
                min: v.min,
                max: v.max,
                start: current(&v.name).clamp(v.min, v.max),
            })
            .collect();
        let names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let problem = Problem {
            deck: model.deck.clone(),
            fixed: data.overrides.clone(),
            params,
            goals: self.goals.clone(),
            limits: Limits {
                // Only what the starting design doesn't have already.
                clashes: self.avoid_clashes.then(|| {
                    model.numeric.as_ref().map_or(0, |n| {
                        straws_core::analysis::checks::clashes(&model.segments, n, data.over_ground)
                            .len()
                    })
                }),
                size: self.size.map(|(on, m)| on.then_some(m)),
            },
        };
        let (tx, rx) = channel();
        let cancel = CancelToken::new();
        let mut options = Options::new(self.method, self.max_evaluations);
        options.stop_when_met = self.stop_when_met;
        let c = cancel.clone();
        std::thread::spawn(move || {
            let mut last = Instant::now() - Duration::from_secs(1);
            let tx2 = tx.clone();
            let ctx2 = ctx.clone();
            let best = optimise(&problem, &engine, options, &c, move |p| {
                // Report improvements, and the count now and then.
                if p.improved || last.elapsed() > Duration::from_millis(200) {
                    last = Instant::now();
                    let _ = tx2.send(Msg::Progress {
                        evaluations: p.evaluations,
                        best: p.best.clone(),
                        stage: p.stage,
                        rejected: p.rejected,
                        last_rejection: p.last_rejection.map(str::to_string),
                    });
                    ctx2.request_repaint();
                }
            });
            let _ = tx.send(Msg::Done(best));
            ctx.request_repaint();
        });
        self.before = Some(data.overrides.clone());
        self.best = None;
        self.evaluations = 0;
        self.rejected = 0;
        self.last_rejection = None;
        self.history.clear();
        self.stage = "Starting";
        self.best_names = names.clone();
        self.run = Some(Running {
            cancel,
            rx,
            started: Instant::now(),
            names,
            deck_text: model.deck.to_text(),
            last_shown: Instant::now() - Duration::from_secs(1),
        });
    }

    /// Takes progress from the running optimiser and shows the best design.
    fn poll(&mut self, ctx: &egui::Context, data: &mut Data) {
        let Some(run) = &mut self.run else { return };
        self.elapsed = run.started.elapsed();
        ctx.request_repaint_after(Duration::from_millis(250));
        // Editing the deck itself while optimising would mix two designs.
        if data
            .model
            .as_ref()
            .is_some_and(|m| m.deck.to_text() != run.deck_text)
        {
            run.cancel.cancel();
        }
        let mut done = None;
        let mut newest = None;
        while let Ok(msg) = run.rx.try_recv() {
            match msg {
                Msg::Progress {
                    evaluations,
                    best,
                    stage,
                    rejected,
                    last_rejection,
                } => {
                    self.evaluations = evaluations;
                    self.rejected = rejected;
                    self.last_rejection = last_rejection;
                    self.stage = stage;
                    if best.cost < FAILED {
                        let point = [evaluations as f64, best.cost];
                        if self.history.last().is_none_or(|l| l[1] != point[1]) {
                            self.history.push(point);
                        }
                    }
                    newest = Some(best);
                }
                Msg::Done(best) => done = Some(best),
            }
        }
        if let Some(best) = newest {
            let changed = self.best.as_ref().is_none_or(|b| b.values != best.values);
            self.best = Some(best);
            // Show the best design so far, a few times a second at most.
            if changed && run.last_shown.elapsed() > Duration::from_millis(300) {
                run.last_shown = Instant::now();
                if let Some(b) = &self.best
                    && b.cost < FAILED
                {
                    set_values(data, &run.names, &b.values, false);
                }
            }
        }
        if let Some(best) = done {
            let names = run.names.clone();
            let edited = data
                .model
                .as_ref()
                .is_some_and(|m| m.deck.to_text() != run.deck_text);
            self.run = None;
            if let Some(&[_, c]) = self.history.last() {
                self.history.push([self.evaluations as f64, c]);
            }
            if edited {
                data.message(
                    Level::Warning,
                    "Optimiser stopped because the deck was edited.",
                );
            } else if best.cost < FAILED {
                set_values(data, &names, &best.values, true);
                let met = if best.all_met() {
                    "every goal is met"
                } else {
                    "not every goal is met"
                };
                data.message(
                    Level::Info,
                    format!(
                        "Optimiser finished after {} designs: {met}. The best design is applied.",
                        self.evaluations
                    ),
                );
            } else {
                data.message(Level::Warning, "The optimiser found no design that runs.");
            }
            self.best = Some(best);
        }
    }
}

fn set_values(data: &mut Data, names: &[String], values: &[f64], finished: bool) {
    for (n, v) in names.iter().zip(values) {
        data.overrides.retain(|(o, _)| !o.eq_ignore_ascii_case(n));
        data.overrides.push((n.clone(), *v));
    }
    data.edit_event = Some(data.edit_event.unwrap_or(false) || finished);
}

impl Drop for OptimiserView {
    /// Opening another model replaces the views: stop a run that's going.
    fn drop(&mut self) {
        if let Some(r) = &self.run {
            r.cancel.cancel();
        }
    }
}
