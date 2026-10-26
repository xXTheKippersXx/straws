//! Bad and extreme inputs for the analysis added since the first testing
//! protocol run (phasing, hybrid, feed system, bands, build sheet,
//! connectors): nothing may panic or hang, and what comes back is finite or
//! a plain refusal.

use num_complex::Complex64;
use straws_core::analysis::feedline::{CABLES, CONNECTORS, Connector};
use straws_core::analysis::feedsystem::{Free, Stage, evaluate, l_starts, optimise, worst};
use straws_core::analysis::{hybrid, phasing};
use straws_core::bands::{Band, containing, itu_region, tidy};

fn c(re: f64, im: f64) -> Complex64 {
    Complex64::new(re, im)
}

/// Impedances from ordinary to absurd.
fn odd_impedances() -> Vec<Complex64> {
    vec![
        c(50.0, 0.0),
        c(0.0, 0.0),
        c(1e-9, 0.0),
        c(1e9, -1e9),
        c(-20.0, 5.0),
        c(f64::NAN, 0.0),
        c(f64::INFINITY, 0.0),
        c(0.0, 1e6),
    ]
}

#[test]
fn phasing_survives_odd_inputs() {
    let cable = CABLES[2];
    for z in odd_impedances() {
        let zs = [c(36.0, 0.0), z];
        for want in [
            [c(1.0, 0.0), c(0.0, -1.0)],
            [c(0.0, 0.0), c(0.0, 0.0)],
            [c(1.0, 0.0), c(0.0, 0.0)],
            [c(f64::NAN, 0.0), c(1.0, 0.0)],
        ] {
            for mhz in [0.001, 7.1, 1e5] {
                if let Some(d) = phasing::design(&zs, &want, &[0.0, 0.0], &cable, mhz, 0.25) {
                    let _ = d.with_networks(&zs, &cable, mhz);
                    let band = vec![(mhz, zs.to_vec())];
                    let _ = phasing::band_errors(
                        &band,
                        &want,
                        &d.lengths,
                        &d.nets,
                        &cable,
                        d.reference,
                    );
                    let _ = phasing::design_band(&band, &zs, &[0.0, 0.0], &cable, mhz, &d);
                }
            }
        }
    }
    // One element, or none: nothing to design.
    assert!(phasing::design(&[c(50.0, 0.0)], &[c(1.0, 0.0)], &[0.0], &cable, 7.0, 0.25).is_none());
    assert!(phasing::design(&[], &[], &[], &cable, 7.0, 0.25).is_none());
    // Mismatched lengths of the inputs.
    assert!(
        phasing::design(
            &[c(50.0, 0.0); 2],
            &[c(1.0, 0.0)],
            &[0.0; 2],
            &cable,
            7.0,
            0.25
        )
        .is_none()
    );
}

#[test]
fn hybrid_survives_odd_inputs() {
    let cable = CABLES[2];
    for z in odd_impedances() {
        let zs = [c(36.0, 0.0), z];
        for on_b in [vec![false, true], vec![false, false], vec![true, true]] {
            let feed = hybrid::HybridFeed {
                on_b: on_b.clone(),
                lengths: vec![0.0, 5.0],
            };
            let r = hybrid::evaluate(&zs, &feed, &cable, 7.1);
            assert_eq!(r.currents.len(), 2);
            assert!((0.0..=1.0).contains(&r.dumped) || r.dumped.is_nan());
            let want = [c(1.0, 0.0), c(0.0, -1.0)];
            let f = hybrid::design(&zs, &want, &on_b, &[1.0, 1.0], &cable, 7.1, &[]);
            assert_eq!(f.lengths.len(), 2);
        }
    }
    // Every element on one output: the other is open, and nothing breaks.
    let feed = hybrid::HybridFeed {
        on_b: vec![false, false],
        lengths: vec![1.0, 2.0],
    };
    let r = hybrid::evaluate(&[c(50.0, 0.0), c(50.0, 0.0)], &feed, &cable, 7.1);
    assert!(r.input.re.is_finite() && r.dumped.is_finite());
}

#[test]
fn feed_system_survives_odd_inputs() {
    let cable = CABLES[2];
    let stages = [
        Stage::Balun {
            ratio: 0.0,
            loss_db: -5.0,
        },
        Stage::Line { cable, length: 0.0 },
        Stage::Stub {
            cable,
            length: 0.0,
            shorted: false,
        },
        Stage::Loss { db: 100.0 },
    ];
    for z in odd_impedances() {
        for mhz in [0.0, 1e-6, 7.1, 1e6] {
            let (zin, swr, eff) = evaluate(&stages, z, mhz, 50.0);
            let _ = (zin, swr);
            assert!(eff.is_nan() || (0.0..=1.0).contains(&eff));
        }
    }
    // Tuning with no points or nothing free gives the chain back.
    let chain = vec![Stage::Line { cable, length: 3.0 }];
    assert_eq!(optimise(&chain, &[], &[], 50.0, 1.0), chain);
    let free = [Free {
        stage: 0,
        min: 1.0,
        max: 10.0,
    }];
    assert_eq!(optimise(&chain, &free, &[], 50.0, 1.0), chain);
    // Starts for an impossible load don't panic.
    for z in odd_impedances() {
        let _ = l_starts(&[], &[], z, 7.1, 50.0, 100.0);
    }
    let _ = worst(&chain, &[(7.1, c(f64::NAN, 0.0))], 50.0);
}

#[test]
fn connectors_and_bands_survive_odd_inputs() {
    for c in CONNECTORS.iter().copied().chain([Connector::other(-1.0)]) {
        for mhz in [0.0, -5.0, 1e9] {
            let l = c.loss_db(mhz);
            assert!(!l.is_nan(), "{} at {mhz}: {l}", c.name);
        }
    }
    for r in [0, 1, 3, 4, 255] {
        let b = itu_region(r);
        assert!(!b.is_empty());
    }
    let odd = [
        Band::new("", f64::NAN, 5.0),
        Band::new("x", f64::INFINITY, 1.0),
        Band::new("y", -3.0, -1.0),
        Band::new("z", 7.0, 7.0),
    ];
    assert!(tidy(&odd).is_empty());
    assert!(containing(&[], 7.0).is_none());
}

/// How long the Feeds view's designers take (they rerun while a slider is
/// dragged): `cargo test --release -p straws-core --test robustness --
/// --ignored --nocapture designer_speed`.
#[test]
#[ignore]
fn designer_speed() {
    let cable = CABLES[2];
    let z = [
        c(67.6, -60.6),
        c(107.3, -72.1),
        c(107.3, -72.1),
        c(150.4, -4.5),
    ];
    let want = [c(1.0, 0.0), c(0.0, -1.0), c(0.0, -1.0), c(-1.0, 0.0)];
    let band: Vec<_> = (0..51)
        .map(|k| (6.85 + 0.01 * k as f64, z.to_vec()))
        .collect();
    let time = |what: &str, f: &dyn Fn()| {
        let t = std::time::Instant::now();
        f();
        println!("{what}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    };
    time("lines at one frequency", &|| {
        phasing::design(&z, &want, &[7.5; 4], &cable, 7.1, 0.25);
    });
    time("lines tuned over 51 frequencies", &|| {
        let d = phasing::design(&z, &want, &[7.5; 4], &cable, 7.1, 0.25).unwrap();
        phasing::design_band(&band, &z, &[7.5; 4], &cable, 7.1, &d);
    });
    time("exact L networks", &|| {
        let d = phasing::design(&z, &want, &[7.5; 4], &cable, 7.1, 0.25).unwrap();
        d.with_networks(&z, &cable, 7.1);
    });
    time("hybrid at one frequency", &|| {
        hybrid::design(
            &z,
            &want,
            &[false, true, true, false],
            &[7.5; 4],
            &cable,
            7.1,
            &[],
        );
    });
    time("hybrid over 51 frequencies", &|| {
        hybrid::design(
            &z,
            &want,
            &[false, true, true, false],
            &[7.5; 4],
            &cable,
            7.1,
            &band,
        );
    });
}

#[test]
fn segment_estimates_match_the_engine() {
    use straws_core::deck::Deck;
    // Wires, an arc and a helix, copied by GR, mirrored by GX, and a patch
    // mesh: the estimate counts what the engine would make.
    for (text, want) in [
        (
            "GW 1 11 0 0 0 0 0 1 .001
GE 0
",
            11,
        ),
        (
            "GW 1 11 0 0 0 0 0 1 .001
GA 2 8 1 0 90 .001
GH 3 20 .1 1 .1 .1 .1 .1 .001
GE 0
",
            39,
        ),
        (
            "GW 1 5 1 0 0 1 0 1 .001
GR 1 4
GE 0
",
            20,
        ),
        (
            "GW 1 5 1 0 0 1 0 1 .001
GX 2 110
GE 0
",
            20,
        ),
        (
            "SM 4 3 0 0 0 1 0 0
SC 0 0 0 1 1 0
GE 0
",
            12,
        ),
        (
            "GW 1 4606 0 -1 0 -.5 0 0 .03
GW 2 4606 -.5 0 0 -1 1 0 .06
GE 0
",
            9212,
        ),
    ] {
        let n = Deck::parse(text).expand().unwrap();
        assert_eq!(n.segment_estimate(), want, "{text}");
    }
}
