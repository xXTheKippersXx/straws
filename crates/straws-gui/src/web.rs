//! The browser version: starting the app in the page's canvas, and the NEC
//! engine in Web Workers (`web/engine-worker.js`, running nec2c built for
//! WebAssembly), the browser's version of the desktop's worker processes.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::{Rc, Weak};
use std::time::Duration;

use eframe::egui;
use straws_core::deck::NumericDeck;
use straws_core::engine::{CancelToken, EngineError, RawRun, finish_run};
use straws_core::output::NecOutput;
use straws_core::runner::Runner;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Starts the app in the page's `<canvas id="straws">`.
pub fn start() {
    // Panics show in the browser's console instead of vanishing.
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("STRAWS: {info}").into());
    }));
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Some(canvas) = document
            .get_element_by_id("straws")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            return;
        };
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(crate::app::App::new(cc, Vec::new())))),
            )
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match started {
                Ok(()) => loading.remove(),
                Err(e) => loading.set_inner_html(&format!(
                    "<p>STRAWS couldn't start in this browser: {}</p>\
                     <p>It needs WebGPU or WebGL 2 (any recent Chrome, Edge, Firefox or Safari).</p>",
                    e.as_string().unwrap_or_else(|| format!("{e:?}"))
                )),
            }
        }
    });
}

/// Engine runs in Web Workers: each job is one deck, run in a fresh
/// instance of the engine. Jobs queue until a worker is free.
#[derive(Clone)]
pub struct EnginePool {
    inner: Rc<RefCell<Inner>>,
}

/// What to do with a job's result, if not left for [`EnginePool::take`].
type Callback = Box<dyn FnOnce(Result<RawRun, EngineError>)>;

struct Inner {
    ctx: egui::Context,
    slots: Vec<Slot>,
    queue: VecDeque<Queued>,
    done: Vec<(u64, Result<RawRun, EngineError>)>,
    callbacks: Vec<(u64, Callback)>,
    next_id: u64,
    /// Sommerfeld ground tables the workers have worked out (oldest first),
    /// passed on so no worker works one out twice.
    tables: VecDeque<(String, js_sys::Uint8Array)>,
}

/// Ground tables kept (17 kB each, in every worker).
const MAX_TABLES: usize = 400;

struct Queued {
    id: u64,
    deck: String,
    binary: bool,
}

struct Slot {
    worker: Option<web_sys::Worker>,
    job: Option<u64>,
    started: f64,
    /// Ground tables this worker holds.
    tables: std::collections::HashSet<String>,
    _on_message: Option<Closure<dyn FnMut(web_sys::MessageEvent)>>,
}

/// Workers running at once: the browser's count of cores, within reason.
fn pool_size() -> usize {
    let cores = web_sys::window()
        .map(|w| w.navigator().hardware_concurrency())
        .unwrap_or(4.0);
    (cores as usize).clamp(2, 8)
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

impl EnginePool {
    pub fn new(ctx: egui::Context) -> Self {
        let slots = (0..pool_size())
            .map(|_| Slot {
                worker: None,
                job: None,
                started: 0.0,
                tables: Default::default(),
                _on_message: None,
            })
            .collect();
        EnginePool {
            inner: Rc::new(RefCell::new(Inner {
                ctx,
                slots,
                queue: VecDeque::new(),
                done: Vec::new(),
                callbacks: Vec::new(),
                next_id: 1,
                tables: VecDeque::new(),
            })),
        }
    }

    /// How many jobs can run at once.
    pub fn size(&self) -> usize {
        self.inner.borrow().slots.len()
    }

    /// Queues a deck; its result comes back from [`EnginePool::take`].
    pub fn submit(&self, deck: String, binary: bool) -> u64 {
        self.queue(deck, binary, false)
    }

    /// Queues a deck ahead of everything waiting (the slider's exact run,
    /// which someone is looking at).
    pub fn submit_first(&self, deck: String, binary: bool) -> u64 {
        self.queue(deck, binary, true)
    }

    fn queue(&self, deck: String, binary: bool, first: bool) -> u64 {
        let id = {
            let mut inner = self.inner.borrow_mut();
            let id = inner.next_id;
            inner.next_id += 1;
            let job = Queued { id, deck, binary };
            if first {
                inner.queue.push_front(job);
            } else {
                inner.queue.push_back(job);
            }
            id
        };
        dispatch(&self.inner);
        id
    }

    /// Queues a deck whose result goes to `callback` instead of
    /// [`EnginePool::take`].
    pub fn submit_with(
        &self,
        deck: String,
        binary: bool,
        callback: impl FnOnce(Result<RawRun, EngineError>) + 'static,
    ) -> u64 {
        let id = self.submit(deck, binary);
        self.inner
            .borrow_mut()
            .callbacks
            .push((id, Box::new(callback)));
        // A job that failed at once is already done: hand it over now.
        deliver(&self.inner);
        id
    }

    /// Drops a job: unstarted, it never starts; running, its worker is
    /// stopped at once (like killing a worker process) and replaced.
    pub fn cancel(&self, id: u64) {
        {
            let mut inner = self.inner.borrow_mut();
            inner.queue.retain(|q| q.id != id);
            inner.done.retain(|(d, _)| *d != id);
            inner.callbacks.retain(|(d, _)| *d != id);
            for slot in &mut inner.slots {
                if slot.job == Some(id) {
                    if let Some(w) = slot.worker.take() {
                        w.terminate();
                    }
                    slot.job = None;
                    slot.tables.clear();
                    slot._on_message = None;
                }
            }
        }
        dispatch(&self.inner);
    }

    /// Results that have arrived since the last call.
    pub fn take(&self) -> Vec<(u64, Result<RawRun, EngineError>)> {
        let mut inner = self.inner.borrow_mut();
        let inner = &mut *inner;
        let (mine, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut inner.done)
            .into_iter()
            .partition(|(id, _)| !inner.callbacks.iter().any(|(j, _)| j == id));
        inner.done = kept;
        mine
    }
}

/// Hands finished jobs that have a callback to it (outside the borrow, so
/// a callback may submit more work).
fn deliver(inner_rc: &Rc<RefCell<Inner>>) {
    let ready: Vec<(Callback, Result<RawRun, EngineError>)> = {
        let mut inner = inner_rc.borrow_mut();
        let inner = &mut *inner;
        let mut ready = Vec::new();
        let mut k = 0;
        while k < inner.done.len() {
            let id = inner.done[k].0;
            if let Some(c) = inner.callbacks.iter().position(|(j, _)| *j == id) {
                let (_, callback) = inner.callbacks.remove(c);
                let (_, result) = inner.done.remove(k);
                ready.push((callback, result));
            } else {
                k += 1;
            }
        }
        ready
    };
    for (callback, result) in ready {
        callback(result);
    }
}

/// The page's one engine pool.
pub fn pool(ctx: &egui::Context) -> EnginePool {
    thread_local! {
        static POOL: RefCell<Option<EnginePool>> = const { RefCell::new(None) };
    }
    POOL.with(|p| {
        p.borrow_mut()
            .get_or_insert_with(|| EnginePool::new(ctx.clone()))
            .clone()
    })
}

/// Starts queued jobs on free workers (starting workers as needed).
fn dispatch(inner_rc: &Rc<RefCell<Inner>>) {
    let weak = Rc::downgrade(inner_rc);
    let mut inner = inner_rc.borrow_mut();
    for k in 0..inner.slots.len() {
        if inner.slots[k].job.is_some() {
            continue;
        }
        let Some(job) = inner.queue.pop_front() else {
            break;
        };
        if inner.slots[k].worker.is_none() {
            match new_worker(weak.clone(), k) {
                Ok((worker, on_message)) => {
                    inner.slots[k].worker = Some(worker);
                    inner.slots[k]._on_message = Some(on_message);
                }
                Err(e) => {
                    let message = format!("the browser couldn't start the engine: {e:?}");
                    inner.done.push((job.id, Err(failed(message))));
                    inner.ctx.request_repaint();
                    continue;
                }
            }
        }
        let message = js_sys::Object::new();
        let set = |key: &str, value: &JsValue| {
            let _ = js_sys::Reflect::set(&message, &key.into(), value);
        };
        set("id", &JsValue::from_f64(job.id as f64));
        set("deck", &JsValue::from_str(&job.deck));
        set("binary", &JsValue::from_bool(job.binary));
        // Ground tables this worker lacks, and ones it can forget.
        let add = js_sys::Array::new();
        let drop = js_sys::Array::new();
        {
            let inner = &mut *inner;
            let slot = &mut inner.slots[k];
            for (name, bytes) in &inner.tables {
                if slot.tables.insert(name.clone()) {
                    // A copy, so the pool keeps its own.
                    let copy = js_sys::Uint8Array::new(&bytes.buffer().slice(0));
                    add.push(&js_sys::Array::of2(
                        &JsValue::from_str(name),
                        &copy.buffer(),
                    ));
                }
            }
            slot.tables.retain(|name| {
                let kept = inner.tables.iter().any(|(n, _)| n == name);
                if !kept {
                    drop.push(&JsValue::from_str(name));
                }
                kept
            });
        }
        set("add", &add);
        set("drop", &drop);
        let slot = &mut inner.slots[k];
        if let Some(worker) = &slot.worker
            && worker.post_message(&message).is_ok()
        {
            slot.job = Some(job.id);
            slot.started = now_ms();
        } else {
            inner
                .done
                .push((job.id, Err(failed("the engine worker stopped".into()))));
        }
    }
}

fn failed(message: String) -> EngineError {
    EngineError::Failed {
        exit_code: None,
        messages: vec![message],
    }
}

type Started = (web_sys::Worker, Closure<dyn FnMut(web_sys::MessageEvent)>);

fn new_worker(pool: Weak<RefCell<Inner>>, slot: usize) -> Result<Started, JsValue> {
    let worker = web_sys::Worker::new("engine-worker.js")?;
    let on_message =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let Some(pool) = pool.upgrade() else { return };
            receive(&pool, slot, &event.data());
            deliver(&pool);
            dispatch(&pool);
        });
    worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    Ok((worker, on_message))
}

/// A worker's answer (see `web/engine-worker.js` for its fields).
fn receive(pool: &Rc<RefCell<Inner>>, slot: usize, data: &JsValue) {
    let get = |key: &str| js_sys::Reflect::get(data, &key.into()).unwrap_or(JsValue::UNDEFINED);
    let id = get("id").as_f64().unwrap_or(0.0) as u64;
    let mut inner = pool.borrow_mut();
    let Some(s) = inner.slots.get_mut(slot) else {
        return;
    };
    // A cancelled job's late answer (its worker was replaced) is dropped.
    if s.job != Some(id) {
        return;
    }
    s.job = None;
    let elapsed = Duration::from_secs_f64(((now_ms() - s.started) / 1000.0).max(0.0));
    let result = if let Some(error) = get("error").as_string() {
        // A crash inside the engine: the instance is gone, the worker isn't.
        Err(failed(format!("the engine crashed: {error}")))
    } else {
        let code = get("code").as_f64().map(|c| c as i32);
        let text = get("output").as_string().unwrap_or_default();
        let stderr = get("stderr").as_string().unwrap_or_default();
        let binary = get("binary");
        let binary = if binary.is_object() {
            js_sys::Uint8Array::new(&binary).to_vec()
        } else {
            Vec::new()
        };
        finish_run(code, text, &stderr, binary, elapsed)
    };
    // New ground tables: kept, to pass on to the other workers.
    let fresh = get("fresh");
    if let Some(fresh) = fresh.dyn_ref::<js_sys::Array>() {
        for entry in fresh.iter() {
            let pair = js_sys::Array::from(&entry);
            let Some(name) = pair.get(0).as_string() else {
                continue;
            };
            if inner.tables.iter().any(|(n, _)| *n == name) {
                continue;
            }
            inner.slots[slot].tables.insert(name.clone());
            inner
                .tables
                .push_back((name, js_sys::Uint8Array::new(&pair.get(1))));
            if inner.tables.len() > MAX_TABLES {
                inner.tables.pop_front();
            }
        }
    }
    inner.done.push((id, result));
    inner.ctx.request_repaint();
}

/// The engine Web Workers as a [`Runner`], for the analyses that run many
/// decks (the optimiser, the checks, the buried-radial estimate): each run
/// is a job for the pool, and its future finishes when the answer arrives,
/// so a batch of them runs on every worker at once.
#[derive(Clone)]
pub struct PoolRunner(pub EnginePool);

// SAFETY: the browser app is built without threads (wasm32 with no atomics),
// so nothing is ever shared between threads; the analyses only ask for Sync
// because on the desktop they spread work over threads.
unsafe impl Sync for PoolRunner {}

/// A job's answer, and who to wake when it comes.
type Answer = Rc<
    RefCell<(
        Option<Result<RawRun, EngineError>>,
        Option<std::task::Waker>,
    )>,
>;

impl Runner for PoolRunner {
    fn solve(
        &self,
        deck: &NumericDeck,
        cancel: &CancelToken,
    ) -> impl std::future::Future<Output = Result<NecOutput, EngineError>> {
        let answer: Answer = Rc::new(RefCell::new((None, None)));
        if !cancel.is_cancelled() {
            let slot = answer.clone();
            self.0.submit_with(deck.to_nec_text(), true, move |r| {
                let mut s = slot.borrow_mut();
                s.0 = Some(r);
                if let Some(w) = s.1.take() {
                    w.wake();
                }
            });
        }
        let cancel = cancel.clone();
        std::future::poll_fn(move |cx| {
            if cancel.is_cancelled() {
                return std::task::Poll::Ready(Err(EngineError::Cancelled));
            }
            let mut s = answer.borrow_mut();
            match s.0.take() {
                Some(r) => std::task::Poll::Ready(r.map(|raw| {
                    NecOutput::parse_with_binary(&raw.text, &raw.binary)
                        .unwrap_or_else(|| NecOutput::parse(&raw.text))
                })),
                None => {
                    s.1 = Some(cx.waker().clone());
                    std::task::Poll::Pending
                }
            }
        })
    }
}
