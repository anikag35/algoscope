//! algoscope: an interactive terminal visualizer for classic algorithms.
//!
//! The crate is usable as a library as well as a binary. [`algo`] has no
//! dependency on the terminal or on `ratatui`, so the tracing model can be
//! reused to drive any other front end.

pub mod algo;
pub mod app;
