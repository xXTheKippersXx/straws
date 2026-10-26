//! The worker side of the engine.
//!
//! nec2c keeps its working data in global and `static` variables and calls
//! `exit()` on any input error, so it must never run inside the app itself.
//! Every solve runs in a worker process: either the standalone
//! `straws-worker` binary, or any program built on straws-core that calls
//! [`run_if_worker`] first thing in `main()` (the app and the CLI do this, so
//! they can be their own workers).
//!
//! A worker either runs one deck and ends ([`WORKER_FLAG`]), or stays
//! running and takes deck after deck ([`SERVER_FLAG`]): nec2c's `stop()`
//! then jumps back instead of exiting, and `straws_reset()` puts its globals
//! back as a fresh process has them (see `straws_run()` in nec2c's main.c).
//! Starting a process costs about 16 ms a run on Windows.

use std::ffi::{CString, OsString, c_char, c_int};

/// Command-line flag that tells a program to act as a nec2c worker.
pub const WORKER_FLAG: &str = "--nec2c-worker";

/// Command-line flag for a worker that stays running: it reads one job a
/// line from stdin (`RUN <tab> folder <tab> solver threads <tab> binary
/// patterns 0/1`), runs `deck.nec` in that folder, and answers
/// `DONE <tab> exit code`. It ends when stdin closes.
pub const SERVER_FLAG: &str = "--nec2c-server";

/// Environment variable set on every worker process. The engine refuses to
/// start workers from inside a worker, so a program that forgets to call
/// [`run_if_worker`] can never spawn copies of itself without end.
pub const WORKER_ENV: &str = "STRAWS_WORKER";

unsafe extern "C" {
    /// nec2c's run-and-return entry (main.c, straws: persistent workers).
    fn straws_run(
        argc: c_int,
        argv: *mut *mut c_char,
        stderr_path: *const c_char,
        binary_path: *const c_char,
    ) -> c_int;

    /// nec2c's original `main()`, renamed at compile time (see build.rs).
    fn nec2c_main(argc: c_int, argv: *mut *mut c_char) -> c_int;
}

/// If this process was started as a nec2c worker, runs the solve and exits.
/// Otherwise returns immediately. Call it at the very start of `main()`.
pub fn run_if_worker() {
    let mut args = std::env::args_os();
    let _program = args.next();
    let flag = args.next();
    if flag.as_deref() == Some(SERVER_FLAG.as_ref()) {
        serve();
        std::process::exit(0);
    }
    if flag.as_deref() != Some(WORKER_FLAG.as_ref()) {
        return;
    }
    let code = run_nec2c(args.collect());
    std::process::exit(code);
}

/// The persistent worker's loop (see [`SERVER_FLAG`]).
pub fn serve() {
    use std::io::{BufRead, Write};
    keep_rust_symbols();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let args: Vec<CString> = ["nec2c", "-i", "deck.nec", "-o", "deck.out"]
        .into_iter()
        .map(|a| CString::new(a).expect("no NUL"))
        .collect();
    let stderr_path = CString::new("stderr.txt").expect("no NUL");
    let binary_path = CString::new("deck.pat").expect("no NUL");
    let no_binary = CString::new("").expect("no NUL");
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let parts: Vec<&str> = line.split('\t').collect();
        let ["RUN", folder, threads, binary] = parts[..] else {
            break;
        };
        let code = if std::env::set_current_dir(folder).is_err() {
            -1
        } else {
            crate::solver::set_threads(threads.parse().unwrap_or(0));
            let mut argv: Vec<*mut c_char> = args.iter().map(|s| s.as_ptr().cast_mut()).collect();
            argv.push(std::ptr::null_mut());
            let bin = if binary == "1" {
                &binary_path
            } else {
                &no_binary
            };
            // SAFETY: argv is a NULL-terminated array of valid C strings
            // that outlive the call, and the paths are valid C strings;
            // nec2c only reads them. One run at a time, on this thread.
            unsafe {
                straws_run(
                    (args.len()) as c_int,
                    argv.as_mut_ptr(),
                    stderr_path.as_ptr(),
                    bin.as_ptr(),
                )
            }
        };
        if writeln!(stdout, "DONE\t{code}")
            .and_then(|()| stdout.flush())
            .is_err()
        {
            break;
        }
    }
}

/// nec2c calls these Rust functions (see solver.rs). Referencing them here
/// makes sure single-pass linkers keep them.
fn keep_rust_symbols() {
    std::hint::black_box((
        crate::solver::nec_factr as *const () as usize,
        crate::solver::nec_solve as *const () as usize,
        crate::solver::nec_fill_threads as *const () as usize,
        crate::solver::nec_parallel_for as *const () as usize,
    ));
}

/// Runs nec2c with the given arguments (normally `-i deck -o output`).
/// nec2c usually ends the process itself via `exit()`; the return value is
/// only used if it ever returns normally.
pub fn run_nec2c(args: Vec<OsString>) -> i32 {
    keep_rust_symbols();
    let mut owned: Vec<CString> = vec![CString::new("nec2c").expect("no NUL")];
    for arg in args {
        let text = arg.to_string_lossy().into_owned();
        owned.push(CString::new(text).unwrap_or_else(|_| CString::new("").expect("no NUL")));
    }
    let mut argv: Vec<*mut c_char> = owned.iter().map(|s| s.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    let argc = c_int::try_from(owned.len()).expect("few arguments");
    // SAFETY: argv is a NULL-terminated array of valid C strings that outlive
    // the call. nec2c only reads them. This runs in a dedicated worker
    // process, so nec2c's global state and exit() calls affect nothing else.
    unsafe { nec2c_main(argc, argv.as_mut_ptr()) }
}
