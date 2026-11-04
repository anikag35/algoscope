//! The event-log model.
//!
//! An algorithm here does not animate itself. It runs to completion against a
//! [`Tracer`], which records every comparison and mutation as a [`Step`]. The
//! result is a [`Trace`]: an immutable log the UI replays. Rendering becomes a
//! pure function of `(trace, cursor)`, which is what makes stepping backwards,
//! scrubbing to an arbitrary position, and reverse playback fall out for free
//! instead of needing coroutines or a parked worker thread.
//!
//! The invariant that buys all of that: **every [`Step`] carries enough
//! information to undo itself.** [`Step::Swap`] is self-inverse, but
//! [`Step::Write`] has to record the value it overwrote. Miss that and
//! rewinding silently corrupts the array, so `tests/properties.rs` asserts the
//! round trip for every algorithm.

use std::cmp::Ordering;

/// A single reversible event in an algorithm's execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The algorithm examined slots `i` and `j`. Observational: it moves no
    /// data, but it drives the highlight and the comparison counter.
    Compare { i: usize, j: usize },
    /// Exchange the values in slots `i` and `j`. Self-inverse.
    Swap { i: usize, j: usize },
    /// Overwrite slot `idx`, replacing `old` with `new`.
    ///
    /// `old` is redundant going forwards and essential going backwards.
    /// Insertion and merge sort shift values rather than exchanging them, so
    /// without it there is no way back.
    Write { idx: usize, old: u32, new: u32 },
    /// Slot `idx` has reached its final position. Recorded at most once per
    /// index, so that undoing it is unambiguous.
    MarkSorted { idx: usize },
}

/// Static facts about an algorithm, for the metrics panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Complexity {
    pub best: &'static str,
    pub average: &'static str,
    pub worst: &'static str,
    pub space: &'static str,
    pub stable: bool,
}

/// The visible state of the array at one cursor position.
///
/// Built by replaying [`Step`]s, never by re-running the algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Current contents of the array.
    pub values: Vec<u32>,
    /// Slots the algorithm has settled.
    ///
    /// Final positions for bubble and selection sort; for insertion sort this
    /// is the growing sorted prefix, which is ordered but can still be shifted
    /// right by a later insertion.
    pub sorted: Vec<bool>,
    /// Slots the most recent step touched, for highlighting. Cleared when
    /// stepping backwards: the highlight is cosmetic, so it is not worth
    /// keeping a previous-highlight stack just to recover it.
    pub active: Vec<usize>,
    /// Comparisons performed so far. Shown against the theoretical bound in the
    /// metrics panel.
    pub comparisons: u64,
    /// Slot writes performed so far. A swap counts as two.
    pub writes: u64,
}

impl Frame {
    /// The frame before any step has been applied.
    pub fn new(values: Vec<u32>) -> Self {
        let n = values.len();
        Self {
            values,
            sorted: vec![false; n],
            active: Vec::with_capacity(2),
            comparisons: 0,
            writes: 0,
        }
    }

    /// Whether `values` is in non-decreasing order.
    pub fn is_sorted(&self) -> bool {
        self.values.windows(2).all(|w| w[0] <= w[1])
    }

    fn apply(&mut self, step: Step) {
        self.active.clear();
        match step {
            Step::Compare { i, j } => {
                self.comparisons += 1;
                self.active.extend([i, j]);
            }
            Step::Swap { i, j } => {
                self.values.swap(i, j);
                self.writes += 2;
                self.active.extend([i, j]);
            }
            Step::Write { idx, new, .. } => {
                self.values[idx] = new;
                self.writes += 1;
                self.active.push(idx);
            }
            Step::MarkSorted { idx } => self.sorted[idx] = true,
        }
    }

    fn unapply(&mut self, step: Step) {
        self.active.clear();
        match step {
            Step::Compare { .. } => self.comparisons -= 1,
            Step::Swap { i, j } => {
                self.values.swap(i, j);
                self.writes -= 2;
            }
            Step::Write { idx, old, .. } => {
                self.values[idx] = old;
                self.writes -= 1;
            }
            Step::MarkSorted { idx } => self.sorted[idx] = false,
        }
    }
}

/// A completed algorithm run: where it started, every step it took, and what it
/// is.
#[derive(Debug, Clone)]
pub struct Trace {
    pub initial: Vec<u32>,
    pub steps: Vec<Step>,
    pub name: &'static str,
    pub complexity: Complexity,
}

impl Trace {
    /// Number of steps in the log.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Rebuild the frame after `cursor` steps, from scratch.
    ///
    /// O(cursor). [`Playback`] moves incrementally instead and should be
    /// preferred during playback; this exists for tests and for jumping to a
    /// position with no frame already in hand.
    pub fn frame_at(&self, cursor: usize) -> Frame {
        let mut frame = Frame::new(self.initial.clone());
        for &step in &self.steps[..cursor.min(self.len())] {
            frame.apply(step);
        }
        frame
    }
}

/// A [`Trace`] paired with a cursor into it.
///
/// All cursor movement goes through here, so the frame and the cursor cannot
/// drift out of sync: that pairing is the one invariant the UI depends on.
#[derive(Debug, Clone)]
pub struct Playback {
    trace: Trace,
    frame: Frame,
    cursor: usize,
}

impl Playback {
    pub fn new(trace: Trace) -> Self {
        let frame = Frame::new(trace.initial.clone());
        Self {
            trace,
            frame,
            cursor: 0,
        }
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn trace(&self) -> &Trace {
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

    /// The step about to be applied, if any.
    pub fn next_step(&self) -> Option<Step> {
        self.trace.steps.get(self.cursor).copied()
    }

    /// Apply one step. Returns `false` if already at the end.
    pub fn step_forward(&mut self) -> bool {
        match self.trace.steps.get(self.cursor) {
            Some(&step) => {
                self.frame.apply(step);
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
                self.frame.unapply(self.trace.steps[prev]);
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

/// Recorder the algorithms write through.
///
/// Algorithms read and mutate the array by way of these methods, which keeps
/// their code close to a textbook implementation while still producing the log.
/// The algorithm runs to completion immediately; the `Tracer` never blocks or
/// yields.
pub struct Tracer {
    initial: Vec<u32>,
    values: Vec<u32>,
    sorted: Vec<bool>,
    steps: Vec<Step>,
}

impl Tracer {
    pub fn new(values: Vec<u32>) -> Self {
        Self {
            initial: values.clone(),
            sorted: vec![false; values.len()],
            values,
            steps: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Read a slot without recording anything.
    ///
    /// Use [`Tracer::compare`] for decisions, so that they reach the counter;
    /// this is for moving values once a decision has been made.
    pub fn get(&self, i: usize) -> u32 {
        self.values[i]
    }

    /// Compare two slots, recording the comparison.
    pub fn compare(&mut self, i: usize, j: usize) -> Ordering {
        self.steps.push(Step::Compare { i, j });
        self.values[i].cmp(&self.values[j])
    }

    /// Record a comparison for the counter and the highlight without reading
    /// the array.
    ///
    /// For algorithms that compare against a value held outside it, such as
    /// insertion sort's key: the work is real and belongs in the count, but the
    /// operands are not both slots.
    pub fn note_compare(&mut self, i: usize, j: usize) {
        self.steps.push(Step::Compare { i, j });
    }

    /// Exchange two slots. A self-swap is a genuine no-op and is not recorded.
    pub fn swap(&mut self, i: usize, j: usize) {
        if i == j {
            return;
        }
        self.steps.push(Step::Swap { i, j });
        self.values.swap(i, j);
    }

    /// Overwrite a slot, capturing the previous value so the step can be undone.
    ///
    /// Recorded even when the value is unchanged: the algorithm really did
    /// perform the write, and the counter should say so.
    pub fn write(&mut self, idx: usize, new: u32) {
        let old = self.values[idx];
        self.steps.push(Step::Write { idx, old, new });
        self.values[idx] = new;
    }

    /// Mark a slot final. Idempotent, because a second mark would make the undo
    /// ambiguous.
    pub fn mark_sorted(&mut self, idx: usize) {
        if !self.sorted[idx] {
            self.sorted[idx] = true;
            self.steps.push(Step::MarkSorted { idx });
        }
    }

    /// Mark every remaining slot final, for the tail of an early exit.
    pub fn mark_all_sorted(&mut self) {
        for i in 0..self.len() {
            self.mark_sorted(i);
        }
    }

    pub fn finish(self, name: &'static str, complexity: Complexity) -> Trace {
        Trace {
            initial: self.initial,
            steps: self.steps,
            name,
            complexity,
        }
    }
}
