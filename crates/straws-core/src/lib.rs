//! Core library for the NEC antenna modeller: reading decks, running the
//! NEC engine, and reading its results. The GUI, the command-line tool,
//! batch runs and tests all build on this crate.

pub mod analysis;
pub mod bands;
pub mod deck;
pub mod dxf;
pub mod engine;
pub mod export;
pub mod optimise;
pub mod output;
pub mod reference;
pub mod sheetfit;
pub mod solver;
pub mod step;
pub mod stl;
pub mod templates;
pub mod units;
pub mod validate;
pub mod worker;

pub use num_complex::Complex64;
