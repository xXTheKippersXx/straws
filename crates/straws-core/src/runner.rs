//! Running decks for the analyses that run many of them (the optimiser,
//! the convergence, tolerance and sensitivity checks, the buried-radial
//! estimate), on the desktop and in a browser alike.
//!
//! They're written as async functions against [`Runner`], which turns a
//! deck into its results. On the desktop the [`Engine`] is the runner: each
//! run finishes before its future is first polled, the sync wrappers wait
//! with [`block_on`], and [`par_map`] spreads work over threads. In the
//! browser app a runner hands decks to the engine's Web Workers and the
//! futures finish as their answers come back, so [`par_map`] runs a whole
//! batch at once without threads (which a browser page hasn't got).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake};

use crate::deck::NumericDeck;
use crate::engine::{CancelToken, Engine, EngineError};
use crate::output::NecOutput;

/// Turns decks into results.
pub trait Runner {
    fn solve(
        &self,
        deck: &NumericDeck,
        cancel: &CancelToken,
    ) -> impl Future<Output = Result<NecOutput, EngineError>>;
}

impl Runner for Engine {
    /// Runs the deck at once (in a worker process); the future is ready.
    fn solve(
        &self,
        deck: &NumericDeck,
        cancel: &CancelToken,
    ) -> impl Future<Output = Result<NecOutput, EngineError>> {
        std::future::ready(self.run(deck, cancel))
    }
}

/// Waits for a future on this thread (for the desktop's sync wrappers).
pub fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Arc::new(Unpark(std::thread::current())).into();
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// Waits for all of `futures`, giving their outputs in order.
pub async fn join_all<F: Future>(futures: Vec<F>) -> Vec<F::Output> {
    let mut futures: Vec<Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut outputs: Vec<Option<F::Output>> = futures.iter().map(|_| None).collect();
    std::future::poll_fn(move |cx| {
        let mut waiting = false;
        for (f, out) in futures.iter_mut().zip(outputs.iter_mut()) {
            if out.is_none() {
                match f.as_mut().poll(cx) {
                    Poll::Ready(v) => *out = Some(v),
                    Poll::Pending => waiting = true,
                }
            }
        }
        if waiting {
            Poll::Pending
        } else {
            Poll::Ready(outputs.drain(..).flatten().collect())
        }
    })
    .await
}

/// `f` on every item, several at once: on up to `threads` threads on the
/// desktop, all together in a browser (the runner queues the decks). The
/// outputs are in the items' order.
pub async fn par_map<T, U, F, Fut>(items: Vec<T>, threads: usize, f: F) -> Vec<U>
where
    T: Send,
    U: Send,
    F: Fn(T) -> Fut + Sync,
    Fut: Future<Output = U>,
{
    #[cfg(target_arch = "wasm32")]
    {
        let _ = threads;
        join_all(items.into_iter().map(f).collect()).await
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let n = items.len();
        if threads <= 1 || n <= 1 {
            let mut out = Vec::with_capacity(n);
            for item in items {
                out.push(f(item).await);
            }
            return out;
        }
        let items: Vec<Mutex<Option<T>>> = items.into_iter().map(|t| Mutex::new(Some(t))).collect();
        let outputs: Vec<Mutex<Option<U>>> = (0..n).map(|_| Mutex::new(None)).collect();
        let next = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..threads.min(n) {
                s.spawn(|| {
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(slot) = items.get(k) else { break };
                        let Some(item) = slot.lock().ok().and_then(|mut i| i.take()) else {
                            continue;
                        };
                        let out = block_on(f(item));
                        if let Ok(mut o) = outputs[k].lock() {
                            *o = Some(out);
                        }
                    }
                });
            }
        });
        outputs
            .into_iter()
            .filter_map(|o| o.into_inner().ok().flatten())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn par_map_keeps_the_order() {
        let out = block_on(par_map(
            (0..50).collect(),
            4,
            |k: usize| async move { k * k },
        ));
        assert_eq!(out, (0..50).map(|k| k * k).collect::<Vec<_>>());
        let ready = |k: i32| async move { k };
        let joined = block_on(join_all(vec![ready(1), ready(2)]));
        assert_eq!(joined, vec![1, 2]);
    }
}
