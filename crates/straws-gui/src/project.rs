//! Project files (.straws): the deck as plain NEC text, then, after a
//! marker comment, everything else the design was set up with (notes, the
//! Feedline settings, the optimiser, snapshots and measurements).

use eframe::egui::Color32;
use serde::{Deserialize, Serialize};
use straws_core::Complex64;
use straws_core::analysis::feedline::{CABLES, CONNECTORS};
use straws_core::optimise::{Goal, RadioChain};

use crate::app::Settings;

/// The extension, without its dot.
pub const EXTENSION: &str = "straws";
/// The comment line that ends the deck and starts the rest.
const MARKER: &str = "'straws project";
/// The project format's version.
const VERSION: u32 = 1;

/// Whether `path` is a project file.
pub fn is_project(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// Everything a project keeps beyond the deck.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Extras {
    pub version: u32,
    pub notes: String,
    /// The frequency the slider was at (MHz).
    pub mhz: f64,
    /// Reference impedance for SWR (ohms).
    pub z0: f64,
    pub feed: FeedSave,
    pub optimiser: OptimiserSave,
    pub snapshots: Vec<SnapshotSave>,
    pub measurements: Vec<MeasurementSave>,
}

impl Extras {
    /// The parts that count as a change worth saving: the slider's place
    /// doesn't, nor optimiser variables that aren't ticked, nor the
    /// optimiser's starting goal (it adds one by itself when it has none).
    pub fn comparable(&self) -> Extras {
        let mut e = self.clone();
        e.mhz = 0.0;
        e.optimiser.vars.retain(|v| v.vary);
        if let [
            GoalSave::Swr {
                points: 11,
                at_radio: false,
                ..
            },
        ] = e.optimiser.goals.as_slice()
        {
            e.optimiser.goals.clear();
        }
        e
    }
}

/// Splits a project file into its deck text and the rest (`None` when the
/// file has no project part, e.g. a plain deck saved as .straws).
pub fn split(text: &str) -> (String, Option<Result<Extras, String>>) {
    let mut deck = String::new();
    let mut rest = None;
    for (k, line) in text.split_inclusive('\n').enumerate() {
        if line.trim_start().starts_with(MARKER) {
            let start: usize = text.split_inclusive('\n').take(k + 1).map(str::len).sum();
            rest = Some(&text[start..]);
            break;
        }
        deck.push_str(line);
    }
    let extras = rest.map(|r| ron::from_str::<Extras>(r).map_err(|e| e.to_string()));
    (deck, extras)
}

/// A project file from its deck text and the rest.
pub fn join(deck: &str, extras: &Extras) -> Result<String, String> {
    let mut extras = extras.clone();
    extras.version = VERSION;
    let body = ron::ser::to_string_pretty(&extras, ron::ser::PrettyConfig::default())
        .map_err(|e| e.to_string())?;
    let mut text = deck.to_string();
    if !text.ends_with('\n') {
        text.push_str("\r\n");
    }
    // The deck's own line endings throughout.
    let body = if text.ends_with("\r\n") {
        body.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        body
    };
    let newline = if text.ends_with("\r\n") { "\r\n" } else { "\n" };
    text.push_str(&format!(
        "{MARKER} {VERSION}: settings kept with this design (NEC ignores what follows)"
    ));
    text.push_str(newline);
    text.push_str(&body);
    text.push_str(newline);
    Ok(text)
}

/// The Feedline view's settings, cables and connectors by name (so a file
/// still opens right if those lists grow).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedSave {
    /// A cable's name, or "custom" for the numbers below.
    pub cable: String,
    pub length_m: f64,
    pub z0: f64,
    pub velocity_factor: f64,
    pub loss_db_per_100m_at_10mhz: f64,
    pub loss_db_per_100m_at_100mhz: f64,
    pub balun_ratio: f64,
    pub balun_loss_db: f64,
    pub shield_choke_ohms: f64,
    pub shield_length_m: f64,
    /// A connector's name, "none", or "other" (with its own loss).
    pub antenna_connector: String,
    pub antenna_connector_db: f64,
    pub radio_connector: String,
    pub radio_connector_db: f64,
    pub connector_line_mm: f64,
}

impl FeedSave {
    pub fn from_settings(s: &Settings) -> FeedSave {
        let connector = |k: usize| match k {
            0 => "none".to_string(),
            k if k <= CONNECTORS.len() => CONNECTORS[k - 1].name.to_string(),
            _ => "other".to_string(),
        };
        FeedSave {
            cable: CABLES
                .get(s.feed_cable)
                .map_or("custom".into(), |c| c.name.to_string()),
            length_m: s.feed_length,
            z0: s.feed_z0,
            velocity_factor: s.feed_vf,
            loss_db_per_100m_at_10mhz: s.feed_loss10,
            loss_db_per_100m_at_100mhz: s.feed_loss100,
            balun_ratio: s.balun_ratio,
            balun_loss_db: s.balun_loss,
            shield_choke_ohms: s.shield_choke,
            shield_length_m: s.shield_length,
            antenna_connector: connector(s.conn_antenna),
            antenna_connector_db: s.conn_antenna_db,
            radio_connector: connector(s.conn_radio),
            radio_connector_db: s.conn_radio_db,
            connector_line_mm: s.conn_length_mm,
        }
    }

    /// Puts these settings into `s` (names not known here leave that
    /// choice as it is).
    pub fn apply(&self, s: &mut Settings) {
        if self.cable == "custom" {
            s.feed_cable = CABLES.len();
        } else if let Some(k) = CABLES.iter().position(|c| c.name == self.cable) {
            s.feed_cable = k;
        }
        let connector = |name: &str, now: usize| match name {
            "none" => 0,
            "other" => CONNECTORS.len() + 1,
            n => CONNECTORS
                .iter()
                .position(|c| c.name == n)
                .map_or(now, |k| k + 1),
        };
        s.conn_antenna = connector(&self.antenna_connector, s.conn_antenna);
        s.conn_radio = connector(&self.radio_connector, s.conn_radio);
        let set = |to: &mut f64, v: f64| {
            if v.is_finite() && v > 0.0 {
                *to = v;
            }
        };
        set(&mut s.feed_length, self.length_m);
        set(&mut s.feed_z0, self.z0);
        set(&mut s.feed_vf, self.velocity_factor);
        set(&mut s.feed_loss10, self.loss_db_per_100m_at_10mhz);
        set(&mut s.feed_loss100, self.loss_db_per_100m_at_100mhz);
        set(&mut s.balun_ratio, self.balun_ratio);
        // These may rightly be 0.
        s.balun_loss = self.balun_loss_db.max(0.0);
        s.shield_choke = self.shield_choke_ohms.max(0.0);
        s.shield_length = self.shield_length_m.max(0.0);
        s.conn_antenna_db = self.antenna_connector_db.max(0.0);
        s.conn_radio_db = self.radio_connector_db.max(0.0);
        s.conn_length_mm = self.connector_line_mm.max(0.0);
    }
}

/// The optimiser's set-up.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OptimiserSave {
    pub vars: Vec<VarSave>,
    pub goals: Vec<GoalSave>,
    /// Search the whole range first (otherwise fine-tune from here).
    pub whole_range: bool,
    pub max_designs: usize,
    pub stop_when_met: bool,
    pub skip_unbuildable: bool,
    /// Size limits along x, y and z (on, metres).
    pub size: [(bool, f64); 3],
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VarSave {
    pub name: String,
    pub vary: bool,
    pub min: f64,
    pub max: f64,
}

/// An optimiser goal (see `straws_core::optimise::Goal`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GoalSave {
    Swr {
        start_mhz: f64,
        stop_mhz: f64,
        points: usize,
        z0: f64,
        target: f64,
        weight: f64,
        at_radio: bool,
    },
    Gain {
        mhz: f64,
        elevation: f64,
        azimuth: f64,
        peak: bool,
        target: f64,
        weight: f64,
    },
    CommonMode {
        mhz: f64,
        target: f64,
        weight: f64,
    },
    FrontToBack {
        mhz: f64,
        elevation: f64,
        azimuth: f64,
        rear: bool,
        target: f64,
        weight: f64,
    },
    Match {
        name: String,
        start_mhz: f64,
        stop_mhz: f64,
        /// Measured impedance at each frequency: (R, X).
        measured: Vec<(f64, f64)>,
        z0: f64,
        target: f64,
        weight: f64,
    },
}

impl GoalSave {
    pub fn from_goal(g: &Goal) -> GoalSave {
        match g.clone() {
            Goal::Swr {
                start_mhz,
                stop_mhz,
                points,
                z0,
                target,
                weight,
                through,
            } => GoalSave::Swr {
                start_mhz,
                stop_mhz,
                points,
                z0,
                target,
                weight,
                at_radio: through.is_some(),
            },
            Goal::Gain {
                mhz,
                elevation,
                azimuth,
                peak,
                target,
                weight,
            } => GoalSave::Gain {
                mhz,
                elevation,
                azimuth,
                peak,
                target,
                weight,
            },
            Goal::CommonMode {
                mhz,
                target,
                weight,
            } => GoalSave::CommonMode {
                mhz,
                target,
                weight,
            },
            Goal::FrontToBack {
                mhz,
                elevation,
                azimuth,
                rear,
                target,
                weight,
            } => GoalSave::FrontToBack {
                mhz,
                elevation,
                azimuth,
                rear,
                target,
                weight,
            },
            Goal::Match {
                name,
                start_mhz,
                stop_mhz,
                measured,
                z0,
                target,
                weight,
            } => GoalSave::Match {
                name,
                start_mhz,
                stop_mhz,
                measured: measured.iter().map(|z| (z.re, z.im)).collect(),
                z0,
                target,
                weight,
            },
        }
    }

    /// The goal (an SWR goal at the radio gets its feed chain from the
    /// Feedline view when the optimiser view next draws).
    pub fn to_goal(&self) -> Goal {
        match self.clone() {
            GoalSave::Swr {
                start_mhz,
                stop_mhz,
                points,
                z0,
                target,
                weight,
                at_radio,
            } => Goal::Swr {
                start_mhz,
                stop_mhz,
                points: points.max(1),
                z0,
                target,
                weight,
                through: at_radio.then(|| RadioChain {
                    stages: Vec::new(),
                    vars: Vec::new(),
                }),
            },
            GoalSave::Gain {
                mhz,
                elevation,
                azimuth,
                peak,
                target,
                weight,
            } => Goal::Gain {
                mhz,
                elevation,
                azimuth,
                peak,
                target,
                weight,
            },
            GoalSave::CommonMode {
                mhz,
                target,
                weight,
            } => Goal::CommonMode {
                mhz,
                target,
                weight,
            },
            GoalSave::FrontToBack {
                mhz,
                elevation,
                azimuth,
                rear,
                target,
                weight,
            } => Goal::FrontToBack {
                mhz,
                elevation,
                azimuth,
                rear,
                target,
                weight,
            },
            GoalSave::Match {
                name,
                start_mhz,
                stop_mhz,
                measured,
                z0,
                target,
                weight,
            } => Goal::Match {
                name,
                start_mhz,
                stop_mhz,
                measured: measured
                    .iter()
                    .map(|&(r, x)| Complex64::new(r, x))
                    .collect(),
                z0,
                target,
                weight,
            },
        }
    }
}

/// A comparison snapshot: its design, and the sweep to run again for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapshotSave {
    pub name: String,
    pub colour: [u8; 3],
    pub visible: bool,
    pub start_mhz: f64,
    pub stop_mhz: f64,
    pub points: usize,
    pub deck: String,
}

/// A measured VNA sweep.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeasurementSave {
    pub name: String,
    pub colour: [u8; 3],
    pub visible: bool,
    pub through_feedline: bool,
    /// (MHz, R, X) as measured.
    pub points: Vec<(f64, f64, f64)>,
}

pub fn rgb(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

pub fn colour(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_reads_back() {
        let deck = "CM test\r\nGW 1 11 0 0 0 0 0 1 0.001\r\nGE 0\r\nEX 0 1 6 0 1 0\r\nFR 0 1 0 0 14.2 0\r\nEN\r\n";
        let extras = Extras {
            notes: "Built 2026: \"quoted\" and\nlines".into(),
            mhz: 14.1,
            z0: 50.0,
            optimiser: OptimiserSave {
                goals: vec![GoalSave::from_goal(&Goal::Match {
                    name: "VNA".into(),
                    start_mhz: 14.0,
                    stop_mhz: 14.3,
                    measured: vec![Complex64::new(50.0, -3.0), Complex64::new(60.0, 12.5)],
                    z0: 50.0,
                    target: 0.05,
                    weight: 1.0,
                })],
                ..Default::default()
            },
            measurements: vec![MeasurementSave {
                name: "VNA".into(),
                colour: [204, 121, 167],
                visible: true,
                through_feedline: true,
                points: vec![(14.0, 50.0, -3.0), (14.3, 60.0, 12.5)],
            }],
            ..Default::default()
        };
        let text = join(deck, &extras).unwrap();
        // The deck comes first, unchanged; CRLF throughout.
        assert!(text.starts_with(deck));
        assert!(!text.replace("\r\n", "").contains('\n'));
        let (d, e) = split(&text);
        assert_eq!(d, deck);
        let mut back = e.unwrap().unwrap();
        assert_eq!(back.version, VERSION);
        back.version = 0;
        assert_eq!(back, extras);
        // A plain deck has no project part; a damaged one says why.
        assert!(split(deck).1.is_none());
        let broken = text.replace("notes", "nötes:(");
        assert!(split(&broken).1.unwrap().is_err());
    }

    #[test]
    fn feedline_settings_go_by_name() {
        let s = Settings {
            feed_cable: 2,
            conn_antenna: 2,
            conn_radio: CONNECTORS.len() + 1,
            conn_radio_db: 0.3,
            feed_length: 12.5,
            ..Default::default()
        };
        let saved = FeedSave::from_settings(&s);
        assert_eq!(saved.cable, CABLES[2].name);
        assert_eq!(saved.antenna_connector, CONNECTORS[1].name);
        assert_eq!(saved.radio_connector, "other");
        let mut t = Settings::default();
        saved.apply(&mut t);
        assert_eq!(
            (
                t.feed_cable,
                t.conn_antenna,
                t.conn_radio,
                t.feed_length,
                t.conn_radio_db
            ),
            (2, 2, CONNECTORS.len() + 1, 12.5, 0.3)
        );
    }

    #[test]
    fn every_goal_survives_the_round_trip() {
        let goals = [
            Goal::Swr {
                start_mhz: 14.0,
                stop_mhz: 14.35,
                points: 11,
                z0: 50.0,
                target: 1.5,
                weight: 1.0,
                through: None,
            },
            Goal::Gain {
                mhz: 14.2,
                elevation: 10.0,
                azimuth: 0.0,
                peak: false,
                target: 7.0,
                weight: 2.0,
            },
            Goal::CommonMode {
                mhz: 14.2,
                target: 5.0,
                weight: 1.0,
            },
            Goal::FrontToBack {
                mhz: 14.2,
                elevation: 10.0,
                azimuth: 0.0,
                rear: true,
                target: 20.0,
                weight: 1.0,
            },
        ];
        for g in &goals {
            assert_eq!(&GoalSave::from_goal(g).to_goal(), g);
        }
        // At the radio: the chain is filled in later, the choice is kept.
        let mut at_radio = goals[0].clone();
        if let Goal::Swr { through, .. } = &mut at_radio {
            *through = Some(RadioChain {
                stages: Vec::new(),
                vars: Vec::new(),
            });
        }
        assert_eq!(GoalSave::from_goal(&at_radio).to_goal(), at_radio);
    }
}
