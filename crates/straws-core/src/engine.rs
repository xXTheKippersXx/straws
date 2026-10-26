//! Running decks through nec2c in worker processes.
//!
//! Each solve runs in its own short-lived process (see [`crate::worker`]),
//! which makes solves safe to run in parallel, impossible to crash the
//! caller, and instantly cancellable: cancelling kills the process.
//! Frequency sweeps are split across processes so they use every core.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crate::deck::{NumCard, NumericDeck};
use crate::output::{FrequencyResult, NecOutput};
use crate::worker::{SERVER_FLAG, WORKER_ENV, WORKER_FLAG};

#[derive(Debug)]
pub enum EngineError {
    /// Starting the worker or handling its files failed.
    Io(std::io::Error),
    /// The engine stopped with an error. `messages` holds what it reported.
    Failed {
        exit_code: Option<i32>,
        messages: Vec<String>,
    },
    /// The run was cancelled.
    Cancelled,
    /// A worker process tried to start workers of its own.
    InsideWorker,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Io(e) => write!(f, "could not run the NEC engine: {e}"),
            EngineError::Failed {
                exit_code,
                messages,
            } => {
                write!(f, "the NEC engine stopped with an error")?;
                if let Some(c) = exit_code {
                    write!(f, " (exit code {c})")?;
                }
                for m in messages {
                    write!(f, "\n  {m}")?;
                }
                Ok(())
            }
            EngineError::Cancelled => f.write_str("the run was cancelled"),
            EngineError::InsideWorker => f.write_str(
                "a NEC worker process tried to start another worker \
                 (the program's main() must call straws_core::worker::run_if_worker first)",
            ),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        EngineError::Io(e)
    }
}

/// A flag shared between a caller and running work; setting it cancels.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Raw output of one worker run.
#[derive(Debug, Clone)]
pub struct RawRun {
    pub text: String,
    /// Wall-clock time from starting the process to reading its output.
    pub elapsed: Duration,
    /// Far-field pattern points written in binary (see
    /// [`NecOutput::parse_with_binary`]), if the run asked for that.
    pub binary: Vec<u8>,
}

/// How to start a worker process.
#[derive(Debug, Clone)]
pub struct Engine {
    program: PathBuf,
    args: Vec<OsString>,
    /// Threads each worker may use for the matrix solve (`None`: all).
    solver_threads: Option<usize>,
    /// Workers keep and reuse Sommerfeld tables ([`somnec_cache`]).
    somnec_cache: bool,
    /// Runs go to persistent workers (see [`crate::worker::SERVER_FLAG`])
    /// rather than a new process each.
    persistent: bool,
}

/// Options for [`Engine::sweep`].
#[derive(Debug, Clone, Copy)]
pub struct SweepOptions {
    /// Worker processes to run at once.
    pub threads: usize,
    /// Split the sweep into about this many jobs per thread. More jobs make
    /// results arrive in more, smaller batches, at the cost of starting more
    /// processes.
    pub jobs_per_thread: usize,
    /// Least estimated work (ms) worth a worker process of its own. Starting
    /// a worker costs ~15-25 ms on Windows, so smaller sweeps run whole.
    pub min_job_ms: f64,
}

impl Default for SweepOptions {
    fn default() -> Self {
        Self {
            threads: std::thread::available_parallelism().map_or(4, |n| n.get()),
            jobs_per_thread: 1,
            min_job_ms: 60.0,
        }
    }
}

static JOB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The folder nec2c keeps its Sommerfeld ground tables in (see
/// `somnec.c`): they depend only on the ground and the frequency, so later
/// runs read them instead of working them out again. Made once per
/// process, trimmed to the newest files when it has grown past
/// [`SOMNEC_CACHE_BYTES`]. `None` if the folder can't be made.
pub fn somnec_cache() -> Option<&'static Path> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join("straws-sommerfeld");
        std::fs::create_dir_all(&dir).ok()?;
        trim_cache(&dir, SOMNEC_CACHE_BYTES);
        Some(dir)
    })
    .as_deref()
}

/// Most bytes kept in the Sommerfeld table cache (about 1,900 frequencies
/// and grounds, 17 KB each).
pub const SOMNEC_CACHE_BYTES: u64 = 32 << 20;

/// Deletes the oldest files in `dir` until the rest fit in `limit` bytes.
fn trim_cache(dir: &Path, limit: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            if !m.is_file() {
                return None;
            }
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

fn cores() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

impl Engine {
    /// Uses the current program as its own worker. Its `main()` must call
    /// [`crate::worker::run_if_worker`] before anything else.
    pub fn self_hosted() -> std::io::Result<Engine> {
        Ok(Engine {
            program: std::env::current_exe()?,
            args: vec![WORKER_FLAG.into()],
            solver_threads: None,
            somnec_cache: true,
            persistent: std::env::var_os("STRAWS_FRESH_WORKERS").is_none(),
        })
    }

    /// Uses a standalone worker program, such as `straws-worker`.
    pub fn with_worker_program(path: impl Into<PathBuf>) -> Engine {
        Engine {
            program: path.into(),
            args: Vec::new(),
            solver_threads: None,
            somnec_cache: true,
            persistent: std::env::var_os("STRAWS_FRESH_WORKERS").is_none(),
        }
    }

    /// Limits each worker to `threads` threads for the matrix fill and solve
    /// (by default a single run uses every core).
    pub fn with_solver_threads(mut self, threads: usize) -> Engine {
        self.solver_threads = Some(threads.max(1));
        self
    }

    /// Every run in a new worker process, as before persistent workers (for
    /// tests and timing; `STRAWS_FRESH_WORKERS` does the same for every
    /// engine).
    pub fn fresh_processes(mut self) -> Engine {
        self.persistent = false;
        self
    }

    /// Workers work out the Sommerfeld tables every time (for tests and
    /// timing).
    pub fn without_somnec_cache(mut self) -> Engine {
        self.somnec_cache = false;
        self
    }

    /// Starts a worker on deck text and returns at once.
    pub fn start(&self, deck_text: &str) -> Result<Job, EngineError> {
        self.start_with(deck_text, false)
    }

    /// Like [`Engine::start`]; with `binary`, far-field pattern points are
    /// written to a binary file instead of the text (full precision, and
    /// much quicker for big patterns).
    fn start_with(&self, deck_text: &str, binary: bool) -> Result<Job, EngineError> {
        if std::env::var_os(WORKER_ENV).is_some() {
            return Err(EngineError::InsideWorker);
        }
        let dir = JobDir::create()?;
        std::fs::write(dir.path().join("deck.nec"), deck_text)?;
        if self.persistent {
            return self.start_on_server(dir, binary);
        }
        let stderr = std::fs::File::create(dir.path().join("stderr.txt"))?;
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            // Short relative names: nec2c limits file names to 75 characters.
            .args(["-i", "deck.nec", "-o", "deck.out"])
            .current_dir(dir.path())
            .env(WORKER_ENV, "1")
            .env(
                "STRAWS_BINARY_PATTERN",
                if binary { "deck.pat" } else { "" },
            )
            .env(
                "STRAWS_SOMNEC_CACHE",
                somnec_cache()
                    .filter(|_| self.somnec_cache)
                    .map(Path::as_os_str)
                    .unwrap_or_default(),
            )
            .env(
                crate::solver::THREADS_ENV,
                self.solver_threads.unwrap_or_else(cores).to_string(),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console at all (its output goes to files): with
            // CREATE_NO_WINDOW, Windows still made a hidden console and a
            // conhost.exe for every worker, about 13 ms a run.
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            cmd.creation_flags(DETACHED_PROCESS);
        }
        let started = Instant::now();
        let child = cmd.spawn()?;
        Ok(Job {
            work: Work::Process(child),
            dir,
            started,
        })
    }

    /// Hands the job in `dir` to an idle persistent worker (starting one if
    /// none is free).
    fn start_on_server(&self, dir: JobDir, binary: bool) -> Result<Job, EngineError> {
        let key = (self.program.clone(), self.somnec_cache);
        let threads = self.solver_threads.unwrap_or_else(cores);
        let line = format!(
            "RUN\t{}\t{threads}\t{}\n",
            dir.path().to_string_lossy(),
            u8::from(binary)
        );
        let started = Instant::now();
        // An idle worker may have died since; then start a new one.
        for _ in 0..3 {
            let idle = IDLE_SERVERS.lock().ok().and_then(|mut idle| {
                let k = idle.iter().position(|s| s.key == key)?;
                Some(idle.swap_remove(k))
            });
            let mut server = match idle {
                Some(s) => s,
                None => self.spawn_server()?,
            };
            if server.send(&line).is_ok() {
                return Ok(Job {
                    work: Work::Server(Some(server)),
                    dir,
                    started,
                });
            }
        }
        Err(EngineError::Io(std::io::Error::other(
            "couldn't start a persistent worker",
        )))
    }

    fn spawn_server(&self) -> std::io::Result<Server> {
        let mut cmd = Command::new(&self.program);
        cmd.arg(SERVER_FLAG)
            .env(WORKER_ENV, "1")
            .env(
                "STRAWS_SOMNEC_CACHE",
                somnec_cache()
                    .filter(|_| self.somnec_cache)
                    .map(Path::as_os_str)
                    .unwrap_or_default(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            cmd.creation_flags(DETACHED_PROCESS);
        }
        let mut child = cmd.spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("no pipes to the worker"));
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Server {
            key: (self.program.clone(), self.somnec_cache),
            child,
            stdin: Some(stdin),
            rx,
        })
    }

    /// Runs deck text to completion.
    pub fn run_text(&self, deck_text: &str, cancel: &CancelToken) -> Result<RawRun, EngineError> {
        self.start(deck_text)?.wait(cancel)
    }

    /// Runs deck text, stopping the engine if it takes longer than `limit`.
    pub fn run_text_with_limit(
        &self,
        deck_text: &str,
        limit: Duration,
    ) -> Result<RawRun, EngineError> {
        let cancel = CancelToken::new();
        let job = self.start(deck_text)?;
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let watchdog = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                if done_rx.recv_timeout(limit) == Err(mpsc::RecvTimeoutError::Timeout) {
                    cancel.cancel();
                }
            })
        };
        let result = job.wait(&cancel);
        let _ = done_tx.send(());
        let _ = watchdog.join();
        result
    }

    /// Runs a deck as one job and parses the results.
    pub fn run(&self, deck: &NumericDeck, cancel: &CancelToken) -> Result<NecOutput, EngineError> {
        let raw = self.start_with(&deck.to_nec_text(), true)?.wait(cancel)?;
        match NecOutput::parse_with_binary(&raw.text, &raw.binary) {
            Some(out) => Ok(out),
            // A short binary file (not seen in testing, but once a pattern
            // came back unusable): run it again as plain text.
            None => Ok(NecOutput::parse(
                &self.run_text(&deck.to_nec_text(), cancel)?.text,
            )),
        }
    }

    /// Runs a deck, splitting its frequency sweep across worker processes.
    /// `on_result` is called with each frequency's index and result as soon
    /// as it is available (in no particular order). The returned output has
    /// every frequency in sweep order.
    ///
    /// Decks with more than one FR card, or that change sources, loads or
    /// ground between calculations, run as a single job.
    pub fn sweep(
        &self,
        deck: &NumericDeck,
        options: SweepOptions,
        cancel: &CancelToken,
        mut on_result: impl FnMut(usize, &FrequencyResult),
    ) -> Result<NecOutput, EngineError> {
        let threads = options.threads.max(1);
        let total = deck.frequencies().len();
        let worth_it = if options.min_job_ms > 0.0 {
            (estimated_run_ms(deck) / options.min_job_ms).floor() as usize
        } else {
            usize::MAX
        };
        let parts = (threads * options.jobs_per_thread.max(1))
            .min(total)
            .min(worth_it);
        let jobs = split_sweep(deck, parts);
        let Some(jobs) = jobs else {
            let out = self.run(deck, cancel)?;
            for (i, f) in out.frequencies.iter().enumerate() {
                on_result(i, f);
            }
            self.tables_cached(deck);
            return Ok(out);
        };

        // Share the cores between the workers running at once, so each
        // worker's multi-threaded solve doesn't fight the others.
        let running = threads.min(jobs.len()).max(1);
        let engine = Engine {
            solver_threads: Some((cores() / running).max(1)),
            ..self.clone()
        };
        let engine = &engine;
        let next = AtomicUsize::new(0);
        let inner_cancel = CancelToken::new();
        let (tx, rx) = mpsc::channel::<(usize, Result<NecOutput, EngineError>)>();
        let mut results: Vec<Option<NecOutput>> = vec![None; jobs.len()];
        let mut frequencies: Vec<Option<FrequencyResult>> = vec![None; total];
        let mut first_error = None;

        std::thread::scope(|scope| {
            for _ in 0..threads.min(jobs.len()) {
                let tx = tx.clone();
                let (jobs, next, inner_cancel) = (&jobs, &next, &inner_cancel);
                scope.spawn(move || {
                    loop {
                        let k = next.fetch_add(1, Ordering::SeqCst);
                        if k >= jobs.len() || inner_cancel.is_cancelled() {
                            break;
                        }
                        let result = engine.run(&jobs[k].deck, inner_cancel);
                        if tx.send((k, result)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tx);

            // Collect on this thread so `on_result` needs no locking. Poll
            // so an outside cancel reaches the workers promptly.
            let mut remaining = jobs.len();
            while remaining > 0 {
                if cancel.is_cancelled() {
                    inner_cancel.cancel();
                }
                let (k, result) = match rx.recv_timeout(Duration::from_millis(5)) {
                    Ok(msg) => msg,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                remaining -= 1;
                match result {
                    Ok(out) => {
                        for (j, f) in out.frequencies.iter().enumerate() {
                            if let Some(&index) = jobs[k].indices.get(j) {
                                on_result(index, f);
                                frequencies[index] = Some(f.clone());
                            }
                        }
                        results[k] = Some(out);
                    }
                    Err(e) => {
                        if first_error.is_none() {
                            first_error = Some(e);
                        }
                        inner_cancel.cancel();
                    }
                }
            }
        });

        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        if let Some(e) = first_error {
            return Err(e);
        }
        let mut combined = NecOutput::default();
        for out in results.iter().flatten() {
            if combined.segments.is_empty() {
                combined.segments = out.segments.clone();
            }
            for m in &out.messages {
                if !combined.messages.contains(m) {
                    combined.messages.push(m.clone());
                }
            }
        }
        combined.frequencies = frequencies.into_iter().flatten().collect();
        self.tables_cached(deck);
        Ok(combined)
    }

    /// Notes that `deck`'s Sommerfeld tables are now in the cache.
    fn tables_cached(&self, deck: &NumericDeck) {
        if self.somnec_cache
            && somnec_cache().is_some()
            && let Some(key) = sommerfeld_key(deck)
            && let Ok(mut warm) = WARM_TABLES.lock()
        {
            warm.insert(key);
        }
    }
}

/// Grounds and frequencies whose Sommerfeld tables this program's workers
/// have cached (see [`sommerfeld_key`]), so sweeps of them are quick.
static WARM_TABLES: std::sync::Mutex<std::collections::BTreeSet<Vec<u64>>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// The ground card's numbers and every frequency in millihertz (the key
/// nec2c's cache uses), for a deck over Sommerfeld ground.
fn sommerfeld_key(deck: &NumericDeck) -> Option<Vec<u64>> {
    if !deck.uses_sommerfeld() {
        return None;
    }
    let gn = deck.cards.iter().rev().find(|c| c.is("GN"))?;
    let mut key: Vec<u64> = gn.floats.iter().map(|v| v.to_bits()).collect();
    key.extend(deck.frequencies().iter().map(|f| (f * 1e9).round() as u64));
    Some(key)
}

/// A run in progress: its own worker process, or a job on a persistent
/// worker.
pub struct Job {
    work: Work,
    dir: JobDir,
    started: Instant,
}

enum Work {
    Process(Child),
    /// Taken back (to the idle pool) once the job is done.
    Server(Option<Server>),
}

/// A persistent worker (see [`crate::worker::SERVER_FLAG`]).
struct Server {
    /// The program it runs and whether it caches Sommerfeld tables.
    key: (PathBuf, bool),
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    /// Lines it writes (`DONE <tab> code`), read on a thread of their own.
    rx: mpsc::Receiver<String>,
}

impl Server {
    fn send(&mut self, line: &str) -> std::io::Result<()> {
        use std::io::Write;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("closed"))?;
        stdin.write_all(line.as_bytes())?;
        stdin.flush()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Closing its stdin ends an idle worker; a busy one is stopped.
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Persistent workers waiting for work.
static IDLE_SERVERS: std::sync::Mutex<Vec<Server>> = std::sync::Mutex::new(Vec::new());

/// Most idle persistent workers kept (more are stopped when their job ends).
fn kept_servers() -> usize {
    2 * cores()
}

impl Job {
    /// Stops the worker at once.
    pub fn kill(&mut self) {
        match &mut self.work {
            Work::Process(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            // Dropping the server stops it; it never goes back to the pool.
            Work::Server(server) => *server = None,
        }
    }

    /// Waits for the worker to finish, killing it if `cancel` is set. The
    /// exit code (`None` if the worker died).
    fn finish(&mut self, cancel: &CancelToken) -> Result<Option<i32>, EngineError> {
        match &mut self.work {
            Work::Process(child) => {
                let mut pause = Duration::from_micros(100);
                loop {
                    if cancel.is_cancelled() {
                        self.kill();
                        return Err(EngineError::Cancelled);
                    }
                    if let Some(status) = child.try_wait()? {
                        return Ok(if status.success() {
                            Some(0)
                        } else {
                            Some(status.code().unwrap_or(-1))
                        });
                    }
                    std::thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_millis(2));
                }
            }
            Work::Server(server) => {
                let Some(s) = server else {
                    return Ok(None);
                };
                loop {
                    if cancel.is_cancelled() {
                        self.kill();
                        return Err(EngineError::Cancelled);
                    }
                    match s.rx.recv_timeout(Duration::from_millis(5)) {
                        Ok(line) => {
                            let Some(code) = line.strip_prefix("DONE\t") else {
                                continue;
                            };
                            let code = code.trim().parse().unwrap_or(-1);
                            // Back to the pool for the next job.
                            if let Some(s) = server.take()
                                && let Ok(mut idle) = IDLE_SERVERS.lock()
                                && idle.len() < kept_servers()
                            {
                                idle.push(s);
                            }
                            return Ok(Some(code));
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            // It crashed: it never goes back to the pool.
                            *server = None;
                            return Ok(None);
                        }
                    }
                }
            }
        }
    }

    /// Waits for the worker to finish, killing it if `cancel` is set.
    pub fn wait(mut self, cancel: &CancelToken) -> Result<RawRun, EngineError> {
        let code = self.finish(cancel)?;
        let text = std::fs::read(self.dir.path().join("deck.out"))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let elapsed = self.started.elapsed();
        if code == Some(0) {
            let binary = std::fs::read(self.dir.path().join("deck.pat")).unwrap_or_default();
            return Ok(RawRun {
                text,
                elapsed,
                binary,
            });
        }
        let mut messages = Vec::new();
        if let Ok(stderr) = std::fs::read_to_string(self.dir.path().join("stderr.txt")) {
            messages.extend(
                stderr
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(String::from),
            );
        }
        messages.extend(engine_error_lines(&text));
        Err(EngineError::Failed {
            exit_code: code,
            messages,
        })
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        match &mut self.work {
            Work::Process(child) => {
                if let Ok(None) = child.try_wait() {
                    self.kill();
                }
            }
            // Still holding its server: the job wasn't waited for, so the
            // server may be busy; stop it.
            Work::Server(server) => *server = None,
        }
    }
}

/// The error text nec2c wrote at the end of its output before stopping.
fn engine_error_lines(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(12)..];
    let start = tail
        .iter()
        .position(|l| l.contains("ERROR") || l.contains("error"));
    match start {
        Some(s) => tail[s..]
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        None => Vec::new(),
    }
}

/// A temporary folder for one job. Folders are reused (making and deleting
/// one cost about 9 ms a run on Windows): a finished job's folder goes back
/// to a pool, and its files are deleted before the next job uses it.
struct JobDir(PathBuf);

/// Job folders free for reuse.
static FREE_DIRS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Most folders kept for reuse (more are deleted when their job ends).
const KEPT_DIRS: usize = 64;

/// Files a job leaves in its folder.
const JOB_FILES: [&str; 4] = ["deck.nec", "deck.out", "deck.pat", "stderr.txt"];

impl JobDir {
    fn create() -> std::io::Result<JobDir> {
        let reused = FREE_DIRS.lock().ok().and_then(|mut free| free.pop());
        if let Some(path) = reused {
            for f in JOB_FILES {
                match std::fs::remove_file(path.join(f)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    // Can't be cleared (still locked?): use a new folder.
                    Err(_) => return JobDir::fresh(),
                }
            }
            return Ok(JobDir(path));
        }
        JobDir::fresh()
    }

    fn fresh() -> std::io::Result<JobDir> {
        let root = std::env::temp_dir().join("straws");
        static TIDIED: std::sync::Once = std::sync::Once::new();
        TIDIED.call_once(|| tidy_old_dirs(&root));
        let n = JOB_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = root.join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(JobDir(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for JobDir {
    fn drop(&mut self) {
        if let Ok(mut free) = FREE_DIRS.lock()
            && free.len() < KEPT_DIRS
        {
            free.push(std::mem::take(&mut self.0));
            return;
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Deletes job folders other programs left behind (kept for reuse until
/// they quit): those untouched for an hour.
fn tidy_old_dirs(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let hour = Duration::from_secs(3600);
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > hour));
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Rough single-process run time of a deck in milliseconds, used only to
/// decide how far to split a sweep. Calibrated on the benchmark set
/// (docs/benchmarks.md) on the dev machine: per frequency, filling the
/// matrix costs about 0.00013·N² ms for N unknowns and solving it about
/// 6e-7·N³ ms; Sommerfeld-Norton ground adds about 14 + 0.00046·N² ms.
pub fn estimated_run_ms(deck: &NumericDeck) -> f64 {
    let n = deck.estimated_unknowns() as f64;
    let mut per_freq = 1.3e-4 * n * n + 6e-7 * n * n * n;
    if deck.uses_sommerfeld() {
        // Working out the ground tables, unless this program has already
        // cached them (then they're read back in well under a millisecond).
        let cached =
            sommerfeld_key(deck).is_some_and(|k| WARM_TABLES.lock().is_ok_and(|w| w.contains(&k)));
        per_freq += if cached { 0.5 } else { 14.0 } + 4.6e-4 * n * n;
    }
    per_freq * deck.frequencies().len().max(1) as f64
}

/// One piece of a split sweep: a deck and which overall frequencies
/// (indices into the full sweep) it computes, in order.
#[derive(Debug, Clone)]
pub struct SweepJob {
    pub deck: NumericDeck,
    pub indices: Vec<usize>,
}

/// Splits a single-FR-card sweep into up to `parts` interleaved jobs: job
/// `p` computes frequencies p, p+N, p+2N, ... so early results are spread
/// across the band. Returns `None` if the deck can't be split safely.
pub fn split_sweep(deck: &NumericDeck, parts: usize) -> Option<Vec<SweepJob>> {
    let fr: Vec<usize> = deck
        .cards
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is("FR"))
        .map(|(i, _)| i)
        .collect();
    if fr.len() != 1 || parts < 2 {
        return None;
    }
    let fr = fr[0];
    // Anything after the first calculation that changes the problem means
    // the deck is a multi-step run; keep it whole.
    let first_exec = deck
        .cards
        .iter()
        .position(|c| crate::deck::expand::EXECUTE_CARDS.contains(&c.mnemonic.as_str()));
    if let Some(e) = first_exec {
        let changes_later = deck.cards[e..].iter().any(|c| {
            matches!(
                c.mnemonic.as_str(),
                "EX" | "LD" | "GN" | "NT" | "TL" | "FR" | "GD" | "EK" | "KH" | "PT" | "NX"
            )
        });
        if e < fr || changes_later {
            return None;
        }
    }
    let card = &deck.cards[fr];
    let n = card.ints[1].max(1) as usize;
    let parts = parts.min(n);
    if parts < 2 {
        return None;
    }
    let (start, step, log) = (card.floats[0], card.floats[1], card.ints[0] == 1);
    let mut jobs = Vec::with_capacity(parts);
    for p in 0..parts {
        let indices: Vec<usize> = (p..n).step_by(parts).collect();
        let (s, d) = if log {
            (start * step.powi(p as i32), step.powi(parts as i32))
        } else {
            (start + step * p as f64, step * parts as f64)
        };
        let mut sub = deck.clone();
        let mut new_fr = NumCard::new("FR", &[card.ints[0], indices.len() as i64], &[s, d]);
        new_fr.line = card.line;
        sub.cards[fr] = new_fr;
        jobs.push(SweepJob { deck: sub, indices });
    }
    Some(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deck::Deck;

    #[test]
    fn splits_sweeps_into_interleaved_jobs() {
        let deck =
            Deck::parse("CE\nGW 1 9 0 0 -1 0 0 1 .001\nGE\nEX 0 1 5 0 1\nFR 0 7 0 0 10 1\nXQ\n")
                .expand()
                .unwrap();
        let jobs = split_sweep(&deck, 3).unwrap();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].indices, vec![0, 3, 6]);
        assert_eq!(jobs[2].indices, vec![2, 5]);
        assert_eq!(jobs[1].deck.frequencies(), vec![11.0, 14.0]);
        assert_eq!(jobs[0].deck.frequencies(), vec![10.0, 13.0, 16.0]);
    }

    #[test]
    fn keeps_multi_step_decks_whole() {
        let deck = Deck::parse(
            "CE\nGW 1 9 0 0 -1 0 0 1 .001\nGE\nEX 0 1 5 0 1\nFR 0 7 0 0 10 1\nXQ\nLD 5 0 0 0 5.8e7\nXQ\n",
        )
        .expand()
        .unwrap();
        assert!(split_sweep(&deck, 3).is_none());
    }
}
