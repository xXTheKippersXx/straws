//! The dockable views.

pub mod bands;
pub mod cadimport;
mod checks;
pub mod compare;
pub mod curves;
mod cuts;
mod feedline;
pub mod feeds;
mod feedsystem;
pub mod grids;
mod ground;
mod matching;
mod nearfield;
pub mod newvar;
mod optimiser;
pub mod patches;
pub mod plates;
pub mod radials;
mod receive;
pub mod reflectors;
pub mod smith;
mod swr;
mod templates;
mod text;
mod variables;
mod view3d;
pub mod wires;

use eframe::egui;

/// A number box that applies what's typed when the typing is done (Enter or
/// leaving the box), not on every key: half-typed numbers ("0" on the way
/// to "0.15") could otherwise break the model mid-edit. Dragging still
/// updates as it goes.
pub fn num<N: egui::emath::Numeric>(value: &mut N) -> egui::DragValue<'_> {
    egui::DragValue::new(value).update_while_editing(false)
}

use crate::app::{Data, Settings, Tab};
use crate::render3d::Gpu;

/// State each view keeps between frames.
#[derive(Default)]
pub struct ViewStates {
    pub model: view3d::ModelView,
    pub pattern: view3d::PatternView,
    pub cuts: cuts::CutsView,
    pub wires: wires::WiresView,
    pub deck: text::DeckView,
    pub compare: compare::CompareView,
    pub optimiser: optimiser::OptimiserView,
    pub matching: matching::MatchingView,
    pub templates: templates::TemplatesView,
    pub near: nearfield::NearFieldView,
    pub checks: checks::ChecksView,
    pub variables: variables::VariablesView,
    pub feed_system: feedsystem::FeedSystemView,
}

impl ViewStates {
    /// Colours depend on the theme, so cached 3D geometry must be rebuilt.
    pub fn theme_changed(&mut self) {
        self.model.invalidate();
        self.pattern.invalidate();
    }
}

/// Draws each tab, borrowing what it needs from the app for one frame.
pub struct TabView<'a> {
    pub data: &'a mut Data,
    pub settings: &'a mut Settings,
    pub views: &'a mut ViewStates,
    pub gpu: Option<&'a Gpu>,
}

impl egui_dock::TabViewer for TabView<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        self.data.tab_rects.push((*tab, ui.max_rect()));
        match tab {
            Tab::Model => self.views.model.ui(ui, self.gpu, self.data, self.settings),
            Tab::Pattern => self
                .views
                .pattern
                .ui(ui, self.gpu, self.data, self.settings),
            Tab::Cuts => self.views.cuts.ui(ui, self.data, self.settings),
            Tab::Swr => swr::ui(ui, self.data, self.settings),
            Tab::Deck => self.views.deck.ui(ui, self.data),
            Tab::Messages => text::messages(ui, self.data),
            Tab::Variables => self.views.variables.ui(ui, self.data, self.settings),
            Tab::Wires => self.views.wires.ui(ui, self.data, self.settings),
            Tab::Checks => self.views.checks.ui(ui, self.data, self.settings),
            Tab::Feeds => feeds::ui(ui, self.data, self.settings),
            Tab::Receive => receive::ui(ui, self.data, self.settings),
            Tab::FeedSystem => self.views.feed_system.ui(ui, self.data, self.settings),
            Tab::Ground => ground::ui(ui, self.data, self.settings),
            Tab::Smith => smith::ui(ui, self.data, self.settings),
            Tab::Compare => self.views.compare.ui(ui, self.data, self.settings),
            Tab::Optimiser => self.views.optimiser.ui(ui, self.data, self.settings),
            Tab::Matching => self.views.matching.ui(ui, self.data, self.settings),
            Tab::Templates => self.views.templates.ui(ui, self.data),
            Tab::Feedline => feedline::ui(ui, self.data, self.settings),
            Tab::NearField => self.views.near.ui(ui, self.data, self.settings),
            Tab::Curves => empty_note(
                ui,
                "Curves are drawn in the 3D model view (Draw, then Arc, Semicircle, Circle, Helix or Spiral); click one to adjust it.",
            ),
        }
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        // Views fill their space; the text views scroll themselves.
        [false, false]
    }
}

/// A placeholder shown when a view has nothing to show yet.
pub fn empty_note(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(egui::RichText::new(text).color(ui.visuals().weak_text_color()));
    });
}
