//! Engine work in the background, so the window never freezes.
//!
//! Each request runs on its own thread (and in engine worker processes);
//! results come back through a channel and wake the UI. Starting a new job
//! of the same kind cancels the old one at once. Every update carries the
//! generation number of the request it belongs to, so late results from
//! cancelled work are recognised and dropped.

use std::time::Duration;

use straws_core::analysis::sweep::FreqPoint;
use straws_core::engine::EngineError;
use straws_core::output::{Patch, Segment};

#[cfg(target_arch = "wasm32")]
pub use crate::compute_web::Compute;

/// What the browser version can't do yet.
pub const DESKTOP_ONLY: &str = "The browser version runs the engine for sweeps, the frequency \
     slider, near fields and snapshots. This needs the desktop app (free, from the Releases \
     page on GitHub), which also runs several times faster.";
#[cfg(not(target_arch = "wasm32"))]
pub use native::Compute;

pub enum Update {
    Geometry {
        generation: u64,
        result: Result<(Vec<Segment>, Vec<Patch>), String>,
    },
    SweepPoint {
        generation: u64,
        index: usize,
        point: FreqPoint,
    },
    SweepDone {
        generation: u64,
        elapsed: Duration,
        messages: Vec<String>,
    },
    SweepFailed {
        generation: u64,
        message: String,
    },
    Refined {
        generation: u64,
        point: FreqPoint,
    },
    RefineFailed {
        generation: u64,
        message: String,
    },
    /// Something to tell the user that doesn't stop the work.
    Note {
        generation: u64,
        message: String,
    },
}

/// Turns engine errors into something a person can act on, keeping the
/// engine's own words underneath for reference.
pub fn explain(e: &EngineError) -> String {
    let text = e.to_string();
    let hints: &[(&str, &str)] = &[
        (
            "EXTENDS BELOW GROUND",
            "A wire goes below the ground (z < 0). Raise it, or remove the ground.",
        ),
        (
            "LIES IN PLANE OF SYMMETRY",
            "A wire lies in a plane the GX card reflects in. Move it off that plane.",
        ),
        (
            "NO SEGMENT HAS AN ITAG",
            "A card refers to a wire tag that doesn't exist. Check the tag numbers on EX, LD, TL and NT cards.",
        ),
        (
            "SEGMENT",
            "A card refers to a segment that doesn't exist on its wire.",
        ),
        (
            "PATCH DATA ERROR",
            "The surface patch cards aren't in a form NEC-2 accepts.",
        ),
        ("CONNECTION", "Wires don't join where NEC expects them to."),
    ];
    let upper = text.to_uppercase();
    match hints.iter().find(|(key, _)| upper.contains(key)) {
        Some((_, hint)) => format!("{hint}\n\n{text}"),
        None => text,
    }
}

/// Runs one deck in the background (a thread and a worker process on the
/// desktop, an engine Web Worker in a browser), then `then` on its output;
/// the outcome arrives on the receiver. To stop: cancel `cancel` and drop
/// the receiver.
pub fn run_once<T: Send + 'static>(
    ctx: &eframe::egui::Context,
    deck: &straws_core::deck::NumericDeck,
    cancel: straws_core::engine::CancelToken,
    then: impl FnOnce(straws_core::output::NecOutput) -> Result<T, String> + Send + 'static,
) -> std::sync::mpsc::Receiver<Result<T, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let deck = deck.clone();
        std::thread::spawn(move || {
            let r = straws_core::engine::Engine::self_hosted()
                .map_err(|e| e.to_string())
                .and_then(|e| e.run(&deck, &cancel).map_err(|e| explain(&e)))
                .and_then(then);
            let _ = tx.send(r);
            ctx.request_repaint();
        });
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = cancel;
        let repaint = ctx.clone();
        crate::web::pool(&ctx).submit_with(deck.to_nec_text(), true, move |r| {
            let r = r
                .map(|raw| {
                    straws_core::output::NecOutput::parse_with_binary(&raw.text, &raw.binary)
                        .unwrap_or_else(|| straws_core::output::NecOutput::parse(&raw.text))
                })
                .map_err(|e| explain(&e))
                .and_then(then);
            let _ = tx.send(r);
            repaint.request_repaint();
        });
    }
    rx
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::Instant;

    use eframe::egui;
    use straws_core::analysis::buried::{self, Setup};
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, geometry_deck, run_deck};
    use straws_core::analysis::sweep::{FreqPoint, SweepResults};
    use straws_core::deck::NumericDeck;
    use straws_core::engine::{CancelToken, Engine, EngineError, SweepOptions};
    use straws_core::output::NecOutput;

    use super::{Update, explain};

    /// Frequencies across a sweep where buried radials' share is worked out
    /// (it changes slowly; in between it's interpolated).
    const ESTIMATE_SAMPLES: usize = 5;

    pub struct Compute {
        engine: Result<Engine, String>,
        tx: Sender<Update>,
        rx: Receiver<Update>,
        ctx: egui::Context,
        sweep: Option<CancelToken>,
        refine: Option<CancelToken>,
    }

    impl Compute {
        pub fn new(ctx: egui::Context) -> Self {
            let (tx, rx) = channel();
            Self {
                engine: Engine::self_hosted()
                    .map_err(|e| format!("can't start the NEC engine: {e}")),
                tx,
                rx,
                ctx,
                sweep: None,
                refine: None,
            }
        }

        /// Updates that have arrived since the last call.
        /// The egui context, for waking the window from other threads.
        pub fn ctx(&self) -> egui::Context {
            self.ctx.clone()
        }

        pub fn poll(&mut self) -> Vec<Update> {
            self.rx.try_iter().collect()
        }

        pub fn engine(&self) -> Result<Engine, String> {
            self.engine.clone()
        }

        fn spawn(&self, work: impl FnOnce(Sender<Update>) + Send + 'static) {
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                work(tx);
                ctx.request_repaint();
            });
        }

        /// Lists the segments without solving: quick, so the model appears at
        /// once while the sweep runs.
        pub fn geometry(&self, generation: u64, deck: &NumericDeck) {
            let text = geometry_deck(deck).to_nec_text();
            let engine = self.engine();
            self.spawn(move |tx| {
                let result = engine.and_then(|e| {
                    e.run_text(&text, &CancelToken::new())
                        .map(|raw| {
                            let out = NecOutput::parse(&raw.text);
                            (out.segments, out.patches)
                        })
                        .map_err(|err| explain(&err))
                });
                let _ = tx.send(Update::Geometry { generation, result });
            });
        }

        pub fn cancel_all(&mut self) {
            for token in [self.sweep.take(), self.refine.take()]
                .into_iter()
                .flatten()
            {
                token.cancel();
            }
        }

        /// Starts a sweep with a coarse pattern at every frequency, cancelling
        /// any sweep still running. Points stream back as they're computed.
        pub fn sweep(
            &mut self,
            generation: u64,
            deck: &NumericDeck,
            plan: FrequencyPlan,
            step: f64,
            buried: Option<Setup>,
        ) {
            if let Some(old) = self.sweep.take() {
                old.cancel();
            }
            let token = CancelToken::new();
            self.sweep = Some(token.clone());
            // Radials on or under the ground: their estimate is worked out
            // alongside the sweep, and put into each result as it arrives.
            let estimate = buried.map(|setup| {
                let (etx, erx) = std::sync::mpsc::channel();
                let (engine, deck, token) = (self.engine(), deck.clone(), token.clone());
                std::thread::spawn(move || {
                    let r = engine.and_then(|e| {
                        buried::estimate(&e, &deck, &setup, &plan, ESTIMATE_SAMPLES, &token)
                    });
                    let _ = etx.send(r);
                });
                erx
            });
            let mut estimated: Option<Option<buried::Estimate>> = None;
            let notes = self.tx.clone();
            let run = run_deck(deck, &plan, Some(PatternRequest::new(step)));
            let helpers = deck.clone();
            let engine = self.engine();
            let ctx = self.ctx.clone();
            self.spawn(move |tx| {
                let started = Instant::now();
                let engine = match engine {
                    Ok(e) => e,
                    Err(message) => {
                        let _ = tx.send(Update::SweepFailed {
                            generation,
                            message,
                        });
                        return;
                    }
                };
                let options = SweepOptions {
                    // More, smaller jobs make results arrive in more batches.
                    jobs_per_thread: 3,
                    ..SweepOptions::default()
                };
                let result = engine.sweep(&run, options, &token, |index, r| {
                    let mut point = FreqPoint::from_result(r, Some(step)).without_helpers(&helpers);
                    if let Some(rx) = &estimate {
                        let e = estimated.get_or_insert_with(|| match rx.recv() {
                            Ok(Ok(e)) => Some(e),
                            Ok(Err(message)) => {
                                let _ = notes.send(Update::Note {
                                    generation,
                                    message: format!(
                                        "The radials on or under the ground couldn't be estimated: {message}."
                                    ),
                                });
                                None
                            }
                            Err(_) => None,
                        });
                        if let Some(z) = e.as_ref().and_then(|e| e.z(point.mhz)) {
                            point = point.with_estimate(z);
                        }
                    }
                    let _ = tx.send(Update::SweepPoint {
                        generation,
                        index,
                        point,
                    });
                    ctx.request_repaint();
                });
                let _ = tx.send(match result {
                    Ok(out) => Update::SweepDone {
                        generation,
                        elapsed: started.elapsed(),
                        messages: out.messages,
                    },
                    Err(EngineError::Cancelled) => return,
                    Err(e) => Update::SweepFailed {
                        generation,
                        message: explain(&e),
                    },
                });
            });
        }

        /// Computes one frequency exactly, with a finer pattern, cancelling any
        /// earlier refinement.
        pub fn refine(
            &mut self,
            generation: u64,
            deck: &NumericDeck,
            mhz: f64,
            step: f64,
            buried: Option<Setup>,
        ) {
            let estimate_deck = deck.clone();
            self.cancel_refine();
            let token = CancelToken::new();
            self.refine = Some(token.clone());
            let run = run_deck(
                deck,
                &FrequencyPlan::single(mhz),
                Some(PatternRequest::new(step)),
            );
            let helpers = deck.clone();
            let engine = self.engine();
            self.spawn(move |tx| {
                let result = engine.and_then(|e| {
                    e.run(&run, &token).map_err(|err| match err {
                        EngineError::Cancelled => String::new(),
                        other => explain(&other),
                    })
                });
                match result {
                    Ok(out) => {
                        if let Some(r) = out.frequencies.first() {
                            let mut point =
                                FreqPoint::from_result(r, Some(step)).without_helpers(&helpers);
                            if let Some(setup) = &buried
                                && let Ok(e) = Engine::self_hosted()
                                && let Ok(p) = buried::point(&e, &estimate_deck, setup, mhz, &token)
                            {
                                point = point.with_estimate(p.z_perfect + p.dz);
                            }
                            // NEC prints the frequency to 5 figures; keep the
                            // exact one asked for, so the slider recognises it.
                            point.mhz = mhz;
                            let _ = tx.send(Update::Refined { generation, point });
                        }
                    }
                    Err(message) if message.is_empty() => {}
                    Err(message) => {
                        let _ = tx.send(Update::RefineFailed {
                            generation,
                            message,
                        });
                    }
                }
            });
        }

        /// Sweeps another design (a snapshot) alongside, with no updates.
        pub fn sweep_into(
            &mut self,
            deck: NumericDeck,
            plan: FrequencyPlan,
            step: f64,
        ) -> Receiver<Option<SweepResults>> {
            let (tx, rx) = channel();
            let engine = self.engine();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let run = run_deck(&deck, &plan, Some(PatternRequest::new(step)));
                let mut results = SweepResults::new(plan);
                let ok = engine.is_ok_and(|e| {
                    e.sweep(
                        &run,
                        SweepOptions::default(),
                        &CancelToken::new(),
                        |index, r| {
                            results.insert(
                                index,
                                FreqPoint::from_result(r, Some(step)).without_helpers(&deck),
                            );
                        },
                    )
                    .is_ok()
                });
                let _ = tx.send(ok.then_some(results));
                ctx.request_repaint();
            });
            rx
        }

        pub fn cancel_refine(&mut self) {
            if let Some(old) = self.refine.take() {
                old.cancel();
            }
        }
    }
}
