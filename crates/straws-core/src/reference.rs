//! Running an external NEC-2D engine, such as 4nec2's `nec2dxs*.exe`, to
//! produce reference results. These engines ask for the input and output
//! file names on standard input.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::engine::{EngineError, RawRun};

use crate::deck::NumericDeck;

/// One of 4nec2's engine builds and its fixed limits (from each build's
/// start-up banner).
#[derive(Debug, Clone, Copy)]
pub struct EngineBuild {
    pub file: &'static str,
    pub segments: usize,
    pub loads: usize,
    pub sources: usize,
    pub lines: usize,
}

pub const NEC2DXS_BUILDS: &[EngineBuild] = &[
    EngineBuild {
        file: "nec2dxs500.exe",
        segments: 500,
        loads: 50,
        sources: 64,
        lines: 64,
    },
    EngineBuild {
        file: "nec2dxs1K5.exe",
        segments: 1500,
        loads: 150,
        sources: 99,
        lines: 128,
    },
    EngineBuild {
        file: "nec2dxs3k0.exe",
        segments: 3000,
        loads: 300,
        sources: 128,
        lines: 256,
    },
    EngineBuild {
        file: "nec2dxs5k0.exe",
        segments: 5000,
        loads: 500,
        sources: 128,
        lines: 256,
    },
    EngineBuild {
        file: "nec2dxs8k0.exe",
        segments: 8000,
        loads: 800,
        sources: 128,
        lines: 256,
    },
    EngineBuild {
        file: "nec2dxs11k.exe",
        segments: 11000,
        loads: 1100,
        sources: 256,
        lines: 256,
    },
];

#[derive(Debug, Clone)]
pub struct ReferenceEngine {
    pub exe: PathBuf,
}

impl ReferenceEngine {
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        Self { exe: exe.into() }
    }

    /// Picks the smallest 4nec2 engine in `exe_dir` (e.g. `C:\4nec2\exe`)
    /// whose limits fit the deck. `segments` is the segment count (from a
    /// nec2c run of the same deck). Surface patches each add two unknowns,
    /// which the builds don't list separately, so decks with patches use
    /// the largest build.
    pub fn for_deck(exe_dir: &Path, deck: &NumericDeck, segments: usize) -> Option<Self> {
        let count = |m: &[&str]| {
            deck.cards
                .iter()
                .filter(|c| m.contains(&c.mnemonic.as_str()))
                .count()
        };
        let patches = count(&["SP", "SM", "SC"]) > 0;
        let (loads, sources, lines) = (count(&["LD"]), count(&["EX"]), count(&["TL", "NT"]));
        NEC2DXS_BUILDS
            .iter()
            .filter(|b| {
                (!patches || b.segments == 11000)
                    && b.segments >= segments
                    && b.loads >= loads
                    && b.sources >= sources
                    && b.lines >= lines
            })
            .map(|b| exe_dir.join(b.file))
            .find(|p| p.exists())
            .map(Self::new)
    }

    /// Runs deck text and returns the engine's output, giving up (and
    /// stopping the engine) after `timeout`.
    pub fn run_text(&self, deck_text: &str, timeout: Duration) -> Result<RawRun, EngineError> {
        let dir = std::env::temp_dir().join("straws").join(format!(
            "ref-{}-{}",
            std::process::id(),
            unique()
        ));
        std::fs::create_dir_all(&dir)?;
        let result = (|| {
            std::fs::write(dir.join("deck.nec"), deck_text)?;
            let started = Instant::now();
            let mut child = Command::new(&self.exe)
                .current_dir(&dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(b"deck.nec\r\ndeck.out\r\n")?;
            }
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(EngineError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(2));
            };
            let text = std::fs::read(dir.join("deck.out"))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            let elapsed = started.elapsed();
            if !status.success() || text.is_empty() {
                return Err(EngineError::Failed {
                    exit_code: status.code(),
                    messages: vec![format!("{} produced no output", self.exe.display())],
                });
            }
            Ok(RawRun {
                text,
                elapsed,
                binary: Vec::new(),
            })
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::SeqCst)
}
