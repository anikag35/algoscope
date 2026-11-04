//! The event-log model, generic over what kind of event is being logged.
//!
//! An algorithm here does not animate itself. It runs to completion against a
//! recorder, which captures every interesting operation as a [`Step`]. The
//! result is a [`Trace`]: an immutable log the UI replays. Rendering becomes a
//! pure function of `(trace, cursor)`, which is what makes stepping backwards,
//! scrubbing to an arbitrary position, and reverse playback fall out for free
//! instead of needing coroutines or a parked worker thread.
//!
//! The invariant that buys all of that: **every [`Step`] carries enough
//! information to undo itself.** A swap is self-inverse, but overwriting a
//! value has to record what was there before. Miss that and rewinding silently
//! corrupts the state, so `tests/properties.rs` asserts the round trip for
//! every algorithm in both modes.
//!
//! [`Trace`] and [`Playback`] are generic over the step type, so sorting and
//! pathfinding share one cursor implementation rather than two that have to be
//! kept in agreement.

/// A reversible operation on some state.
///
/// Implementors are expected to be small and `Copy`: a trace holds one per
/// operation the algorithm performed, which runs to tens of thousands.
pub trait Step: Copy {
    /// The state that a sequence of these steps mutates.
    type State: Clone;

    /// Apply this step.
    fn apply(self, state: &mut Self::State);

    /// Undo this step, exactly.
    ///
    /// `unapply(apply(s))` must leave the state as it was found. Purely
    /// cosmetic fields are the one allowed exception, and the ones that take it
    /// say so.
    fn unapply(self, state: &mut Self::State);
}

/// Static facts about an algorithm, for the metrics panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Complexity {
    pub best: &'static str,
    pub average: &'static str,
    pub worst: &'static str,
    pub space: &'static str,
    /// A short note on the algorithm's defining guarantee, or the lack of one.
    pub note: &'static str,
}

/// A completed algorithm run: where it started, every step it took, and what it
/// is.
#[derive(Debug, Clone)]
pub struct Trace<S: Step> {
    pub initial: S::State,
    pub steps: Vec<S>,
    pub name: &'static str,
    pub complexity: Complexity,
}

impl<S: Step> Trace<S> {
    /// Number of steps in the log.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Rebuild the state after `cursor` steps, from scratch.
    ///
    /// O(cursor). [`Playback`] moves incrementally instead and should be
    /// preferred during playback; this exists for tests and for jumping to a
    /// position with no state already in hand.
    pub fn state_at(&self, cursor: usize) -> S::State {
        let mut state = self.initial.clone();
        for &step in &self.steps[..cursor.min(self.len())] {
            step.apply(&mut state);
        }
        state
    }
}

/// A [`Trace`] paired with a cursor into it.
///
/// All cursor movement goes through here, so the state and the cursor cannot
/// drift out of sync: that pairing is the one invariant the UI depends on.
#[derive(Debug, Clone)]
pub struct Playback<S: Step> {
    trace: Trace<S>,
    state: S::State,
    cursor: usize,
}

impl<S: Step> Playback<S> {
    pub fn new(trace: Trace<S>) -> Self {
        let state = trace.initial.clone();
        Self {
            trace,
            state,
            cursor: 0,
        }
    }

    pub fn state(&self) -> &S::State {
        &self.state
    }

    pub fn trace(&self) -> &Trace<S> {
        &self.trace
    }

    /// How many steps have been applied.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn len(&self) -> usize {
        self.trace.len()
    }

    pub fn is_empty(&self) -> bool {
        self.trace.is_empty()
    }

    pub fn at_start(&self) -> bool {
        self.cursor == 0
    }

    pub fn at_end(&self) -> bool {
        self.cursor >= self.trace.len()
    }

    /// Fraction of the run completed, in `0.0..=1.0`.
    pub fn progress(&self) -> f64 {
        if self.trace.is_empty() {
            1.0
        } else {
            self.cursor as f64 / self.trace.len() as f64
        }
    }

    /// Apply one step. Returns `false` if already at the end.
    pub fn step_forward(&mut self) -> bool {
        match self.trace.steps.get(self.cursor) {
            Some(&step) => {
                step.apply(&mut self.state);
                self.cursor += 1;
                true
            }
            None => false,
        }
    }

    /// Undo one step. Returns `false` if already at the start.
    pub fn step_back(&mut self) -> bool {
        match self.cursor.checked_sub(1) {
            Some(prev) => {
                self.trace.steps[prev].unapply(&mut self.state);
                self.cursor = prev;
                true
            }
            None => false,
        }
    }

    /// Move to an arbitrary cursor position, clamped to the log.
    ///
    /// Walks from wherever it is now, so a short scrub costs only the steps it
    /// actually crosses rather than a full replay from the start.
    pub fn seek(&mut self, target: usize) {
        let target = target.min(self.trace.len());
        while self.cursor < target {
            self.step_forward();
        }
        while self.cursor > target {
            self.step_back();
        }
    }

    pub fn rewind(&mut self) {
        self.seek(0);
    }

    pub fn fast_forward(&mut self) {
        self.seek(self.trace.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal step type, so the cursor logic is tested on its own rather
    /// than through a sorting algorithm.
    #[derive(Debug, Clone, Copy)]
    enum Counter {
        Add(i64),
        Double,
    }

    impl Step for Counter {
        type State = i64;

        fn apply(self, state: &mut i64) {
            match self {
                Self::Add(n) => *state += n,
                Self::Double => *state *= 2,
            }
        }

        fn unapply(self, state: &mut i64) {
            match self {
                Self::Add(n) => *state -= n,
                Self::Double => *state /= 2,
            }
        }
    }

    fn trace() -> Trace<Counter> {
        Trace {
            initial: 1,
            steps: vec![
                Counter::Add(4),
                Counter::Double,
                Counter::Add(-3),
                Counter::Double,
            ],
            name: "counter",
            complexity: Complexity {
                best: "-",
                average: "-",
                worst: "-",
                space: "-",
                note: "-",
            },
        }
    }

    #[test]
    fn state_at_matches_stepping_forward() {
        let mut playback = Playback::new(trace());
        for cursor in 0..=playback.len() {
            assert_eq!(
                *playback.state(),
                trace().state_at(cursor),
                "mismatch at cursor {cursor}"
            );
            playback.step_forward();
        }
    }

    #[test]
    fn round_trip_returns_to_the_start() {
        let mut playback = Playback::new(trace());
        playback.fast_forward();
        assert_eq!(*playback.state(), 14);
        playback.rewind();
        assert_eq!(*playback.state(), 1);
        assert!(playback.at_start());
    }

    #[test]
    fn seeking_from_either_direction_agrees() {
        for target in 0..=4 {
            let mut up = Playback::new(trace());
            up.seek(target);

            let mut down = Playback::new(trace());
            down.fast_forward();
            down.seek(target);

            assert_eq!(up.state(), down.state(), "disagreement at {target}");
            assert_eq!(up.cursor(), down.cursor());
        }
    }

    #[test]
    fn stepping_past_either_end_is_a_no_op() {
        let mut playback = Playback::new(trace());
        assert!(!playback.step_back());
        assert_eq!(playback.cursor(), 0);

        playback.fast_forward();
        assert!(!playback.step_forward());
        assert_eq!(playback.cursor(), 4);
    }

    #[test]
    fn seek_clamps_beyond_the_end() {
        let mut playback = Playback::new(trace());
        playback.seek(999);
        assert_eq!(playback.cursor(), 4);
        assert!(playback.at_end());
    }

    #[test]
    fn an_empty_trace_is_complete() {
        let empty: Trace<Counter> = Trace {
            initial: 7,
            steps: vec![],
            ..trace()
        };
        let playback = Playback::new(empty);
        assert!(playback.at_start() && playback.at_end());
        assert_eq!(playback.progress(), 1.0);
    }
}
