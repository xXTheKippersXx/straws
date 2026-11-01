//! Files: real files and dialogs on the desktop; in a browser, opening a
//! file uploads it into memory and saving one downloads it.
//!
//! The rest of the app works with paths either way. In a browser, a picked
//! or dropped file is kept in memory under its name, and [`read`] finds it
//! there; [`write`] hands the bytes to the browser as a download named
//! after the path's file name.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui;

/// True in the browser version.
pub const WEB: bool = cfg!(target_arch = "wasm32");

/// A dialog's answer: the chosen path, or `None` if it was cancelled.
pub type Answer = Receiver<Option<PathBuf>>;

/// What an open or save dialog offers (the parts of rfd's builder the app
/// uses, so both versions share one description).
#[derive(Default)]
pub struct Dialog {
    filters: Vec<(String, Vec<String>)>,
    file_name: Option<String>,
    directory: Option<PathBuf>,
}

impl Dialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_filter(mut self, name: &str, extensions: &[&str]) -> Self {
        self.filters.push((
            name.to_string(),
            extensions.iter().map(|e| e.to_string()).collect(),
        ));
        self
    }

    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    pub fn set_directory(mut self, dir: &Path) -> Self {
        self.directory = Some(dir.to_path_buf());
        self
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;

    fn native(dialog: Dialog) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        for (name, exts) in &dialog.filters {
            d = d.add_filter(name, exts);
        }
        if let Some(name) = dialog.file_name {
            d = d.set_file_name(name);
        }
        if let Some(dir) = dialog.directory {
            d = d.set_directory(dir);
        }
        d
    }

    /// Runs a blocking dialog on its own thread, so the window keeps
    /// drawing while it's open.
    fn on_thread(
        ctx: &egui::Context,
        show: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    ) -> Answer {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(show());
            ctx.request_repaint();
        });
        rx
    }

    pub fn pick(dialog: Dialog, ctx: &egui::Context) -> Answer {
        let d = native(dialog);
        on_thread(ctx, move || d.pick_file())
    }

    pub fn save(dialog: Dialog, ctx: &egui::Context) -> Answer {
        let d = native(dialog);
        on_thread(ctx, move || d.save_file())
    }

    pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::write(path, bytes)
    }

    /// A file dropped on the window: its path, at once.
    pub fn dropped(file: egui::DroppedFileHandle, _ctx: &egui::Context) -> Answer {
        let (tx, rx) = channel();
        let _ = tx.send(Some(file.path().to_path_buf()));
        rx
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    thread_local! {
        /// Files picked or dropped in this browser tab, by name.
        static FILES: RefCell<HashMap<PathBuf, Vec<u8>>> = RefCell::new(HashMap::new());
    }

    /// Opens the browser's file picker. Picking a file reads it into
    /// memory; closing the picker answers `None`.
    pub fn pick(dialog: Dialog, ctx: &egui::Context) -> Answer {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            let _ = tx.send(None);
            return rx;
        };
        let Ok(input) = document
            .create_element("input")
            .map(|e| e.unchecked_into::<web_sys::HtmlInputElement>())
        else {
            let _ = tx.send(None);
            return rx;
        };
        input.set_type("file");
        let accept: Vec<String> = dialog
            .filters
            .iter()
            .flat_map(|(_, exts)| exts.iter())
            .filter(|e| e.as_str() != "*")
            .map(|e| format!(".{}", e.to_lowercase()))
            .collect();
        if !accept.is_empty()
            && !dialog
                .filters
                .iter()
                .any(|(_, e)| e.iter().any(|x| x == "*"))
        {
            input.set_accept(&accept.join(","));
        }
        let picked = input.clone();
        let (tx2, ctx2) = (tx.clone(), ctx.clone());
        let on_change = Closure::once(move || {
            let file = picked.files().and_then(|list| list.get(0));
            let Some(file) = file else {
                let _ = tx2.send(None);
                ctx2.request_repaint();
                return;
            };
            wasm_bindgen_futures::spawn_local(async move {
                let bytes = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
                    .await
                    .ok()
                    .map(|b| js_sys::Uint8Array::new(&b).to_vec());
                let answer = bytes.map(|b| remember(&file.name(), &b));
                let _ = tx2.send(answer);
                ctx2.request_repaint();
            });
        });
        let on_cancel = Closure::once(move || {
            let _ = tx.send(None);
            ctx.request_repaint();
        });
        input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
        let _ =
            input.add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref());
        // The closures live as long as the page (one small leak per dialog).
        on_change.forget();
        on_cancel.forget();
        input.click();
        rx
    }

    /// A save "dialog" in a browser: the download is named after the
    /// suggested file name, and the browser decides where it goes.
    pub fn save(dialog: Dialog, _ctx: &egui::Context) -> Answer {
        let (tx, rx) = channel();
        let name = dialog
            .file_name
            .unwrap_or_else(|| "antenna.nec".to_string());
        let _ = tx.send(Some(PathBuf::from(name)));
        rx
    }

    pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
        FILES
            .with(|f| f.borrow().get(path).cloned())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the browser version can only open files picked or dropped in this tab",
                )
            })
    }

    /// Downloads `bytes` as a file named after the path's file name.
    pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let name = path.file_name().map_or_else(
            || "download".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        download(&name, bytes).map_err(|e| {
            std::io::Error::other(format!("the browser refused the download: {e:?}"))
        })?;
        // Keep it, so it can be opened again in this tab.
        remember(&name, bytes);
        Ok(())
    }

    fn download(name: &str, bytes: &[u8]) -> Result<(), JsValue> {
        let window = web_sys::window().ok_or("no window")?;
        let document = window.document().ok_or("no document")?;
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes).into());
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("application/octet-stream");
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;
        let a: web_sys::HtmlAnchorElement = document.create_element("a")?.unchecked_into();
        a.set_href(&url);
        a.set_download(name);
        a.click();
        // Give the browser a moment to start the download before letting go.
        let revoke = Closure::once(move || {
            let _ = web_sys::Url::revoke_object_url(&url);
        });
        window.set_timeout_with_callback_and_timeout_and_arguments_0(
            revoke.as_ref().unchecked_ref(),
            10_000,
        )?;
        revoke.forget();
        Ok(())
    }

    /// A file dropped on the page: read into memory, then its path.
    pub fn dropped(file: egui::DroppedFileHandle, ctx: &egui::Context) -> Answer {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let name = file.path().to_string_lossy().into_owned();
            let answer = file.bytes_async().await.ok().map(|b| remember(&name, &b));
            let _ = tx.send(answer);
            ctx.request_repaint();
        });
        rx
    }

    /// Keeps a file's bytes in memory under its name; returns the path the
    /// rest of the app uses for it.
    pub fn remember(name: &str, bytes: &[u8]) -> PathBuf {
        let path = PathBuf::from(name);
        FILES.with(|f| f.borrow_mut().insert(path.clone(), bytes.to_vec()));
        path
    }
}

pub use imp::{dropped, pick, read, save, write};
