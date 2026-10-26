//! `straws-cli`: run decks, check results against 4nec2's engine, benchmark.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use straws_core::deck::write::FORTRAN_COLUMNS;
use straws_core::deck::{Deck, NumCard, NumericDeck};
use straws_core::engine::{CancelToken, Engine, EngineError, SweepOptions};
use straws_core::output::NecOutput;
use straws_core::reference::ReferenceEngine;
use straws_core::validate::{self, Tolerances};

#[derive(Parser)]
#[command(
    name = "straws-cli",
    version,
    about = "NEC antenna modeller: command-line tools"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a deck and print impedance and SWR for each frequency.
    Run {
        deck: PathBuf,
        /// Worker processes to use (default: all cores).
        #[arg(long)]
        threads: Option<usize>,
        /// Reference impedance for SWR, in ohms.
        #[arg(long, default_value_t = 50.0)]
        z0: f64,
        /// Save the engine's full text output here (single-job run).
        #[arg(long)]
        save_output: Option<PathBuf>,
    },
    /// Print a deck with all symbols and expressions evaluated.
    Expand { deck: PathBuf },
    /// Run the geometry checks (the app's Checks view) on decks.
    Check {
        /// Decks, or folders of decks (searched recursively).
        decks: Vec<PathBuf>,
        /// List warnings too, not just errors.
        #[arg(long)]
        warnings: bool,
    },
    /// Compare our expansion of 4nec2 decks with the .inp files 4nec2 wrote.
    CheckExpand {
        /// Folder of .nec decks (searched recursively), e.g. C:\4nec2\models.
        models: PathBuf,
        /// Folder of 4nec2's expanded .inp files, e.g. C:\4nec2\out.
        inp: PathBuf,
    },
    /// Make reference results with 4nec2's engine.
    Reference {
        /// Decks, or folders of decks.
        decks: Vec<PathBuf>,
        /// Folder holding 4nec2's nec2dxs*.exe engines.
        #[arg(long, default_value = r"C:\4nec2\exe")]
        engine_dir: PathBuf,
        /// Where to write the .out files.
        #[arg(long)]
        out: PathBuf,
        /// Add a pattern request to decks that have none.
        #[arg(long)]
        pattern: bool,
    },
    /// Check nec2c's results against reference results.
    Validate {
        /// Decks, or folders of decks (searched recursively).
        decks: Vec<PathBuf>,
        /// Folder of stored reference .out files (one per deck, same name).
        #[arg(long)]
        refs: Option<PathBuf>,
        /// Or: run 4nec2's engines from this folder to make references live.
        #[arg(long)]
        engine_dir: Option<PathBuf>,
        /// Add a pattern request to decks that have none.
        #[arg(long)]
        pattern: bool,
        /// Skip decks with more segments than this.
        #[arg(long, default_value_t = 1500)]
        max_segments: usize,
        /// Decks to check at once.
        #[arg(long)]
        threads: Option<usize>,
        /// Print every deck, not just failures and a summary.
        #[arg(long)]
        verbose: bool,
        /// Give up on a deck after this many seconds per engine run.
        #[arg(long, default_value_t = 120)]
        timeout: u64,
    },
    /// Time the benchmark decks with nec2c and (optionally) 4nec2's engine.
    Bench {
        /// Benchmark decks, or a folder of them.
        decks: Vec<PathBuf>,
        /// Folder holding 4nec2's nec2dxs*.exe engines, to time them too.
        #[arg(long)]
        engine_dir: Option<PathBuf>,
        /// Runs per measurement; the median is reported.
        #[arg(long, default_value_t = 5)]
        repeat: usize,
    },
}

fn main() -> ExitCode {
    straws_core::worker::run_if_worker();
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Run {
            deck,
            threads,
            z0,
            save_output,
        } => run(&deck, threads, z0, save_output.as_deref()),
        Cmd::Expand { deck } => load(&deck).map(|d| print!("{}", d.to_nec_text())),
        Cmd::CheckExpand { models, inp } => check_expand(&models, &inp),
        Cmd::Check { decks, warnings } => check_cmd(&decks, warnings),
        Cmd::Reference {
            decks,
            engine_dir,
            out,
            pattern,
        } => reference(&decks, &engine_dir, &out, pattern),
        Cmd::Validate {
            decks,
            refs,
            engine_dir,
            pattern,
            max_segments,
            threads,
            verbose,
            timeout,
        } => validate_cmd(
            &decks,
            &ValidateSettings {
                refs: refs.as_deref(),
                engine_dir: engine_dir.as_deref(),
                pattern,
                max_segments,
                limit: Duration::from_secs(timeout),
            },
            threads,
            verbose,
        ),
        Cmd::Bench {
            decks,
            engine_dir,
            repeat,
        } => bench(&decks, engine_dir.as_deref(), repeat),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type Res<T> = Result<T, String>;

/// Runs the geometry checks on each deck, as the app's Checks view does (at
/// the deck's highest frequency), and sums up what they found by kind.
fn check_cmd(paths: &[PathBuf], warnings: bool) -> Res<()> {
    use straws_core::analysis::checks::{Severity, check};
    use straws_core::analysis::plan::{geometry_deck, has_ground};
    let engine = engine()?;
    let decks = collect_decks(paths);
    // (kind, errors, warnings, decks)
    let mut kinds: Vec<(String, usize, usize, usize)> = Vec::new();
    let (mut clean, mut failed) = (0, 0);
    for path in &decks {
        let name = path.display();
        let deck = match Deck::from_file(path)
            .map_err(|e| e.to_string())
            .and_then(|d| d.expand().map_err(|e| e.to_string()))
        {
            Ok(d) => d,
            Err(e) => {
                println!("{name}: can't be read: {e}");
                failed += 1;
                continue;
            }
        };
        let text = geometry_deck(&deck).to_nec_text();
        let raw = match engine.run_text_with_limit(&text, Duration::from_secs(60)) {
            Ok(r) => r,
            Err(e) => {
                println!("{name}: the engine couldn't lay it out: {e}");
                failed += 1;
                continue;
            }
        };
        let out = NecOutput::parse(&raw.text);
        let mhz = deck.frequencies().into_iter().fold(0.0, f64::max);
        let mhz = if mhz > 0.0 { mhz } else { 299.8 };
        let mut issues = check(&out.segments, &deck, mhz, has_ground(&deck));
        issues.extend(straws_core::analysis::checks::patch_checks(
            &out.patches,
            mhz,
            has_ground(&deck),
        ));
        if issues.is_empty() {
            clean += 1;
        }
        let mut seen: Vec<String> = Vec::new();
        for i in &issues {
            let error = i.severity == Severity::Error;
            if error || warnings {
                let what = if error { "error" } else { "warning" };
                println!("{name}: {what}: {}", i.message);
            }
            // The kind: the message up to its first number or quote.
            let kind: String = i
                .message
                .split(|c: char| c.is_ascii_digit() || c == '"')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            let k = match kinds.iter().position(|x| x.0 == kind) {
                Some(k) => k,
                None => {
                    kinds.push((kind.clone(), 0, 0, 0));
                    kinds.len() - 1
                }
            };
            if error {
                kinds[k].1 += 1;
            } else {
                kinds[k].2 += 1;
            }
            if !seen.contains(&kind) {
                kinds[k].3 += 1;
                seen.push(kind);
            }
        }
    }
    kinds.sort_by_key(|k| std::cmp::Reverse((k.1, k.2)));
    println!();
    println!("{:>7} {:>9} {:>6}  kind", "errors", "warnings", "decks");
    for (kind, e, w, d) in &kinds {
        println!("{e:>7} {w:>9} {d:>6}  {kind}");
    }
    println!(
        "{} decks: {clean} with nothing to report, {failed} couldn't be checked",
        decks.len()
    );
    Ok(())
}

fn engine() -> Res<Engine> {
    Engine::self_hosted().map_err(|e| e.to_string())
}

/// Reads and expands a deck, ready to run.
fn load(path: &Path) -> Res<NumericDeck> {
    let deck = Deck::from_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut n = deck
        .expand()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    for w in &n.warnings {
        eprintln!("warning: {}: {w}", path.display());
    }
    n.ensure_execute();
    Ok(n)
}

/// Adds a 10° whole-sphere pattern request if the deck has no RP card.
fn add_pattern(deck: &mut NumericDeck) {
    if !deck.cards.iter().any(|c| c.is("RP")) {
        deck.cards
            .retain(|c| !c.is("XQ") || c.ints[0] != 0 || c.line.is_some());
        deck.cards.push(NumCard::new(
            "RP",
            &[0, 19, 37, 1000],
            &[0.0, 0.0, 10.0, 10.0],
        ));
    }
}

fn swr(z: straws_core::Complex64, z0: f64) -> f64 {
    let g = ((z - z0) / (z + z0)).norm();
    if g >= 1.0 {
        f64::INFINITY
    } else {
        (1.0 + g) / (1.0 - g)
    }
}

/// Models bigger than this aren't solved: the matrix alone would need
/// 16 N² bytes (20,000 segments: 6.4 GB).
const MAX_SOLVE_SEGMENTS: i64 = 20_000;

/// Refuses a deck too big to solve, saying why.
fn check_size(deck: &straws_core::deck::NumericDeck) -> Res<()> {
    let n = deck.segment_estimate();
    if n > MAX_SOLVE_SEGMENTS {
        return Err(format!(
            "this model has about {n} segments: solving it needs about {:.0} GB of memory \
             for the matrix alone, so it isn't run (the limit is {MAX_SOLVE_SEGMENTS} segments)",
            16.0 * (n as f64).powi(2) / 1e9
        ));
    }
    Ok(())
}

fn run(path: &Path, threads: Option<usize>, z0: f64, save: Option<&Path>) -> Res<()> {
    let deck = load(path)?;
    check_size(&deck)?;
    let engine = engine()?;
    let cancel = CancelToken::new();
    let started = Instant::now();
    let out = if let Some(save) = save {
        let raw = engine
            .run_text(&deck.to_nec_text(), &cancel)
            .map_err(|e| e.to_string())?;
        std::fs::write(save, &raw.text).map_err(|e| e.to_string())?;
        NecOutput::parse(&raw.text)
    } else {
        let mut options = SweepOptions::default();
        if let Some(t) = threads {
            options.threads = t;
        }
        engine
            .sweep(&deck, options, &cancel, |_, _| {})
            .map_err(|e| e.to_string())?
    };
    let elapsed = started.elapsed();

    println!(
        "{:>12} {:>12} {:>12} {:>8} {:>10}",
        "MHz", "R (ohm)", "X (ohm)", "SWR", "Gmax dBi"
    );
    for f in &out.frequencies {
        let gmax = f
            .patterns
            .iter()
            .flat_map(|t| t.points.iter().map(|p| p.gain_total))
            .fold(f64::NAN, f64::max);
        match f.inputs.first() {
            Some(i) => println!(
                "{:>12.6} {:>12.4} {:>12.4} {:>8.3} {:>10.2}",
                f.mhz,
                i.impedance.re,
                i.impedance.im,
                swr(i.impedance, z0),
                gmax
            ),
            None => println!(
                "{:>12.6} {:>12} {:>12} {:>8} {:>10.2}",
                f.mhz, "-", "-", "-", gmax
            ),
        }
    }
    for m in &out.messages {
        println!("engine: {m}");
    }
    println!(
        "{} segments, {} frequencies in {:.1} ms",
        out.segments.len(),
        out.frequencies.len(),
        elapsed.as_secs_f64() * 1e3
    );
    Ok(())
}

/// All .nec files under the given paths.
fn collect_decks(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(p: &Path, out: &mut Vec<PathBuf>) {
        if p.is_dir() {
            if let Ok(entries) = std::fs::read_dir(p) {
                let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
                entries.sort();
                for e in entries {
                    walk(&e, out);
                }
            }
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("nec")) {
            out.push(p.to_path_buf());
        }
    }
    let mut out = Vec::new();
    for p in paths {
        walk(p, &mut out);
    }
    out
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// 4nec2 names its .inp files after the first 8 characters of the deck
/// name, with spaces as underscores.
fn short_name(p: &Path) -> String {
    stem(p)
        .replace(' ', "_")
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn check_expand(models: &Path, inp: &Path) -> Res<()> {
    let mut by_short: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for d in collect_decks(&[models.to_path_buf()]) {
        by_short.entry(short_name(&d)).or_default().push(d);
    }
    let mut inps: Vec<PathBuf> = std::fs::read_dir(inp)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("inp")))
        .collect();
    inps.sort();

    let (mut matched, mut differ, mut unknown) = (0, 0, 0);
    for inp_path in &inps {
        let key = stem(inp_path).to_ascii_lowercase();
        let Some(candidates) = by_short.get(&key) else {
            unknown += 1;
            println!("?    {}: no matching deck found", inp_path.display());
            continue;
        };
        let theirs = match Deck::from_file(inp_path)
            .map_err(|e| e.to_string())
            .and_then(|d| d.expand().map_err(|e| e.to_string()))
        {
            Ok(t) => t,
            Err(e) => {
                println!("?    {}: can't read: {e}", inp_path.display());
                unknown += 1;
                continue;
            }
        };
        let mut best: Option<(PathBuf, Result<(), String>)> = None;
        for cand in candidates {
            let result = Deck::from_file(cand)
                .map_err(|e| e.to_string())
                .and_then(|d| d.expand().map_err(|e| e.to_string()))
                .and_then(|ours| same_model(&ours, &theirs));
            let ok = result.is_ok();
            best = Some((cand.clone(), result));
            if ok {
                break;
            }
        }
        match best {
            Some((deck, Ok(()))) => {
                matched += 1;
                println!("ok   {} = {}", inp_path.display(), deck.display());
            }
            Some((deck, Err(e))) => {
                differ += 1;
                println!("DIFF {} vs {}: {e}", inp_path.display(), deck.display());
            }
            None => {}
        }
    }
    println!("\n{matched} match, {differ} differ, {unknown} without a usable deck");
    Ok(())
}

/// Compares two expanded decks: geometry exactly (to the precision 4nec2
/// writes), and the source/load/ground/network cards. Frequency and output
/// requests are skipped because 4nec2 replaces them with its own settings.
fn same_model(ours: &NumericDeck, theirs: &NumericDeck) -> Res<()> {
    // 4nec2 leaves out the dummy wires for current sources when it only
    // writes a deck for its geometry view, so skip them if 4nec2 did.
    let theirs_have_dummies = theirs.cards.iter().any(|c| c.is("GW") && c.ints[0] > 9900);
    let geo = |d: &NumericDeck| -> Vec<NumCard> {
        d.cards[..d.control_start()]
            .iter()
            .filter(|c| theirs_have_dummies || !(c.is("GW") && c.ints[0] > 9900))
            .cloned()
            .collect()
    };
    let ctl = |d: &NumericDeck, which: &[&str]| -> Vec<NumCard> {
        d.cards[d.control_start()..]
            .iter()
            .filter(|c| which.contains(&c.mnemonic.as_str()))
            .cloned()
            .collect()
    };
    compare_cards(&geo(ours), &geo(theirs)).map_err(|e| format!("geometry: {e}"))?;
    // 4nec2 rewrites control cards grouped by type, so compare each type
    // on its own, keeping the order within the type.
    for card in ["EX", "LD", "GN", "TL", "NT", "GD", "EK", "KH"] {
        compare_cards(&ctl(ours, &[card]), &ctl(theirs, &[card]))
            .map_err(|e| format!("{card} cards: {e}"))?;
    }
    Ok(())
}

fn compare_cards(a: &[NumCard], b: &[NumCard]) -> Res<()> {
    if a.len() != b.len() {
        return Err(format!("{} cards vs 4nec2's {}", a.len(), b.len()));
    }
    for (x, y) in a.iter().zip(b) {
        let scale = y.floats.iter().fold(1e-6f64, |m, v| m.max(v.abs()));
        let floats_ok = x
            .floats
            .iter()
            .zip(&y.floats)
            .all(|(p, q)| (p - q).abs() <= 2e-5 * scale);
        if x.mnemonic != y.mnemonic || x.ints != y.ints || !floats_ok {
            return Err(format!(
                "line {}: ours `{}`, 4nec2 `{}`",
                x.line.map_or("?".into(), |l| l.to_string()),
                x.to_nec_line(),
                y.to_nec_line()
            ));
        }
    }
    Ok(())
}

fn reference(decks: &[PathBuf], engine_dir: &Path, out: &Path, pattern: bool) -> Res<()> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let engine = engine()?;
    for path in collect_decks(decks) {
        let mut deck = load(&path)?;
        if pattern {
            add_pattern(&mut deck);
        }
        // The same deck goes to both engines, so keep it Fortran-width.
        let text = deck.to_nec_text_width(FORTRAN_COLUMNS);
        // Run nec2c first to learn the segment count, which picks the engine.
        let ours = engine
            .run_text(&text, &CancelToken::new())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let segments = NecOutput::parse(&ours.text).segments.len();
        let reference =
            ReferenceEngine::for_deck(engine_dir, &deck, segments).ok_or_else(|| {
                format!(
                    "no 4nec2 engine in {} for {segments} segments",
                    engine_dir.display()
                )
            })?;
        let raw = reference
            .run_text(&text, Duration::from_secs(600))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let dest = out.join(format!("{}.out", stem(&path)));
        std::fs::write(&dest, raw.text).map_err(|e| e.to_string())?;
        println!("{} -> {}", path.display(), dest.display());
    }
    Ok(())
}

enum Outcome {
    Pass(String),
    Fail(String),
    Skip(String),
}

/// What to check decks against, and how.
struct ValidateSettings<'a> {
    /// Folder of stored reference outputs...
    refs: Option<&'a Path>,
    /// ...or 4nec2's engine folder, to make references live.
    engine_dir: Option<&'a Path>,
    pattern: bool,
    max_segments: usize,
    limit: Duration,
}

fn validate_one(path: &Path, settings: &ValidateSettings, engine: &Engine) -> Outcome {
    let ValidateSettings {
        refs,
        engine_dir,
        pattern,
        max_segments,
        limit,
    } = *settings;
    let mut deck = match load(path) {
        Ok(d) => d,
        Err(e) => return Outcome::Skip(e),
    };
    if deck.segment_estimate() > max_segments as i64 {
        return Outcome::Skip(format!("about {} segments", deck.segment_estimate()));
    }
    if pattern {
        add_pattern(&mut deck);
    }
    // The same deck goes to both engines, so keep it Fortran-width.
    let text = deck.to_nec_text_width(FORTRAN_COLUMNS);
    let ours = match engine.run_text_with_limit(&text, limit) {
        Ok(r) => NecOutput::parse(&r.text),
        Err(EngineError::Cancelled) => {
            return Outcome::Skip(format!("nec2c took over {} s", limit.as_secs()));
        }
        Err(e) => {
            // If 4nec2's engine rejects the deck too, the engines agree.
            if let Some(dir) = engine_dir
                && let Some(re) = ReferenceEngine::for_deck(dir, &deck, 1)
            {
                let rejected = match re.run_text(&text, limit) {
                    Ok(r) => !NecOutput::parse(&r.text).has_results(),
                    Err(_) => true,
                };
                if rejected {
                    return Outcome::Skip(format!("both engines reject this deck: {e}"));
                }
            }
            return Outcome::Fail(format!("nec2c failed: {e}"));
        }
    };
    if ours.segments.len() > max_segments {
        return Outcome::Skip(format!("{} segments", ours.segments.len()));
    }
    let reference = if let Some(dir) = refs {
        match std::fs::read(dir.join(format!("{}.out", stem(path)))) {
            Ok(b) => NecOutput::parse(&String::from_utf8_lossy(&b)),
            Err(_) => return Outcome::Skip("no reference file".into()),
        }
    } else if let Some(dir) = engine_dir {
        let Some(re) = ReferenceEngine::for_deck(dir, &deck, ours.segments.len()) else {
            return Outcome::Skip("no suitable 4nec2 engine".into());
        };
        match re.run_text(&text, limit) {
            Ok(r) => NecOutput::parse(&r.text),
            Err(e) => return Outcome::Skip(format!("reference engine failed: {e}")),
        }
    } else {
        return Outcome::Skip("no reference given".into());
    };
    let c = validate::compare(&ours, &reference, &Tolerances::default());
    if c.passed() {
        Outcome::Pass(c.to_string())
    } else {
        Outcome::Fail(format!("{c}\n        {}", c.failures.join("\n        ")))
    }
}

fn validate_cmd(
    decks: &[PathBuf],
    settings: &ValidateSettings,
    threads: Option<usize>,
    verbose: bool,
) -> Res<()> {
    if settings.refs.is_none() && settings.engine_dir.is_none() {
        return Err("give --refs <folder> or --engine-dir <folder>".into());
    }
    let paths = collect_decks(decks);
    let engine = engine()?;
    let threads = threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
        .max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let started = Instant::now();
    let (mut pass, mut fail, mut skip) = (0, 0, 0);
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Outcome)>();
    std::thread::scope(|s| {
        for _ in 0..threads {
            let tx = tx.clone();
            let (next, paths, engine) = (&next, &paths, &engine);
            s.spawn(move || {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(p) = paths.get(k) else { break };
                    let o = validate_one(p, settings, engine);
                    if tx.send((k, o)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        // Report each deck as it finishes.
        for (done, (k, outcome)) in rx.iter().enumerate() {
            let p = paths[k].display();
            let n = format!("[{}/{}]", done + 1, paths.len());
            match outcome {
                Outcome::Pass(s) => {
                    pass += 1;
                    if verbose {
                        println!("{n} pass {p}: {s}");
                    }
                }
                Outcome::Fail(s) => {
                    fail += 1;
                    println!("{n} FAIL {p}: {s}");
                }
                Outcome::Skip(s) => {
                    skip += 1;
                    if verbose {
                        println!("{n} skip {p}: {s}");
                    }
                }
            }
        }
    });
    println!(
        "\n{pass} passed, {fail} failed, {skip} skipped, in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    if fail > 0 {
        Err(format!("{fail} deck(s) failed validation"))
    } else {
        Ok(())
    }
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn ms(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1e3;
    if ms >= 100.0 {
        format!("{ms:.0}")
    } else {
        format!("{ms:.1}")
    }
}

fn bench(decks: &[PathBuf], engine_dir: Option<&Path>, repeat: usize) -> Res<()> {
    let engine = engine()?;
    let cancel = CancelToken::new();
    let repeat = repeat.max(1);
    let threads = SweepOptions::default().threads;
    println!(
        "| Deck | Segments | Freqs | nec2c, 1 process (ms) | nec2c, split sweep, up to {threads} processes (ms) | 4nec2 engine (ms) | Speed-up |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for path in collect_decks(decks) {
        let deck = load(&path)?;
        let text = deck.to_nec_text();
        let first = engine
            .run_text(&text, &cancel)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let parsed = NecOutput::parse(&first.text);
        let segments = parsed.segments.len();
        let freqs = parsed.frequencies.len();

        let single = median(
            (0..repeat)
                .map(|_| {
                    let t = Instant::now();
                    let raw = engine.run_text(&text, &cancel).expect("ran once already");
                    let _ = NecOutput::parse(&raw.text);
                    t.elapsed()
                })
                .collect(),
        );
        let parallel = median(
            (0..repeat)
                .map(|_| {
                    let t = Instant::now();
                    engine
                        .sweep(&deck, SweepOptions::default(), &cancel, |_, _| {})
                        .expect("ran once already");
                    t.elapsed()
                })
                .collect(),
        );
        let reference = engine_dir
            .and_then(|dir| ReferenceEngine::for_deck(dir, &deck, segments))
            .map(|re| {
                median(
                    (0..repeat)
                        .map(|_| {
                            let t = Instant::now();
                            re.run_text(&text, Duration::from_secs(600))
                                .expect("reference engine runs");
                            t.elapsed()
                        })
                        .collect(),
                )
            });
        let best = single.min(parallel);
        println!(
            "| {} | {segments} | {freqs} | {} | {} | {} | {} |",
            stem(&path),
            ms(single),
            ms(parallel),
            reference.map_or("-".into(), ms),
            reference.map_or("-".into(), |r| format!(
                "{:.1}×",
                r.as_secs_f64() / best.as_secs_f64()
            ))
        );
    }
    Ok(())
}
