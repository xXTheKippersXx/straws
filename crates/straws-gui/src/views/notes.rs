//! Notes kept with the design (saved in its project file).

use eframe::egui::{self, RichText};

use crate::app::Data;

use super::empty_note;

pub fn ui(ui: &mut egui::Ui, data: &mut Data) {
    if data.model.is_none() {
        return empty_note(ui, "Open a model to keep notes with it.");
    }
    let is_project = data
        .model
        .as_ref()
        .and_then(|m| m.path.as_deref())
        .is_none_or(crate::project::is_project);
    ui.label(
        RichText::new(if is_project {
            "Notes about this design: how it was built, what was measured, what to try next. \
             They're saved in its project file (.straws)."
        } else {
            "Notes about this design. A .nec deck can't hold them: save it as a project \
             (.straws) with File > Save as to keep them."
        })
        .weak(),
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.add_sized(
                ui.available_size(),
                egui::TextEdit::multiline(&mut data.notes)
                    .hint_text("Type here")
                    .desired_width(f32::INFINITY),
            );
        });
}
