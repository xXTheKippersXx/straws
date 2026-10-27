//! Regression suite: every deck in tests/decks is run through nec2c and
//! compared with reference results from 4nec2's engine (tests/references,
//! made with `straws-cli reference tests/decks --out tests/references`).

use std::path::PathBuf;

use straws_core::deck::Deck;
use straws_core::deck::write::FORTRAN_COLUMNS;
use straws_core::engine::{CancelToken, Engine, SweepOptions};
use straws_core::output::NecOutput;
use straws_core::validate::{Tolerances, compare};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn engine() -> Engine {
    Engine::with_worker_program(env!("CARGO_BIN_EXE_straws-worker"))
}

fn decks() -> Vec<PathBuf> {
    let mut decks: Vec<PathBuf> = std::fs::read_dir(repo().join("tests/decks"))
        .expect("tests/decks exists")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "nec"))
        .collect();
    decks.sort();
    decks
}

#[test]
fn decks_match_4nec2_reference_results() {
    let engine = engine();
    let decks = decks();
    assert!(decks.len() >= 10, "expected the regression decks");
    let mut failures = Vec::new();
    for path in &decks {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let reference =
            std::fs::read_to_string(repo().join(format!("tests/references/{name}.out")))
                .unwrap_or_else(|_| panic!("no reference result for {name}"));
        let mut deck = Deck::from_file(path).unwrap().expand().unwrap();
        deck.ensure_execute();
        // References were made from the Fortran-width deck; run the same text.
        let raw = engine
            .run_text(
                &deck.to_nec_text_width(FORTRAN_COLUMNS),
                &CancelToken::new(),
            )
            .unwrap();
        let ours = NecOutput::parse(&raw.text);
        let c = compare(&ours, &NecOutput::parse(&reference), &Tolerances::default());
        println!("{name}: {c}");
        if !c.passed() {
            failures.push(format!("{name}: {}", c.failures.join("; ")));
        }
    }
    assert!(
        failures.is_empty(),
        "decks differ from reference:\n{}",
        failures.join("\n")
    );
}

#[test]
fn parallel_sweep_matches_single_run() {
    let engine = engine();
    let mut deck = Deck::from_file(repo().join("tests/decks/yagi_3el_aluminium.nec"))
        .unwrap()
        .expand()
        .unwrap();
    deck.ensure_execute();
    let single = engine.run(&deck, &CancelToken::new()).unwrap();
    let mut seen = Vec::new();
    let options = SweepOptions {
        threads: 3,
        jobs_per_thread: 2,
        // Force splitting even though this small deck wouldn't normally.
        min_job_ms: 0.0,
    };
    let parallel = engine
        .sweep(&deck, options, &CancelToken::new(), |i, _| seen.push(i))
        .unwrap();
    seen.sort();
    assert_eq!(seen, (0..7).collect::<Vec<_>>());
    let c = compare(&parallel, &single, &Tolerances::default());
    assert!(c.passed(), "{:?}", c.failures);
    assert_eq!(
        c.max_impedance_rel, 0.0,
        "split sweep should give identical numbers"
    );
}

#[test]
fn cancelling_stops_a_run() {
    // A big sweep that would take a while; cancel it straight away.
    let deck =
        Deck::parse("CE\nGW 1 400 0 0 -50 0 0 50 .01\nGE\nEX 0 1 200 0 1\nFR 0 200 0 0 1 1\nXQ\n")
            .expand()
            .unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let result = engine().sweep(&deck, SweepOptions::default(), &cancel, |_, _| {});
    assert!(matches!(
        result,
        Err(straws_core::engine::EngineError::Cancelled)
    ));
}

#[test]
fn engine_errors_are_reported() {
    // Segment 30 doesn't exist on a 9-segment wire.
    let deck =
        Deck::parse("CE\nGW 1 9 0 0 -1 0 0 1 .001\nGE\nEX 0 1 30 0 1\nFR 0 1 0 0 100 0\nXQ\n")
            .expand()
            .unwrap();
    let err = engine().run(&deck, &CancelToken::new()).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("stopped with an error"), "{text}");
}

#[test]
fn geometry_only_run_lists_segments_without_solving() {
    use straws_core::analysis::plan::geometry_deck;
    let deck = Deck::from_file(repo().join("tests/decks/symmetry_gx_gm.nec"))
        .unwrap()
        .expand()
        .unwrap();
    let raw = engine()
        .run_text(&geometry_deck(&deck).to_nec_text(), &CancelToken::new())
        .unwrap();
    let out = NecOutput::parse(&raw.text);
    // 9 segments reflected by GX: 18.
    assert_eq!(out.segments.len(), 18);
    assert!(out.frequencies.is_empty());
    // Segment ends join up into wires: segment 1 ends where segment 2 starts.
    let (_, end1) = out.segments[0].ends();
    let (start2, _) = out.segments[1].ends();
    for k in 0..3 {
        assert!((end1[k] - start2[k]).abs() < 1e-3, "{end1:?} vs {start2:?}");
    }
}

#[test]
fn circular_components_agree_with_nec_sense() {
    use straws_core::analysis::pattern::{Component, PatternGrid};
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    use straws_core::output::Sense;
    // Turnstile: crossed half-wave dipoles fed 90 degrees apart.
    for (phase, sense) in [(1.0, Sense::Left), (-1.0, Sense::Right)] {
        let text = format!(
            "CE\nGW 1 11 -0.24 0 0 0.24 0 0 .001\nGW 2 11 0 -0.24 0 0 0.24 0 .001\nGE\n\
             EX 0 1 6 0 1 0\nEX 0 2 6 0 0 {phase}\nFR 0 1 0 0 299.8\n"
        );
        let deck = Deck::parse(&text).expand().unwrap();
        let run = run_deck(
            &deck,
            &FrequencyPlan::single(299.8),
            Some(PatternRequest::new(30.0)),
        );
        let out = engine().run(&run, &CancelToken::new()).unwrap();
        let table = &out.frequencies[0].patterns[0];
        let zenith = table.points.iter().find(|p| p.theta == 0.0).unwrap();
        assert_eq!(zenith.sense, sense, "NEC's own sense for phase {phase}");
        let grid = PatternGrid::from_table(table).unwrap();
        let (r, l) = (
            grid.gain(Component::Rhcp, 0, 0),
            grid.gain(Component::Lhcp, 0, 0),
        );
        match sense {
            Sense::Right => assert!(r > l + 20.0, "RHCP {r} LHCP {l}"),
            _ => assert!(l > r + 20.0, "RHCP {r} LHCP {l}"),
        }
    }
}

/// A deck big enough for the matrix fill to run on several threads: a
/// wire grid (four-way junctions) with a fed dipole, wire losses and
/// Sommerfeld ground.
fn grid_deck() -> String {
    let mut s = String::from("CE\n");
    let (n, d, x0, h) = (8, 1.4, -2.5, 6.0);
    let off = (n - 1) as f64 * d / 2.0;
    let mut tag = 1;
    for i in 0..n {
        for j in 0..n - 1 {
            let (y1, y2, z) = (
                -off + j as f64 * d,
                -off + (j + 1) as f64 * d,
                h - off + i as f64 * d,
            );
            s += &format!("GW {tag} 1 {x0} {y1} {z} {x0} {y2} {z} 0.002\n");
            tag += 1;
            let (y, z1, z2) = (
                -off + i as f64 * d,
                h - off + j as f64 * d,
                h - off + (j + 1) as f64 * d,
            );
            s += &format!("GW {tag} 1 {x0} {y} {z1} {x0} {y} {z2} 0.002\n");
            tag += 1;
        }
    }
    s += &format!("GW {tag} 11 0 -5 {h} 0 5 {h} 0.003\nGE 1\nGN 2 0 0 0 13 0.005\n");
    // A 703-point pattern, enough to use the parallel pattern code too.
    s +=
        &format!("LD 5 0 0 0 3.5e7\nEX 0 {tag} 6 0 1\nFR 0 1 0 0 14.2\nRP 0 19 37 1000 0 0 5 10\n");
    s
}

#[test]
fn parallel_fill_and_pattern_give_the_same_answer() {
    let deck = Deck::parse(&grid_deck()).expand().unwrap();
    let one = engine()
        .with_solver_threads(1)
        .run(&deck, &CancelToken::new())
        .unwrap();
    let four = engine()
        .with_solver_threads(4)
        .run(&deck, &CancelToken::new())
        .unwrap();
    assert_eq!(one.segments.len(), 2 * 8 * 7 + 11);
    let c = compare(&four, &one, &Tolerances::default());
    assert!(c.passed(), "{:?}", c.failures);
    assert!(c.max_impedance_rel < 1e-6, "{c}");
    assert!(c.pattern_points > 500, "{c}");
    assert!(c.max_gain_db < 0.011, "{c}");
}

#[test]
fn geometry_checks_on_the_regression_decks() {
    use straws_core::analysis::checks::{Severity, average_gain, check};
    use straws_core::analysis::pattern::PatternGrid;
    use straws_core::analysis::plan::{
        FrequencyPlan, PatternRequest, geometry_deck, has_ground, run_deck,
    };
    let mut errors = Vec::new();
    for path in decks() {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let deck = Deck::from_file(&path).unwrap().expand().unwrap();
        let raw = engine()
            .run_text(&geometry_deck(&deck).to_nec_text(), &CancelToken::new())
            .unwrap();
        let out = NecOutput::parse(&raw.text);
        let mhz = deck.frequencies().into_iter().fold(0.0, f64::max);
        let issues = check(&out.segments, &deck, mhz, has_ground(&deck));
        for i in &issues {
            println!("{name}: {:?} {} {}", i.severity, i.message, i.fix);
            if i.severity == Severity::Error {
                errors.push(format!("{name}: {}", i.message));
            }
        }
    }
    // A free-space dipole: the average gain test should give about 1.
    let deck = Deck::parse("CE\nGW 1 21 0 -5.2 0 0 5.2 0 .001\nGE\nEX 0 1 11 0 1\nFR 0 1 0 0 14\n")
        .expand()
        .unwrap();
    let run = run_deck(
        &deck,
        &FrequencyPlan::single(14.0),
        Some(PatternRequest::new(5.0)),
    );
    let out = engine().run(&run, &CancelToken::new()).unwrap();
    let grid = PatternGrid::from_table(&out.frequencies[0].patterns[0]).unwrap();
    let ag = average_gain(&grid).unwrap();
    assert!((ag - 1.0).abs() < 0.01, "average gain {ag}");
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn tube_load_matches_nec_conductivity_for_thick_walls() {
    use straws_core::deck::material::tube_impedance;
    // A stainless 12 mm tube dipole (radius ~50 skin depths), lossy enough
    // for the loss to show in the impedance.
    let (sigma, a, mhz) = (1.39e6, 0.006, 14.0);
    let base = "CE\nGW 1 21 0 -5.2 0 0 5.2 0 0.006\nGE 0\n";
    let ld5 = format!("{base}LD 5 1 0 0 {sigma}\nEX 0 1 11 0 1\nFR 0 1 0 0 {mhz}\nXQ\n");
    let z = tube_impedance(mhz, sigma, a, 1e-3);
    let l = z.im / (2.0 * std::f64::consts::PI * mhz * 1e6);
    let ld2 = format!(
        "{base}LD 2 1 0 0 {} {l}\nEX 0 1 11 0 1\nFR 0 1 0 0 {mhz}\nXQ\n",
        z.re
    );
    let run = |text: &str| {
        let deck = Deck::parse(text).expand().unwrap();
        engine()
            .run(&deck, &CancelToken::new())
            .unwrap()
            .frequencies[0]
            .inputs[0]
            .impedance
    };
    let (z5, z2) = (run(&ld5), run(&ld2));
    let lossless = run(&format!("{base}EX 0 1 11 0 1\nFR 0 1 0 0 {mhz}\nXQ\n"));
    let (loss5, loss2) = (z5.re - lossless.re, z2.re - lossless.re);
    assert!(loss5 > 0.5, "stainless should add loss: {z5} vs {lossless}");
    assert!((loss5 - loss2).abs() / loss5 < 0.02, "LD 5 {z5}, LD 2 {z2}");
}

#[test]
fn optimiser_finds_a_resonant_dipole() {
    use straws_core::optimise::{Goal, Method, Options, Param, Problem, optimise};
    let deck = Deck::parse(
        "CE\nSY len=5.6\nGW 1 21 0 -len 0 0 len 0 0.001\nGE 0\nEX 0 1 11 0 1\nFR 0 1 0 0 14.2\nEN\n",
    );
    let problem = Problem {
        deck,
        fixed: vec![],
        params: vec![Param {
            name: "len".into(),
            min: 4.5,
            max: 6.0,
            start: 5.6,
        }],
        goals: vec![Goal::Swr {
            start_mhz: 14.2,
            stop_mhz: 14.2,
            points: 1,
            z0: 70.0,
            target: 1.05,
            weight: 1.0,
            through: None,
        }],
        limits: Default::default(),
    };
    let engine = engine().with_solver_threads(1);
    for method in [Method::NelderMead, Method::DifferentialEvolution] {
        let mut calls = 0;
        let best = optimise(
            &problem,
            &engine,
            Options::new(method, 150),
            &CancelToken::new(),
            |_| calls += 1,
        );
        let len = best.values[0];
        println!(
            "{method:?}: len {len:.4}, SWR {:.3}, {calls} designs",
            best.results[0].value
        );
        assert!(best.results[0].value < 1.15, "{best:?}");
        assert!((4.9..5.3).contains(&len), "{best:?}");
    }
}

#[test]
fn the_optimiser_skips_designs_that_cant_be_built() {
    use straws_core::optimise::{Goal, Limits, Param, Problem};
    // A dipole and a parasitic element `d` in front: at d = 0 they cross.
    let deck = Deck::parse(
        "CE
SY d=0.3, len=5
GW 1 21 0 -len 0 0 len 0 0.001
GW 2 21 -2 0 d 2 0 d 0.001
GE 0
EX 0 1 11 0 1
FR 0 1 0 0 14.2
EN
",
    );
    let problem = |limits| Problem {
        deck: deck.clone(),
        fixed: vec![],
        params: vec![
            Param {
                name: "d".into(),
                min: -1.0,
                max: 1.0,
                start: 0.3,
            },
            Param {
                name: "len".into(),
                min: 4.0,
                max: 6.0,
                start: 5.0,
            },
        ],
        goals: vec![Goal::Swr {
            start_mhz: 14.2,
            stop_mhz: 14.2,
            points: 1,
            z0: 70.0,
            target: 1.05,
            weight: 1.0,
            through: None,
        }],
        limits,
    };
    let cancel = CancelToken::new();
    let free = problem(Limits::default());
    assert!(
        free.evaluate(&engine(), &[0.0, 5.0], &cancel)
            .rejected
            .is_none()
    );
    let careful = problem(Limits {
        clashes: Some(0),
        size: [None, Some(10.5), None],
    });
    let ok = careful.evaluate(&engine(), &[0.3, 5.0], &cancel);
    assert!(
        ok.rejected.is_none() && ok.cost < straws_core::optimise::FAILED,
        "{ok:?}"
    );
    let crossing = careful.evaluate(&engine(), &[0.0, 5.0], &cancel);
    assert!(
        crossing
            .rejected
            .as_deref()
            .is_some_and(|r| r.contains("cross")),
        "{crossing:?}"
    );
    let big = careful.evaluate(&engine(), &[0.3, 5.5], &cancel);
    assert!(
        big.rejected
            .as_deref()
            .is_some_and(|r| r.contains("along y")),
        "{big:?}"
    );
}

#[test]
fn one_run_answers_every_goal() {
    use straws_core::optimise::{Goal, Param, Problem};
    // A 2-element beam firing along +X: reflector behind at x = -1.5.
    let deck = Deck::parse(
        "CE\nSY r=5.4\nGW 1 21 0 -5.1 0 0 5.1 0 0.002\nGW 2 21 -1.5 -r 0 -1.5 r 0 0.002\nGE 0\nEX 0 1 11 0 1\nFR 0 1 0 0 14.2\nEN\n",
    );
    let goals = vec![
        Goal::Swr {
            start_mhz: 14.0,
            stop_mhz: 14.3,
            points: 4,
            z0: 50.0,
            target: 2.0,
            weight: 1.0,
            through: None,
        },
        Goal::Gain {
            mhz: 14.2,
            elevation: 0.0,
            azimuth: 0.0,
            peak: false,
            target: 5.0,
            weight: 1.0,
        },
        Goal::Gain {
            mhz: 14.2,
            elevation: 0.0,
            azimuth: 0.0,
            peak: true,
            target: 5.0,
            weight: 1.0,
        },
        Goal::FrontToBack {
            mhz: 14.2,
            elevation: 0.0,
            azimuth: 0.0,
            rear: false,
            target: 10.0,
            weight: 1.0,
        },
        Goal::FrontToBack {
            mhz: 14.2,
            elevation: 0.0,
            azimuth: 0.0,
            rear: true,
            target: 10.0,
            weight: 1.0,
        },
    ];
    let problem = Problem {
        deck,
        fixed: vec![],
        params: vec![Param {
            name: "r".into(),
            min: 5.0,
            max: 6.0,
            start: 5.4,
        }],
        goals,
        limits: Default::default(),
    };
    let t = problem.evaluate(&engine(), &[5.4], &CancelToken::new());
    println!("{t:?}");
    assert_eq!(t.results.len(), 5, "{t:?}");
    let (gain, peak, fb, fr) = (
        t.results[1].value,
        t.results[2].value,
        t.results[3].value,
        t.results[4].value,
    );
    assert!(gain > 4.0 && gain < 8.0, "forward gain {gain}");
    assert!(peak >= gain - 0.01, "peak {peak} vs forward {gain}");
    assert!(fb > 3.0, "F/B {fb}");
    assert!(fr <= fb + 0.01, "F/R {fr} can't beat F/B {fb}");
}

/// Timing check (not run by default): `cargo test --release -- --ignored optimiser_speed --nocapture`.
#[test]
#[ignore]
fn optimiser_speed() {
    use straws_core::optimise::{Goal, Method, Options, Param, Problem, optimise};
    let path = repo().join("nec-examples-test/yagi_opti_test.nec");
    let Ok(deck) = Deck::from_file(&path) else {
        return;
    };
    let names = [
        "len1", "len2", "len3", "len4", "len5", "sp2", "sp3", "sp4", "sp5",
    ];
    let starts = [0.5, 0.47, 0.44, 0.43, 0.42, 0.2, 0.3, 0.2, 0.2];
    let params = names
        .iter()
        .zip(starts)
        .map(|(n, s)| Param {
            name: n.to_string(),
            min: s * 0.8,
            max: s * 1.2,
            start: s,
        })
        .collect();
    let goals = vec![
        Goal::Swr {
            start_mhz: 295.0,
            stop_mhz: 305.0,
            points: 5,
            z0: 50.0,
            target: 1.5,
            weight: 1.0,
            through: None,
        },
        Goal::Gain {
            mhz: 300.0,
            elevation: 0.0,
            azimuth: 0.0,
            peak: true,
            target: 12.0,
            weight: 1.0,
        },
        Goal::FrontToBack {
            mhz: 300.0,
            elevation: 3.0,
            azimuth: 0.0,
            rear: true,
            target: 20.0,
            weight: 1.0,
        },
    ];
    let problem = Problem {
        deck,
        fixed: vec![],
        params,
        goals,
        limits: Default::default(),
    };
    let engine = engine().with_solver_threads(1);
    for method in [Method::NelderMead, Method::DifferentialEvolution] {
        let t = std::time::Instant::now();
        let mut n = 0;
        let best = optimise(
            &problem,
            &engine,
            Options::new(method, 400),
            &CancelToken::new(),
            |p| n = p.evaluations,
        );
        let s = t.elapsed().as_secs_f64();
        println!(
            "{method:?}: {n} designs in {s:.1} s ({:.1}/s); cost {:.3}; {:?}",
            n as f64 / s,
            best.cost,
            best.results
                .iter()
                .map(|r| (r.value * 100.0).round() / 100.0)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn templates_run_and_behave() {
    use straws_core::analysis::checks::{Severity, check};
    use straws_core::analysis::plan::{
        FrequencyPlan, PatternRequest, geometry_deck, has_ground, run_deck,
    };
    use straws_core::templates::{Common, TEMPLATES};
    let mut bad = Vec::new();
    for t in TEMPLATES {
        let mhz = if t.name.starts_with("Log") {
            20.0
        } else {
            14.2
        };
        let common = Common {
            mhz: 14.2,
            diameter_mm: 2.0,
            ground: t.ground,
            height_wl: t.height_wl,
        };
        let deck = Deck::parse(&t.deck(&common, &t.defaults()))
            .expand()
            .unwrap();
        let raw = engine()
            .run_text(&geometry_deck(&deck).to_nec_text(), &CancelToken::new())
            .unwrap();
        let segments = NecOutput::parse(&raw.text).segments;
        let errors: Vec<String> = check(&segments, &deck, mhz * 1.02, has_ground(&deck))
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.message)
            .collect();
        assert!(errors.is_empty(), "{}: {errors:?}", t.name);
        let run = run_deck(
            &deck,
            &FrequencyPlan::single(mhz),
            Some(PatternRequest::new(5.0)),
        );
        let out = engine()
            .run(&run, &CancelToken::new())
            .unwrap_or_else(|e| panic!("{}: {e}", t.name));
        let f = &out.frequencies[0];
        let z = f.inputs[0].impedance;
        let grid = straws_core::analysis::pattern::PatternGrid::from_table(&f.patterns[0]).unwrap();
        let comp = straws_core::analysis::pattern::Component::Total;
        let (peak, theta, phi) = grid.max(comp);
        let el = 90.0 - theta;
        let fwd = grid.sample(comp, theta, 0.0);
        let back = grid.sample(comp, theta, 180.0);
        println!(
            "{:<24} Z = {:8.2} {:+8.2}j  peak {peak:6.2} dBi at el {el:4.0} az {phi:4.0}  F/B {:6.2}",
            t.name,
            z.re,
            z.im,
            fwd - back
        );
        let ok = match t.name {
            "Dipole" => (50.0..110.0).contains(&z.re) && z.im.abs() < 25.0,
            "Moxon rectangle" => {
                (30.0..80.0).contains(&z.re) && z.im.abs() < 30.0 && fwd - back > 15.0
            }
            "Yagi" | "Quad (2 elements)" | "Phased verticals" | "Log-periodic (LPDA)" => {
                fwd - back > 5.0
            }
            _ => true,
        };
        if !ok {
            bad.push(t.name);
        }
    }
    assert!(bad.is_empty(), "templates that don't behave: {bad:?}");
}

#[test]
fn near_fields_are_read() {
    use straws_core::analysis::nearfield::{Grid, Plane, near_field_deck};
    let deck = Deck::parse(
        "CE\nGW 1 21 0 -5.2 10 0 5.2 10 0.001\nGE 0\nGN -1\nEX 0 1 11 0 1\nFR 0 1 0 0 14.2\nEN\n",
    )
    .expand()
    .unwrap();
    let grid = Grid {
        plane: Plane::Xy,
        centre: [0.0, 0.0],
        size: 10.0,
        offset: 9.0,
        n: 5,
    };
    let out = engine()
        .run(&near_field_deck(&deck, 14.2, &grid), &CancelToken::new())
        .unwrap();
    let near = &out.frequencies[0].near_e;
    assert_eq!(near.len(), 25);
    // Strongest right under the middle of the dipole's ends, weakest far off.
    let at = |x: f64, y: f64| {
        near.iter()
            .find(|p| p.position[0] == x && p.position[1] == y)
            .unwrap()
            .magnitude()
    };
    assert!(near.iter().all(|p| (p.position[2] - 9.0).abs() < 1e-9));
    assert!(
        at(0.0, 5.0) > at(5.0, 0.0),
        "near the wire end beats off to the side"
    );
}

/// Writes a sample report (not run by default):
/// `cargo test -p straws-core --test regression -- --ignored sample_report`.
#[test]
#[ignore]
fn sample_report() {
    use straws_core::analysis::pattern::Component;
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    use straws_core::analysis::sweep::{FreqPoint, SweepResults};
    use straws_core::export::{ReportInput, report_html};
    use straws_core::templates::{Common, TEMPLATES};
    let t = TEMPLATES
        .iter()
        .find(|t| t.name.starts_with("Moxon"))
        .unwrap();
    let common = Common {
        mhz: 14.2,
        diameter_mm: 2.0,
        ground: t.ground,
        height_wl: t.height_wl,
    };
    // STRAWS_REPORT_DECK and STRAWS_REPORT_MHZ pick another antenna.
    let (name, text, mhz) = match std::env::var("STRAWS_REPORT_DECK") {
        Ok(path) => (
            std::path::Path::new(&path)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            std::fs::read_to_string(&path).unwrap(),
            std::env::var("STRAWS_REPORT_MHZ")
                .unwrap()
                .parse::<f64>()
                .unwrap(),
        ),
        Err(_) => (
            "Moxon for 14.2 MHz".to_string(),
            t.deck(&common, &t.defaults()),
            14.2,
        ),
    };
    let parsed = Deck::parse(&text);
    let deck = parsed.expand().unwrap();
    let plan = FrequencyPlan {
        start_mhz: mhz * 0.98,
        stop_mhz: mhz * 1.02,
        points: 25,
    };
    let out = engine()
        .run(&run_deck(&deck, &plan, None), &CancelToken::new())
        .unwrap();
    let mut sweep = SweepResults::new(plan);
    for (k, f) in out.frequencies.iter().enumerate() {
        sweep.insert(k, FreqPoint::from_result(f, None));
    }
    let one = engine()
        .run(
            &run_deck(
                &deck,
                &FrequencyPlan::single(mhz),
                Some(PatternRequest::new(2.0)),
            ),
            &CancelToken::new(),
        )
        .unwrap();
    let point = FreqPoint::from_result(&one.frequencies[0], Some(2.0));
    let g = point.pattern.as_ref().unwrap();
    let (_, th, ph) = g.max(Component::Total);
    let cuts = Some((
        g.elevation_cut(Component::Total, ph),
        g.azimuth_cut(Component::Total, 90.0 - th),
    ));
    let wires = straws_core::deck::edit::wires(&parsed, &deck);
    let input = ReportInput {
        name: &name,
        deck_text: &text,
        sweep: Some(&sweep),
        point: Some(&point),
        wires: &wires,
        z0: 50.0,
        cuts,
        over_ground: deck.cards.iter().any(|c| c.is("GN") && c.ints[0] != -1),
        feed: 0,
    };
    let path = std::env::temp_dir().join("straws-sample-report.html");
    std::fs::write(&path, report_html(&input)).unwrap();
    println!("wrote {}", path.display());
    let pdf = straws_core::report::report_pdf(&input, &one.segments);
    let path = std::env::temp_dir().join("straws-sample-report.pdf");
    std::fs::write(&path, pdf).unwrap();
    println!("wrote {}", path.display());
}

#[test]
fn a_choke_cuts_common_mode_current() {
    // Dipole 10 m up over perfect ground, coax shield straight down from
    // one end of the fed segment. Shield current relative to the feed:
    let ratio = |r: f64| {
        let mut deck = Deck::parse(
            "CE\nGW 1 11 0 -5.1 10 0 5.1 10 0.001\nGE 1\nGN 1\nEX 0 1 6 0 1\nFR 0 1 0 0 14.2\nEN\n",
        );
        let half = 10.2 / 11.0 / 2.0;
        deck.add_shield(2, [0.0, -half, 10.0], [0.0, -half, 0.0], 11, 0.005, r, 0.0);
        let mut n = deck.expand().unwrap();
        n.ensure_execute();
        let out = engine().run(&n, &CancelToken::new()).unwrap();
        let f = &out.frequencies[0];
        let shield = f
            .currents
            .iter()
            .find(|c| c.tag == 2)
            .unwrap()
            .current
            .norm();
        shield / f.inputs[0].current.norm()
    };
    let (open, choked) = (ratio(0.0), ratio(5000.0));
    println!("common-mode current: {open:.3} without a choke, {choked:.4} with 5 kΩ");
    assert!(open > 0.05, "the shield should carry current: {open}");
    assert!(
        choked < open / 5.0,
        "the choke should cut it: {choked} vs {open}"
    );
}

/// Curves land where `CurveSpec::place` says, every kind runs, and a
/// one-wavelength circular loop has the loop's usual feed impedance.
#[test]
fn curves_are_placed_and_run() {
    use straws_core::analysis::checks::{Severity, check};
    use straws_core::analysis::plan::geometry_deck;
    use straws_core::deck::curves::{CurveKind, CurveSpec};
    for kind in CurveKind::ALL {
        let mut deck = Deck::parse("CE\nGE 0\nEX 0 1 1 0 1\nFR 0 1 0 0 14.2\nEN\n");
        let mut spec = CurveSpec::new(kind, "c", [1.0, -2.0, 5.0], 1.0);
        spec.tilt = 60.0;
        spec.azimuth = 40.0;
        deck.add_curve(&spec, 1).unwrap();
        let numeric = deck.expand().unwrap();
        let raw = engine()
            .run_text(&geometry_deck(&numeric).to_nec_text(), &CancelToken::new())
            .unwrap();
        let segments = NecOutput::parse(&raw.text).segments;
        assert_eq!(segments.len() as i64, spec.segments, "{kind:?}");
        // Every segment end lies on the curve's surface: at the radius
        // from the axis for circles and arcs, in the plane for spirals.
        let axis = {
            let p = spec.place([0.0, 0.0, 1.0]);
            [0, 1, 2].map(|k| p[k] - spec.centre[k])
        };
        for s in &segments {
            let (a, _) = s.ends();
            let d = [0, 1, 2].map(|k| a[k] - spec.centre[k]);
            let along: f64 = (0..3).map(|k| d[k] * axis[k]).sum();
            let across = ((0..3).map(|k| d[k] * d[k]).sum::<f64>() - along * along).sqrt();
            match kind {
                CurveKind::Circle | CurveKind::Arc => {
                    assert!(
                        along.abs() < 1e-3 && (across - 1.0).abs() < 1e-3,
                        "{kind:?} {a:?}"
                    )
                }
                CurveKind::Helix => assert!((across - 1.0).abs() < 1e-3, "{kind:?}"),
                _ => assert!(along.abs() < 1e-3 && across > 0.99, "{kind:?}"),
            }
        }
        let errors: Vec<String> = check(&segments, &numeric, 14.2, false)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.message)
            .collect();
        assert!(errors.is_empty(), "{kind:?}: {errors:?}");
    }

    // A circular loop one wavelength round, fed at a segment: about
    // 100-140 ohms near resonance, as published for full-wave loops.
    let lam = 299.792458 / 14.2;
    let mut deck = Deck::parse("CE\nGE 0\nEX 0 1 1 0 1\nFR 0 1 0 0 14.2\nEN\n");
    let mut spec = CurveSpec::new(
        CurveKind::Circle,
        "loop",
        [0.0; 3],
        lam / std::f64::consts::TAU,
    );
    spec.segments = 36;
    spec.tilt = 90.0;
    deck.add_curve(&spec, 1).unwrap();
    let mut numeric = deck.expand().unwrap();
    numeric.ensure_execute();
    let raw = engine()
        .run_text(&numeric.to_nec_text(), &CancelToken::new())
        .unwrap();
    let out = NecOutput::parse(&raw.text);
    let z = out.frequencies[0].inputs[0].impedance;
    assert!((90.0..160.0).contains(&z.re), "{z}");
}

/// Two wires folded back on each other (10 degrees apart) get a sharp-bend
/// warning; a right angle doesn't.
#[test]
fn sharp_bends_are_flagged() {
    use straws_core::analysis::checks::check;
    use straws_core::analysis::plan::geometry_deck;
    for (x2, warned) in [(0.1763, true), (1.0, false)] {
        let text = format!(
            "CE\nGW 1 5 0 0 0 1 0 0 .001\nGW 2 5 0 0 0 1 {x2} 0 .001\nGE 0\nEX 0 1 1 0 1\nFR 0 1 0 0 14\nEN\n"
        );
        let text = if warned {
            text
        } else {
            text.replace("1 1.0 0", "0 1 0")
        };
        let numeric = Deck::parse(&text).expand().unwrap();
        let raw = engine()
            .run_text(&geometry_deck(&numeric).to_nec_text(), &CancelToken::new())
            .unwrap();
        let segments = NecOutput::parse(&raw.text).segments;
        let found = check(&segments, &numeric, 14.0, false)
            .iter()
            .any(|i| i.message.contains("sharply"));
        assert_eq!(found, warned, "{text}");
    }
}

/// Testing protocol stage 2: bad, odd and extreme decks never crash or hang
/// the reader or the engine. Each either reads and runs, or stops with a
/// message (a run that takes too long is cancelled and counted as a hang).
#[test]
fn bad_and_extreme_decks_are_handled() {
    use std::time::{Duration, Instant};
    use straws_core::analysis::plan::geometry_deck;
    let ok = "GW 1 11 0 -5 0 0 5 0 .001\nGE 0\nEX 0 1 6 0 1\nFR 0 1 0 0 14\n";
    let huge: String = (1..=1500)
        .map(|t| {
            format!(
                "GW {t} 1 {} 0 0 {} 0.1 0 .001\n",
                t as f64 * 0.2,
                t as f64 * 0.2
            )
        })
        .collect::<String>()
        + "GE 0\nEX 0 1 1 0 1\nFR 0 1 0 0 14\n";
    let cases: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("only comments", "CM nothing\nCE\n".into()),
        (
            "no GE or EN",
            "GW 1 11 0 -5 0 0 5 0 .001\nEX 0 1 6 0 1\nFR 0 1 0 0 14\n".into(),
        ),
        (
            "no FR",
            "GW 1 11 0 -5 0 0 5 0 .001\nGE 0\nEX 0 1 6 0 1\nEN\n".into(),
        ),
        ("unknown card", format!("ZZ 1 2 3\n{ok}")),
        ("junk", format!("hello world !!\n{ok}")),
        (
            "half line",
            "GW 1 11 0 -5\nGE 0\nEX 0 1 6 0 1\nFR 0 1 0 0 14\n".into(),
        ),
        (
            "extra fields",
            format!("GW 1 11 0 -5 0 0 5 0 .001 7 8 9\n{}", &ok[26..]),
        ),
        ("tabs and commas", ok.replace(' ', ",\t")),
        ("CRLF", ok.replace('\n', "\r\n")),
        ("old Mac CR", ok.replace('\n', "\r")),
        ("BOM", format!("\u{feff}{ok}")),
        ("latin-1 comment", format!("CM caf\u{e9} \u{b0}\n{ok}")),
        (
            "divide by zero",
            format!("SY a=1/0\n{}", ok.replace("-5", "-a")),
        ),
        ("unknown symbol", ok.replace("-5", "-nothing")),
        (
            "symbol loop",
            format!("SY a=b, b=a\n{}", ok.replace("-5", "-a")),
        ),
        (
            "long expression",
            ok.replace("-5", &format!("-({})", vec!["1"; 2000].join("+"))),
        ),
        (
            "zero-length wire",
            ok.replace("0 -5 0 0 5 0", "0 0 0 0 0 0"),
        ),
        (
            "one segment",
            ok.replace("GW 1 11", "GW 1 1")
                .replace("EX 0 1 6", "EX 0 1 1"),
        ),
        (
            "1501 segments (bigger ones take over a minute on slow CI runners in a debug build)",
            ok.replace("GW 1 11", "GW 1 1501")
                .replace("EX 0 1 6", "EX 0 1 751"),
        ),
        ("tiny radius", ok.replace(".001", "1e-9")),
        ("radius > segment", ok.replace(".001", "2")),
        (
            "micro coordinates",
            ok.replace("0 -5 0 0 5 0", "0 -5e-6 0 0 5e-6 0"),
        ),
        (
            "mega coordinates",
            ok.replace("0 -5 0 0 5 0", "0 -5e5 0 0 5e5 0"),
        ),
        (
            "below ground",
            ok.replace("GE 0", "GE 1\nGN 1")
                .replace("0 -5 0 0 5 0", "0 -5 -3 0 5 -3"),
        ),
        ("1 kHz", ok.replace("FR 0 1 0 0 14", "FR 0 1 0 0 0.001")),
        ("100 GHz", ok.replace("FR 0 1 0 0 14", "FR 0 1 0 0 100000")),
        (
            "source on a missing wire",
            ok.replace("EX 0 1 6", "EX 0 7 6"),
        ),
        ("source past the end", ok.replace("EX 0 1 6", "EX 0 1 60")),
        (
            "load on a missing wire",
            ok.replace("EX 0", "LD 4 9 1 1 50 0\nEX 0"),
        ),
        ("huge file", huge),
    ];
    let engine = engine();
    let mut problems = Vec::new();
    for (name, text) in &cases {
        let deck = Deck::parse(text);
        let Ok(mut numeric) = deck.expand() else {
            continue; // refused with a message: fine
        };
        numeric.ensure_execute();
        for (what, run) in [
            ("geometry", geometry_deck(&numeric)),
            ("full", numeric.clone()),
        ] {
            let cancel = CancelToken::new();
            let timer = {
                let cancel = cancel.clone();
                std::thread::spawn(move || {
                    let start = Instant::now();
                    // A hang never finishes; slow machines (CI's macOS runners,
                    // debug builds) only take longer: two minutes is generous.
                    while start.elapsed() < Duration::from_secs(120) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    cancel.cancel();
                })
            };
            let start = Instant::now();
            let result = engine.run_text(&run.to_nec_text(), &cancel);
            if start.elapsed() >= Duration::from_secs(119) {
                problems.push(format!("{name} ({what}): hung"));
            }
            if let Ok(raw) = &result {
                // Whatever came back must parse without panicking.
                let _ = NecOutput::parse(&raw.text);
            }
            drop(timer);
        }
    }
    // Old Mac line endings must read as separate lines.
    assert!(Deck::parse(&ok.replace('\n', "\r")).lines.len() >= 4);
    assert!(problems.is_empty(), "{problems:?}");
}

/// Testing protocol stage 2: every template at the edges of its settings
/// (other bands, every ground, settings scaled down and up, thin and thick
/// wire) reads, lays out and passes the geometry checks.
#[test]
fn templates_at_the_edges() {
    use straws_core::analysis::checks::{Severity, check};
    use straws_core::analysis::plan::{geometry_deck, has_ground};
    use straws_core::templates::{Common, TEMPLATES, TemplateGround};
    let engine = engine();
    let mut bad = Vec::new();
    for t in TEMPLATES {
        for mhz in [1.85, 50.1, 432.0] {
            for ground in [
                TemplateGround::FreeSpace,
                TemplateGround::Real,
                TemplateGround::Perfect,
            ] {
                // Tube sizes that make sense on each band.
                let thick = if mhz > 100.0 {
                    6.0
                } else if mhz > 20.0 {
                    12.0
                } else {
                    25.0
                };
                for (scale, diameter_mm) in [(0.7, 1.0), (1.0, 4.0), (1.3, thick)] {
                    // Settings in MHz follow the band; the others scale.
                    let values: Vec<f64> = t
                        .settings
                        .iter()
                        .map(|s| match s.kind {
                            straws_core::templates::Kind::Mhz => s.default * mhz / 14.2,
                            _ => s.default * scale,
                        })
                        .collect();
                    let common = Common {
                        mhz,
                        diameter_mm,
                        ground,
                        height_wl: t.height_wl,
                    };
                    let text = t.deck(&common, &values);
                    let label =
                        format!("{} {mhz} MHz {ground:?} x{scale} {diameter_mm} mm", t.name);
                    let numeric = match Deck::parse(&text).expand() {
                        Ok(n) => n,
                        Err(e) => {
                            bad.push(format!("{label}: {e}"));
                            continue;
                        }
                    };
                    let raw = match engine
                        .run_text(&geometry_deck(&numeric).to_nec_text(), &CancelToken::new())
                    {
                        Ok(r) => r,
                        Err(e) => {
                            bad.push(format!("{label}: engine {e}"));
                            continue;
                        }
                    };
                    let segments = NecOutput::parse(&raw.text).segments;
                    for i in check(&segments, &numeric, mhz * 1.02, has_ground(&numeric)) {
                        if i.severity == Severity::Error {
                            bad.push(format!("{label}: {}", i.message));
                        }
                    }
                }
            }
        }
    }
    for b in &bad {
        println!("{b}");
    }
    assert!(bad.is_empty(), "{} problems", bad.len());
}

/// Testing protocol stage 3: textbook answers. A thin half-wave dipole in
/// free space has about 73 ohms at resonance and 2.15 dBi; a quarter-wave
/// vertical over perfect ground about 36 ohms and 5.15 dBi.
#[test]
fn textbook_dipole_and_monopole() {
    // (deck geometry, lower and upper R at resonance, lower and upper gain)
    let cases = [
        (
            "GW 1 41 0 0 -0.24 0 0 0.24 1e-4\nGE 0\nEX 0 1 21 0 1\n",
            68.0,
            76.0,
            2.05,
            2.25,
        ),
        (
            "GW 1 21 0 0 0 0 0 0.24 1e-4\nGE 1\nGN 1\nEX 0 1 1 0 1\n",
            33.0,
            38.0,
            5.05,
            5.25,
        ),
    ];
    let engine = engine();
    for (geometry, r_lo, r_hi, g_lo, g_hi) in cases {
        // Find resonance (reactance through zero) in a sweep, then look at it.
        let sweep = format!("{geometry}FR 0 81 0 0 280 1\nXQ\nEN\n");
        let raw = engine.run_text(&sweep, &CancelToken::new()).unwrap();
        let out = NecOutput::parse(&raw.text);
        let z: Vec<(f64, f64, f64)> = out
            .frequencies
            .iter()
            .map(|f| (f.mhz, f.inputs[0].impedance.re, f.inputs[0].impedance.im))
            .collect();
        let k = z
            .windows(2)
            .position(|w| w[0].2 < 0.0 && w[1].2 >= 0.0)
            .expect("resonance");
        let (a, b) = (z[k], z[k + 1]);
        let t = -a.2 / (b.2 - a.2);
        let (mhz, r) = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
        assert!((r_lo..r_hi).contains(&r), "R {r:.1} at {mhz:.2} MHz");
        let one = format!("{geometry}FR 0 1 0 0 {mhz}\nRP 0 19 37 1000 0 0 5 10\nEN\n");
        let raw = engine.run_text(&one, &CancelToken::new()).unwrap();
        let out = NecOutput::parse(&raw.text);
        let peak = out.frequencies[0].patterns[0]
            .points
            .iter()
            .map(|p| p.gain_total)
            .fold(f64::MIN, f64::max);
        assert!((g_lo..g_hi).contains(&peak), "gain {peak:.2} dBi");
    }
}

/// A ground-plane vertical with radials from the radial designer: level
/// radials give the textbook low feed resistance (about 20-25 ohm in free
/// space), and drooping them 45 degrees raises it towards 50 ohm.
#[test]
fn drooping_radials_raise_the_feed_resistance() {
    use straws_core::deck::radials::RadialSpec;
    let engine = engine();
    let r_at = |droop: f64| {
        let mut d = Deck::parse(
            "GW 1 11 0 0 0 0 0 5.0 .001\nGE 0\nEX 0 1 1 0 1\nFR 0 1 0 0 14.2 0\nXQ\nEN\n",
        );
        let spec = RadialSpec {
            name: "radials1".into(),
            count: 4,
            hub: [0.0; 3],
            length: 5.2,
            droop,
            turn: 0.0,
            segments: 11,
            wire_radius: 0.001,
        };
        d.add_radials(&spec, 2).unwrap();
        let text = d.expand().unwrap().to_nec_text();
        let raw = engine.run_text(&text, &CancelToken::new()).unwrap();
        let out = NecOutput::parse(&raw.text);
        out.frequencies[0].inputs[0].impedance
    };
    let (level, drooped) = (r_at(0.0), r_at(45.0));
    assert!((15.0..32.0).contains(&level.re), "level radials {level}");
    assert!(
        drooped.re > level.re + 8.0,
        "drooped {drooped}, level {level}"
    );
}

/// The convergence check: a well-segmented dipole is settled, a dipole of 3
/// segments isn't, and every regression deck's finer copy runs.
#[test]
fn convergence_check() {
    use straws_core::analysis::convergence::{Verdict, compare};
    use straws_core::analysis::plan::FrequencyPlan;
    let engine = engine();
    let plan = FrequencyPlan {
        start_mhz: 13.0,
        stop_mhz: 15.0,
        points: 21,
    };
    let dipole = |segs: i64| {
        Deck::parse(&format!(
            "GW 1 {segs} 0 -5.1 10 0 5.1 10 .001\nGE 0\nEX 0 1 {} 0 1\nFR 0 1 0 0 14\nEN\n",
            segs / 2 + 1
        ))
        .expand()
        .unwrap()
    };
    let good = compare(&engine, &dipole(31), &plan, 14.0, None, &CancelToken::new()).unwrap();
    assert_eq!(good.verdict(), Verdict::Settled, "{good:?}");
    assert!(good.resonance_change().unwrap() < 0.003, "{good:?}");
    let coarse = compare(&engine, &dipole(3), &plan, 14.0, None, &CancelToken::new()).unwrap();
    assert_ne!(coarse.verdict(), Verdict::Settled, "{coarse:?}");
    for path in decks() {
        let deck = Deck::from_file(&path).unwrap().expand().unwrap();
        let mhz = deck.frequencies().first().copied().unwrap_or(14.0);
        let plan = FrequencyPlan::single(mhz);
        let r = compare(&engine, &deck, &plan, mhz, None, &CancelToken::new());
        if let Err(e) = &r {
            // Only a model the check explains it can't refine.
            assert!(
                e.contains("tag 0") || e.contains("no source"),
                "{}: {e}",
                path.display()
            );
        }
    }
}

/// Buried radials, estimated, against Brown, Lewis and Epstein's 1937
/// measurements (Proc. IRE 25(6), Figs. 36-37, as digitised in
/// github.com/stevenmburns/momwire issue 838): 3 MHz, a 21.4 m mast of 2.5 in
/// diameter, No. 8 copper radials 15 cm deep in 2 mS/m soil (permittivity
/// not given: 5 is used, the spread for 5-30 is a couple of ohms).
#[test]
fn buried_radials_match_brown_lewis_and_epstein() {
    use straws_core::analysis::buried::{Need, point};
    use straws_core::deck::radials::RadialSpec;
    let engine = engine();
    let lam = 299.792458 / 3.0;
    // (radials, length in wavelengths, measured base resistance in ohms)
    let cases = [
        (15, 0.411, 34.0),
        (30, 0.411, 30.0),
        (60, 0.411, 26.0),
        (113, 0.411, 24.3),
        (15, 0.137, 33.0),
        (30, 0.137, 31.0),
        (60, 0.137, 31.0),
        (113, 0.137, 31.0),
    ];
    let estimate = |count: usize, length: f64| {
        let mut d = Deck::parse(
            "GW 1 30 0 0 0 0 0 21.4 0.0318\nGE 1\nGN 2 0 0 0 5 0.002\nEX 0 1 1 0 1\nFR 0 1 0 0 3\nEN\n",
        );
        let spec = RadialSpec {
            name: "radials1".into(),
            count,
            hub: [0.0, 0.0, -0.152],
            length: length * lam,
            droop: 0.0,
            turn: 0.0,
            segments: 9,
            wire_radius: 0.00163,
        };
        d.add_radials(&spec, 2).unwrap();
        let n = d.expand().unwrap();
        d.sync_radials(&n, 3.0);
        let n = d.expand().unwrap();
        let Need::Estimate(setup) = straws_core::analysis::buried::need(&d, &n) else {
            panic!("the buried radials weren't found");
        };
        let p = point(&engine, &n, &setup, 3.0, &CancelToken::new()).unwrap();
        p.z_perfect + p.dz
    };
    for (count, length, measured) in cases {
        let z = estimate(count, length);
        println!(
            "{count} x {length} λ: {:.1} ohm (measured {measured})",
            z.re
        );
        assert!((z.re - measured).abs() < 4.0, "{count} x {length}: {z}");
    }
    // Two radials: well above 50 ohms measured.
    assert!(estimate(2, 0.411).re > 45.0);
}

/// Ring and grid counterpoises: raised, they're joined wires NEC runs
/// without geometry errors; buried, a denser grid loses less. (A mesh screens
/// poorly right round a vertical's base, where its cells are as big as
/// anywhere but the return current is strongest, so it needs to be much
/// finer than radials, which crowd together there, to do as well.)
#[test]
fn ring_and_grid_counterpoises() {
    use straws_core::analysis::buried::{Need, need, point};
    use straws_core::analysis::checks::{Severity, check};
    use straws_core::analysis::plan::geometry_deck;
    use straws_core::deck::grids::GridSpec;
    use straws_core::deck::radials::RadialSpec;
    let engine = engine();
    let lam = 299.792458 / 7.0;
    let base =
        "GW 1 20 0 0 0 0 0 10.2 .001\nGE 1\nGN 2 0 0 0 13 .005\nEX 0 1 1 0 1\nFR 0 1 0 0 7\nEN\n";
    let grid = |cells: usize, side: f64, z: f64| GridSpec {
        name: "grid1".into(),
        centre: [0.0, 0.0, z],
        width: side,
        length: side,
        turn: 0.0,
        cells: [cells, cells],
        segments: 3,
        wire_radius: 0.001,
    };
    let errors = |d: &Deck| {
        let n = d.expand().unwrap();
        let raw = engine
            .run_text(&geometry_deck(&n).to_nec_text(), &CancelToken::new())
            .unwrap();
        let segs = NecOutput::parse(&raw.text).segments;
        check(&segs, &n, 7.0, true)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.message)
            .collect::<Vec<_>>()
    };
    // Raised 2 m (the vertical stands on the grid's middle node / the hub).
    let raised = base.replace("0 0 0 0 0 10.2", "0 0 2 0 0 12.2");
    let mut d = Deck::parse(&raised);
    d.add_grid(&grid(4, 6.0, 2.0), 2).unwrap();
    assert!(errors(&d).is_empty(), "{:?}", errors(&d));
    let mut d = Deck::parse(&raised);
    let spec = RadialSpec {
        name: "radials1".into(),
        count: 6,
        hub: [0.0, 0.0, 2.0],
        length: lam / 4.0,
        droop: 0.0,
        turn: 0.0,
        segments: 9,
        wire_radius: 0.001,
    };
    d.add_radials(&spec, 2).unwrap();
    d.set_radial_ring("radials1", Some(5)).unwrap();
    assert!(errors(&d).is_empty(), "{:?}", errors(&d));
    // Buried 10 cm: the estimate.
    let estimate = |d: &mut Deck| {
        let n = d.expand().unwrap();
        d.sync_radials(&n, 7.0);
        d.sync_grids(&n, 7.0);
        let n = d.expand().unwrap();
        let Need::Estimate(setup) = need(d, &n) else {
            panic!("not estimated");
        };
        let p = point(&engine, &n, &setup, 7.0, &CancelToken::new()).unwrap();
        (p.z_perfect + p.dz).re
    };
    let side = lam / 2.0;
    let r: Vec<f64> = [2, 4, 8, 16, 40]
        .into_iter()
        .map(|cells| {
            let mut d = Deck::parse(base);
            d.add_grid(&grid(cells, side, -0.1), 2).unwrap();
            estimate(&mut d)
        })
        .collect();
    println!("buried grid, λ/2 square, 2-40 cells: {r:?}");
    assert!(r.windows(2).all(|w| w[1] < w[0]), "{r:?}");
    // All better than bare soil (about 130 ohm here); the finest within
    // reach of 16 radials.
    assert!(r[0] < 125.0 && r[4] < 50.0, "{r:?}");
    // 16 quarter-wave radials (about 9 ohm of loss) do better than an
    // 8 x 8 grid across the same reach, as the mesh is coarse at the base.
    let mut d = Deck::parse(base);
    let mut s16 = spec.clone();
    (s16.count, s16.hub) = (16, [0.0, 0.0, -0.1]);
    d.add_radials(&s16, 2).unwrap();
    let radials = estimate(&mut d);
    println!("16 radials: {radials:.1}, 8 x 8 grid: {:.1}", r[2]);
    assert!(radials < r[2] && radials > 38.0);
}

/// Two parallel wires close side by side are "close", not a near miss
/// between ends (that advice, "make the end points the same", was wrong for
/// them); two ends close together still are a near miss.
#[test]
fn side_by_side_wires_are_close_not_a_near_miss() {
    use straws_core::analysis::checks::check;
    use straws_core::analysis::plan::geometry_deck;
    let engine = engine();
    let messages = |geometry: &str| -> Vec<String> {
        let n = Deck::parse(&format!(
            "CE\n{geometry}GE 0\nEX 0 1 11 0 1\nFR 0 1 0 0 14.2\nEN\n"
        ))
        .expand()
        .unwrap();
        let raw = engine
            .run_text(&geometry_deck(&n).to_nec_text(), &CancelToken::new())
            .unwrap();
        check(&NecOutput::parse(&raw.text).segments, &n, 14.2, false)
            .into_iter()
            .map(|i| i.message)
            .collect()
    };
    let side = messages("GW 1 21 0 -5 10 0 5 10 .001\nGW 2 21 0 -5 10.003 0 5 10.003 .001\n");
    assert!(side.iter().any(|m| m.contains("side by side")), "{side:?}");
    assert!(!side.iter().any(|m| m.contains("nearly meet")), "{side:?}");
    let ends = messages("GW 1 21 0 -5 10 0 5 10 .001\nGW 2 21 0 5.003 10 0 10 10 .001\n");
    assert!(ends.iter().any(|m| m.contains("nearly meet")), "{ends:?}");
}

/// A half-wave dipole half a wavelength in front of a 90° corner reflector:
/// with big sheets (2 λ by 2 λ) the textbook 10 dBd (12.15 dBi); smaller
/// ones give less (1 λ: about 10.4 dBi). A flat plate a quarter wavelength
/// behind gives less than the corner. Mesh 0.1 λ, 300 MHz.
#[test]
fn corner_and_plate_reflectors_beam_forwards() {
    use straws_core::analysis::pattern::{Component, PatternGrid};
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    use straws_core::deck::plates::{PlateKind, PlateSpec};
    let engine = engine();
    // (kind, side or width, height, cells across, cells up, dipole spacing)
    let gain = |kind: PlateKind, width: f64, height: f64, cells: [usize; 2], spacing: f64| {
        let mut d = Deck::parse(&format!(
            "GW 1 21 {spacing} 0 -0.24 {spacing} 0 0.24 .001
GE 0
EX 0 1 11 0 1
FR 0 1 0 0 299.8
EN
"
        ));
        let spec = PlateSpec {
            name: "r1".into(),
            kind,
            centre: [0.0, 0.0, 0.0],
            width,
            height,
            facing: 0.0,
            angle: if kind == PlateKind::Corner { 90.0 } else { 0.0 },
            cells,
            segments: 1,
            wire_radius: 0.001,
        };
        d.add_plate(&spec, 2).unwrap();
        let n = d.expand().unwrap();
        let run = run_deck(
            &n,
            &FrequencyPlan::single(299.8),
            Some(PatternRequest::new(5.0)),
        );
        let out = engine.run(&run, &CancelToken::new()).unwrap();
        let grid = PatternGrid::from_table(&out.frequencies[0].patterns[0]).unwrap();
        let (peak, theta, phi) = grid.max(Component::Total);
        assert!((theta - 90.0).abs() < 1.0 && (phi.abs() < 1.0 || (phi - 360.0).abs() < 1.0));
        let back = grid.sample(Component::Total, 90.0, 180.0);
        (peak, peak - back)
    };
    let (big, big_fb) = gain(PlateKind::Corner, 2.0, 2.0, [40, 20], 0.5);
    println!("corner 2 λ: {big:.2} dBi, F/B {big_fb:.1} dB");
    assert!((big - 12.15).abs() < 0.5 && big_fb > 15.0, "{big} {big_fb}");
    let (small, _) = gain(PlateKind::Corner, 1.0, 1.0, [20, 10], 0.5);
    println!("corner 1 λ: {small:.2} dBi");
    assert!((9.8..11.0).contains(&small), "{small}");
    let (flat, flat_fb) = gain(PlateKind::Flat, 1.0, 1.0, [10, 10], 0.25);
    println!("flat plate 1 λ: {flat:.2} dBi, F/B {flat_fb:.1} dB");
    assert!(
        (6.0..small).contains(&flat) && flat_fb > 10.0,
        "{flat} {flat_fb}"
    );
}

/// A 3 λ dish (f/D 0.4, mesh 0.1 λ) fed at its focus. (πD/λ)² is 19.5 dBi
/// at full aperture efficiency. A bare dipole radiates as much away from the
/// dish as into it, so it loses about half: around 13 dBi. With a reflector
/// element a quarter wave behind it (the usual dish feed) it does much better.
#[test]
fn a_dish_with_a_dipole_at_its_focus() {
    use straws_core::analysis::pattern::{Component, PatternGrid};
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    use straws_core::deck::reflectors::{Outline, Profile, ReflectorSpec};
    let gain = |feed_reflector: bool| {
        let mut d = Deck::parse(
            "GE 0
FR 0 1 0 0 299.8
EN
",
        );
        let spec = ReflectorSpec {
            name: "dish1".into(),
            outline: Outline::Ellipse,
            profile: Profile::Paraboloid,
            vertex: [0.0; 3],
            facing: [0.0, 0.0],
            size: [3.0, 3.0],
            offset: [0.0, 0.0],
            focal: [1.2, 1.2],
            radius: 1.0,
            angle: 90.0,
            cells: [30, 30],
            segments: 1,
            wire_radius: 0.001,
        };
        d.add_reflector(&spec, 1).unwrap();
        d.dipole_at_focus("dish1", 0.24, 11, 0.001, 1).unwrap();
        if feed_reflector {
            // On the far side from the dish, a little longer than the dipole.
            let tag = 9000;
            d.add_wire(tag, 11, [1.45, -0.255, 0.0], [1.45, 0.255, 0.0], 0.001);
        }
        let n = d.expand().unwrap();
        let run = run_deck(
            &n,
            &FrequencyPlan::single(299.8),
            Some(PatternRequest::new(2.0)),
        );
        let out = engine().run(&run, &CancelToken::new()).unwrap();
        let grid = PatternGrid::from_table(&out.frequencies[0].patterns[0]).unwrap();
        let (peak, theta, phi) = grid.max(Component::Total);
        assert!((theta - 90.0).abs() < 2.1 && (phi.abs() < 2.1 || (phi - 360.0).abs() < 2.1));
        peak
    };
    let bare = gain(false);
    let fed = gain(true);
    println!("dish: bare dipole {bare:.2} dBi, dipole and reflector {fed:.2} dBi");
    assert!((11.5..15.0).contains(&bare), "{bare}");
    assert!(fed > bare + 2.0 && fed < 19.5, "{fed}");
}

#[test]
fn a_monopole_on_a_patch_box() {
    use straws_core::analysis::checks::patch_checks;
    use straws_core::deck::patches::{BoxSpec, patch_count};
    // 300 MHz: a quarter-wave monopole on the middle of a 1 m x 1 m x 0.3 m
    // box (odd patch counts put a patch centre where the wire stands).
    let mut deck = Deck::parse(
        "CE\nGW 1 9 0 0 0.3 0 0 0.55 0.002\nGE 0\nEX 0 1 1 0 1\nFR 0 1 0 0 300\nXQ\nEN\n",
    );
    let cells = [11, 11, 3];
    deck.add_patch_box(&BoxSpec {
        name: "box1".into(),
        base: [0.0; 3],
        size: [1.0, 1.0, 0.3],
        facing: 0.0,
        cells,
    })
    .unwrap();
    let n = deck.expand().unwrap();
    let out = engine().run(&n, &CancelToken::new()).unwrap();
    // The patch under the wire is split into four.
    assert_eq!(out.patches.len(), patch_count(cells) + 3);
    for p in &out.patches {
        let r = [p.centre[0], p.centre[1], p.centre[2] - 0.15];
        let outwards: f64 = (0..3).map(|k| r[k] * p.normal[k]).sum();
        assert!(outwards > 0.0, "{p:?}");
    }
    assert!(patch_checks(&out.patches, 300.0, false).is_empty());
    let z = out.frequencies[0].inputs[0].impedance;
    println!("monopole on a patch box: {z}");
    // Roughly a monopole on a ground plane about a wavelength across.
    assert!((25.0..60.0).contains(&z.re), "{z}");
    // A flat sheet of patches is flagged as open.
    let open = Deck::parse(
        "CE\nSM 6 6 -.5 -.5 0 .5 -.5 0\nSC 0 0 .5 .5 0\nGW 1 9 0 0 .1 0 0 .35 .002\nGE 0\nEX 0 1 1 0 1\nFR 0 1 0 0 300\nXQ\nEN\n",
    );
    let out = engine()
        .run(&open.expand().unwrap(), &CancelToken::new())
        .unwrap();
    assert_eq!(patch_checks(&out.patches, 300.0, false).len(), 1);
}

/// The optimiser scores a model with buried radials by their estimate (as
/// the views show it), not by the bare soil the engine sees.
#[test]
fn the_optimiser_includes_the_buried_radial_estimate() {
    use straws_core::analysis::buried::{Need, need, point};
    use straws_core::analysis::feed::swr;
    use straws_core::deck::radials::RadialSpec;
    use straws_core::optimise::{Goal, Param, Problem};
    let engine = engine();
    let mut d = Deck::parse(
        "SY h=10.2\nGW 1 20 0 0 0 0 0 h .001\nGE 1\nGN 2 0 0 0 13 .005\nEX 0 1 1 0 1\nFR 0 1 0 0 7\nEN\n",
    );
    d.add_radials(
        &RadialSpec {
            name: "radials1".into(),
            count: 16,
            hub: [0.0, 0.0, -0.1],
            length: 299.792458 / 7.0 / 4.0,
            droop: 0.0,
            turn: 0.0,
            segments: 9,
            wire_radius: 0.001,
        },
        2,
    )
    .unwrap();
    let n = d.expand().unwrap();
    d.sync_radials(&n, 7.0);
    let n = d.expand().unwrap();
    let Need::Estimate(setup) = need(&d, &n) else {
        panic!("not estimated");
    };
    let p = point(&engine, &n, &setup, 7.0, &CancelToken::new()).unwrap();
    let z = p.z_perfect + p.dz;
    let problem = Problem {
        deck: d,
        fixed: vec![],
        params: vec![Param {
            name: "h".into(),
            min: 9.0,
            max: 11.0,
            start: 10.2,
        }],
        goals: vec![Goal::Swr {
            start_mhz: 7.0,
            stop_mhz: 7.0,
            points: 1,
            z0: 50.0,
            target: 1.5,
            weight: 1.0,
            through: None,
        }],
        limits: Default::default(),
    };
    let t = problem.evaluate(&engine, &[10.2], &CancelToken::new());
    println!(
        "estimate {z}: SWR {:.3}, scored {:.3}",
        swr(z, 50.0),
        t.results[0].value
    );
    assert!((t.results[0].value - swr(z, 50.0)).abs() < 0.01, "{t:?}");
}

/// Sommerfeld tables read back from the cache give exactly the output the
/// worked-out tables do.
#[test]
fn cached_sommerfeld_tables_change_nothing() {
    let deck = Deck::from_file(repo().join("tests/decks/inverted_v_sommerfeld.nec"))
        .unwrap()
        .expand()
        .unwrap();
    let text = deck.to_nec_text();
    let cancel = CancelToken::new();
    let strip = |t: String| {
        t.lines()
            .filter(|l| !l.contains("RUN TIME") && !l.contains("FILL:"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let fresh = strip(
        engine()
            .without_somnec_cache()
            .run_text(&text, &cancel)
            .unwrap()
            .text,
    );
    // The first cached run may write the tables; the second reads them.
    for _ in 0..2 {
        let cached = strip(engine().run_text(&text, &cancel).unwrap().text);
        assert!(cached == fresh, "cached tables changed the output");
    }
    assert!(straws_core::engine::somnec_cache().is_some());
}

/// Random builds of a dipole: a sloppier build spreads its resonance more.
#[test]
fn tolerance_spread_grows_with_build_accuracy() {
    use straws_core::analysis::plan::FrequencyPlan;
    use straws_core::analysis::tolerance::{percentiles, spread};
    let deck = Deck::parse(
        "CE\nGW 1 21 0 -5.1 10 0 5.1 10 .001\nGE 0\nEX 0 1 11 0 1\nFR 0 1 0 0 14\nEN\n",
    )
    .expand()
    .unwrap();
    let plan = FrequencyPlan {
        start_mhz: 13.0,
        stop_mhz: 15.0,
        points: 21,
    };
    let width = |accuracy: f64| {
        let s = spread(
            &engine(),
            &deck,
            &plan,
            14.0,
            accuracy,
            12,
            6,
            false,
            None,
            &CancelToken::new(),
            &|_, _| {},
        )
        .unwrap();
        assert_eq!(s.builds.len(), 12);
        let mut r: Vec<f64> = s.builds.iter().filter_map(|b| b.resonance).collect();
        let p = percentiles(&mut r).unwrap();
        p[4] - p[0]
    };
    let (tight, loose) = (width(0.002), width(0.05));
    println!("resonance spread: ±2 mm {tight:.4} MHz, ±50 mm {loose:.4} MHz");
    assert!(tight < 0.02 && loose > 5.0 * tight, "{tight} {loose}");
}

/// Patterns read from the engine's binary file match the printed ones (to
/// the 0.01 dB and 5 figures the text keeps), for every public deck.
#[test]
fn binary_patterns_match_the_text() {
    let engine = engine();
    let cancel = CancelToken::new();
    let (mut tables, mut currents) = (0, 0);
    for path in decks() {
        let mut deck = Deck::from_file(&path).unwrap().expand().unwrap();
        deck.ensure_execute();
        let binary = engine.run(&deck, &cancel).unwrap();
        let text = NecOutput::parse(&engine.run_text(&deck.to_nec_text(), &cancel).unwrap().text);
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(binary.frequencies.len(), text.frequencies.len(), "{name}");
        for (fb, ft) in binary.frequencies.iter().zip(&text.frequencies) {
            // Currents: printed to 5 figures of the largest component.
            assert_eq!(fb.currents.len(), ft.currents.len(), "{name}");
            let largest = ft
                .currents
                .iter()
                .map(|c| c.current.norm())
                .fold(0.0, f64::max);
            for (b, t) in fb.currents.iter().zip(&ft.currents) {
                currents += 1;
                assert_eq!((b.segment, b.tag), (t.segment, t.tag), "{name}");
                assert!(
                    (b.current - t.current).norm() <= 1e-4 * largest + 1e-30,
                    "{name}: {b:?} {t:?}"
                );
                for k in 0..3 {
                    assert!((b.centre_wl[k] - t.centre_wl[k]).abs() <= 6e-5, "{name}");
                }
            }
            assert_eq!(fb.patterns.len(), ft.patterns.len(), "{name}");
            for (tb, tt) in fb.patterns.iter().zip(&ft.patterns) {
                tables += 1;
                assert_eq!(tb.gain_kind, tt.gain_kind, "{name}");
                assert_eq!(tb.points.len(), tt.points.len(), "{name}");
                for (b, t) in tb.points.iter().zip(&tt.points) {
                    let close = |x: f64, y: f64, tol: f64| (x - y).abs() <= tol;
                    assert!(
                        close(b.theta, t.theta, 0.006) && close(b.phi, t.phi, 0.006),
                        "{name}"
                    );
                    assert!(
                        close(b.gain_total, t.gain_total, 0.0051),
                        "{name}: {b:?} {t:?}"
                    );
                    assert!(close(b.gain_vert, t.gain_vert, 0.0051), "{name}");
                    assert!(close(b.gain_hor, t.gain_hor, 0.0051), "{name}");
                    assert_eq!(b.sense, t.sense, "{name}");
                    let rel = |x: f64, y: f64| (x - y).abs() <= 1e-4 * x.abs().max(y.abs()) + 1e-30;
                    assert!(
                        rel(b.e_theta.0, t.e_theta.0) && rel(b.e_phi.0, t.e_phi.0),
                        "{name}"
                    );
                }
            }
        }
    }
    assert!(tables > 10, "only {tables} pattern tables compared");
    assert!(currents > 100, "only {currents} currents compared");
}

/// Trimming the Yagi: a longer driven element lowers the resonance, and it
/// moves the resonance more than the parasitic elements do.
#[test]
fn sensitivity_finds_the_driven_element() {
    use straws_core::analysis::plan::FrequencyPlan;
    use straws_core::analysis::sensitivity::trims;
    let deck = Deck::from_file(repo().join("tests/decks/yagi_3el_aluminium.nec"))
        .unwrap()
        .expand()
        .unwrap();
    let plan = FrequencyPlan {
        start_mhz: 13.5,
        stop_mhz: 14.8,
        points: 27,
    };
    let t = trims(
        &engine(),
        &deck,
        &[1, 2, 3],
        &plan,
        14.15,
        0.1,
        3,
        false,
        None,
        &CancelToken::new(),
        &|_, _| {},
    )
    .unwrap();
    let r0 = t.design.resonance.unwrap();
    let shift: Vec<(i64, f64)> = t
        .wires
        .iter()
        .map(|(tag, f)| (*tag, f.resonance.unwrap() - r0))
        .collect();
    println!("resonance shifts for 10 cm longer: {shift:?}");
    assert_eq!(shift.len(), 3);
    let driven = shift.iter().find(|s| s.0 == 2).unwrap().1;
    assert!(driven < 0.0, "{shift:?}");
    assert!(
        shift.iter().all(|s| s.0 == 2 || s.1.abs() < driven.abs()),
        "{shift:?}"
    );
}

/// Timing check (not run by default): one slider update (a single
/// frequency with a 5-degree pattern) of the 315-segment Yagi over
/// Sommerfeld ground, as the app runs it while dragging.
/// `cargo test --release -- --ignored slider_update_speed --nocapture`.
#[test]
#[ignore]
fn slider_update_speed() {
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    let deck = Deck::from_file(repo().join("benchmarks/04_medium_yagi_sweep_ground.nec"))
        .unwrap()
        .expand()
        .unwrap();
    let engine = engine();
    let cancel = CancelToken::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let _ = engine
            .run_text(
                "CE
GW 1 3 0 0 0 0 0 1 .001
GE 0
EN
",
                &cancel,
            )
            .unwrap();
        println!(
            "tiny geometry run: {:.1} ms",
            t.elapsed().as_secs_f64() * 1e3
        );
    }
    for mhz in [28.3, 28.31, 28.3] {
        let run = run_deck(
            &deck,
            &FrequencyPlan::single(mhz),
            Some(PatternRequest::new(5.0)),
        );
        let t = std::time::Instant::now();
        let out = engine.run(&run, &cancel).unwrap();
        let geometry = std::time::Instant::now();
        let _ = engine
            .run_text(
                &straws_core::analysis::plan::geometry_deck(&deck).to_nec_text(),
                &cancel,
            )
            .unwrap();
        println!(
            "{mhz} MHz: solve + pattern {:.0} ms ({} points), geometry {:.0} ms",
            (geometry - t).as_secs_f64() * 1e3,
            out.frequencies[0].patterns[0].points.len(),
            geometry.elapsed().as_secs_f64() * 1e3
        );
    }
}

/// Stress check (not run by default): many pattern runs at once must all
/// come back complete.
#[test]
#[ignore]
fn binary_patterns_under_load() {
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    let deck = Deck::from_file(repo().join("tests/decks/yagi_3el_aluminium.nec"))
        .unwrap()
        .expand()
        .unwrap();
    let run = run_deck(
        &deck,
        &FrequencyPlan::single(14.15),
        Some(PatternRequest::new(5.0)),
    );
    let engine = engine();
    let bad = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for t in 0..12 {
            let (run, engine, bad) = (&run, &engine, &bad);
            s.spawn(move || {
                for k in 0..10 {
                    match engine.run(run, &CancelToken::new()) {
                        Ok(out) => {
                            let n = out
                                .frequencies
                                .first()
                                .and_then(|f| f.patterns.first())
                                .map_or(0, |p| p.points.len());
                            if n != 2701 {
                                bad.lock()
                                    .unwrap()
                                    .push(format!("thread {t} run {k}: {n} points"));
                            }
                        }
                        Err(e) => bad.lock().unwrap().push(format!("thread {t} run {k}: {e}")),
                    }
                }
            });
        }
    });
    let bad = bad.into_inner().unwrap();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Persistent workers give exactly a fresh process's output, deck after
/// deck in any order and after errors, with the Sommerfeld tables worked
/// out each time (so nothing can carry over between runs unseen).
#[test]
fn persistent_workers_match_fresh_processes() {
    let strip = |t: &str| {
        t.lines()
            .filter(|l| !l.contains("RUN TIME") && !l.contains("FILL:"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let cancel = CancelToken::new();
    let fresh = engine()
        .fresh_processes()
        .without_somnec_cache()
        .with_solver_threads(1);
    // One solver thread: the persistent worker is reused for every run.
    let kept = engine().without_somnec_cache().with_solver_threads(1);
    let texts: Vec<(String, String)> = decks()
        .iter()
        .map(|p| {
            let mut d = Deck::from_file(p).unwrap().expand().unwrap();
            d.ensure_execute();
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (name, d.to_nec_text())
        })
        .collect();
    let want: Vec<String> = texts
        .iter()
        .map(|(_, t)| strip(&fresh.run_text(t, &cancel).unwrap().text))
        .collect();
    let broken = "CE\nGW 1 9 0 0 -1 0 0 1 .001\nGE\nEX 0 1 30 0 1\nFR 0 1 0 0 100 0\nXQ\nEN\n";
    // Forwards, then backwards, with a deck that stops with an error between.
    let order: Vec<usize> = (0..texts.len()).chain((0..texts.len()).rev()).collect();
    for (k, &i) in order.iter().enumerate() {
        if k % 5 == 2 {
            assert!(kept.run_text(broken, &cancel).is_err());
        }
        let got = strip(&kept.run_text(&texts[i].1, &cancel).unwrap().text);
        assert!(
            got == want[i],
            "{} differs from a fresh process (run {k})",
            texts[i].0
        );
    }
}

/// Leak check (not run by default): one persistent worker runs a deck many
/// times; watch its memory meanwhile (`straws-worker` in Task Manager).
/// `cargo test --release -- --ignored persistent_worker_memory --nocapture`.
#[test]
#[ignore]
fn persistent_worker_memory() {
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    let deck = Deck::from_file(repo().join("benchmarks/04_medium_yagi_sweep_ground.nec"))
        .unwrap()
        .expand()
        .unwrap();
    let run = run_deck(
        &deck,
        &FrequencyPlan::single(28.3),
        Some(PatternRequest::new(5.0)),
    );
    let engine = engine().with_solver_threads(1);
    for k in 0..200 {
        engine.run(&run, &CancelToken::new()).unwrap();
        if k % 50 == 0 {
            println!("run {k}");
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }
}

/// Two quarter-wave verticals a quarter wave apart, fed equal currents 90°
/// apart: a cardioid towards the lagging one (the textbook end-fire pair).
/// Swapping the phases turns it round; the feed results of both come back.
#[test]
fn phased_verticals_steer_with_their_phases() {
    use num_complex::Complex64;
    use straws_core::analysis::pattern::{Component, PatternGrid};
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    use straws_core::analysis::sweep::FreqPoint;
    use straws_core::deck::sources::sources;
    let mut d = Deck::parse(
        "CE\nGW 1 11 0 0 0 0 0 10.56 .01\nGW 2 11 10.56 0 0 10.56 0 10.56 .01\nGE 1\nGN 1\nEX 6 1 1 0 1 0\nEX 6 2 1 0 0 -1\nFR 0 1 0 0 7.1 0\nEN\n",
    );
    let engine = engine();
    let run = |d: &Deck| {
        let n = d.expand().unwrap();
        let r = run_deck(
            &n,
            &FrequencyPlan::single(7.1),
            Some(PatternRequest::new(5.0)),
        );
        let out = engine.run(&r, &CancelToken::new()).unwrap();
        let p = FreqPoint::from_result(&out.frequencies[0], Some(5.0)).without_helpers(&n);
        let g = PatternGrid::from_table(&out.frequencies[0].patterns[0]).unwrap();
        let front = g.sample(Component::Total, 70.0, 0.0);
        let back = g.sample(Component::Total, 70.0, 180.0);
        (p, front - back)
    };
    let (p, fb) = run(&d);
    println!(
        "end-fire: F/B {fb:.1} dB, feeds {:?}",
        p.inputs.iter().map(|i| i.impedance).collect::<Vec<_>>()
    );
    assert_eq!(p.inputs.len(), 2);
    for i in &p.inputs {
        assert!((i.current.norm() - 1.0).abs() < 1e-3, "{i:?}");
        assert!(i.impedance.re > 0.0 && i.power > 0.0, "{i:?}");
    }
    assert!(fb > 15.0, "F/B {fb}");
    // Lead the second element instead: the beam turns round.
    let n = d.expand().unwrap();
    let line = sources(&d, &n)[1].line;
    d.set_source(line, Complex64::new(0.0, 1.0)).unwrap();
    let (_, fb) = run(&d);
    assert!(fb < -15.0, "F/B {fb}");
}

/// RDF: a half-wave dipole in free space has directivity 2.15 dBi, so its
/// peak is 2.15 dB above the average over all directions; losses don't
/// change it (they lower the peak and the average alike).
#[test]
fn rdf_of_a_dipole_is_its_directivity() {
    use straws_core::analysis::checks::rdf;
    use straws_core::analysis::pattern::PatternGrid;
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    let run = |text: &str| {
        let n = Deck::parse(text).expand().unwrap();
        let r = run_deck(
            &n,
            &FrequencyPlan::single(14.2),
            Some(PatternRequest::new(5.0)),
        );
        let out = engine().run(&r, &CancelToken::new()).unwrap();
        rdf(&PatternGrid::from_table(&out.frequencies[0].patterns[0]).unwrap()).unwrap()
    };
    let bare = "CE\nGW 1 21 0 -5.1 0 0 5.1 0 .001\nGE 0\nEX 0 1 11 0 1\nFR 0 1 0 0 14.2\nEN\n";
    let lossy = bare.replace("GE 0\n", "GE 0\nLD 5 0 0 0 1e5\n");
    let (a, b) = (run(bare), run(&lossy));
    println!("dipole RDF {a:.3} dB, lossy {b:.3} dB");
    assert!((a - 2.15).abs() < 0.05 && (a - b).abs() < 0.02, "{a} {b}");
}

/// The build sheet of a 3-element Yagi with a split driven element: a cut
/// list grouping the driven halves, the stock total, and element positions
/// along the boom (the halves sharing one row).
#[test]
fn build_sheet_of_a_yagi() {
    use straws_core::analysis::build::build_sheet;
    let text = "CE\n\
        GW 1 21 -2 -5.3 10 -2 5.3 10 .006 ' Reflector\n\
        GW 2 10 0 -5 10 0 0 10 .006 ' Driven left\n\
        GW 3 10 0 0 10 0 5 10 .006 ' Driven right\n\
        GW 4 21 1.8 -4.8 10 1.8 4.8 10 .006 ' Director\n\
        GE 0\nLD 5 0 0 0 3.5e7\nEX 0 2 10 0 1\nFR 0 1 0 0 14.2\nEN\n";
    let deck = Deck::parse(text);
    let n = deck.expand().unwrap();
    let parsed = engine().run(&n, &CancelToken::new()).unwrap();
    let sheet = build_sheet(&parsed.segments, &deck, &n, &[]);
    assert_eq!(sheet.pieces.len(), 4);
    let names: Vec<&str> = sheet.pieces.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["Reflector", "Driven left", "Driven right", "Director"]
    );
    // The driven halves are one cut of two.
    let halves = sheet.cuts.iter().find(|c| c.count == 2).unwrap();
    assert!((halves.length - 5.0).abs() < 1e-3, "{halves:?}");
    assert!(
        sheet
            .cuts
            .iter()
            .all(|c| c.straight && (c.diameter - 0.012).abs() < 1e-6)
    );
    assert_eq!(sheet.stock.len(), 1);
    assert!(
        (sheet.stock[0].total - (10.6 + 10.0 + 9.6)).abs() < 1e-9,
        "{:?}",
        sheet.stock
    );
    let p = sheet.positions.unwrap();
    let at: Vec<f64> = p.rows.iter().map(|r| r.1).collect();
    assert_eq!(p.rows.len(), 3);
    assert!(
        (at[0]).abs() < 1e-6 && (at[1] - 2.0).abs() < 1e-3 && (at[2] - 3.8).abs() < 1e-3,
        "{at:?}"
    );
    assert_eq!(p.rows[1].0, "Driven left + Driven right");
    let html = straws_core::export::build_sheet_html(
        "test",
        &build_sheet(&parsed.segments, &deck, &n, &[]),
        straws_core::units::Length::Millimetres,
        14.2,
    );
    assert!(
        html.contains("3800.0 mm")
            && html.contains("10600.0 mm")
            && html.contains("about Aluminium"),
        "{html}"
    );
}

/// Writes a sample build sheet (the Yagi template) to the temp folder, to
/// look at: `cargo test -p straws-core --test regression -- --ignored
/// sample_build_sheet`.
#[test]
#[ignore]
fn sample_build_sheet() {
    use straws_core::templates::{Common, TEMPLATES};
    let t = TEMPLATES
        .iter()
        .find(|t| t.name.starts_with("Yagi"))
        .unwrap();
    let common = Common {
        mhz: 14.2,
        diameter_mm: 25.0,
        ground: t.ground,
        height_wl: t.height_wl,
    };
    let deck = Deck::parse(&t.deck(&common, &t.defaults()));
    let n = deck.expand().unwrap();
    let out = engine().run(&n, &CancelToken::new()).unwrap();
    let sheet = straws_core::analysis::build::build_sheet(&out.segments, &deck, &n, &[]);
    let html = straws_core::export::build_sheet_html(
        "Yagi template",
        &sheet,
        straws_core::units::Length::Millimetres,
        14.2,
    );
    let path = std::env::temp_dir().join("straws-build-sheet.html");
    std::fs::write(&path, html).unwrap();
    println!("{}", path.display());
}

/// Phasing lines checked by NEC: two quarter-wave verticals a quarter wave
/// apart over perfect ground, wanted 1 A at 0° and 1 A at -90° (end-fire).
/// The lines are designed from the active impedances of a run with those
/// currents, written into the deck as TL cards fed from one junction, and
/// run: the elements must get the designed currents (coupling and all),
/// and the junction the designed impedance.
#[test]
fn phasing_lines_give_the_currents_they_promise() {
    use num_complex::Complex64;
    use straws_core::analysis::feedline::{CABLES, Cable};
    use straws_core::analysis::phasing::{design, write_lines};
    use straws_core::analysis::plan::{FrequencyPlan, run_deck};
    use straws_core::analysis::sweep::FreqPoint;
    use straws_core::deck::sources::sources;
    let mhz = 7.1;
    let deck = Deck::parse(
        "CE\nGW 1 11 0 0 0 0 0 10.56 .01\nGW 2 11 10.56 0 0 10.56 0 10.56 .01\nGE 1\nGN 1\nEX 6 1 1 0 1 0\nEX 6 2 1 0 0 -1\nFR 0 1 0 0 7.1 0\nEN\n",
    );
    let engine = engine();
    let run = |d: &Deck| {
        let n = d.expand().unwrap();
        let out = engine
            .run(
                &run_deck(&n, &FrequencyPlan::single(mhz), None),
                &CancelToken::new(),
            )
            .unwrap();
        (
            FreqPoint::from_result(&out.frequencies[0], None).without_helpers(&n),
            out.segments,
        )
    };
    let (p, _) = run(&deck);
    let z: Vec<Complex64> = p.inputs.iter().map(|i| i.impedance).collect();
    let want: Vec<Complex64> = p.inputs.iter().map(|i| i.current).collect();
    // RG-213's velocity factor, without its loss (NEC's lines are
    // lossless), so the design and the check describe the same lines.
    let cable = Cable {
        loss_10: 0.0,
        loss_100: 0.0,
        ..CABLES[2]
    };
    let d = design(&z, &want, &[6.0, 6.0], &cable, mhz, 0.25).unwrap();
    println!(
        "lines {:?} m, worst {:.2} dB {:.3}°, junction {}",
        d.lengths, d.worst_db, d.worst_deg, d.junction
    );
    assert!(d.worst_deg < 0.01);
    let mut lined = deck.clone();
    let n = deck.expand().unwrap();
    write_lines(
        &mut lined,
        &sources(&deck, &n),
        &d,
        &cable,
        mhz,
        3,
        [5.28, 0.0, 12.0],
    );
    // The lines are found again, with what they're meant to give.
    let net = straws_core::analysis::phasing::network(&lined);
    assert_eq!(net.len(), 2);
    assert_eq!((net[1].tag, net[1].segment), (2, 1));
    assert_eq!(net[1].cable, cable.name);
    assert!(
        (net[1].length_m.unwrap() - d.lengths[1]).abs() < 1e-3,
        "{net:?}"
    );
    assert!((net[1].wanted.unwrap() - want[1]).norm() < 1e-3, "{net:?}");
    // Taking them out again gives two current sources.
    let mut back = lined.clone();
    straws_core::analysis::phasing::remove_network(&mut back);
    let nb = back.expand().unwrap();
    assert_eq!(sources(&back, &nb).len(), 2);
    assert!(straws_core::analysis::phasing::network(&back).is_empty());
    let (q, segs) = run(&lined);
    // The current on each element's feed segment (its first).
    let on = |tag: u32| {
        let first = segs.iter().find(|s| s.tag == tag).unwrap().number;
        q.currents
            .iter()
            .find(|c| c.segment == first)
            .unwrap()
            .current
    };
    for (e, tag) in [1u32, 2].into_iter().enumerate() {
        let got = on(tag);
        let r = got / d.currents[e];
        println!("element {tag}: got {got}, designed {}", d.currents[e]);
        assert!(
            (r.norm() - 1.0).abs() < 0.01 && r.arg().to_degrees().abs() < 0.5,
            "{r}"
        );
    }
    // The junction: its feed is the only source now.
    let zj = q.inputs[0].impedance;
    assert!(
        (zj - d.junction).norm() / d.junction.norm() < 0.01,
        "{zj} vs {}",
        d.junction
    );
    // With real (lossy) RG-213 the lines are cut for its loss; NEC's lines
    // are lossless, so the app estimates the real currents from NEC's
    // element impedances (its network table) through the lossy cable. That
    // estimate must land on the design.
    let rg213 = CABLES[2];
    let d = design(&z, &want, &[6.0, 6.0], &rg213, mhz, 0.25).unwrap();
    let mut lossy = deck.clone();
    write_lines(
        &mut lossy,
        &sources(&deck, &n),
        &d,
        &rg213,
        mhz,
        3,
        [5.28, 0.0, 12.0],
    );
    let (q, segs) = run(&lossy);
    let v = q.inputs[0].voltage;
    for (e, tag) in [1u32, 2].into_iter().enumerate() {
        let number = segs.iter().find(|s| s.tag == tag).unwrap().number;
        let zp = q
            .network
            .iter()
            .find(|i| i.segment == number)
            .unwrap()
            .impedance;
        let est = v * straws_core::analysis::phasing::delivered(&rg213, mhz, zp, d.lengths[e]);
        let r = est / d.currents[e];
        println!(
            "lossy element {tag}: estimate {est}, designed {}",
            d.currents[e]
        );
        assert!(
            (r.norm() - 1.0).abs() < 0.03 && r.arg().to_degrees().abs() < 2.0,
            "{r}"
        );
    }
}

/// SWR at the radio as an optimiser goal: a dipole fed through a 4:1 balun
/// and 75 ohm RG-6 (so its length transforms the impedance the 50 ohm radio
/// sees) whose length is the variable `feed_len`. A trial's score is the
/// chain maths on the antenna's impedance with that trial's cable length,
/// without the cable's loss (so a longer, lossier cable can't fake a match),
/// and tuning the dipole and the cable together lowers it.
#[test]
fn the_optimiser_tunes_the_swr_at_the_radio() {
    use straws_core::analysis::feedline::CABLES;
    use straws_core::analysis::feedsystem::{Stage, evaluate};
    use straws_core::optimise::{Goal, Method, Options, Param, Problem, RadioChain, optimise};
    let deck = Deck::parse(
        "CE\nSY len=5.0, feed_len=15\nGW 1 21 0 -len 0 0 len 0 0.001\nGE 0\nEX 0 1 11 0 1\nFR 0 1 0 0 14.2\nEN\n",
    );
    let chain = RadioChain {
        stages: vec![
            Stage::Balun {
                ratio: 4.0,
                loss_db: 0.0,
            },
            Stage::Line {
                cable: CABLES[4],
                length: 15.0,
            },
        ],
        vars: vec![(1, "feed_len".into())],
    };
    let problem = Problem {
        deck: deck.clone(),
        fixed: vec![],
        params: vec![
            Param {
                name: "len".into(),
                min: 4.5,
                max: 6.0,
                start: 5.0,
            },
            Param {
                name: "feed_len".into(),
                min: 2.0,
                max: 20.0,
                start: 15.0,
            },
        ],
        goals: vec![Goal::Swr {
            start_mhz: 14.0,
            stop_mhz: 14.35,
            points: 5,
            z0: 50.0,
            target: 1.3,
            weight: 1.0,
            through: Some(chain.clone()),
        }],
        limits: Default::default(),
    };
    let engine = engine().with_solver_threads(1);
    // The score of one trial, by hand: the antenna's Z at the goal's
    // frequencies through the chain with feed_len = 9.
    let trial = problem.evaluate(&engine, &[5.0, 9.0], &CancelToken::new());
    let n = deck.expand_with(&[("len".into(), 5.0)]).unwrap();
    let mut stages = chain.stages.clone();
    stages[1] = stages[1].with_value(9.0);
    // Scored without the cable's loss.
    if let Stage::Line { cable, .. } = &mut stages[1] {
        cable.loss_10 = 0.0;
        cable.loss_100 = 0.0;
    }
    let swept = engine
        .run(
            &straws_core::analysis::plan::run_deck(
                &n,
                &straws_core::analysis::plan::FrequencyPlan {
                    start_mhz: 14.0,
                    stop_mhz: 14.35,
                    points: 5,
                },
                None,
            ),
            &CancelToken::new(),
        )
        .unwrap();
    let by_hand = swept
        .frequencies
        .iter()
        .map(|f| evaluate(&stages, f.inputs[0].impedance, f.mhz, 50.0).1)
        .fold(0.0, f64::max);
    println!("trial {:.4}, by hand {by_hand:.4}", trial.results[0].value);
    assert!((trial.results[0].value - by_hand).abs() < 1e-3 * by_hand);
    let best = optimise(
        &problem,
        &engine,
        Options::new(Method::NelderMead, 120),
        &CancelToken::new(),
        |_| {},
    );
    let start = problem.evaluate(&engine, &[5.0, 15.0], &CancelToken::new());
    println!(
        "worst SWR at the radio {:.3} -> {:.3} (len {:.3}, feed_len {:.2})",
        start.results[0].value, best.results[0].value, best.values[0], best.values[1]
    );
    assert!(best.results[0].value < start.results[0].value - 0.3);
}

/// Common-mode current as an optimiser goal: a dipole over perfect ground
/// with its coax shield modelled straight down, the choke's resistance the
/// variable `choke`. From 10 ohms (no real choke) the optimiser finds one
/// that keeps the shield's current under 5% of the feed current.
#[test]
fn the_optimiser_chokes_the_common_mode_current() {
    use straws_core::optimise::{Goal, Method, Options, Param, Problem, optimise};
    let mut deck = Deck::parse(
        "CE\nSY choke=10\nGW 1 11 0 -5.1 10 0 5.1 10 0.001\nGE 1\nGN 1\nEX 0 1 6 0 1\nFR 0 1 0 0 14.2\nEN\n",
    );
    let half = 10.2 / 11.0 / 2.0;
    deck.add_shield(
        2,
        [0.0, -half, 10.0],
        [0.0, -half, 0.0],
        11,
        0.005,
        10.0,
        0.0,
    );
    let line = straws_core::deck::shield::choke_line(&deck).unwrap();
    deck.set_field(line, 4, "choke");
    let problem = Problem {
        deck,
        fixed: vec![],
        params: vec![Param {
            name: "choke".into(),
            min: 1.0,
            max: 10_000.0,
            start: 10.0,
        }],
        goals: vec![Goal::CommonMode {
            mhz: 14.2,
            target: 5.0,
            weight: 1.0,
        }],
        limits: Default::default(),
    };
    let engine = engine().with_solver_threads(1);
    let start = problem.evaluate(&engine, &[10.0], &CancelToken::new());
    let best = optimise(
        &problem,
        &engine,
        Options::new(Method::NelderMead, 60),
        &CancelToken::new(),
        |_| {},
    );
    println!(
        "common-mode {:.1}% at 10 ohms -> {:.2}% with {:.0} ohms",
        start.results[0].value, best.results[0].value, best.values[0]
    );
    assert!(start.results[0].value > 20.0, "{start:?}");
    assert!(
        best.results[0].met && best.results[0].value <= 5.0,
        "{best:?}"
    );
}

/// Phasing lines with an L network at each element, checked by NEC: the
/// two verticals of `phasing_lines_give_the_currents_they_promise`, wanted
/// 1 A at 0° and 0.7 A at -120° (which plain lines can't give exactly),
/// lossless lines; with the networks NEC gives exactly the wanted currents.
#[test]
fn element_networks_give_exact_currents_in_nec() {
    use num_complex::Complex64;
    use straws_core::analysis::feedline::{CABLES, Cable};
    use straws_core::analysis::phasing::{design, network, write_lines};
    use straws_core::analysis::plan::{FrequencyPlan, run_deck};
    use straws_core::analysis::sweep::FreqPoint;
    use straws_core::deck::sources::sources;
    let mhz = 7.1;
    let deck = Deck::parse(
        "CE\nGW 1 11 0 0 0 0 0 10.56 .01\nGW 2 11 10.56 0 0 10.56 0 10.56 .01\nGE 1\nGN 1\nEX 6 1 1 0 1 0\nEX 6 2 1 0 -0.35 -0.6062\nFR 0 1 0 0 7.1 0\nEN\n",
    );
    let engine = engine();
    let run = |d: &Deck| {
        let n = d.expand().unwrap();
        let out = engine
            .run(
                &run_deck(&n, &FrequencyPlan::single(mhz), None),
                &CancelToken::new(),
            )
            .unwrap();
        (
            FreqPoint::from_result(&out.frequencies[0], None).without_helpers(&n),
            out.segments,
        )
    };
    let (p, _) = run(&deck);
    let z: Vec<Complex64> = p.inputs.iter().map(|i| i.impedance).collect();
    let want: Vec<Complex64> = p.inputs.iter().map(|i| i.current).collect();
    let cable = Cable {
        loss_10: 0.0,
        loss_100: 0.0,
        ..CABLES[2]
    };
    let lines = design(&z, &want, &[6.0, 6.0], &cable, mhz, 0.25).unwrap();
    let exact = lines.with_networks(&z, &cable, mhz).unwrap();
    println!(
        "lines alone {:.2} dB {:.1}°; networks {:?}",
        lines.worst_db,
        lines.worst_deg,
        exact.nets.iter().map(|n| n.describe()).collect::<Vec<_>>()
    );
    let n = deck.expand().unwrap();
    let mut lined = deck.clone();
    write_lines(
        &mut lined,
        &sources(&deck, &n),
        &exact,
        &cable,
        mhz,
        3,
        [5.28, 0.0, 12.0],
    );
    // The networks are read back from the deck.
    let net = network(&lined);
    assert_eq!(net.len(), 2);
    for (k, l) in net.iter().enumerate() {
        assert_eq!(l.net.series.is_some(), exact.nets[k].series.is_some());
        assert_eq!(l.net.shunt.is_some(), exact.nets[k].shunt.is_some());
    }
    let (q, segs) = run(&lined);
    let on = |tag: u32| {
        let first = segs.iter().find(|s| s.tag == tag).unwrap().number;
        q.currents
            .iter()
            .find(|c| c.segment == first)
            .unwrap()
            .current
    };
    let r = (on(2) / on(1)) / (want[1] / want[0]);
    println!(
        "NEC: element 2 against 1 is {:.4} at {:.2}° of what's wanted",
        r.norm(),
        r.arg().to_degrees()
    );
    assert!(
        (r.norm() - 1.0).abs() < 0.01 && r.arg().to_degrees().abs() < 0.5,
        "{r}"
    );
    let zj = q.inputs[0].impedance;
    assert!(
        (zj - exact.junction).norm() / exact.junction.norm() < 0.01,
        "{zj} vs {}",
        exact.junction
    );
}

/// Fitting the model to a measurement: a dipole "measured" at a known
/// length and loss (the engine's own sweep), then a model that starts too
/// short and lossless is fitted to it and finds both again.
#[test]
fn the_optimiser_fits_the_model_to_a_measurement() {
    use straws_core::analysis::measured::MeasuredPoint;
    use straws_core::optimise::{Goal, Method, Options, Param, Problem, optimise};
    let deck = Deck::parse(
        "CE\nSY len=5.0, rl=1\nGW 1 21 0 -len 0 0 len 0 0.001\nGE 0\nLD 0 1 11 11 rl 0 0\nEX 0 1 11 0 1\nFR 0 1 0 0 14.2\nEN\n",
    );
    let engine = engine().with_solver_threads(1);
    let truth = deck
        .expand_with(&[("len".into(), 5.18), ("rl".into(), 7.0)])
        .unwrap();
    let swept = engine
        .run(
            &straws_core::analysis::plan::run_deck(
                &truth,
                &straws_core::analysis::plan::FrequencyPlan {
                    start_mhz: 13.5,
                    stop_mhz: 14.5,
                    points: 41,
                },
                None,
            ),
            &CancelToken::new(),
        )
        .unwrap();
    let measured: Vec<MeasuredPoint> = swept
        .frequencies
        .iter()
        .map(|f| MeasuredPoint {
            mhz: f.mhz,
            z: f.inputs[0].impedance,
        })
        .collect();
    let goal = Goal::fit_to("the dipole", &measured, 13.0, 15.0, 11, 50.0).unwrap();
    let Goal::Match {
        start_mhz,
        stop_mhz,
        ..
    } = goal
    else {
        unreachable!()
    };
    assert_eq!((start_mhz, stop_mhz), (13.5, 14.5));
    let problem = Problem {
        deck: deck.clone(),
        fixed: vec![],
        params: vec![
            Param {
                name: "len".into(),
                min: 4.8,
                max: 5.6,
                start: 5.0,
            },
            Param {
                name: "rl".into(),
                min: 0.0,
                max: 20.0,
                start: 1.0,
            },
        ],
        goals: vec![goal],
        limits: Default::default(),
    };
    let start = problem.evaluate(&engine, &[5.0, 1.0], &CancelToken::new());
    let exact = problem.evaluate(&engine, &[5.18, 7.0], &CancelToken::new());
    let best = optimise(
        &problem,
        &engine,
        Options::new(Method::NelderMead, 200),
        &CancelToken::new(),
        |_| {},
    );
    println!(
        "RMS reflection difference {:.4} -> {:.5} (exact {:.6}); len {:.4}, rl {:.3}",
        start.results[0].value,
        best.results[0].value,
        exact.results[0].value,
        best.values[0],
        best.values[1]
    );
    assert!(exact.results[0].value < 1e-3);
    assert!(start.results[0].value > 0.2);
    assert!((best.values[0] - 5.18).abs() < 0.005);
    assert!((best.values[1] - 7.0).abs() < 0.5);
}

/// STEP export: each set of joined wires is its own body (the driven
/// halves together, the reflector and director apart), every tube reads
/// back exactly where the deck puts it, and a plate goes out as a solid
/// sheet of its own.
#[test]
fn step_export_keeps_parts_apart_and_in_place() {
    use straws_core::analysis::build::build_sheet;
    use straws_core::deck::plates::{PlateKind, PlateSpec, plate_sets};
    use straws_core::step;
    let text = "CE\n\
        GW 1 21 -2 -5.3 10 -2 5.3 10 .006 ' Reflector\n\
        GW 2 10 0 -5 10 0 0 10 .006 ' Driven left\n\
        GW 3 10 0 0 10 0 5 10 .006 ' Driven right\n\
        GW 4 21 1.8 -4.8 10 1.8 4.8 10 .006 ' Director\n\
        GE 0\nEX 0 2 10 0 1\nFR 0 1 0 0 14.2\nEN\n";
    let mut deck = Deck::parse(text);
    deck.add_plate(
        &PlateSpec {
            name: "screen".into(),
            kind: PlateKind::Flat,
            centre: [-6.0, 0.0, 10.0],
            width: 12.0,
            height: 8.0,
            facing: 0.0,
            angle: 0.0,
            cells: [6, 4],
            segments: 1,
            wire_radius: 0.002,
        },
        5,
    )
    .unwrap();
    let n = deck.expand().unwrap();
    let parsed = engine().run(&n, &CancelToken::new()).unwrap();
    let plate = &plate_sets(&deck)[0];
    let sheet = build_sheet(&parsed.segments, &deck, &n, &plate.tags);
    let mut bodies = step::bodies(&sheet, &parsed.segments);
    let names: Vec<&str> = bodies.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        names,
        ["Reflector", "part 2: Driven left, Driven right", "Director"]
    );
    let wires = straws_core::deck::edit::wires(&deck, &n);
    let mesh = step::Mesh {
        name: "screen".into(),
        wires: wires
            .iter()
            .filter(|w| plate.tags.contains(&w.tag))
            .map(|w| (w.a, w.b))
            .collect(),
        thickness: 0.003,
        radius: 0.002,
    };
    let slab = step::sheet_solid(&mesh).unwrap();
    assert_eq!(slab.faces.len(), 6);
    assert!(slab.loose.is_empty());
    bodies.push(step::Body {
        name: "screen".into(),
        rods: Vec::new(),
        sheets: vec![mesh],
    });
    let file = step::write(&bodies, "yagi");
    std::fs::write(std::env::temp_dir().join("straws-yagi.step"), &file).unwrap();
    assert_eq!(file.matches("NEXT_ASSEMBLY_USAGE_OCCURRENCE").count(), 4);
    let read = step::read_step(&file).unwrap();
    assert_eq!(read.metres_per_unit, Some(0.001));
    let tubes = &read.parts.tubes;
    assert_eq!(tubes.len(), 4, "{tubes:?}");
    for w in wires.iter().filter(|w| w.tag <= 4) {
        let (a, b) = (w.a.map(|v| v * 1000.0), w.b.map(|v| v * 1000.0));
        let near = |p: [f64; 3], q: [f64; 3]| (0..3).all(|i| (p[i] - q[i]).abs() < 1e-6);
        assert!(
            tubes.iter().any(|t| (t.radius - 6.0).abs() < 1e-9
                && ((near(t.a, a) && near(t.b, b)) || (near(t.a, b) && near(t.b, a)))),
            "tag {}",
            w.tag
        );
    }
}

/// Each design in the library does what its designer published (gain,
/// front-to-back, feed impedance), at its own frequency and made for
/// another band.
#[test]
fn library_designs_do_what_they_say() {
    use straws_core::analysis::pattern::{Component, PatternGrid};
    use straws_core::analysis::plan::{FrequencyPlan, PatternRequest, run_deck};
    use straws_core::library::LIBRARY;
    let figures = |text: &str, mhz: f64| {
        let n = Deck::parse(text).expand().unwrap();
        let out = engine()
            .run(
                &run_deck(
                    &n,
                    &FrequencyPlan::single(mhz),
                    Some(PatternRequest::new(2.0)),
                ),
                &CancelToken::new(),
            )
            .unwrap();
        let f = &out.frequencies[0];
        let z = f.inputs[0].impedance;
        let g = PatternGrid::from_table(&f.patterns[0]).unwrap();
        let (peak, th, ph) = g.max(Component::Total);
        let az = g.azimuth_cut(Component::Total, 90.0 - th);
        let fb = az
            .figures()
            .and_then(|c| c.front_to_back)
            .unwrap_or(f64::NAN);
        (z, peak, fb, th, ph)
    };
    // (name starts, published gain dBi and tolerance, least F/B dB, feed
    // resistance and tolerance, largest |reactance|)
    let checks: [(&str, f64, f64, f64, f64, f64, f64); 7] = [
        ("DK7ZB 3", 8.5, 0.3, 15.0, 28.0, 3.0, 6.0),
        ("DK7ZB 5", 11.1, 0.3, 19.5, 28.0, 4.0, 8.0),
        ("DK7ZB 6", 11.6, 0.3, 30.0, 40.0, 4.0, 4.0),
        ("Biquad", 10.5, 1.0, 15.0, 60.0, 15.0, 20.0),
        ("Moxon", 6.0, 0.5, 20.0, 50.0, 6.0, 8.0),
        ("Two-element", 7.5, 0.6, 10.0, 80.0, 10.0, 45.0),
        ("Quarter-wave", 2.0, 0.5, -1.0, 52.0, 5.0, 5.0),
    ];
    for d in LIBRARY {
        let (z, peak, fb, th, ph) = figures(&d.deck(d.mhz(), 2.0), d.mhz());
        println!(
            "{}: {:.3} MHz Z {:.1} {:+.1}j, peak {peak:.2} dBi at theta {th} phi {ph}, F/B {fb:.1} dB",
            d.name,
            d.mhz(),
            z.re,
            z.im
        );
        let (_, gain, tol, least_fb, r, rtol, xmax) = checks
            .iter()
            .find(|c| d.name.starts_with(c.0))
            .copied()
            .unwrap_or_else(|| panic!("no check for {}", d.name));
        assert!((peak - gain).abs() <= tol, "{}: {peak} dBi", d.name);
        assert!(least_fb < 0.0 || fb >= least_fb, "{}: F/B {fb}", d.name);
        assert!(
            (z.re - r).abs() <= rtol && z.im.abs() <= xmax,
            "{}: {z}",
            d.name
        );
        // Made for a third of the frequency: the same antenna, scaled.
        let (z3, peak3, ..) = figures(&d.deck(d.mhz() / 3.0, 2.0 * 3.0), d.mhz() / 3.0);
        assert!(
            (z3 - z).norm() < 0.05 * z.norm() && (peak3 - peak).abs() < 0.05,
            "{}: {z3} {peak3}",
            d.name
        );
    }
}
