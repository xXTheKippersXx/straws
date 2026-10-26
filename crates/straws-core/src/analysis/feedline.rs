//! Feedlines and baluns: what the radio sees at the far end of a lossy
//! transmission line, and where the power goes.

use std::f64::consts::PI;

use num_complex::Complex64;

use crate::units::C;

/// A transmission line: impedance, velocity factor and loss.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cable {
    pub name: &'static str,
    pub z0: f64,
    pub velocity_factor: f64,
    /// Matched loss (dB per 100 m) at 10 MHz and at 100 MHz. Loss between
    /// and beyond follows a·√f + b·f (conductor plus dielectric loss).
    pub loss_10: f64,
    pub loss_100: f64,
}

/// Typical published figures; a particular brand can differ a little.
pub const CABLES: &[Cable] = &[
    Cable {
        name: "RG-58C/U",
        z0: 50.0,
        velocity_factor: 0.66,
        loss_10: 4.6,
        loss_100: 16.1,
    },
    Cable {
        name: "RG-8X (mini-8)",
        z0: 50.0,
        velocity_factor: 0.78,
        loss_10: 3.6,
        loss_100: 12.1,
    },
    Cable {
        name: "RG-213/U",
        z0: 50.0,
        velocity_factor: 0.66,
        loss_10: 2.0,
        loss_100: 6.2,
    },
    Cable {
        name: "LMR-400",
        z0: 50.0,
        velocity_factor: 0.85,
        loss_10: 1.3,
        loss_100: 3.9,
    },
    Cable {
        name: "RG-6 (75 Ω)",
        z0: 75.0,
        velocity_factor: 0.82,
        loss_10: 2.0,
        loss_100: 6.6,
    },
    Cable {
        name: "450 Ω window line",
        z0: 450.0,
        velocity_factor: 0.91,
        loss_10: 0.33,
        loss_100: 1.3,
    },
    Cable {
        name: "300 Ω twin lead",
        z0: 300.0,
        velocity_factor: 0.82,
        loss_10: 2.6,
        loss_100: 9.2,
    },
    Cable {
        name: "Ideal (lossless) 50 Ω",
        z0: 50.0,
        velocity_factor: 1.0,
        loss_10: 0.0,
        loss_100: 0.0,
    },
];

impl Cable {
    /// Matched loss in dB per metre at `mhz`.
    pub fn loss_db_per_m(&self, mhz: f64) -> f64 {
        // Solve a·√10 + b·10 = L10 and a·10 + b·100 = L100.
        let (l10, l100) = (self.loss_10, self.loss_100);
        let s10 = 10f64.sqrt();
        let det = s10 * 100.0 - 10.0 * 10.0;
        let a = (l10 * 100.0 - 10.0 * l100) / det;
        let b = (s10 * l100 - 10.0 * l10) / det;
        ((a * mhz.sqrt() + b * mhz) / 100.0).max(0.0)
    }
}

/// A mated pair of coaxial connectors (plug and socket) in the feed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Connector {
    pub name: &'static str,
    /// The pair's maximum loss (dB) at `at_ghz`, scaled with √f (it's
    /// conductor loss); `at_ghz` 0: the same at every frequency.
    pub loss_db: f64,
    pub at_ghz: f64,
    /// The top of its rated frequency range (GHz).
    pub max_ghz: f64,
    /// Where the figures come from.
    pub source: &'static str,
}

/// Published maximum losses (makers' and military specification sheets,
/// checked 2026-10-28); typical parts lose less. UHF (PL-259/SO-239) isn't
/// listed: its published loss is "0.5 dB max" with no frequency given.
pub const CONNECTORS: &[Connector] = &[
    Connector {
        name: "BNC",
        loss_db: 0.2,
        at_ghz: 3.0,
        max_ghz: 4.0,
        source: "Amphenol RF 50 Ω BNC datasheets: 0.2 dB max at 3 GHz, rated DC to 4 GHz",
    },
    Connector {
        name: "N",
        loss_db: 0.15,
        at_ghz: 10.0,
        max_ghz: 11.0,
        source: "Amphenol RF type N: 0.15 dB max at 10 GHz, rated DC to 11 GHz",
    },
    Connector {
        name: "SMA",
        loss_db: 0.06,
        at_ghz: 1.0,
        max_ghz: 18.0,
        source: "MIL-PRF-39012 SMA specification sheet: 0.06·√f(GHz) dB max; commonly rated DC to 18 GHz",
    },
];

impl Connector {
    /// One whose loss the user gives (dB, the same at every frequency).
    pub fn other(loss_db: f64) -> Connector {
        Connector {
            name: "Other",
            loss_db,
            at_ghz: 0.0,
            max_ghz: f64::INFINITY,
            source: "the loss you entered",
        }
    }

    /// The pair's loss in dB at `mhz`.
    pub fn loss_db(&self, mhz: f64) -> f64 {
        if self.at_ghz <= 0.0 {
            self.loss_db
        } else {
            self.loss_db * (mhz / 1000.0 / self.at_ghz).max(0.0).sqrt()
        }
    }
}

/// The 50 Ω line inside connectors and adapters (PTFE-filled: velocity
/// factor 1/√2.07), for the length they add at the antenna.
pub const CONNECTOR_LINE: Cable = Cable {
    name: "connector",
    z0: 50.0,
    velocity_factor: 0.695,
    loss_10: 0.0,
    loss_100: 0.0,
};

/// A feed system: the line from the radio to the balun, and the balun.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Feed {
    pub cable: Cable,
    pub length_m: f64,
    /// Balun/unun impedance ratio at the antenna (4 for 4:1, 49 for
    /// 49:1); 1 for none.
    pub balun_ratio: f64,
    /// Balun loss in dB.
    pub balun_loss_db: f64,
    /// Connectors between the balun and the cable, and the length of 50 Ω
    /// line they (and any adapter) add, in metres.
    pub antenna_connector: Option<Connector>,
    pub connector_length_m: f64,
    /// Connectors at the radio end.
    pub radio_connector: Option<Connector>,
}

/// What the radio sees, and where the power goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtRadio {
    /// Impedance at the radio end of the line.
    pub impedance: Complex64,
    /// SWR on the line (at the radio), against the line's own Z0.
    pub swr_line: f64,
    /// Matched line loss (dB).
    pub matched_loss_db: f64,
    /// Total loss, including the extra loss from SWR and the balun (dB).
    pub total_loss_db: f64,
    /// Share of the radio's power that reaches the antenna.
    pub efficiency: f64,
}

/// Impedance looking into `length` metres of line with `load` at the end.
pub fn line_input(cable: &Cable, length: f64, mhz: f64, load: Complex64) -> Complex64 {
    let alpha = cable.loss_db_per_m(mhz) / (20.0 / std::f64::consts::LN_10);
    let beta = 2.0 * PI * mhz * 1e6 / (C * cable.velocity_factor);
    let gl = Complex64::new(alpha, beta) * length;
    // Through the reflection coefficient (Γ e^(-2γl)), not tanh(γl), which
    // is infinite on an exact lossless quarter wave.
    let z0 = cable.z0;
    let gamma = (load - z0) / (load + z0) * (-2.0 * gl).exp();
    z0 * (1.0 + gamma) / (1.0 - gamma)
}

/// The radio's view of an antenna with feed impedance `antenna` at `mhz`.
pub fn at_radio(feed: &Feed, mhz: f64, antenna: Complex64) -> AtRadio {
    let mut load = antenna / feed.balun_ratio.max(1e-6);
    if feed.antenna_connector.is_some() && feed.connector_length_m > 0.0 {
        load = line_input(&CONNECTOR_LINE, feed.connector_length_m, mhz, load);
    }
    let z_in = line_input(&feed.cable, feed.length_m, mhz, load);
    let z0 = feed.cable.z0;
    let gamma_load = ((load - z0) / (load + z0)).norm();
    let gamma_in = ((z_in - z0) / (z_in + z0)).norm();
    let matched = feed.cable.loss_db_per_m(mhz) * feed.length_m;
    // Loss with mismatch: 10·log10[(a² − |Γ|²) / (a(1 − |Γ|²))], a = matched loss ratio.
    let a = 10f64.powf(matched / 10.0);
    let g2 = gamma_load * gamma_load;
    let line_loss = if g2 < 1.0 {
        10.0 * ((a * a - g2) / (a * (1.0 - g2))).log10()
    } else {
        f64::INFINITY
    };
    let connectors: f64 = [feed.antenna_connector, feed.radio_connector]
        .iter()
        .flatten()
        .map(|c| c.loss_db(mhz))
        .sum();
    let total = line_loss + feed.balun_loss_db + connectors;
    AtRadio {
        impedance: z_in,
        swr_line: (1.0 + gamma_in) / (1.0 - gamma_in).max(1e-12),
        matched_loss_db: matched,
        total_loss_db: total,
        efficiency: 10f64.powf(-total / 10.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_basics() {
        let rg213 = CABLES[2];
        // The fit goes through its two published points.
        assert!((rg213.loss_db_per_m(10.0) * 100.0 - 2.0).abs() < 1e-9);
        assert!((rg213.loss_db_per_m(100.0) * 100.0 - 6.2).abs() < 1e-9);
        // A lossless half-wave line repeats its load.
        let ideal = CABLES[7];
        let half = C / 14e6 / 2.0;
        let z = line_input(&ideal, half, 14.0, Complex64::new(20.0, 35.0));
        assert!((z - Complex64::new(20.0, 35.0)).norm() < 1e-6, "{z}");
        // Matched: the loss is the matched loss and SWR is 1.
        let feed = Feed {
            cable: rg213,
            length_m: 30.0,
            balun_ratio: 1.0,
            balun_loss_db: 0.0,
            antenna_connector: None,
            connector_length_m: 0.0,
            radio_connector: None,
        };
        let r = at_radio(&feed, 14.0, Complex64::new(50.0, 0.0));
        assert!((r.total_loss_db - r.matched_loss_db).abs() < 1e-9);
        assert!((r.swr_line - 1.0).abs() < 1e-6);
        // A mismatch adds loss and the line's loss makes the SWR look better.
        let r2 = at_radio(&feed, 14.0, Complex64::new(150.0, 0.0));
        assert!(r2.total_loss_db > r.total_loss_db);
        assert!(r2.swr_line < 3.0);
        // A 4:1 balun turns 200 Ω into 50 Ω.
        let b = Feed {
            balun_ratio: 4.0,
            ..feed
        };
        let r3 = at_radio(&b, 14.0, Complex64::new(200.0, 0.0));
        assert!((r3.swr_line - 1.0).abs() < 1e-6);
    }

    #[test]
    fn connectors() {
        let [bnc, n, sma] = [CONNECTORS[0], CONNECTORS[1], CONNECTORS[2]];
        // The published points, and √f between.
        assert!((bnc.loss_db(3000.0) - 0.2).abs() < 1e-12);
        assert!((n.loss_db(10_000.0) - 0.15).abs() < 1e-12);
        assert!((sma.loss_db(4000.0) - 0.12).abs() < 1e-12);
        assert!((bnc.loss_db(750.0) - 0.1).abs() < 1e-12);
        assert_eq!(Connector::other(0.3).loss_db(7.0), 0.3);
        // Matched: their loss adds to the line's.
        let feed = Feed {
            cable: CABLES[3],
            length_m: 10.0,
            balun_ratio: 1.0,
            balun_loss_db: 0.0,
            antenna_connector: Some(n),
            connector_length_m: 0.0,
            radio_connector: Some(sma),
        };
        let r = at_radio(&feed, 1296.0, Complex64::new(50.0, 0.0));
        let want = CABLES[3].loss_db_per_m(1296.0) * 10.0 + n.loss_db(1296.0) + sma.loss_db(1296.0);
        assert!((r.total_loss_db - want).abs() < 1e-9, "{r:?}");
        // A connector's own length turns a mismatched load like line does:
        // a quarter wave of it (in PTFE) turns 100 Ω into 25 Ω.
        let quarter = C / 1296e6 * 0.695 / 4.0;
        let lossless = Feed {
            cable: CABLES[7],
            length_m: 0.0,
            antenna_connector: Some(n),
            connector_length_m: quarter,
            radio_connector: None,
            ..feed
        };
        let z = at_radio(&lossless, 1296.0, Complex64::new(100.0, 0.0)).impedance;
        assert!((z - Complex64::new(25.0, 0.0)).norm() < 1e-6, "{z}");
    }
}
