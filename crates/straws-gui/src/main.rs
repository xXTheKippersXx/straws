//! STRAWS desktop app.

// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod colormap;
mod compute;
mod icon;
mod model;
mod render3d;
mod scene;
mod views;

/// The name the app's settings and layout are saved under.
const APP_ID: &str = "STRAWS";

fn main() -> eframe::Result {
    // The app is also its own engine worker (see straws_core::worker).
    straws_core::worker::run_if_worker();
    bring_old_settings();

    let open_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("STRAWS")
            .with_inner_size([1280.0, 760.0])
            .with_maximized(true)
            .with_min_inner_size([800.0, 500.0])
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(icon::icon())),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, open_path)))),
    )
}

/// Settings and layout saved before the app was called STRAWS come along
/// (once: only while there's nothing saved under the new name).
fn bring_old_settings() {
    let (Some(old), Some(new)) = (
        eframe::storage_dir("NEC modeller"),
        eframe::storage_dir(APP_ID),
    ) else {
        return;
    };
    if !old.is_dir() || new.exists() || std::fs::create_dir_all(&new).is_err() {
        return;
    }
    for entry in std::fs::read_dir(&old).into_iter().flatten().flatten() {
        if entry.path().is_file() {
            let _ = std::fs::copy(entry.path(), new.join(entry.file_name()));
        }
    }
}
