//! Engine work in the browser version: the same requests and updates as
//! the desktop's `Compute`, but run in the page's engine Web Workers
//! ([`EnginePool`]) and gathered each frame by [`Compute::poll`], since a
//! browser page has no threads of its own to wait on.

use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use straws_core::analysis::buried::Setup;
use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, geometry_deck, run_deck};
use straws_core::analysis::sweep::{FreqPoint, SweepResults};
use straws_core::deck::NumericDeck;
use straws_core::engine::{EngineError, RawRun, split_sweep};
use straws_core::output::NecOutput;
use web_time::Instant;

use crate::compute::{Update, explain};
use crate::web::EnginePool;

pub struct Compute {
    pool: EnginePool,
    ctx: egui::Context,
    geometry: Vec<(u64, u64)>,
    sweep: Option<SweepRun>,
    refine: Option<RefineRun>,
    background: Vec<Background>,
    updates: Vec<Update>,
}

/// A sweep split over the engine workers.
struct SweepRun {
    generation: u64,
    /// Each job's id and which sweep frequencies it computes (`None`: the
    /// whole deck in one job, in its own order).
    jobs: Vec<(u64, Option<Vec<usize>>)>,
    remaining: usize,
    step: f64,
    helpers: NumericDeck,
    started: Instant,
    messages: Vec<String>,
}

struct RefineRun {
    generation: u64,
    job: u64,
    mhz: f64,
    step: f64,
    helpers: NumericDeck,
}

/// A sweep for something else (a snapshot), filling its own results.
struct Background {
    jobs: Vec<(u64, Option<Vec<usize>>)>,
    remaining: usize,
    step: f64,
    helpers: NumericDeck,
    results: SweepResults,
    failed: bool,
    tx: Sender<Option<SweepResults>>,
}

/// The deck's frequencies split into jobs for the pool.
fn submit_split(pool: &EnginePool, run: &NumericDeck) -> Vec<(u64, Option<Vec<usize>>)> {
    let parts = (pool.size() * 2).min(run.frequencies().len());
    match split_sweep(run, parts) {
        Some(jobs) => jobs
            .into_iter()
            .map(|j| (pool.submit(j.deck.to_nec_text(), true), Some(j.indices)))
            .collect(),
        None => vec![(pool.submit(run.to_nec_text(), true), None)],
    }
}

fn parse(raw: &RawRun) -> NecOutput {
    NecOutput::parse_with_binary(&raw.text, &raw.binary)
        .unwrap_or_else(|| NecOutput::parse(&raw.text))
}

impl Compute {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            pool: crate::web::pool(&ctx),
            ctx,
            geometry: Vec::new(),
            sweep: None,
            refine: None,
            background: Vec::new(),
            updates: Vec::new(),
        }
    }

    /// The egui context, for waking the window.
    pub fn ctx(&self) -> egui::Context {
        self.ctx.clone()
    }

    /// Updates that have arrived since the last call.
    pub fn poll(&mut self) -> Vec<Update> {
        for (id, result) in self.pool.take() {
            self.finished(id, result);
        }
        std::mem::take(&mut self.updates)
    }

    pub fn geometry(&mut self, generation: u64, deck: &NumericDeck) {
        let id = self.pool.submit(geometry_deck(deck).to_nec_text(), false);
        self.geometry.push((id, generation));
    }

    pub fn cancel_all(&mut self) {
        if let Some(s) = self.sweep.take() {
            for (id, _) in s.jobs {
                self.pool.cancel(id);
            }
        }
        self.cancel_refine();
    }

    pub fn sweep(
        &mut self,
        generation: u64,
        deck: &NumericDeck,
        plan: FrequencyPlan,
        step: f64,
        buried: Option<Setup>,
    ) {
        if let Some(s) = self.sweep.take() {
            for (id, _) in s.jobs {
                self.pool.cancel(id);
            }
        }
        if buried.is_some() {
            self.updates.push(Update::Note {
                generation,
                message: "Radials on or under the ground are left out in the browser version \
                          (their estimate needs the desktop app), so the results are as if \
                          they weren't there."
                    .into(),
            });
        }
        let run = run_deck(deck, &plan, Some(PatternRequest::new(step)));
        let jobs = submit_split(&self.pool, &run);
        self.sweep = Some(SweepRun {
            generation,
            remaining: jobs.len(),
            jobs,
            step,
            helpers: deck.clone(),
            started: Instant::now(),
            messages: Vec::new(),
        });
    }

    pub fn refine(
        &mut self,
        generation: u64,
        deck: &NumericDeck,
        mhz: f64,
        step: f64,
        _buried: Option<Setup>,
    ) {
        self.cancel_refine();
        let run = run_deck(
            deck,
            &FrequencyPlan::single(mhz),
            Some(PatternRequest::new(step)),
        );
        let job = self.pool.submit_first(run.to_nec_text(), true);
        self.refine = Some(RefineRun {
            generation,
            job,
            mhz,
            step,
            helpers: deck.clone(),
        });
    }

    pub fn cancel_refine(&mut self) {
        if let Some(r) = self.refine.take() {
            self.pool.cancel(r.job);
        }
    }

    /// Sweeps another design (a snapshot) alongside, with no updates.
    pub fn sweep_into(
        &mut self,
        deck: NumericDeck,
        plan: FrequencyPlan,
        step: f64,
    ) -> Receiver<Option<SweepResults>> {
        let (tx, rx) = channel();
        let run = run_deck(&deck, &plan, Some(PatternRequest::new(step)));
        let jobs = submit_split(&self.pool, &run);
        self.background.push(Background {
            remaining: jobs.len(),
            jobs,
            step,
            helpers: deck,
            results: SweepResults::new(plan),
            failed: false,
            tx,
        });
        rx
    }

    fn finished(&mut self, id: u64, result: Result<RawRun, EngineError>) {
        if let Some(k) = self.geometry.iter().position(|(j, _)| *j == id) {
            let (_, generation) = self.geometry.remove(k);
            let result = result
                .map(|raw| {
                    let out = NecOutput::parse(&raw.text);
                    (out.segments, out.patches)
                })
                .map_err(|e| explain(&e));
            self.updates.push(Update::Geometry { generation, result });
            return;
        }
        if self.refine.as_ref().is_some_and(|r| r.job == id) {
            let Some(r) = self.refine.take() else { return };
            match result {
                Ok(raw) => {
                    if let Some(f) = parse(&raw).frequencies.first() {
                        let mut point =
                            FreqPoint::from_result(f, Some(r.step)).without_helpers(&r.helpers);
                        point.mhz = r.mhz;
                        self.updates.push(Update::Refined {
                            generation: r.generation,
                            point,
                        });
                    }
                }
                Err(e) => self.updates.push(Update::RefineFailed {
                    generation: r.generation,
                    message: explain(&e),
                }),
            }
            return;
        }
        if let Some(s) = &mut self.sweep
            && let Some(k) = s.jobs.iter().position(|(j, _)| *j == id)
        {
            let generation = s.generation;
            match result {
                Ok(raw) => {
                    let out = parse(&raw);
                    for (j, f) in out.frequencies.iter().enumerate() {
                        let index = match &s.jobs[k].1 {
                            Some(indices) => match indices.get(j) {
                                Some(&i) => i,
                                None => continue,
                            },
                            None => j,
                        };
                        let point =
                            FreqPoint::from_result(f, Some(s.step)).without_helpers(&s.helpers);
                        self.updates.push(Update::SweepPoint {
                            generation,
                            index,
                            point,
                        });
                    }
                    for m in out.messages {
                        if !s.messages.contains(&m) {
                            s.messages.push(m);
                        }
                    }
                    s.remaining -= 1;
                    if s.remaining == 0 {
                        let elapsed = s.started.elapsed();
                        let messages = std::mem::take(&mut s.messages);
                        self.sweep = None;
                        self.updates.push(Update::SweepDone {
                            generation,
                            elapsed,
                            messages,
                        });
                    }
                }
                Err(e) => {
                    if let Some(s) = self.sweep.take() {
                        for (j, _) in s.jobs {
                            self.pool.cancel(j);
                        }
                    }
                    self.updates.push(Update::SweepFailed {
                        generation,
                        message: explain(&e),
                    });
                }
            }
            return;
        }
        if let Some(b) = self
            .background
            .iter()
            .position(|b| b.jobs.iter().any(|(j, _)| *j == id))
        {
            let bg = &mut self.background[b];
            let k = bg.jobs.iter().position(|(j, _)| *j == id).unwrap_or(0);
            match result {
                Ok(raw) => {
                    for (j, f) in parse(&raw).frequencies.iter().enumerate() {
                        let index = match &bg.jobs[k].1 {
                            Some(indices) => indices.get(j).copied(),
                            None => Some(j),
                        };
                        if let Some(index) = index {
                            bg.results.insert(
                                index,
                                FreqPoint::from_result(f, Some(bg.step))
                                    .without_helpers(&bg.helpers),
                            );
                        }
                    }
                }
                Err(_) => bg.failed = true,
            }
            bg.remaining -= 1;
            if bg.remaining == 0 {
                let bg = self.background.remove(b);
                let _ = bg.tx.send((!bg.failed).then_some(bg.results));
                self.ctx.request_repaint();
            }
        }
    }
}
