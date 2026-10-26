//! The main window: menus, toolbar with the frequency slider, docked views
//! and the status bar.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;
use egui_dock::{DockArea, DockState, NodeIndex};
use serde::{Deserialize, Serialize};
use straws_core::analysis::checks::{Issue, check_in};
use straws_core::analysis::pattern::Component;
use straws_core::analysis::plan::{FrequencyPlan, has_ground};
use straws_core::analysis::sweep::{FreqPoint, SweepResults};
use straws_core::deck::Deck;
use straws_core::units::{Length, WireSize};

use crate::compute::{Compute, Update};
use crate::model::LoadedModel;
use crate::render3d::Gpu;
use crate::views::{self, ViewStates};

/// Pattern grid step (degrees) computed at every sweep frequency.
const SWEEP_PATTERN_STEP: f64 = 5.0;
/// Finer step computed where the frequency slider comes to rest.
const REFINED_PATTERN_STEP: f64 = 2.0;
/// How long the slider must rest before the finer calculation starts.
const REST_BEFORE_REFINE: Duration = Duration::from_millis(250);

/// The dockable views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    Model,
    Pattern,
    Cuts,
    Swr,
    Deck,
    Messages,
    Variables,
    Wires,
    Checks,
    Ground,
    Smith,
    Compare,
    Optimiser,
    Matching,
    Templates,
    Feedline,
    NearField,
    /// Sources: amplitude and phase of every feed point.
    Feeds,
    /// Receive figures (RDF, F/B) across the sweep.
    Receive,
    /// The feed system (balun, cable, matching network) tuned over a band.
    FeedSystem,
    /// No longer a view (curves are drawn and edited in the 3D model view);
    /// kept so layouts saved with it still load, and removed from them.
    Curves,
}

impl Tab {
    pub const ALL: [Tab; 20] = [
        Tab::Model,
        Tab::Pattern,
        Tab::Cuts,
        Tab::Swr,
        Tab::Deck,
        Tab::Messages,
        Tab::Variables,
        Tab::Wires,
        Tab::Checks,
        Tab::Ground,
        Tab::Smith,
        Tab::Compare,
        Tab::Optimiser,
        Tab::Matching,
        Tab::Templates,
        Tab::Feedline,
        Tab::NearField,
        Tab::Feeds,
        Tab::Receive,
        Tab::FeedSystem,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Model => "3D model",
            Tab::Pattern => "3D pattern",
            Tab::Cuts => "Pattern cuts",
            Tab::Swr => "SWR and impedance",
            Tab::Deck => "Deck",
            Tab::Messages => "Messages",
            Tab::Variables => "Variables",
            Tab::Wires => "Wires",
            Tab::Checks => "Checks",
            Tab::Ground => "Ground",
            Tab::Smith => "Smith chart",
            Tab::Compare => "Compare",
            Tab::Optimiser => "Optimiser",
            Tab::Matching => "Matching",
            Tab::Templates => "Templates",
            Tab::Feedline => "Feedline",
            Tab::NearField => "Near field",
            Tab::Feeds => "Feeds",
            Tab::Receive => "Receive",
            Tab::FeedSystem => "Feed system",
            Tab::Curves => "Curves",
        }
    }
}

/// Settings kept between sessions.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    /// Reference impedance for SWR, ohms.
    pub z0: f64,
    /// Index into `Component::ALL`.
    pub component: usize,
    /// dB below the peak shown in patterns.
    pub range_db: f64,
    /// Draw the pattern over the antenna in the model view.
    pub pattern_on_model: bool,
    /// Colour the wires by their current in the model view.
    pub show_currents: bool,
    pub recent: Vec<PathBuf>,
    /// Index into `Length::ALL`.
    pub length_unit: usize,
    /// Index into `WireSize::ALL`.
    pub wire_unit: usize,
    /// Snap to the grid and to wire ends when drawing and dragging.
    pub snap: bool,
    /// Feedline: index into `CABLES` (past the end: custom), length (m).
    pub feed_cable: usize,
    pub feed_length: f64,
    pub feed_z0: f64,
    pub feed_vf: f64,
    pub feed_loss10: f64,
    pub feed_loss100: f64,
    /// Balun impedance ratio and loss (dB).
    pub balun_ratio: f64,
    pub balun_loss: f64,
    /// Coax shield model: choke resistance (ohms, 0 for none) and length
    /// (metres, 0 to reach the ground or half a wavelength).
    pub shield_choke: f64,
    pub shield_length: f64,
    /// Where the Band menus' bands come from: 0-2 = ITU Regions 1-3, 3 =
    /// `my_bands`.
    pub band_plan: usize,
    /// The user's own bands: name, from and to (MHz).
    pub my_bands: Vec<(String, f64, f64)>,
    /// Feedline connectors at each end (0 none, then `CONNECTORS`, then
    /// "other" with its own loss in dB), and the line length (mm) the
    /// antenna end's add.
    pub conn_antenna: usize,
    pub conn_antenna_db: f64,
    pub conn_radio: usize,
    pub conn_radio_db: f64,
    pub conn_length_mm: f64,
    /// Size of everything on screen (text included), 1 = normal.
    pub zoom: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: true,
            z0: 50.0,
            component: 0,
            range_db: 40.0,
            pattern_on_model: false,
            show_currents: false,
            recent: Vec::new(),
            length_unit: 0,
            wire_unit: 0,
            snap: true,
            feed_cable: 2,
            feed_length: 30.0,
            feed_z0: 50.0,
            feed_vf: 0.66,
            feed_loss10: 2.0,
            feed_loss100: 6.2,
            balun_ratio: 1.0,
            balun_loss: 0.1,
            shield_choke: 2000.0,
            shield_length: 0.0,
            band_plan: 0,
            my_bands: Vec::new(),
            conn_antenna: 0,
            conn_antenna_db: 0.1,
            conn_radio: 0,
            conn_radio_db: 0.1,
            conn_length_mm: 0.0,
            zoom: 1.0,
        }
    }
}

impl Settings {
    pub fn component(&self) -> Component {
        Component::ALL[self.component.min(Component::ALL.len() - 1)]
    }

    pub fn length(&self) -> Length {
        Length::ALL[self.length_unit.min(Length::ALL.len() - 1)]
    }

    pub fn wire_size(&self) -> WireSize {
        WireSize::ALL[self.wire_unit.min(WireSize::ALL.len() - 1)]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

pub struct Message {
    pub level: Level,
    pub text: String,
}

/// What the views show: the open model and its results.
#[derive(Default)]
pub struct Data {
    pub model: Option<LoadedModel>,
    /// The feed point (source, in the deck's order) that SWR, the Smith
    /// chart, matching, the feedline and their exports show.
    pub feed: usize,
    pub over_ground: bool,
    pub sweep: Option<SweepResults>,
    /// The exact result at the slider frequency, once computed.
    pub refined: Option<FreqPoint>,
    /// The frequency slider (MHz).
    pub mhz: f64,
    pub messages: Vec<Message>,
    /// Bumped whenever the results so far become out of date.
    pub generation: u64,
    /// The generation the open model was loaded in (geometry belongs to it).
    pub model_generation: u64,
    /// The previous sweep, shown until the new one has results (so views
    /// don't blank while a variable is being dragged).
    pub previous_sweep: Option<SweepResults>,
    /// Variable values set from the Variables view (not yet saved).
    pub overrides: Vec<(String, f64)>,
    /// Set by a view that changed the model (variables, wires, deck text):
    /// `Some(true)` once the change is finished, `Some(false)` while a
    /// slider or wire end is still being dragged.
    pub edit_event: Option<bool>,
    /// Selected wires: indices of their GW lines in the deck.
    pub selection: Vec<usize>,
    /// Geometry check results for the current model.
    pub checks: Vec<Issue>,
    /// Highest frequency of the sweep (for segment-length advice).
    pub max_mhz: f64,
    /// Counts finished and unfinished edits, so views can tell the deck
    /// changed under them.
    pub edits: u64,
    /// Why nothing is being calculated, if the model can't be solved yet.
    pub idle: Option<&'static str>,
    /// Set by a view's "Undo" button.
    pub request_undo: bool,
    /// Snapshots of earlier designs to compare with (kept across files).
    pub comparisons: Vec<Comparison>,
    /// Snapshots taken so far, for names and colours.
    pub snapshot_count: usize,
    /// Measured sweeps to compare with (kept across files).
    pub measurements: Vec<Measurement>,
    /// A new model (name, deck text) a view wants opened, e.g. a template.
    pub new_deck: Option<(String, String)>,
    /// Something a view asked the app to do.
    pub request: Option<Request>,
    /// Where each view on screen was drawn last frame (for view screenshots).
    pub tab_rects: Vec<(Tab, egui::Rect)>,
    /// The curve selected in the 3D view (its name).
    pub selected_curve: Option<String>,
    /// A new variable being named (the prompt is open while this is set).
    pub new_variable: Option<crate::views::newvar::Request>,
    /// Wires coloured in the 3D view by the sensitivity check.
    pub wire_colours: Option<WireColours>,
}

/// Wires coloured by how much trimming them moves a figure.
pub struct WireColours {
    /// The model generation they belong to (stale after any edit).
    pub generation: u64,
    /// Bumped when the colours change, so the 3D scene is rebuilt.
    pub version: u64,
    /// What the colours show, in a few words.
    pub label: String,
    /// (tag, 0-1 of the largest effect).
    pub values: Vec<(u32, f32)>,
}

/// App actions a view can ask for.
pub enum Request {
    New,
    OpenDialog,
    Open(PathBuf),
    ShowTab(Tab),
    /// Pick a Touchstone file and add it to the measurements.
    ImportMeasurement,
    /// Sweep from one frequency to another (MHz).
    Sweep(f64, f64),
}

/// Whether a file is a measured sweep (Touchstone) rather than a deck.
fn is_drawing_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dxf"))
}

/// Whether a file is a measured sweep (Touchstone) rather than a deck.
fn is_sweep_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| straws_core::analysis::measured::ports_from_name(&n.to_string_lossy()))
        .is_some()
}

/// A sweep measured with a VNA, to compare with the model.
pub struct Measurement {
    pub name: String,
    pub colour: egui::Color32,
    pub visible: bool,
    /// In order of frequency, never empty.
    pub points: Vec<straws_core::analysis::measured::MeasuredPoint>,
    /// Measured at the radio end of the Feedline view's feed, which is
    /// taken off again before it's shown.
    pub through_feed: bool,
}

/// A design and its sweep, kept for comparison.
pub struct Comparison {
    pub name: String,
    pub colour: egui::Color32,
    pub visible: bool,
    pub sweep: SweepResults,
    /// The deck text, with variable values written in, to restore it.
    pub deck: String,
}

impl Data {
    /// The best result for the slider frequency: the exact one if it has
    /// arrived, otherwise the nearest sweep frequency.
    pub fn shown_point(&self) -> Option<&FreqPoint> {
        if let Some(r) = &self.refined
            && (r.mhz - self.mhz).abs() < 1e-9 * self.mhz.max(1.0)
        {
            return Some(r);
        }
        self.display_sweep()?.nearest(self.mhz)
    }

    /// The sweep to show: the current one once it has any results,
    /// otherwise the previous one.
    pub fn display_sweep(&self) -> Option<&SweepResults> {
        match &self.sweep {
            Some(s) if s.done() > 0 => Some(s),
            Some(s) => self.previous_sweep.as_ref().or(Some(s)),
            None => self.previous_sweep.as_ref(),
        }
    }

    /// What a results view shows while it has nothing: `no_model` with no
    /// model open, why nothing is calculated, or `waiting`.
    pub fn waiting_note<'a>(&self, no_model: &'a str, waiting: &'a str) -> &'a str {
        match (&self.model, self.idle) {
            (None, _) => no_model,
            (Some(_), Some(why)) => why,
            (Some(_), None) => waiting,
        }
    }

    pub fn message(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        // The same problem again (e.g. while typing in the deck) isn't news.
        if self
            .messages
            .last()
            .is_some_and(|m| m.level == level && m.text == text)
        {
            return;
        }
        self.messages.push(Message { level, text });
    }
}

/// Progress of the current sweep, for the status bar.
#[derive(Default)]
struct SweepStatus {
    started: Option<Instant>,
    finished: Option<Duration>,
    failed: bool,
}

pub struct App {
    settings: Settings,
    dock: DockState<Tab>,
    data: Data,
    views: ViewStates,
    gpu: Option<Gpu>,
    compute: Compute,
    status: SweepStatus,
    /// Sweep range being edited in the toolbar.
    plan_edit: FrequencyPlan,
    /// The length unit the checks' messages were written in.
    checks_unit: straws_core::units::Length,
    /// When the slider last moved, to start a refinement once it rests.
    slider_moved: Option<Instant>,
    /// Frequency a refinement was last started for.
    refine_requested: Option<f64>,
    /// Every finished change, for undo and redo.
    history: Vec<Snapshot>,
    history_pos: usize,
    /// `Data::edits` when the history was last brought up to date.
    recorded_edits: u64,
    /// A file dialog open on its own thread, so the window keeps drawing
    /// (and can be moved, minimised and restored) while it's up.
    dialog: Option<(DialogKind, Receiver<Option<PathBuf>>)>,
    /// Something that would lose unsaved changes, waiting for an answer.
    confirm: Option<Leave>,
    /// The user agreed to quit without saving.
    allow_close: bool,
    /// Help > Keyboard shortcuts or About is open.
    help: Option<HelpPage>,
    /// The editor for the user's own bands is open.
    editing_bands: bool,
    /// A CAD drawing being imported (its dialog is open).
    cad_import: Option<crate::views::cadimport::CadImport>,
    /// The zoom last applied, to notice Ctrl + / Ctrl - changing it.
    applied_zoom: f32,
    /// The view a requested screenshot is cropped to (None: whole window).
    crop: Option<Tab>,
    /// The window title last set.
    last_title: String,
    /// A screenshot was asked for and hasn't arrived yet.
    screenshot_pending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HelpPage {
    Shortcuts,
    About,
}

/// What a file dialog was opened for.
enum DialogKind {
    Open,
    /// Read a CAD drawing.
    ImportDrawing,
    /// Read a measured sweep.
    Import,
    /// Write these bytes to the chosen file.
    Export(Vec<u8>),
    /// Save, then carry on with what was waiting (if anything).
    Save(Option<Leave>),
}

/// Actions that replace or close the open model.
#[derive(Clone)]
enum Leave {
    New,
    /// A new, unsaved model from deck text (a template).
    Text(String, String),
    Open(PathBuf),
    OpenDialog,
    Quit,
}

/// Everything undo and redo restore: the deck, variable values and the
/// settings that change results or how they're shown.
#[derive(Clone, PartialEq)]
struct Snapshot {
    deck: String,
    overrides: Vec<(String, f64)>,
    plan: FrequencyPlan,
    z0: f64,
    component: usize,
    range_db: f64,
    length_unit: usize,
    wire_unit: usize,
}

fn default_dock() -> DockState<Tab> {
    let mut dock = DockState::new(vec![
        Tab::Model,
        Tab::Wires,
        Tab::Ground,
        Tab::Deck,
        Tab::Templates,
        Tab::NearField,
    ]);
    let tree = dock.main_surface_mut();
    let [left, right] = tree.split_right(NodeIndex::root(), 0.5, vec![Tab::Pattern]);
    let [_, _] = tree.split_below(
        left,
        0.62,
        vec![
            Tab::Swr,
            Tab::Smith,
            Tab::Feedline,
            Tab::FeedSystem,
            Tab::Matching,
            Tab::Checks,
            Tab::Messages,
        ],
    );
    let [_, _] = tree.split_below(
        right,
        0.62,
        vec![
            Tab::Cuts,
            Tab::Variables,
            Tab::Feeds,
            Tab::Receive,
            Tab::Optimiser,
            Tab::Compare,
        ],
    );
    dock
}

const SETTINGS_KEY: &str = "settings";
const DOCK_KEY: &str = "dock";

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Vec<PathBuf>) -> App {
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, SETTINGS_KEY))
            .unwrap_or_default();
        let mut dock: DockState<Tab> = cc
            .storage
            .and_then(|s| eframe::get_value(s, DOCK_KEY))
            .unwrap_or_else(default_dock);
        // A saved layout from an older version lacks the newer views: add
        // them to the first panel, keeping the rest of the layout.
        for tab in Tab::ALL {
            if dock.find_tab(&tab).is_none() {
                dock.push_to_first_leaf(tab);
            }
        }
        // Views that no longer exist.
        while let Some(at) = dock.find_tab(&Tab::Curves) {
            dock.remove_tab(at);
        }
        cc.egui_ctx.set_visuals(visuals(settings.dark));
        let mut app = App {
            plan_edit: FrequencyPlan::around(14.2),
            checks_unit: straws_core::units::Length::default(),
            settings,
            dock,
            data: Data::default(),
            views: ViewStates::default(),
            gpu: cc.wgpu_render_state.as_ref().map(Gpu::new),
            compute: Compute::new(cc.egui_ctx.clone()),
            status: SweepStatus::default(),
            slider_moved: None,
            refine_requested: None,
            history: Vec::new(),
            history_pos: 0,
            recorded_edits: 0,
            dialog: None,
            confirm: None,
            allow_close: false,
            help: None,
            editing_bands: false,
            cad_import: None,
            applied_zoom: 1.0,
            crop: None,
            last_title: String::new(),
            screenshot_pending: false,
        };
        if app.gpu.is_none() {
            app.data.message(
                Level::Error,
                "No GPU renderer is available, so the 3D views can't be drawn.",
            );
        }
        // The deck first, so the sweeps' message comes after it.
        let (sweeps, decks): (Vec<_>, Vec<_>) = open.into_iter().partition(|p| is_sweep_file(p));
        let (drawings, decks): (Vec<_>, Vec<_>) =
            decks.into_iter().partition(|p| is_drawing_file(p));
        if let Some(path) = decks.first() {
            app.open(path);
        }
        for path in &sweeps {
            app.import_measurement(path);
        }
        if let Some(path) = drawings.first() {
            app.import_drawing(path);
        }
        app
    }

    fn open(&mut self, path: &Path) {
        // Remember full paths, so "Open recent" works from anywhere.
        let path = &std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let model = match LoadedModel::load(path) {
            Ok(m) => m,
            Err(e) => {
                self.data.message(
                    Level::Error,
                    format!("Couldn't open {}: {e}", path.display()),
                );
                return;
            }
        };
        self.settings.recent.retain(|p| p != path);
        self.settings.recent.insert(0, path.to_path_buf());
        self.settings.recent.truncate(10);
        self.install(model);
        self.data
            .message(Level::Info, format!("Opened {}", path.display()));
    }

    /// File > New: an empty model to draw in.
    fn new_model(&mut self) {
        self.install(LoadedModel::blank());
        self.data.message(
            Level::Info,
            "New model: choose Draw in the 3D model view and click to place wire ends.",
        );
    }

    /// Makes `model` the open model and starts working on it.
    fn install(&mut self, model: LoadedModel) {
        self.compute.cancel_all();
        // Reopening the same file keeps the optimiser's set-up.
        let same_file = model.path.is_some()
            && self.data.model.as_ref().and_then(|m| m.path.as_ref()) == model.path.as_ref();
        let mut optimiser = std::mem::take(&mut self.views.optimiser);
        let generation = self.data.generation + 1;
        let messages = std::mem::take(&mut self.data.messages);
        let comparisons = std::mem::take(&mut self.data.comparisons);
        let measurements = std::mem::take(&mut self.data.measurements);
        let snapshot_count = self.data.snapshot_count;
        self.data = Data {
            generation,
            model_generation: generation,
            messages,
            comparisons,
            measurements,
            snapshot_count,
            ..Data::default()
        };
        self.views = ViewStates::default();
        if same_file {
            optimiser.forget_results();
            self.views.optimiser = optimiser;
        }
        self.history.clear();
        self.history_pos = 0;
        self.status = SweepStatus::default();
        self.refine_requested = None;
        match &model.numeric {
            Ok(n) => {
                for w in &n.warnings {
                    self.data.message(Level::Warning, w.to_string());
                }
                self.data.over_ground = has_ground(n);
                if model.has_structure() {
                    self.compute.geometry(self.data.generation, n);
                }
                self.plan_edit = FrequencyPlan::from_deck(n);
                self.data.mhz = n
                    .frequencies()
                    .first()
                    .copied()
                    .filter(|f| (self.plan_edit.start_mhz..=self.plan_edit.stop_mhz).contains(f))
                    .unwrap_or((self.plan_edit.start_mhz + self.plan_edit.stop_mhz) / 2.0);
            }
            Err(e) => self.data.message(
                Level::Error,
                format!("The deck can't be read: {e}. The Deck view shows the line."),
            ),
        }
        self.data.model = Some(model);
        self.start_sweep();
        self.history.push(self.snapshot());
        self.recorded_edits = self.data.edits;
    }

    /// The model is ready to solve: it has some structure and a source.
    fn runnable(&self) -> bool {
        self.data.model.as_ref().is_some_and(|m| {
            m.has_structure()
                && m.numeric
                    .as_ref()
                    .is_ok_and(|n| n.cards[n.control_start()..].iter().any(|c| c.is("EX")))
        })
    }

    fn idle_reason(&self) -> Option<&'static str> {
        let m = self.data.model.as_ref()?;
        if m.numeric.is_err() {
            Some("The deck can't be read: the Deck view shows where.")
        } else if !m.has_structure() {
            Some(
                "Nothing to calculate yet: draw a wire in the 3D model view (Draw), or click Add wire in the Wires view.",
            )
        } else if !self.runnable() {
            Some(
                "Nothing feeds the antenna yet: select a wire and click Feed this wire in the Wires view.",
            )
        } else {
            None
        }
    }

    /// Re-runs the geometry checks (after the geometry or the sweep range
    /// changed).
    fn refresh_checks(&mut self) {
        self.data.max_mhz = self.plan_edit.stop_mhz;
        let Some(m) = &self.data.model else {
            self.data.checks.clear();
            return;
        };
        // Distances in the chosen unit.
        let unit = self.settings.length();
        let mhz = self.plan_edit.stop_mhz;
        self.checks_unit = unit;
        let length = |m: f64| {
            format!(
                "{} {}",
                crate::views::wires::show_length(m, unit, mhz),
                unit.symbol()
            )
        };
        self.data.checks = match &m.numeric {
            Ok(n) => {
                let mut issues = check_in(&m.segments, n, mhz, self.data.over_ground, &length);
                issues.extend(straws_core::analysis::checks::patch_checks(
                    &m.patches,
                    mhz,
                    self.data.over_ground,
                ));
                issues
            }
            Err(_) => Vec::new(),
        };
    }

    fn start_sweep(&mut self) {
        self.refresh_checks();
        let Some(Ok(numeric)) = self.data.model.as_ref().map(|m| &m.numeric) else {
            return;
        };
        let numeric = numeric.clone();
        self.data.generation += 1;
        self.data.idle = self.idle_reason();
        if !self.runnable() {
            // Nothing to solve yet (no wires, or nothing feeding them).
            self.compute.cancel_all();
            self.data.sweep = None;
            self.data.previous_sweep = None;
            self.data.refined = None;
            self.status = SweepStatus::default();
            return;
        }
        self.data.previous_sweep = self.data.sweep.take();
        self.data.sweep = Some(SweepResults::new(self.plan_edit));
        self.data.refined = None;
        self.refine_requested = None;
        self.data.mhz = self
            .data
            .mhz
            .clamp(self.plan_edit.start_mhz, self.plan_edit.stop_mhz);
        self.status = SweepStatus {
            started: Some(Instant::now()),
            ..SweepStatus::default()
        };
        let buried = self.buried_setup();
        self.compute.sweep(
            self.data.generation,
            &numeric,
            self.plan_edit,
            SWEEP_PATTERN_STEP,
            buried,
        );
        // Refine the slider frequency too, as if the slider had just stopped.
        self.slider_moved = Some(Instant::now());
    }

    fn handle_updates(&mut self) {
        let generation = self.data.generation;
        for update in self.compute.poll() {
            match update {
                Update::Geometry {
                    generation: g,
                    result,
                } if g == self.data.model_generation => {
                    // Geometry stays valid across re-sweeps of the same model.
                    match result {
                        Ok((segments, patches)) => {
                            if let Some(model) = &mut self.data.model {
                                model.segments = segments;
                                model.patches = patches;
                                model.geometry_version += 1;
                                model.geometry_error = None;
                            }
                            self.refresh_checks();
                        }
                        Err(e) => {
                            let text = format!("The engine couldn't read the geometry. {e}");
                            if let Some(m) = &mut self.data.model {
                                m.segments.clear();
                                m.patches.clear();
                                m.geometry_version += 1;
                                m.geometry_error = Some(text.clone());
                            }
                            self.data.message(Level::Error, text);
                        }
                    }
                }
                Update::SweepPoint {
                    generation: g,
                    index,
                    point,
                } if g == generation => {
                    if let Some(s) = &mut self.data.sweep {
                        s.insert(index, point);
                    }
                }
                Update::SweepDone {
                    generation: g,
                    elapsed,
                    messages,
                } if g == generation => {
                    self.status.finished = Some(elapsed);
                    for m in messages {
                        self.data.message(Level::Warning, format!("Engine: {m}"));
                    }
                }
                Update::SweepFailed {
                    generation: g,
                    message,
                } if g == generation => {
                    self.status.failed = true;
                    self.data
                        .message(Level::Error, format!("The sweep stopped: {message}"));
                }
                Update::Refined {
                    generation: g,
                    point,
                } if g == generation => {
                    self.data.refined = Some(point);
                }
                Update::Note {
                    generation: g,
                    message,
                } if g == generation => {
                    self.data.message(Level::Warning, message);
                }
                Update::RefineFailed {
                    generation: g,
                    message,
                } if g == generation => {
                    self.data.message(
                        Level::Error,
                        format!("Couldn't compute this frequency: {message}"),
                    );
                }
                _ => {} // From work that has since been replaced.
            }
        }
    }

    /// Starts the exact, finer calculation once the slider has rested.
    fn maybe_refine(&mut self, ctx: &egui::Context) {
        let Some(moved) = self.slider_moved else {
            return;
        };
        let rested = moved.elapsed();
        if rested < REST_BEFORE_REFINE {
            ctx.request_repaint_after(REST_BEFORE_REFINE - rested);
            return;
        }
        let mhz = self.data.mhz;
        let have = self.data.refined.as_ref().is_some_and(|r| r.mhz == mhz);
        if have || self.refine_requested == Some(mhz) || !self.runnable() {
            return;
        }
        if let Some(Ok(n)) = self.data.model.as_ref().map(|m| &m.numeric) {
            let n = n.clone();
            let buried = self.buried_setup();
            self.compute
                .refine(self.data.generation, &n, mhz, REFINED_PATTERN_STEP, buried);
            self.refine_requested = Some(mhz);
        }
    }

    /// What the estimate for radials on or under the ground needs, if the
    /// model has any (and a real ground to estimate them over).
    fn buried_setup(&self) -> Option<straws_core::analysis::buried::Setup> {
        use straws_core::analysis::buried::{Need, need};
        let m = self.data.model.as_ref()?;
        match need(&m.deck, m.numeric.as_ref().ok()?) {
            Need::Estimate(s) => Some(s),
            _ => None,
        }
    }

    /// Re-expands the deck with the current variable values and recalculates.
    /// While a slider is being dragged (`finished` false) only the slider
    /// frequency is recomputed, quickly; a finished change reruns the sweep.
    fn apply_edits(&mut self, finished: bool) {
        let Some(model) = &mut self.data.model else {
            return;
        };
        model.refresh(&self.data.overrides);
        // Radials that have moved onto, under or off the ground switch
        // between NEC wires and the estimate.
        let mut notes = Vec::new();
        if let Ok(n) = &model.numeric {
            let n = n.clone();
            notes = model.deck.sync_radials(&n, self.plan_edit.stop_mhz);
            notes.extend(model.deck.sync_grids(&n, self.plan_edit.stop_mhz));
            if finished
                && let Some(note) = model
                    .deck
                    .ground_for_low_radials(&n, self.plan_edit.stop_mhz)
            {
                notes.push(note);
            }
            if !notes.is_empty() {
                model.refresh(&self.data.overrides);
            }
            if let straws_core::analysis::buried::Need::Note(note) =
                straws_core::analysis::buried::need(&model.deck, &n)
                && finished
            {
                notes.push(note);
            }
        }
        let lines = model.deck.lines.len();
        self.data.selection.retain(|&l| l < lines);
        self.data.edits += 1;
        self.data.generation += 1;
        self.data.model_generation = self.data.generation;
        let numeric = model.numeric.clone();
        let structure = model.has_structure();
        for note in notes {
            self.data.message(Level::Info, note);
        }
        if !structure && let Some(m) = &mut self.data.model {
            m.segments.clear();
            m.geometry_version += 1;
        }
        match numeric {
            Ok(n) => {
                self.data.over_ground = has_ground(&n);
                if structure {
                    self.compute.geometry(self.data.generation, &n);
                }
                if finished {
                    self.start_sweep();
                } else if self.runnable() {
                    let mhz = self.data.mhz;
                    let buried = self.buried_setup();
                    self.compute
                        .refine(self.data.generation, &n, mhz, SWEEP_PATTERN_STEP, buried);
                    self.refine_requested = Some(mhz);
                }
            }
            Err(e) => {
                // Shown in the Deck view and the Checks view; while typing
                // or dragging, don't flood the messages.
                if finished {
                    let text = format!("The deck can't be run as it stands: {e}");
                    self.data.message(Level::Error, text);
                }
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        let s = &self.settings;
        Snapshot {
            deck: self
                .data
                .model
                .as_ref()
                .map(|m| m.deck.to_text())
                .unwrap_or_default(),
            overrides: self.data.overrides.clone(),
            plan: self.plan_edit,
            z0: s.z0,
            component: s.component,
            range_db: s.range_db,
            length_unit: s.length_unit,
            wire_unit: s.wire_unit,
        }
    }

    /// Adds the current state to the undo history if it has changed and
    /// nothing is still being dragged or typed.
    fn record_history(&mut self, ctx: &egui::Context) {
        if self.data.model.is_none() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let Some(last) = self.history.get(self.history_pos) else {
            return;
        };
        let s = &self.settings;
        let settings_same = last.plan == self.plan_edit
            && last.z0 == s.z0
            && last.component == s.component
            && last.range_db == s.range_db
            && last.length_unit == s.length_unit
            && last.wire_unit == s.wire_unit
            && last.overrides == self.data.overrides;
        if settings_same && self.recorded_edits == self.data.edits {
            return;
        }
        self.recorded_edits = self.data.edits;
        let now = self.snapshot();
        if &now != last {
            self.history.truncate(self.history_pos + 1);
            self.history.push(now);
            self.history_pos = self.history.len() - 1;
            // Keep memory in check on long sessions.
            if self.history.len() > 500 {
                self.history.remove(0);
                self.history_pos -= 1;
            }
        }
    }

    /// Steps back (-1) or forward (+1) through the history.
    fn undo(&mut self, step: isize) {
        let target = self.history_pos as isize + step;
        if target < 0 || target as usize >= self.history.len() {
            return;
        }
        let current = self.snapshot();
        self.history_pos = target as usize;
        let snap = self.history[self.history_pos].clone();
        let s = &mut self.settings;
        s.z0 = snap.z0;
        s.component = snap.component;
        s.range_db = snap.range_db;
        s.length_unit = snap.length_unit;
        s.wire_unit = snap.wire_unit;
        self.plan_edit = snap.plan;
        if snap.deck != current.deck || snap.overrides != current.overrides {
            if let Some(m) = &mut self.data.model {
                m.deck = Deck::parse(&snap.deck);
            }
            self.data.overrides = snap.overrides.clone();
            self.apply_edits(true);
        } else if snap.plan != current.plan {
            self.start_sweep();
        }
        self.recorded_edits = self.data.edits;
    }

    /// Whether the model differs from the file it came from.
    fn changed(&self) -> bool {
        self.data
            .model
            .as_ref()
            .is_some_and(|m| !self.data.overrides.is_empty() || m.deck.to_text() != m.saved_text)
    }

    /// Writes the deck (with the Variables values written into its SY
    /// lines) to its file, asking for a name if it hasn't one.
    fn save_deck(&mut self, ask: bool) {
        self.save_then(ask, None);
    }

    /// Saves (asking for a name if needed), then does `then`. Returns false
    /// if a dialog was opened instead, which finishes the job later.
    fn save_then(&mut self, ask: bool, then: Option<Leave>) {
        let Some(model) = &self.data.model else {
            return;
        };
        match (&model.path, ask) {
            (Some(p), false) => {
                let p = p.clone();
                if self.write_deck(&p)
                    && let Some(t) = then
                {
                    self.perform(t);
                }
            }
            _ => {
                if self.dialog.is_some() {
                    return;
                }
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("NEC decks", &["nec", "NEC"])
                    .set_file_name(format!("{}.nec", model.name.trim_end_matches(".nec")));
                if let Some(dir) = model
                    .path
                    .as_ref()
                    .or(self.settings.recent.first())
                    .and_then(|p| p.parent())
                {
                    dialog = dialog.set_directory(dir);
                }
                self.run_dialog(DialogKind::Save(then), move || dialog.save_file());
            }
        }
    }

    /// Runs a blocking file dialog on its own thread.
    fn run_dialog(
        &mut self,
        kind: DialogKind,
        show: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.compute.ctx();
        std::thread::spawn(move || {
            let _ = tx.send(show());
            ctx.request_repaint();
        });
        self.dialog = Some((kind, rx));
    }

    /// Picks up a file dialog's answer once it's closed.
    fn poll_dialog(&mut self) {
        let Some((_, rx)) = &self.dialog else { return };
        let answer = match rx.try_recv() {
            Ok(a) => a,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        };
        let Some((kind, _)) = self.dialog.take() else {
            return;
        };
        match (kind, answer) {
            (DialogKind::Open, Some(p)) => self.open(&p),
            (DialogKind::Import, Some(p)) => self.import_measurement(&p),
            (DialogKind::ImportDrawing, Some(p)) => self.import_drawing(&p),
            (DialogKind::Export(bytes), Some(p)) => match std::fs::write(&p, bytes) {
                Ok(()) => self
                    .data
                    .message(Level::Info, format!("Saved {}", p.display())),
                Err(e) => self
                    .data
                    .message(Level::Error, format!("Couldn't save {}: {e}", p.display())),
            },
            (DialogKind::Save(then), Some(p)) => {
                if self.write_deck(&p)
                    && let Some(t) = then
                {
                    self.perform(t);
                }
            }
            _ => {} // Cancelled.
        }
    }

    /// Does `action` now if nothing unsaved would be lost, otherwise asks.
    fn leave(&mut self, action: Leave) {
        if self.changed() {
            self.confirm = Some(action);
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: Leave) {
        match action {
            Leave::New => self.new_model(),
            Leave::Text(name, text) => {
                self.install(LoadedModel::from_text(&name, &text));
                self.data.message(
                    Level::Info,
                    format!("New {name} from a template: tune it in Variables or the Optimiser, and save it with Ctrl+S."),
                );
            }
            Leave::Open(p) => self.open(&p),
            Leave::OpenDialog => self.open_dialog(),
            Leave::Quit => {
                self.allow_close = true;
                self.compute
                    .ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// The "save changes?" question, inside the main window.
    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm.clone() else {
            return;
        };
        let name = self
            .data
            .model
            .as_ref()
            .map_or_else(String::new, |m| m.name.clone());
        let mut choice = None;
        egui::Modal::new(egui::Id::new("save changes?")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Save changes?");
            ui.label(format!(
                "{name} has changes that aren't saved. Save them first?"
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(0);
                }
                if ui.button("Don't save").clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(2);
                }
            });
        });
        match choice {
            Some(0) => {
                self.confirm = None;
                self.save_then(false, Some(action));
            }
            Some(1) => {
                self.confirm = None;
                self.perform(action);
            }
            Some(_) => self.confirm = None,
            None => {}
        }
    }

    /// Writes the deck to `path`. Returns whether it worked.
    fn write_deck(&mut self, path: &Path) -> bool {
        let path = path.to_path_buf();
        let Some(model) = &self.data.model else {
            return false;
        };
        let mut deck = model.deck.clone();
        for (name, value) in &self.data.overrides {
            deck.set_variable(name, *value);
        }
        // The toolbar's sweep goes into the FR card, so the file reopens
        // (here and in 4nec2) on the same frequencies.
        let sweep_written = straws_core::analysis::plan::write_sweep(&mut deck, &self.plan_edit);
        // 4nec2 can't model radials on or under the ground: give it NEC's
        // own (rough) radial screen in their place.
        let stand_in = model.numeric.as_ref().ok().and_then(|n| {
            straws_core::deck::radials::radial_numbers(&model.deck, n)
                .into_iter()
                .find(|r| r.buried)
                .map(|r| straws_core::deck::ground::RadialScreen {
                    count: r.count as i64,
                    length: r.length,
                    wire_radius: r.wire_radius,
                })
        });
        deck.set_stand_in_screen(stand_in);
        let text = deck.to_text();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                if let Some(m) = &mut self.data.model {
                    m.deck = deck;
                    m.saved_text = text;
                    m.name = path
                        .file_name()
                        .map_or_else(|| "deck".into(), |n| n.to_string_lossy().into_owned());
                    m.path = Some(path.clone());
                }
                // The values now live in the deck itself.
                self.data.overrides.clear();
                self.apply_edits(true);
                self.settings.recent.retain(|p| p != &path);
                self.settings.recent.insert(0, path.clone());
                self.settings.recent.truncate(10);
                self.data
                    .message(Level::Info, format!("Saved {}", path.display()));
                if sweep_written {
                    let p = self.plan_edit;
                    self.data.message(
                        Level::Info,
                        format!(
                            "The FR card now holds the sweep: {} to {} MHz, {} points.",
                            straws_core::deck::write::format_real_digits(p.start_mhz, 7),
                            straws_core::deck::write::format_real_digits(p.stop_mhz, 7),
                            p.points
                        ),
                    );
                }
                true
            }
            Err(e) => {
                self.data.message(
                    Level::Error,
                    format!("Couldn't save {}: {e}", path.display()),
                );
                false
            }
        }
    }

    /// Asks where to save `bytes`, suggesting `name`.
    fn export(&mut self, bytes: Vec<u8>, name: &str, filter: &str, ext: &str) {
        if self.dialog.is_some() {
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .add_filter(filter, &[ext])
            .set_file_name(name);
        if let Some(dir) = self
            .data
            .model
            .as_ref()
            .and_then(|m| m.path.as_ref())
            .or(self.settings.recent.first())
            .and_then(|p| p.parent())
        {
            dialog = dialog.set_directory(dir);
        }
        self.run_dialog(DialogKind::Export(bytes), move || dialog.save_file());
    }

    /// The model's name without ".nec", for file names.
    fn stem(&self) -> String {
        self.data.model.as_ref().map_or_else(
            || "antenna".to_string(),
            |m| {
                m.name
                    .trim_end_matches(".nec")
                    .trim_end_matches(".NEC")
                    .to_string()
            },
        )
    }

    /// File > Export.
    fn export_menu(&mut self, ui: &mut egui::Ui) {
        use straws_core::export;
        let stem = self.stem();
        let z0 = self.settings.z0;
        let sweep = self.data.display_sweep().cloned();
        let point = self.data.shown_point().cloned();
        let has_sweep = sweep.as_ref().is_some_and(|s| s.done() > 0);
        if ui
            .add_enabled(has_sweep, egui::Button::new("SWR and impedance (CSV)…"))
            .on_hover_text(
                "Every sweep frequency: R, X, SWR, return loss, reflection coefficient, peak gain",
            )
            .clicked()
            && let Some(s) = &sweep
        {
            ui.close();
            self.export(
                export::sweep_csv(s, z0, self.data.feed).into_bytes(),
                &format!("{stem} sweep.csv"),
                "CSV",
                "csv",
            );
        }
        let pattern = point
            .as_ref()
            .and_then(|p| p.pattern.as_ref().map(|g| (p.mhz, g.clone())));
        if ui
            .add_enabled(
                pattern.is_some(),
                egui::Button::new("Pattern at this frequency (CSV)…"),
            )
            .on_hover_text("Gain in every direction, every field component")
            .clicked()
            && let Some((mhz, g)) = &pattern
        {
            ui.close();
            self.export(
                export::pattern_csv(g, *mhz).into_bytes(),
                &format!("{stem} pattern {mhz:.3} MHz.csv"),
                "CSV",
                "csv",
            );
        }
        let has_currents = point.as_ref().is_some_and(|p| !p.currents.is_empty());
        if ui
            .add_enabled(
                has_currents,
                egui::Button::new("Currents at this frequency (CSV)…"),
            )
            .on_hover_text("Every segment's current (RMS, for 100 W) and phase")
            .clicked()
            && let (Some(p), Some(m)) = (&point, &self.data.model)
        {
            let text = export::currents_csv(p, &m.segments, 100.0);
            ui.close();
            self.export(
                text.into_bytes(),
                &format!("{stem} currents {:.3} MHz.csv", p.mhz),
                "CSV",
                "csv",
            );
        }
        if ui
            .add_enabled(has_sweep, egui::Button::new("Touchstone (.s1p)…"))
            .on_hover_text("S11 across the sweep, for circuit simulators and VNA software")
            .clicked()
            && let Some(s) = &sweep
        {
            ui.close();
            self.export(
                export::touchstone(s, z0, &stem, self.data.feed).into_bytes(),
                &format!("{stem}.s1p"),
                "Touchstone",
                "s1p",
            );
        }
        let has_wires = self
            .data
            .model
            .as_ref()
            .is_some_and(|m| !m.segments.is_empty());
        if ui
            .add_enabled(has_wires, egui::Button::new("Wires for CAD (DXF)…"))
            .on_hover_text("Every segment as a 3D line in metres, one layer per wire (FreeCAD, LibreCAD and others)")
            .clicked()
            && let Some(m) = &self.data.model
        {
            let text = export::dxf(&m.visible_segments().copied().collect::<Vec<_>>());
            ui.close();
            self.export(text.into_bytes(), &format!("{stem}.dxf"), "DXF", "dxf");
        }
        if ui
            .add_enabled(has_wires, egui::Button::new("Wires as tubes (STL)…"))
            .on_hover_text("Every segment as a solid tube at its real diameter, in millimetres; FreeCAD can turn it into a solid and save STEP")
            .clicked()
            && let Some(m) = &self.data.model
        {
            let text = export::stl(&m.visible_segments().copied().collect::<Vec<_>>(), &stem);
            ui.close();
            self.export(text.into_bytes(), &format!("{stem}.stl"), "STL", "stl");
        }
        ui.separator();
        if ui
            .add_enabled(self.data.model.is_some(), egui::Button::new("Report (web page)…"))
            .on_hover_text("Key results, plots, wires and the deck on one page; print it to PDF from your browser")
            .clicked()
            && let Some(m) = &self.data.model
        {
            let comp = straws_core::analysis::pattern::Component::Total;
            let cuts = point.as_ref().and_then(|p| p.pattern.as_ref()).map(|g| {
                let (_, t, ph) = g.max(comp);
                (g.elevation_cut(comp, ph), g.azimuth_cut(comp, 90.0 - t))
            });
            let mut deck = m.deck.clone();
            for (n, v) in &self.data.overrides {
                deck.set_variable(n, *v);
            }
            let html = export::report_html(&export::ReportInput {
                name: &stem,
                deck_text: &deck.to_text(),
                sweep: sweep.as_ref(),
                point: point.as_ref(),
                wires: &m.wires,
                z0,
                cuts,
                over_ground: self.data.over_ground,
                feed: self.data.feed,
            });
            ui.close();
            self.export(html.into_bytes(), &format!("{stem} report.html"), "Web page", "html");
        }
        if ui
            .add_enabled(has_wires, egui::Button::new("Build sheet (web page)…"))
            .on_hover_text("A cut list, the stock to buy and where the elements go along a boom, worked out from the model; print it to PDF from your browser")
            .clicked()
            && let Some(m) = &self.data.model
            && let Ok(n) = &m.numeric
        {
            let shield: Vec<i64> = straws_core::deck::shield::shield_tag(&m.deck)
                .into_iter()
                .collect();
            let sheet = straws_core::analysis::build::build_sheet(&m.segments, &m.deck, n, &shield);
            let html = export::build_sheet_html(&stem, &sheet, self.settings.length(), self.data.mhz);
            ui.close();
            self.export(html.into_bytes(), &format!("{stem} build sheet.html"), "Web page", "html");
        }
        ui.separator();
        let sweep_ok = sweep.as_ref().filter(|s| s.done() > 1);
        if ui
            .add_enabled(sweep_ok.is_some(), egui::Button::new("SWR plot (SVG)…"))
            .on_hover_text("A scalable image of SWR across the sweep")
            .clicked()
            && let Some(s) = sweep_ok
        {
            ui.close();
            self.export(
                export::swr_svg(s, z0, self.data.feed).into_bytes(),
                &format!("{stem} SWR.svg"),
                "SVG image",
                "svg",
            );
        }
        if ui
            .add_enabled(sweep_ok.is_some(), egui::Button::new("Smith chart (SVG)…"))
            .clicked()
            && let Some(s) = sweep_ok
        {
            let mark = Some(self.data.mhz);
            ui.close();
            self.export(
                export::smith_svg(s, z0, mark, self.data.feed).into_bytes(),
                &format!("{stem} Smith chart.svg"),
                "SVG image",
                "svg",
            );
        }
        if ui
            .add_enabled(pattern.is_some(), egui::Button::new("Pattern cuts (SVG)…"))
            .on_hover_text("Elevation and azimuth cuts through the peak at this frequency")
            .clicked()
            && let Some((mhz, g)) = &pattern
        {
            let comp = self.settings.component();
            let (_, t, ph) = g.max(comp);
            let svg = export::cuts_svg(
                &g.elevation_cut(comp, ph),
                &format!("Elevation at azimuth {ph:.0}°, {mhz:.3} MHz"),
                &g.azimuth_cut(comp, 90.0 - t),
                &format!("Azimuth at elevation {:.0}°", 90.0 - t),
                self.data.over_ground,
            );
            ui.close();
            self.export(
                svg.into_bytes(),
                &format!("{stem} pattern cuts.svg"),
                "SVG image",
                "svg",
            );
        }
        ui.separator();
        ui.menu_button("Screenshot of one view (PNG)", |ui| {
            let shown: Vec<Tab> = self.data.tab_rects.iter().map(|(t, _)| *t).collect();
            for tab in shown {
                if ui.button(tab.title()).clicked() {
                    ui.close();
                    self.crop = Some(tab);
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                    self.screenshot_pending = true;
                }
            }
        })
        .response
        .on_hover_text("Just one of the views on screen");
        if ui
            .button("Screenshot of the window (PNG)…")
            .on_hover_text("The whole window as it looks now")
            .clicked()
        {
            ui.close();
            self.crop = None;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.screenshot_pending = true;
        }
    }

    /// Saves a screenshot that has arrived.
    fn take_screenshot(&mut self, ctx: &egui::Context) {
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = image else {
            // The picture arrives a frame after it's taken: keep drawing
            // until it does (the window doesn't redraw by itself when idle).
            if self.screenshot_pending {
                ctx.request_repaint();
            }
            return;
        };
        self.screenshot_pending = false;
        let (full_w, full_h) = (image.size[0], image.size[1]);
        // Crop to one view if asked: screen points to image pixels.
        let (mut x0, mut y0, mut w, mut h) = (0, 0, full_w, full_h);
        let mut name = self.stem();
        if let Some(tab) = self.crop.take()
            && let Some((_, rect)) = self.data.tab_rects.iter().find(|(t, _)| *t == tab)
        {
            let scale = full_w as f32 / ctx.content_rect().width().max(1.0);
            x0 = ((rect.left() * scale).max(0.0) as usize).min(full_w);
            y0 = ((rect.top() * scale).max(0.0) as usize).min(full_h);
            w = ((rect.width() * scale) as usize).min(full_w - x0);
            h = ((rect.height() * scale) as usize).min(full_h - y0);
            name = format!("{name} {}", tab.title());
        }
        if w == 0 || h == 0 {
            return;
        }
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, w as u32, h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let rgba: Vec<u8> = (y0..y0 + h)
                .flat_map(|y| image.pixels[y * full_w + x0..y * full_w + x0 + w].iter())
                .flat_map(|c| c.to_srgba_unmultiplied())
                .collect();
            if let Ok(mut writer) = enc.write_header() {
                let _ = writer.write_image_data(&rgba);
            }
        }
        self.export(bytes, &format!("{name}.png"), "PNG image", "png");
    }

    /// Deletes the selected wires (or the selected curve).
    fn delete_selection(&mut self) {
        if let Some(name) = self.data.selected_curve.clone()
            && crate::views::radials::is_set(&self.data, &name)
        {
            crate::views::radials::delete(&mut self.data, &name);
            return;
        }
        if let Some(name) = self.data.selected_curve.clone()
            && crate::views::grids::is_grid(&self.data, &name)
        {
            crate::views::grids::delete(&mut self.data, &name);
            return;
        }
        if let Some(name) = self.data.selected_curve.clone()
            && crate::views::plates::is_plate(&self.data, &name)
        {
            crate::views::plates::delete(&mut self.data, &name);
            return;
        }
        if self.data.selection.is_empty()
            && let Some(name) = self.data.selected_curve.clone()
        {
            crate::views::curves::delete(&mut self.data, &name);
            return;
        }
        let Some(m) = &mut self.data.model else {
            return;
        };
        if self.data.selection.is_empty() {
            return;
        }
        m.deck.remove_lines(&self.data.selection);
        self.data.selection.clear();
        self.apply_edits(true);
    }

    fn set_frequency(&mut self, mhz: f64) {
        if mhz != self.data.mhz {
            self.data.mhz = mhz;
            self.slider_moved = Some(Instant::now());
            self.compute.cancel_refine();
            self.refine_requested = None;
        }
    }

    fn import_dialog(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Touchstone (VNA sweeps)", &["s1p", "s2p", "S1P", "S2P"])
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.settings.recent.first().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        self.run_dialog(DialogKind::Import, move || dialog.pick_file());
    }

    /// Reads a Touchstone file into the measurements, or says why it can't.
    fn import_measurement(&mut self, path: &std::path::Path) {
        use straws_core::analysis::measured;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Measurement".into());
        let file = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let read = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                let text = String::from_utf8_lossy(&bytes);
                measured::read_touchstone(&text, measured::ports_from_name(&file).unwrap_or(1))
            });
        match read {
            Ok(points) => {
                let (a, b) = (points[0].mhz, points[points.len() - 1].mhz);
                self.data.message(
                    Level::Info,
                    format!(
                        "Imported {file}: {} points, {a:.3} to {b:.3} MHz. It shows on the SWR \
                         plot and Smith chart within the sweep; the Compare view has its settings.",
                        points.len()
                    ),
                );
                let colours = crate::views::compare::MEASURED_COLOURS;
                let colour = colours[self.data.measurements.len() % colours.len()];
                self.data.measurements.push(Measurement {
                    name,
                    colour,
                    visible: true,
                    points,
                    through_feed: false,
                });
                if let Some(path) = self.dock.find_tab(&Tab::Compare) {
                    let _ = self.dock.set_active_tab(path);
                }
            }
            Err(e) => self.data.message(
                Level::Error,
                format!("Couldn't read {file} as a VNA sweep: {e}"),
            ),
        }
    }

    fn import_drawing_dialog(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .add_filter("DXF drawings", &["dxf", "DXF"])
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.settings.recent.first().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        self.run_dialog(DialogKind::ImportDrawing, move || dialog.pick_file());
    }

    /// Reads a DXF drawing and opens the import dialog for it.
    fn import_drawing(&mut self, path: &std::path::Path) {
        let file = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let read = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                crate::views::cadimport::CadImport::new(
                    file.clone(),
                    String::from_utf8_lossy(&bytes).into_owned(),
                    // The sweep's top, as the toolbar has it (also with no model open).
                    self.plan_edit.stop_mhz.max(0.001),
                    self.data.model.is_some(),
                )
            });
        match read {
            Ok(import) => self.cad_import = Some(import),
            Err(e) => self.data.message(
                Level::Error,
                format!("Couldn't import {file} as a drawing: {e}"),
            ),
        }
    }

    /// Shows the drawing import dialog while one is being set up, and does
    /// the import when asked.
    fn drawing_dialog(&mut self, ctx: &egui::Context) {
        use crate::views::cadimport::Outcome;
        let Some(import) = &mut self.cad_import else {
            return;
        };
        match import.ui(ctx, self.data.model.is_some()) {
            Outcome::Open => {}
            Outcome::Cancel => self.cad_import = None,
            Outcome::Import {
                wires,
                radius,
                mhz,
                new_model,
            } => {
                let file = import.file.clone();
                self.cad_import = None;
                let first = if new_model {
                    1
                } else {
                    self.data.model.as_ref().map_or(1, |m| m.next_tag())
                };
                let cards: Vec<_> = wires
                    .iter()
                    .enumerate()
                    .map(|(k, w)| {
                        let length = (0..3)
                            .map(|i| (w.b[i] - w.a[i]).powi(2))
                            .sum::<f64>()
                            .sqrt();
                        straws_core::deck::edit::NewWire {
                            tag: first + k as i64,
                            segments: crate::views::wires::auto_segments(length, mhz),
                            a: w.a,
                            b: w.b,
                            radius,
                            comment: Some(w.layer.clone()),
                        }
                    })
                    .collect();
                let message = format!(
                    "Imported {} wires from {file}. To run it, feed a wire: select it, then Feed this wire (Wires view, or right-click it in the 3D view).",
                    cards.len()
                );
                if new_model {
                    let mut deck = straws_core::deck::Deck::parse(&format!(
                        "CM Imported from {file}\nCE\nGE 0\nGN -1\nFR 0 1 0 0 {mhz} 0\nEN\n"
                    ));
                    deck.add_wires(&cards);
                    let name = std::path::Path::new(&file)
                        .file_stem()
                        .map_or("Imported".into(), |s| s.to_string_lossy().into_owned());
                    self.leave(Leave::Text(name, deck.to_text()));
                } else if let Some(m) = &mut self.data.model {
                    m.deck.add_wires(&cards);
                    self.data.edit_event = Some(true);
                }
                self.data.message(Level::Info, message);
            }
        }
    }

    fn open_dialog(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        let mut dialog = rfd::FileDialog::new().add_filter("NEC decks", &["nec", "NEC"]);
        if let Some(dir) = self.settings.recent.first().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        self.run_dialog(DialogKind::Open, move || dialog.pick_file());
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui
                    .add(egui::Button::new("New").shortcut_text("Ctrl+N"))
                    .on_hover_text("Start an empty model and draw it")
                    .clicked()
                {
                    ui.close();
                    self.leave(Leave::New);
                }
                if ui
                    .button("New from template…")
                    .on_hover_text("Dipole, Yagi, Moxon, quad, LPDA, verticals and more")
                    .clicked()
                {
                    ui.close();
                    if let Some(path) = self.dock.find_tab(&Tab::Templates) {
                        let _ = self.dock.set_active_tab(path);
                    }
                }
                if ui
                    .add(egui::Button::new("Open…").shortcut_text("Ctrl+O"))
                    .clicked()
                {
                    ui.close();
                    self.leave(Leave::OpenDialog);
                }
                let reload = self.data.model.as_ref().and_then(|m| m.path.clone());
                if ui
                    .add_enabled(
                        reload.is_some(),
                        egui::Button::new("Reload").shortcut_text("Ctrl+R"),
                    )
                    .clicked()
                {
                    ui.close();
                    if let Some(p) = reload {
                        self.leave(Leave::Open(p));
                    }
                }
                ui.menu_button("Open recent", |ui| {
                    if self.settings.recent.is_empty() {
                        ui.label("Nothing yet");
                    }
                    let mut chosen = None;
                    for p in &self.settings.recent {
                        if ui.button(p.display().to_string()).clicked() {
                            chosen = Some(p.clone());
                        }
                    }
                    if let Some(p) = chosen {
                        ui.close();
                        self.leave(Leave::Open(p));
                    }
                });
                let has_model = self.data.model.is_some();
                if ui
                    .add_enabled(has_model, egui::Button::new("Save").shortcut_text("Ctrl+S"))
                    .on_hover_text("Write the model, with the Variables values, to its deck file")
                    .clicked()
                {
                    ui.close();
                    self.save_deck(false);
                }
                if ui
                    .add_enabled(
                        has_model,
                        egui::Button::new("Save as…").shortcut_text("Ctrl+Shift+S"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.save_deck(true);
                }
                ui.separator();
                ui.menu_button("Export", |ui| self.export_menu(ui));
                if ui
                    .button("Import a VNA sweep…")
                    .on_hover_text("A Touchstone file (.s1p or .s2p) to compare with the model")
                    .clicked()
                {
                    ui.close();
                    self.import_dialog();
                }
                if ui
                    .button("Import a CAD drawing (DXF)…")
                    .on_hover_text(
                        "Lines, arcs, circles and splines from a wire-frame drawing become wires",
                    )
                    .clicked()
                {
                    ui.close();
                    self.import_drawing_dialog();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    self.leave(Leave::Quit);
                }
            });
            ui.menu_button("Edit", |ui| {
                let (can_undo, can_redo) = (
                    self.history_pos > 0,
                    self.history_pos + 1 < self.history.len(),
                );
                if ui
                    .add_enabled(can_undo, egui::Button::new("Undo").shortcut_text("Ctrl+Z"))
                    .clicked()
                {
                    ui.close();
                    self.undo(-1);
                }
                if ui
                    .add_enabled(can_redo, egui::Button::new("Redo").shortcut_text("Ctrl+Y"))
                    .clicked()
                {
                    ui.close();
                    self.undo(1);
                }
                ui.separator();
                if ui
                    .add_enabled(
                        !self.data.selection.is_empty(),
                        egui::Button::new("Delete selected wires").shortcut_text("Del"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.delete_selection();
                }
            });
            ui.menu_button("View", |ui| {
                ui.menu_button("Units", |ui| {
                    for (i, u) in Length::ALL.iter().enumerate() {
                        ui.radio_value(&mut self.settings.length_unit, i, u.symbol());
                    }
                    ui.separator();
                    for (i, u) in WireSize::ALL.iter().enumerate() {
                        ui.radio_value(
                            &mut self.settings.wire_unit,
                            i,
                            format!("Wire size: {}", u.label()),
                        );
                    }
                });
                ui.menu_button("Show", |ui| {
                    for tab in Tab::ALL {
                        if ui.button(tab.title()).clicked() {
                            if let Some(path) = self.dock.find_tab(&tab) {
                                let _ = self.dock.set_active_tab(path);
                            }
                            ui.close();
                        }
                    }
                });
                ui.menu_button(
                    format!("Text size ({:.0}%)", self.settings.zoom * 100.0),
                    |ui| {
                        for (label, z) in [
                            ("Smaller (Ctrl -)", self.settings.zoom / 1.1),
                            ("Normal (Ctrl 0)", 1.0),
                            ("Larger (Ctrl +)", self.settings.zoom * 1.1),
                        ] {
                            if ui.button(label).clicked() {
                                self.settings.zoom = z.clamp(0.5, 3.0);
                            }
                        }
                        for pct in [100, 125, 150, 200] {
                            let z = pct as f32 / 100.0;
                            if ui
                                .radio((self.settings.zoom - z).abs() < 0.01, format!("{pct}%"))
                                .clicked()
                            {
                                self.settings.zoom = z;
                            }
                        }
                    },
                )
                .response
                .on_hover_text(
                    "Everything on screen, text included, bigger or smaller; kept for next time",
                );
                if ui.button("Reset layout").clicked() {
                    self.dock = default_dock();
                    ui.close();
                }
                let label = if self.settings.dark {
                    "Light theme"
                } else {
                    "Dark theme"
                };
                if ui.button(label).clicked() {
                    self.settings.dark = !self.settings.dark;
                    ui.ctx().set_visuals(visuals(self.settings.dark));
                    self.views.theme_changed();
                    ui.close();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard and mouse").clicked() {
                    self.help = Some(HelpPage::Shortcuts);
                    ui.close();
                }
                if ui.button("About").clicked() {
                    self.help = Some(HelpPage::About);
                    ui.close();
                }
            });
        });
    }

    /// Help > Keyboard shortcuts and About, in the main window.
    fn help_ui(&mut self, ctx: &egui::Context) {
        let Some(page) = self.help else { return };
        let mut close = false;
        egui::Modal::new(egui::Id::new("help")).show(ctx, |ui| {
            ui.set_width(460.0);
            match page {
                HelpPage::Shortcuts => {
                    ui.heading("Keyboard and mouse");
                    egui::Grid::new("keys").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                        for (k, what) in [
                            ("Ctrl+N", "New model to draw in"),
                            ("Ctrl+O", "Open a deck"),
                            ("Ctrl+S / Ctrl+Shift+S", "Save / save as"),
                            ("Ctrl+R", "Reload the deck from its file"),
                            ("Ctrl+Z / Ctrl+Y", "Undo / redo any change"),
                            ("Del", "Delete the selected wires"),
                            ("Esc", "Stop drawing wires"),
                            ("Drag (3D)", "Rotate"),
                            ("Right-drag (3D)", "Pan"),
                            ("Scroll (3D)", "Zoom"),
                            ("Double-click (3D)", "Back to the starting view"),
                            ("Alt while dragging", "Turn snapping off for a moment"),
                            ("Click a plot", "Go to that frequency"),
                            ("Ctrl + / Ctrl - / Ctrl 0", "Bigger / smaller / normal text (View > Text size)"),
                        ] {
                            ui.label(egui::RichText::new(k).strong());
                            ui.label(what);
                            ui.end_row();
                        }
                    });
                }
                HelpPage::About => {
                    ui.heading("STRAWS");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.add_space(6.0);
                    ui.label("A fast, modern antenna modeller built on the NEC-2 method of moments.");
                    ui.label("Free software under the GNU General Public Licence, version 3 or later.");
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "The engine is nec2c by Neoklis Kyriazis (public domain), a C translation of NEC-2 by G. J. Burke and A. J. Poggio, Lawrence Livermore National Laboratory. Sommerfeld-ground fixes come from xnec2c. Moxon dimensions follow L. B. Cebik.",
                        )
                        .weak(),
                    );
                    ui.label(
                        egui::RichText::new(
                            "NEC-2 models wires (and simple surface patches): not dielectrics or solid structures such as patch antennas or horns.",
                        )
                        .weak(),
                    );
                }
            }
            ui.add_space(8.0);
            if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
        if close {
            self.help = None;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if ui.button("Open…").clicked() {
                self.leave(Leave::OpenDialog);
            }
            ui.separator();
            let plan = self.data.sweep.as_ref().map_or(self.plan_edit, |s| s.plan);
            ui.label("Frequency");
            let mut mhz = self.data.mhz;
            ui.add(
                crate::views::num(&mut mhz)
                    .speed(plan.step_mhz().max(1e-4) * 0.2)
                    .range(plan.start_mhz..=plan.stop_mhz)
                    .min_decimals(3)
                    .max_decimals(6)
                    .suffix(" MHz"),
            )
            .on_hover_text("Type an exact frequency, or drag");
            // Leave room on the right for the sweep and display settings.
            let width = (ui.available_width() - 650.0).max(120.0);
            ui.spacing_mut().slider_width = width;
            ui.add(
                egui::Slider::new(
                    &mut mhz,
                    plan.start_mhz..=plan.stop_mhz.max(plan.start_mhz + 1e-6),
                )
                .show_value(false),
            )
            .on_hover_text(
                "Scrub the frequency: every view follows. Results refine where you stop.",
            );
            self.set_frequency(mhz);
            ui.separator();

            let range = (self.plan_edit.start_mhz, self.plan_edit.stop_mhz);
            if let Some(b) = crate::views::bands::menu(
                ui,
                "toolbar band",
                &mut self.settings,
                range,
                &mut self.editing_bands,
            ) {
                self.plan_edit.start_mhz = b.start_mhz;
                self.plan_edit.stop_mhz = b.stop_mhz;
                if self.runnable() {
                    self.start_sweep();
                }
                self.set_frequency(b.centre_mhz());
            }
            ui.label("Sweep");
            let p = &mut self.plan_edit;
            let start = ui
                .add(
                    crate::views::num(&mut p.start_mhz)
                        .speed(0.01)
                        .range(0.001..=100_000.0)
                        .max_decimals(4),
                )
                .on_hover_text("Start (MHz)");
            ui.label("to");
            let stop = ui
                .add(
                    crate::views::num(&mut p.stop_mhz)
                        .speed(0.01)
                        .range(0.001..=100_000.0)
                        .max_decimals(4),
                )
                .on_hover_text("Stop (MHz)");
            let typing = start.has_focus() || stop.has_focus();
            ui.add(
                crate::views::num(&mut p.points)
                    .range(1..=401)
                    .suffix(" pts"),
            )
            .on_hover_text("Number of frequencies");
            // Only once typing has finished: while "230" is being typed,
            // "2" is briefly below the start and mustn't be corrected.
            if !typing && p.stop_mhz < p.start_mhz {
                std::mem::swap(&mut p.start_mhz, &mut p.stop_mhz);
            }
            let has_model = self.runnable();
            if ui
                .add_enabled(has_model, egui::Button::new("Run sweep"))
                .on_hover_text("Recalculate with this frequency range")
                .clicked()
            {
                self.start_sweep();
            }
            ui.separator();

            egui::ComboBox::from_id_salt("component")
                .selected_text(self.settings.component().label())
                .show_ui(ui, |ui| {
                    for (i, c) in Component::ALL.iter().enumerate() {
                        ui.selectable_value(&mut self.settings.component, i, c.label());
                    }
                })
                .response
                .on_hover_text("Which part of the field the patterns show");
            ui.label("Z₀");
            ui.add(
                crate::views::num(&mut self.settings.z0)
                    .range(1.0..=1000.0)
                    .suffix(" Ω")
                    .speed(0.5),
            )
            .on_hover_text("Reference impedance for SWR and return loss");
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| match &self.data.model {
            None => {
                ui.label("Start from a template, draw a new antenna, or open a .nec deck (or drop one on the window).");
            }
            Some(m) => {
                ui.label(format!(
                    "{} · {} segments",
                    m.name,
                    m.visible_segments().count()
                ));
                ui.separator();
                if let Some(s) = &self.data.sweep {
                    let (done, total) = (s.done(), s.plan.points);
                    if self.status.failed {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            "Sweep stopped: see Messages",
                        );
                    } else if let Some(t) = self.status.finished {
                        ui.label(format!(
                            "Sweep: {total} frequencies in {:.2} s",
                            t.as_secs_f64()
                        ));
                    } else {
                        ui.add(egui::Spinner::new());
                        let t = self
                            .status
                            .started
                            .map_or(0.0, |t| t.elapsed().as_secs_f64());
                        ui.label(format!("Sweeping: {done} of {total} · {t:.1} s"));
                    }
                }
                ui.separator();
                let exact = self
                    .data
                    .refined
                    .as_ref()
                    .is_some_and(|r| r.mhz == self.data.mhz);
                match self.data.shown_point() {
                    Some(p) if exact => {
                        ui.label(format!(
                            "Showing {:.4} MHz (exact, {}° pattern)",
                            p.mhz, REFINED_PATTERN_STEP
                        ));
                    }
                    Some(p) => {
                        ui.label(format!("Showing nearest sweep point, {:.4} MHz", p.mhz));
                    }
                    None => {}
                }
            }
        });
    }
}

pub fn visuals(dark: bool) -> egui::Visuals {
    if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Text size: Ctrl + / Ctrl - (egui's own) or View > Text size, kept
        // between sessions.
        let now = ctx.zoom_factor();
        if (now - self.applied_zoom).abs() > 1e-4 {
            self.settings.zoom = now;
        }
        self.settings.zoom = self.settings.zoom.clamp(0.5, 3.0);
        if (self.settings.zoom - now).abs() > 1e-4 {
            ctx.set_zoom_factor(self.settings.zoom);
        }
        self.applied_zoom = self.settings.zoom;
        // New wires get segments for the top of the sweep as set now, even
        // before anything has been calculated.
        self.data.max_mhz = self.plan_edit.stop_mhz;
        // The checks' distances follow a change of length unit.
        if self.settings.length() != self.checks_unit {
            self.refresh_checks();
        }
        self.handle_updates();
        self.maybe_refine(&ctx);

        // Keyboard shortcuts and dropped files.
        // A focused text box keeps its own undo and Delete key.
        let typing = ctx.egui_wants_keyboard_input();
        let (open, reload, dropped, save, undo, redo) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (
                cmd && i.key_pressed(egui::Key::O),
                cmd && i.key_pressed(egui::Key::R),
                i.raw.dropped_files.first().map(|f| f.path().to_path_buf()),
                cmd && i.key_pressed(egui::Key::S),
                !typing && cmd && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                !typing
                    && cmd
                    && (i.key_pressed(egui::Key::Y)
                        || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
            )
        });
        let (new, delete, save_as) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (
                cmd && i.key_pressed(egui::Key::N),
                !typing && i.key_pressed(egui::Key::Delete),
                cmd && i.modifiers.shift && i.key_pressed(egui::Key::S),
            )
        });
        if save {
            self.save_deck(save_as);
        }
        if undo {
            self.undo(-1);
        }
        if redo {
            self.undo(1);
        }
        if new {
            self.leave(Leave::New);
        }
        if delete {
            self.delete_selection();
        }
        if open {
            self.leave(Leave::OpenDialog);
        }
        if reload && let Some(p) = self.data.model.as_ref().and_then(|m| m.path.clone()) {
            self.leave(Leave::Open(p));
        }
        if let Some(p) = dropped {
            if is_sweep_file(&p) {
                self.import_measurement(&p);
            } else if is_drawing_file(&p) {
                self.import_drawing(&p);
            } else {
                self.leave(Leave::Open(p));
            }
        }
        // Closing the window with unsaved changes asks first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.changed() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Leave::Quit);
        }
        self.poll_dialog();
        self.take_screenshot(&ctx);
        self.confirm_ui(&ctx);
        self.help_ui(&ctx);
        crate::views::newvar::prompt(&ctx, &mut self.data);
        crate::views::bands::editor(&ctx, &mut self.settings, &mut self.editing_bands);
        self.drawing_dialog(&ctx);
        let title = match &self.data.model {
            Some(m) if self.changed() => {
                format!("• {} — STRAWS (changed)", m.name)
            }
            Some(m) => format!("{} — STRAWS", m.name),
            None => "STRAWS".to_string(),
        };
        // Only when it changes: sending it every frame keeps the window
        // redrawing (and busy) while nothing happens.
        if self.last_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        if self.status.started.is_some() && self.status.finished.is_none() && !self.status.failed {
            // Keep the elapsed time ticking while a sweep runs.
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));

        let before = self.data.mhz;
        self.data.tab_rects.clear();
        let mut tabs = views::TabView {
            data: &mut self.data,
            settings: &mut self.settings,
            views: &mut self.views,
            gpu: self.gpu.as_ref(),
        };
        egui::CentralPanel::no_frame().show(ui, |ui| {
            DockArea::new(&mut self.dock)
                .style(egui_dock::Style::from_egui(ui.style()))
                .show_close_buttons(false)
                .show_leaf_close_all_buttons(false)
                .show_inside(ui, &mut tabs);
        });
        if let Some(finished) = self.data.edit_event.take() {
            self.apply_edits(finished);
        }
        if let Some((name, text)) = self.data.new_deck.take() {
            self.leave(Leave::Text(name, text));
        }
        match self.data.request.take() {
            Some(Request::New) => self.leave(Leave::New),
            Some(Request::OpenDialog) => self.leave(Leave::OpenDialog),
            Some(Request::Open(p)) => self.leave(Leave::Open(p)),
            Some(Request::ShowTab(t)) => {
                if let Some(path) = self.dock.find_tab(&t) {
                    let _ = self.dock.set_active_tab(path);
                }
            }
            Some(Request::ImportMeasurement) => self.import_dialog(),
            Some(Request::Sweep(a, b)) => {
                self.plan_edit.start_mhz = a;
                self.plan_edit.stop_mhz = b;
                if self.runnable() {
                    self.start_sweep();
                }
                self.set_frequency((a + b) / 2.0);
            }
            None => {}
        }
        if std::mem::take(&mut self.data.request_undo) {
            self.undo(-1);
        }
        self.record_history(&ctx);
        // A view (e.g. clicking the SWR plot) may have moved the frequency.
        let after = self.data.mhz;
        if after != before {
            self.data.mhz = before;
            self.set_frequency(after);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        eframe::set_value(storage, DOCK_KEY, &self.dock);
    }

    fn on_exit(&mut self) {
        self.compute.cancel_all();
    }
}
