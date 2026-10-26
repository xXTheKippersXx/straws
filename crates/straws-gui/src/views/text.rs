//! The deck text and the messages list.

use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};
use straws_core::deck::Deck;

use crate::app::{Data, Level};

use super::empty_note;

/// The deck as editable text, kept in step with the other views.
#[derive(Default)]
pub struct DeckView {
    text: String,
    /// `Data::edits` the text was last filled from.
    synced: Option<u64>,
    /// When the text was last typed in, to apply it once typing pauses.
    typed: Option<Instant>,
}

/// Typing pause after which the text is applied to the model.
const APPLY_AFTER: Duration = Duration::from_millis(600);

impl DeckView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data) {
        let Some(model) = &mut data.model else {
            return empty_note(ui, "Open a .nec deck to see its text.");
        };
        // Take changes made elsewhere, unless the user is typing here.
        if self.synced != Some(data.edits) && self.typed.is_none() {
            self.text = model
                .deck
                .lines
                .iter()
                .map(|l| l.raw.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            self.synced = Some(data.edits);
        }
        let error_line = model.numeric.as_ref().err().and_then(|e| e.line);
        if let Err(e) = &model.numeric {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("This deck can't be run: {e}"),
                );
                if ui.small_button("Undo the last change").clicked() {
                    data.request_undo = true;
                }
            });
        } else {
            ui.label(
                RichText::new("Edit the deck here: the model updates when you pause typing.")
                    .weak(),
            );
        }
        let (weak, error) = (ui.visuals().weak_text_color(), ui.visuals().error_fg_color);
        let normal = ui.visuals().text_color();
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = egui::text::LayoutJob::default();
            let font = egui::FontId::monospace(13.0);
            for (k, line) in text.as_str().split_inclusive('\n').enumerate() {
                let colour = if Some(k + 1) == error_line {
                    error
                } else if line.trim_start().starts_with('\'') {
                    weak
                } else {
                    normal
                };
                job.append(line, 0.0, egui::TextFormat::simple(font.clone(), colour));
            }
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let mut finished = false;
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            let r = ui.add(
                egui::TextEdit::multiline(&mut self.text)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(30)
                    .layouter(&mut layouter),
            );
            if r.changed() {
                self.typed = Some(Instant::now());
            }
            if r.lost_focus() && self.typed.is_some() {
                finished = true;
            }
        });
        if let Some(t) = self.typed {
            if t.elapsed() >= APPLY_AFTER {
                finished = true;
            } else {
                ui.ctx().request_repaint_after(APPLY_AFTER - t.elapsed());
            }
        }
        if finished {
            self.typed = None;
            model.deck = Deck::parse(&self.text);
            data.edit_event = Some(true);
            // Our own text already matches; don't refill it.
            self.synced = Some(data.edits + 1);
        }
    }
}

pub fn messages(ui: &mut egui::Ui, data: &Data) {
    if data.messages.is_empty() {
        return empty_note(ui, "No messages.");
    }
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for m in &data.messages {
                let (mark, colour) = match m.level {
                    Level::Info => ("ℹ", ui.visuals().weak_text_color()),
                    Level::Warning => ("⚠", ui.visuals().warn_fg_color),
                    Level::Error => ("✖", ui.visuals().error_fg_color),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(colour, mark);
                    ui.label(&m.text);
                });
            }
        });
}
