//! Engine work in the browser version: the same requests and updates as
//! the desktop's `Compute`, but run in the page's engine Web Workers
//! ([`EnginePool`]) and gathered each frame by [`Compute::poll`], since a
//! browser page has no threads of its own to wait on.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use eframe::egui;
use straws_core::analysis::buried::{self, Estimate, Point, Setup};
use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, geometry_deck, run_deck};
use straws_core::analysis::sweep::{FreqPoint, SweepResults};
use straws_core::deck::NumericDeck;
use straws_core::engine::{CancelToken, EngineError, RawRun, split_sweep};
use straws_core::output::NecOutput;
use web_time::Instant;

use crate::compute::{Update, explain};
use crate::web::{EnginePool, PoolRunner};

/// Frequencies across a sweep where buried radials' share is worked out
/// (as on the desktop).
const ESTIMATE_SAMPLES: usize = 5;

/// A result some background work fills in when it's done.
type Slot<T> = Rc<RefCell<Option<T>>>;

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
    /// Radials on or under the ground: their estimate, worked out
    /// alongside. Points wait for it (as on the desktop).
    estimate: Option<Slot<Result<Estimate, String>>>,
    token: CancelToken,
    held: Vec<(usize, FreqPoint)>,
    /// Every job is in; the sweep ends once the estimate is too.
    done: Option<(Duration, Vec<String>)>,
}

struct RefineRun {
    generation: u64,
    job: u64,
    mhz: f64,
    step: f64,
    helpers: NumericDeck,
    estimate: Option<Slot<Result<Point, String>>>,
    token: CancelToken,
    /// The run's point, waiting for the estimate.
    held: Option<FreqPoint>,
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
        self.release();
        std::mem::take(&mut self.updates)
    }

    pub fn geometry(&mut self, generation: u64, deck: &NumericDeck) {
        let id = self.pool.submit(geometry_deck(deck).to_nec_text(), false);
        self.geometry.push((id, generation));
    }

    pub fn cancel_all(&mut self) {
        self.cancel_sweep();
        self.cancel_refine();
    }

    fn cancel_sweep(&mut self) {
        if let Some(s) = self.sweep.take() {
            s.token.cancel();
            for (id, _) in s.jobs {
                self.pool.cancel(id);
            }
        }
    }

    /// Starts `work` on the page's event loop; its result lands in the slot.
    fn background<T: 'static>(
        &self,
        work: impl std::future::Future<Output = T> + 'static,
    ) -> Slot<T> {
        let slot: Slot<T> = Rc::new(RefCell::new(None));
        let (out, ctx) = (slot.clone(), self.ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let r = work.await;
            *out.borrow_mut() = Some(r);
            ctx.request_repaint();
        });
        slot
    }

    /// Hands on points that were waiting for a buried-radial estimate.
    fn release(&mut self) {
        if let Some(s) = &mut self.sweep {
            let ready = match &s.estimate {
                None => Some(None),
                Some(slot) => match &*slot.borrow() {
                    None => None,
                    Some(Ok(e)) => Some(Some(e.clone())),
                    Some(Err(m)) => {
                        self.updates.push(Update::Note {
                            generation: s.generation,
                            message: format!(
                                "The radials on or under the ground couldn't be estimated: {m}."
                            ),
                        });
                        Some(None)
                    }
                },
            };
            if let Some(estimate) = ready {
                // Said once: from now on the points go straight through.
                s.estimate = None;
                for (index, mut point) in s.held.drain(..) {
                    if let Some(z) = estimate.as_ref().and_then(|e| e.z(point.mhz)) {
                        point = point.with_estimate(z);
                    }
                    self.updates.push(Update::SweepPoint {
                        generation: s.generation,
                        index,
                        point,
                    });
                }
                if let Some((elapsed, messages)) = s.done.take() {
                    let generation = s.generation;
                    self.sweep = None;
                    self.updates.push(Update::SweepDone {
                        generation,
                        elapsed,
                        messages,
                    });
                }
            }
        }
        if let Some(r) = &mut self.refine
            && r.held.is_some()
        {
            let ready = match &r.estimate {
                None => Some(None),
                Some(slot) => slot.borrow().as_ref().map(|p| p.as_ref().ok().cloned()),
            };
            if let Some(estimate) = ready
                && let Some(mut point) = r.held.take()
            {
                if let Some(p) = estimate {
                    point = point.with_estimate(p.z_perfect + p.dz);
                }
                point.mhz = r.mhz;
                let generation = r.generation;
                self.refine = None;
                self.updates.push(Update::Refined { generation, point });
            }
        }
    }

    pub fn sweep(
        &mut self,
        generation: u64,
        deck: &NumericDeck,
        plan: FrequencyPlan,
        step: f64,
        buried: Option<Setup>,
    ) {
        self.cancel_sweep();
        let token = CancelToken::new();
        // Radials on or under the ground: their estimate runs alongside.
        let estimate = buried.map(|setup| {
            let (runner, deck, token) =
                (PoolRunner(self.pool.clone()), deck.clone(), token.clone());
            self.background(async move {
                buried::estimate_with(&runner, &deck, &setup, &plan, ESTIMATE_SAMPLES, &token).await
            })
        });
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
            estimate,
            token,
            held: Vec::new(),
            done: None,
        });
    }

    pub fn refine(
        &mut self,
        generation: u64,
        deck: &NumericDeck,
        mhz: f64,
        step: f64,
        buried: Option<Setup>,
    ) {
        self.cancel_refine();
        let token = CancelToken::new();
        let estimate = buried.map(|setup| {
            let (runner, deck, token) =
                (PoolRunner(self.pool.clone()), deck.clone(), token.clone());
            self.background(
                async move { buried::point_with(&runner, &deck, &setup, mhz, &token).await },
            )
        });
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
            estimate,
            token,
            held: None,
        });
    }

    pub fn cancel_refine(&mut self) {
        if let Some(r) = self.refine.take() {
            r.token.cancel();
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
        if let Some(r) = &mut self.refine
            && r.job == id
        {
            match result {
                // Handed on by `release`, with the estimate if there is one.
                Ok(raw) => match parse(&raw).frequencies.first() {
                    Some(f) => {
                        r.held = Some(
                            FreqPoint::from_result(f, Some(r.step)).without_helpers(&r.helpers),
                        );
                    }
                    None => self.refine = None,
                },
                Err(e) => {
                    let generation = r.generation;
                    self.cancel_refine();
                    self.updates.push(Update::RefineFailed {
                        generation,
                        message: explain(&e),
                    });
                }
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
                        // Handed on by `release`, with the estimate if any.
                        let point =
                            FreqPoint::from_result(f, Some(s.step)).without_helpers(&s.helpers);
                        s.held.push((index, point));
                    }
                    for m in out.messages {
                        if !s.messages.contains(&m) {
                            s.messages.push(m);
                        }
                    }
                    s.remaining -= 1;
                    if s.remaining == 0 {
                        s.done = Some((s.started.elapsed(), std::mem::take(&mut s.messages)));
                    }
                }
                Err(e) => {
                    self.cancel_sweep();
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
