//! A fast replacement for nec2c's matrix factorisation and solve.
//!
//! nec2c spends most of a large run in `factr()`, a textbook one-column-at-
//! a-time LU factorisation on one core. Its `factr()` and `solve()` now call
//! these functions instead, which use faer's blocked, vectorised and
//! multi-threaded LU with partial pivoting: the same maths, many times
//! faster. Set `NEC_SOLVER=classic` to use nec2c's original code (kept for
//! comparison).
//!
//! nec2c hands `factr()` the matrix transposed: in memory, `a[i + j*ndim]`
//! is element (j, i) of the system matrix A. Read column-major, that memory
//! is Aᵀ, so we factor Aᵀ in place (Aᵀ = P·L·U) and solve A·x = b with the
//! transposed solve. The factors are only ever read by `nec_solve`, so their
//! layout doesn't have to match nec2c's LINPACK-style one.

use std::ffi::c_int;
use std::sync::OnceLock;

use faer::dyn_stack::{MemBuffer, MemStack};
use faer::linalg::lu::partial_pivoting::{factor, solve};
use faer::perm::PermRef;
use faer::{Conj, MatMut, MatRef, Par, c64};

/// Environment variable giving the threads each engine worker may use for
/// the factorisation (the engine sets it so parallel workers don't fight
/// over cores).
pub const THREADS_ENV: &str = "NEC_SOLVER_THREADS";

/// Threads for the next runs of a persistent worker (0: from
/// [`THREADS_ENV`]).
static THREADS_NOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Sets the threads a persistent worker's next runs may use (0: back to
/// [`THREADS_ENV`]).
pub fn set_threads(threads: usize) {
    THREADS_NOW.store(threads, std::sync::atomic::Ordering::Relaxed);
}

/// Threads this worker may use (set for this run, else from
/// [`THREADS_ENV`], else all cores).
fn threads() -> usize {
    static THREADS: OnceLock<usize> = OnceLock::new();
    let now = THREADS_NOW.load(std::sync::atomic::Ordering::Relaxed);
    if now > 0 {
        return now;
    }
    *THREADS.get_or_init(|| {
        std::env::var(THREADS_ENV)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
            .max(1)
    })
}

fn factor_par() -> Par {
    match threads() {
        1 => Par::Seq,
        n => Par::rayon(n),
    }
}

/// Threads nec2c's matrix fill may use (see `cmset()` in matrix.c).
#[unsafe(no_mangle)]
pub extern "C" fn nec_fill_threads() -> c_int {
    c_int::try_from(threads()).unwrap_or(1)
}

/// A pointer handed to fill threads. The C side guarantees the threads
/// only write disjoint parts of what it points to.
struct SharedCtx(*mut std::ffi::c_void);
// SAFETY: see above; nec2c's fill threads write disjoint matrix columns and
// keep their scratch state thread-local.
unsafe impl Send for SharedCtx {}
unsafe impl Sync for SharedCtx {}

/// Runs `task(part, parts, ctx)` for every part on its own thread and waits
/// for them all. Used by nec2c's parallel matrix fill.
///
/// # Safety
/// `task` must be safe to call concurrently with the same `ctx`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nec_parallel_for(
    parts: c_int,
    task: extern "C" fn(c_int, c_int, *mut std::ffi::c_void),
    ctx: *mut std::ffi::c_void,
) {
    let shared = SharedCtx(ctx);
    let shared = &shared;
    std::thread::scope(|scope| {
        for part in 0..parts {
            scope.spawn(move || task(part, parts, shared.0));
        }
    });
}

/// Factors the n×n system matrix stored (transposed) in `a` with leading
/// dimension `ndim`, in place. The row permutation goes into `ip`.
///
/// # Safety
/// `a` must point to at least `ndim * (n - 1) + n` complex values and `ip`
/// to `n` ints, as nec2c's `factr()` guarantees.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nec_factr(n: c_int, a: *mut c64, ip: *mut c_int, ndim: c_int) {
    let (n, ndim) = (n.max(0) as usize, ndim.max(0) as usize);
    if n == 0 {
        return;
    }
    // SAFETY: the caller guarantees the extent; column-major view of Aᵀ.
    let mat = unsafe { MatMut::from_raw_parts_mut(a, n, n, 1, ndim as isize) };
    let mut perm = vec![0usize; n];
    let mut perm_inv = vec![0usize; n];
    let par = factor_par();
    let req = factor::lu_in_place_scratch::<usize, c64>(n, n, par, Default::default());
    let mut buf = MemBuffer::new(req);
    factor::lu_in_place(
        mat,
        &mut perm,
        &mut perm_inv,
        par,
        MemStack::new(&mut buf),
        Default::default(),
    );
    for (i, p) in perm.iter().enumerate() {
        // SAFETY: `ip` has n entries.
        unsafe { *ip.add(i) = *p as c_int };
    }
}

/// Solves A·x = b using the factors from [`nec_factr`]; `b` is overwritten
/// with x.
///
/// # Safety
/// `a` and `ip` must hold a factorisation from `nec_factr` with the same `n`
/// and `ndim`, and `b` must point to `n` complex values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nec_solve(
    n: c_int,
    a: *const c64,
    ip: *const c_int,
    b: *mut c64,
    ndim: c_int,
) {
    let (n, ndim) = (n.max(0) as usize, ndim.max(0) as usize);
    if n == 0 {
        return;
    }
    // SAFETY: as documented above.
    let (lu, perm_fwd, rhs) = unsafe {
        (
            MatRef::from_raw_parts(a, n, n, 1, ndim as isize),
            std::slice::from_raw_parts(ip, n),
            MatMut::from_raw_parts_mut(b, n, 1, 1, n as isize),
        )
    };
    let fwd: Vec<usize> = perm_fwd.iter().map(|&p| p as usize).collect();
    let mut inv = vec![0usize; n];
    for (i, &p) in fwd.iter().enumerate() {
        inv[p] = i;
    }
    let perm = PermRef::new_checked(&fwd, &inv, n);
    // One right-hand side is quick; keep it on this thread.
    let par = Par::Seq;
    let req = solve::solve_transpose_in_place_scratch::<usize, c64>(n, 1, par);
    let mut buf = MemBuffer::new(req);
    solve::solve_transpose_in_place_with_conj(
        lu,
        lu,
        perm,
        Conj::No,
        rhs,
        par,
        MemStack::new(&mut buf),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Solves A·x = b through the nec2c conventions (A passed transposed)
    /// and checks A·x reproduces b.
    #[test]
    fn solves_a_complex_system_given_transposed() {
        let n = 37usize;
        let ndim = 40usize;
        let mut seed = 12345u64;
        let mut rnd = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as f64 / (1u64 << 31) as f64) - 0.5
        };
        let a: Vec<Vec<c64>> = (0..n)
            .map(|_| (0..n).map(|_| c64::new(rnd(), rnd())).collect())
            .collect();
        let b: Vec<c64> = (0..n).map(|_| c64::new(rnd(), rnd())).collect();
        // nec2c's layout: mem[i + j*ndim] = A[j][i].
        let mut mem = vec![c64::new(0.0, 0.0); ndim * n];
        for (r, row) in a.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                mem[c + r * ndim] = *v;
            }
        }
        let mut ip = vec![0 as c_int; n];
        let mut x = b.clone();
        unsafe {
            nec_factr(n as c_int, mem.as_mut_ptr(), ip.as_mut_ptr(), ndim as c_int);
            nec_solve(
                n as c_int,
                mem.as_ptr(),
                ip.as_ptr(),
                x.as_mut_ptr(),
                ndim as c_int,
            );
        }
        for r in 0..n {
            let ax: c64 = (0..n).map(|c| a[r][c] * x[c]).sum();
            assert!((ax - b[r]).norm() < 1e-10, "row {r}: {ax} vs {}", b[r]);
        }
    }
}
