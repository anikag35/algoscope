//! Algorithms, and the tracing model they record into.
//!
//! [`trace`] is generic over the step type; [`sort`] and [`grid`] supply the
//! concrete steps and state for their own domains.

pub mod grid;
pub mod sort;
pub mod trace;

pub use trace::{Complexity, Playback, Step, Trace};
