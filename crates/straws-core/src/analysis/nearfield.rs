//! Near fields: the electric field on a flat grid near the antenna, and a
//! guide to RF exposure.

use crate::analysis::plan::{FrequencyPlan, run_deck};
use crate::deck::{NumCard, NumericDeck};

/// Which way a field plane lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// Horizontal, at a height z.
    Xy,
    /// Vertical, at a y.
    Xz,
    /// Vertical, at an x.
    Yz,
}

impl Plane {
    /// The two axes across the plane and the one it's fixed on.
    pub fn axes(self) -> (usize, usize, usize) {
        match self {
            Plane::Xy => (0, 1, 2),
            Plane::Xz => (0, 2, 1),
            Plane::Yz => (1, 2, 0),
        }
    }
}

/// A square grid of `n`×`n` points on a plane: centred on `centre` (the
/// two in-plane coordinates), `size` metres across, at `offset` along the
/// fixed axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub plane: Plane,
    pub centre: [f64; 2],
    pub size: f64,
    pub offset: f64,
    pub n: usize,
}

impl Grid {
    pub fn step(&self) -> f64 {
        self.size / (self.n.max(2) - 1) as f64
    }

    /// The NE card for this grid.
    pub fn ne_card(&self) -> NumCard {
        let (u, v, w) = self.plane.axes();
        let mut start = [0.0; 3];
        let mut step = [0.0; 3];
        let mut count = [1i64; 3];
        start[u] = self.centre[0] - self.size / 2.0;
        start[v] = self.centre[1] - self.size / 2.0;
        start[w] = self.offset;
        step[u] = self.step();
        step[v] = self.step();
        count[u] = self.n as i64;
        count[v] = self.n as i64;
        NumCard::new(
            "NE",
            &[0, count[0], count[1], count[2]],
            &[start[0], start[1], start[2], step[0], step[1], step[2]],
        )
    }
}

/// The deck that works out the near field on `grid` at `mhz`.
pub fn near_field_deck(deck: &NumericDeck, mhz: f64, grid: &Grid) -> NumericDeck {
    let mut run = run_deck(deck, &FrequencyPlan::single(mhz), None);
    run.cards.pop(); // the XQ
    run.cards.push(NumCard::new("PT", &[-1], &[]));
    run.cards.push(grid.ne_card());
    run
}

/// ICNIRP (1998) general-public reference level for the electric field,
/// V/m RMS, at `mhz` (the basis of Australia's ARPANSA standard). A guide
/// only: real assessments average over time and the body.
pub fn icnirp_public_e(mhz: f64) -> f64 {
    if mhz < 1.0 {
        87.0
    } else if mhz < 10.0 {
        87.0 / mhz.sqrt()
    } else if mhz < 400.0 {
        28.0
    } else if mhz < 2000.0 {
        1.375 * mhz.sqrt()
    } else {
        61.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_card_and_limits() {
        let g = Grid {
            plane: Plane::Xz,
            centre: [0.0, 5.0],
            size: 10.0,
            offset: 1.0,
            n: 11,
        };
        let c = g.ne_card();
        assert_eq!(&c.ints[..4], &[0, 11, 1, 11]);
        assert_eq!(&c.floats[..6], &[-5.0, 1.0, 0.0, 1.0, 0.0, 1.0]);
        assert_eq!(icnirp_public_e(14.0), 28.0);
        assert!((icnirp_public_e(7.0) - 87.0 / 7f64.sqrt()).abs() < 1e-12);
    }
}
