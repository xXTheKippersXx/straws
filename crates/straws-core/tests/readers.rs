//! The file readers against broken, cut-short and scrambled files: each
//! must give an answer or a reason, never panic or hang.

use std::sync::mpsc;
use std::time::Duration;

use straws_core::analysis::measured::read_touchstone;
use straws_core::deck::Deck;
use straws_core::dxf::{self, WireOptions};
use straws_core::{sheetfit, step, stl};

/// A small, repeatable random number source.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Values that upset number handling.
const NASTY: [&str; 12] = [
    "NaN",
    "inf",
    "-inf",
    "1e308",
    "-1e308",
    "1e-320",
    "0",
    "-0",
    "99999999999999999999",
    "",
    "x",
    "1.5.5",
];

/// The sample cut short at many places, with bytes changed, removed and
/// added, and with numbers swapped for nasty ones.
fn mangled(sample: &[u8], seed: u64, count: usize) -> Vec<Vec<u8>> {
    mangled_cut(sample, seed, count, 60)
}

/// As `mangled`, cut short at about `cuts` places.
fn mangled_cut(sample: &[u8], seed: u64, count: usize, cuts: usize) -> Vec<Vec<u8>> {
    let mut rng = Rng(seed);
    let mut out = Vec::new();
    let step = (sample.len() / cuts.max(1)).max(1);
    for cut in (0..sample.len()).step_by(step) {
        out.push(sample[..cut].to_vec());
    }
    for _ in 0..count {
        let mut s = sample.to_vec();
        for _ in 0..1 + rng.below(4) {
            if s.is_empty() {
                break;
            }
            let at = rng.below(s.len());
            match rng.below(4) {
                0 => s[at] = rng.next() as u8,
                1 => {
                    s.remove(at);
                }
                2 => {
                    const BYTES: &[u8] = b"0123456789-.,; \n\t()#'eE";
                    s.insert(at, BYTES[rng.below(BYTES.len())]);
                }
                _ => {
                    // A number replaced with a nasty one.
                    let text = String::from_utf8_lossy(&s).into_owned();
                    let starts: Vec<usize> = text
                        .char_indices()
                        .filter(|(i, c)| {
                            c.is_ascii_digit()
                                && (*i == 0 || !text.as_bytes()[i - 1].is_ascii_digit())
                        })
                        .map(|(i, _)| i)
                        .collect();
                    if let Some(&from) = starts.get(rng.below(starts.len())) {
                        let to = text[from..]
                            .find(|c: char| {
                                !(c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E')
                            })
                            .map_or(text.len(), |e| from + e);
                        let mut t = text[..from].to_string();
                        t.push_str(NASTY[rng.below(NASTY.len())]);
                        t.push_str(&text[to..]);
                        s = t.into_bytes();
                    }
                }
            }
        }
        out.push(s);
    }
    out
}

/// Runs `f` on every input on another thread; a panic or a hang (over
/// `limit`) fails, naming the input.
fn survives(name: &str, inputs: Vec<Vec<u8>>, limit: Duration, f: fn(&[u8])) {
    for (k, input) in inputs.into_iter().enumerate() {
        let (tx, rx) = mpsc::channel();
        let copy = input.clone();
        std::thread::spawn(move || {
            let ok = std::panic::catch_unwind(|| f(&copy)).is_ok();
            let _ = tx.send(ok);
        });
        let started = std::time::Instant::now();
        let answer = rx.recv_timeout(limit);
        if started.elapsed() > Duration::from_millis(500) {
            eprintln!(
                "{name} input {k}: slow, {:.1} s",
                started.elapsed().as_secs_f64()
            );
        }
        match answer {
            Ok(true) => {}
            Ok(false) => panic!(
                "{name} input {k} panicked:\n{}",
                String::from_utf8_lossy(&input)
            ),
            Err(_) => panic!(
                "{name} input {k} hung:\n{}",
                String::from_utf8_lossy(&input)
            ),
        }
    }
}

const TOUCHSTONE: &str =
    "! NanoVNA sweep\n# HZ S RI R 50\n14000000 0.1 -0.2\n14100000 0.05 0.01\n14200000 -0.1 0.3\n";

const DXF: &str = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\nA\n10\n0\n20\n0\n30\n0\n11\n100\n21\n0\n31\n0\n\
0\nARC\n8\nA\n10\n0\n20\n0\n30\n0\n40\n50\n50\n0\n51\n90\n\
0\nLWPOLYLINE\n8\nB\n90\n3\n70\n1\n10\n0\n20\n0\n42\n0.5\n10\n10\n20\n0\n10\n10\n20\n10\n\
0\nCIRCLE\n8\nC\n10\n5\n20\n5\n30\n0\n40\n3\n\
0\nSPLINE\n8\nD\n71\n3\n72\n8\n73\n4\n40\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n40\n1\n\
10\n0\n20\n0\n30\n0\n10\n1\n20\n2\n30\n0\n10\n2\n20\n2\n30\n0\n10\n3\n20\n0\n30\n0\n\
0\nENDSEC\n0\nEOF\n";

fn a_tube_stl() -> String {
    let s = straws_core::output::Segment {
        number: 1,
        tag: 1,
        centre: [0.0, 0.0, 0.5],
        length: 1.0,
        alpha: 90.0,
        beta: 0.0,
        radius: 0.01,
        connections: [0, 0],
    };
    straws_core::export::stl(&[s], "tube")
}

#[test]
fn touchstone_files() {
    survives(
        "touchstone",
        mangled(TOUCHSTONE.as_bytes(), 1, 1000),
        Duration::from_secs(5),
        |b| {
            let _ = read_touchstone(&String::from_utf8_lossy(b), 1);
            let _ = read_touchstone(&String::from_utf8_lossy(b), 2);
        },
    );
}

#[test]
fn dxf_drawings() {
    survives(
        "dxf",
        mangled(DXF.as_bytes(), 2, 1500),
        Duration::from_secs(5),
        |b| {
            for step in [10.0, 0.0, -5.0, f64::NAN, 1e-9] {
                if let Ok(d) = dxf::read_dxf(&String::from_utf8_lossy(b), step) {
                    let _ = dxf::wires(
                        &d,
                        &WireOptions {
                            metres_per_unit: 0.001,
                            join_m: 0.001,
                            layers: Vec::new(),
                        },
                    );
                }
            }
        },
    );
}

#[test]
fn stl_meshes() {
    let ascii = a_tube_stl();
    survives(
        "stl",
        mangled(ascii.as_bytes(), 3, 200),
        Duration::from_secs(5),
        |b| {
            if let Ok(t) = stl::read_stl(b) {
                let parts = stl::tubes(&t);
                let _ = stl::wires(&parts.tubes, 0.001, 0.001);
            }
        },
    );
    // A binary file claiming more triangles than it holds, and fewer.
    let mut bin = vec![0u8; 84 + 50];
    bin[80] = 1;
    let mut lying = bin.clone();
    lying[80] = 200;
    survives(
        "binary stl",
        vec![bin, lying],
        Duration::from_secs(5),
        |b| {
            if let Ok(t) = stl::read_stl(b) {
                let _ = stl::tubes(&t);
            }
        },
    );
}

#[test]
fn step_models() {
    let read = |b: &[u8]| {
        if let Ok(s) = step::read_step(&String::from_utf8_lossy(b)) {
            for sheet in &s.sheets {
                let fit = sheet.scaled(0.001);
                let _ = fit.reflector("dish1", fit.cells(0.01));
            }
        }
    };
    for (k, sample) in [
        include_str!("cad/occ_assembly.step"),
        include_str!("cad/occ_sheets.step"),
    ]
    .iter()
    .enumerate()
    {
        survives(
            "step",
            // Fewer for these 80 kB files: debug builds read them slowly.
            mangled_cut(sample.as_bytes(), 10 + k as u64, 40, 12),
            Duration::from_secs(20),
            read,
        );
    }
}

#[test]
fn sheets_from_odd_points() {
    let sets: Vec<Vec<[f64; 3]>> = vec![
        Vec::new(),
        vec![[0.0; 3]; 50],
        (0..50).map(|k| [k as f64, 0.0, 0.0]).collect(),
        (0..50).map(|k| [k as f64, f64::NAN, 0.0]).collect(),
        (0..50).map(|k| [k as f64 * 1e300, 1e300, 0.0]).collect(),
        (0..60)
            .map(|k| {
                [
                    (k % 10) as f64,
                    (k / 10) as f64,
                    if k % 2 == 0 { 0.5 } else { -0.5 },
                ]
            })
            .collect(),
        (0..60)
            .map(|k| [(k % 10) as f64 * 1e-12, (k / 10) as f64 * 1e-12, 0.0])
            .collect(),
    ];
    for (k, pts) in sets.into_iter().enumerate() {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let ok = std::panic::catch_unwind(|| {
                if let Some(f) = sheetfit::fit_sheet(&pts, None) {
                    let _ = f.reflector("x", f.cells(0.1));
                }
            })
            .is_ok();
            let _ = tx.send(ok);
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(10)),
            Ok(true),
            "point set {k}"
        );
    }
}

#[test]
fn sheets_from_odd_meshes() {
    let grid = |f: &dyn Fn(f64, f64) -> [f64; 3]| -> Vec<([f64; 3], [f64; 3])> {
        let mut w = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let p = f(i as f64, j as f64);
                if i < 3 {
                    w.push((p, f(i as f64 + 1.0, j as f64)));
                }
                if j < 3 {
                    w.push((p, f(i as f64, j as f64 + 1.0)));
                }
            }
        }
        w
    };
    let meshes: Vec<Vec<([f64; 3], [f64; 3])>> = vec![
        Vec::new(),
        vec![([0.0; 3], [0.0; 3])],
        vec![([0.0; 3], [1.0, 0.0, 0.0]); 10],
        grid(&|x, y| [x, y, 0.0]),
        grid(&|x, y| [x, y, f64::NAN]),
        grid(&|x, y| [x * 1e300, y, 0.0]),
        grid(&|x, y| [x * 1e-15, y * 1e-15, 0.0]),
        grid(&|x, y| [x, y, (x * y).sin() * 10.0]),
        // Every node on one line.
        grid(&|x, y| [x + y, 0.0, 0.0]),
    ];
    for (k, wires) in meshes.into_iter().enumerate() {
        for thickness in [0.001, 0.0, -1.0, f64::NAN, 1e6] {
            let mesh = step::Mesh {
                name: "m".into(),
                wires: wires.clone(),
                thickness,
                radius: 0.001,
            };
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let ok = std::panic::catch_unwind(|| {
                    let _ = step::sheet_solid(&mesh);
                    let body = step::Body {
                        name: "m".into(),
                        rods: Vec::new(),
                        sheets: vec![mesh],
                    };
                    let _ = step::write(&[body], "m");
                })
                .is_ok();
                let _ = tx.send(ok);
            });
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(10)),
                Ok(true),
                "mesh {k}, thickness {thickness}"
            );
        }
    }
}

#[test]
fn decks() {
    let sample = "CM test\nCE\nSY len=5, h=10\nGW 1 21 0 -len 0 0 len h .001\nGA 2 12 1 0 180 .001\nGH 3 40 0.1 2 0.5 0.5 0.5 0.5 .001\nGM 0 2 0 0 30 0 0 0 1\nGR 1 4\nGE 0\nGN 2 0 0 0 13 .005\nLD 5 1 1 21 5.8e7\nEX 0 1 11 0 1 0\nFR 0 3 0 0 14 0.1\nRP 0 19 37 1000 0 0 10 10\nEN\n";
    survives(
        "deck",
        mangled(sample.as_bytes(), 20, 1500),
        Duration::from_secs(5),
        |b| {
            let deck = Deck::parse(&String::from_utf8_lossy(b));
            if let Ok(n) = deck.expand() {
                let _ = n.segment_estimate();
                let _ = n.to_nec_text();
            }
        },
    );
}
