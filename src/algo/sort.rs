//! Sorting algorithms, written against a [`Tracer`] rather than a bare slice.
//!
//! Each one is a textbook implementation with its reads and writes routed
//! through the recorder. None of them knows that a UI exists, and none of them
//! can pause: they run to completion and hand back a [`Trace`].
//!
//! The three here are deliberately contrasting in the shape of the work they
//! do. Bubble sort is almost all swaps, selection sort is almost all
//! comparisons with exactly one swap per pass, and insertion sort shifts with
//! [`SortStep::Write`] rather than exchanging — which is what exercises the
//! reversibility of writes.

use std::cmp::Ordering;

use super::trace::{Complexity, Step, Trace};

/// A single reversible event in a sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortStep {
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
    /// Slot `idx` has been settled. Recorded at most once per index, so that
    /// undoing it is unambiguous.
    MarkSorted { idx: usize },
}

/// The visible state of the array at one cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortState {
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
    /// keeping a stack of previous highlights just to recover it.
    pub active: Vec<usize>,
    /// Comparisons performed so far. Shown against the theoretical bound in
    /// the metrics panel.
    pub comparisons: u64,
    /// Slot writes performed so far. A swap counts as two.
    pub writes: u64,
}

impl SortState {
    /// The state before any step has been applied.
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
}

impl Step for SortStep {
    type State = SortState;

    fn apply(self, state: &mut SortState) {
        state.active.clear();
        match self {
            Self::Compare { i, j } => {
                state.comparisons += 1;
                state.active.extend([i, j]);
            }
            Self::Swap { i, j } => {
                state.values.swap(i, j);
                state.writes += 2;
                state.active.extend([i, j]);
            }
            Self::Write { idx, new, .. } => {
                state.values[idx] = new;
                state.writes += 1;
                state.active.push(idx);
            }
            Self::MarkSorted { idx } => state.sorted[idx] = true,
        }
    }

    fn unapply(self, state: &mut SortState) {
        state.active.clear();
        match self {
            Self::Compare { .. } => state.comparisons -= 1,
            Self::Swap { i, j } => {
                state.values.swap(i, j);
                state.writes -= 2;
            }
            Self::Write { idx, old, .. } => {
                state.values[idx] = old;
                state.writes -= 1;
            }
            Self::MarkSorted { idx } => state.sorted[idx] = false,
        }
    }
}

/// Which sorting algorithm to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Algorithm {
    #[default]
    Bubble,
    Insertion,
    Selection,
}

impl Algorithm {
    /// Every algorithm, in the order the UI offers them.
    pub const ALL: [Self; 3] = [Self::Bubble, Self::Insertion, Self::Selection];

    /// Run against `values`, producing a replayable log.
    pub fn run(self, values: Vec<u32>) -> Trace<SortStep> {
        match self {
            Self::Bubble => bubble(values),
            Self::Insertion => insertion(values),
            Self::Selection => selection(values),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Bubble => "Bubble sort",
            Self::Insertion => "Insertion sort",
            Self::Selection => "Selection sort",
        }
    }

    /// The next algorithm, wrapping around.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&a| a == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The previous algorithm, wrapping around.
    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|&a| a == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

pub const BUBBLE: Complexity = Complexity {
    best: "O(n)",
    average: "O(n²)",
    worst: "O(n²)",
    space: "O(1)",
    note: "stable, adaptive",
};

pub const INSERTION: Complexity = Complexity {
    best: "O(n)",
    average: "O(n²)",
    worst: "O(n²)",
    space: "O(1)",
    note: "stable, adaptive",
};

pub const SELECTION: Complexity = Complexity {
    best: "O(n²)",
    average: "O(n²)",
    worst: "O(n²)",
    space: "O(1)",
    note: "fewest writes",
};

/// Bubble sort: repeatedly sweep, exchanging adjacent slots that are out of
/// order.
///
/// Each sweep settles one more element at the top, so the swept range shrinks.
/// The `swapped` flag is what gives bubble sort its O(n) best case: a clean
/// sweep means the whole array is ordered and the remaining passes would do
/// nothing.
pub fn bubble(values: Vec<u32>) -> Trace<SortStep> {
    let mut t = Tracer::new(values);
    let n = t.len();

    for end in (1..n).rev() {
        let mut swapped = false;
        for i in 0..end {
            if t.compare(i, i + 1) == Ordering::Greater {
                t.swap(i, i + 1);
                swapped = true;
            }
        }
        t.mark_sorted(end);
        if !swapped {
            break;
        }
    }

    t.mark_all_sorted();
    t.finish(Algorithm::Bubble.name(), BUBBLE)
}

/// Insertion sort: grow a sorted prefix, inserting each new element into place.
///
/// The inner loop shifts rather than swaps — it opens a hole and slides values
/// right one at a time, then drops the key in. That is one write per shift
/// instead of a swap's two, and it is the reason [`SortStep::Write`] has to
/// carry the overwritten value.
pub fn insertion(values: Vec<u32>) -> Trace<SortStep> {
    let mut t = Tracer::new(values);
    let n = t.len();

    if n > 0 {
        t.mark_sorted(0);
    }

    for i in 1..n {
        let key = t.get(i);
        let mut j = i;

        // Slide everything greater than `key` one slot to the right. The
        // comparison is against a value held outside the array, so it is noted
        // for the counter rather than read back out of two slots.
        while j > 0 {
            t.note_compare(j - 1, j);
            if t.get(j - 1) <= key {
                break;
            }
            let shifted = t.get(j - 1);
            t.write(j, shifted);
            j -= 1;
        }

        t.write(j, key);
        t.mark_sorted(i);
    }

    t.mark_all_sorted();
    t.finish(Algorithm::Insertion.name(), INSERTION)
}

/// Selection sort: find the smallest remaining element, swap it into place.
///
/// The scan is unconditional, which is why there is no best case to exploit:
/// selection sort performs exactly `n(n-1)/2` comparisons on every input,
/// sorted or not. In exchange it makes at most one swap per pass, the fewest
/// writes of the three.
pub fn selection(values: Vec<u32>) -> Trace<SortStep> {
    let mut t = Tracer::new(values);
    let n = t.len();

    for i in 0..n {
        let mut min = i;
        for j in (i + 1)..n {
            if t.compare(j, min) == Ordering::Less {
                min = j;
            }
        }
        t.swap(i, min);
        t.mark_sorted(i);
    }

    t.mark_all_sorted();
    t.finish(Algorithm::Selection.name(), SELECTION)
}

/// Recorder the sorting algorithms write through.
///
/// Algorithms read and mutate the array by way of these methods, which keeps
/// their code close to a textbook implementation while still producing the log.
/// The algorithm runs to completion immediately; the `Tracer` never blocks or
/// yields.
pub struct Tracer {
    initial: Vec<u32>,
    values: Vec<u32>,
    sorted: Vec<bool>,
    steps: Vec<SortStep>,
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
        self.steps.push(SortStep::Compare { i, j });
        self.values[i].cmp(&self.values[j])
    }

    /// Record a comparison for the counter and the highlight without reading
    /// the array.
    ///
    /// For algorithms that compare against a value held outside it, such as
    /// insertion sort's key: the work is real and belongs in the count, but the
    /// operands are not both slots.
    pub fn note_compare(&mut self, i: usize, j: usize) {
        self.steps.push(SortStep::Compare { i, j });
    }

    /// Exchange two slots. A self-swap is a genuine no-op and is not recorded.
    pub fn swap(&mut self, i: usize, j: usize) {
        if i == j {
            return;
        }
        self.steps.push(SortStep::Swap { i, j });
        self.values.swap(i, j);
    }

    /// Overwrite a slot, capturing the previous value so the step can be undone.
    ///
    /// Recorded even when the value is unchanged: the algorithm really did
    /// perform the write, and the counter should say so.
    pub fn write(&mut self, idx: usize, new: u32) {
        let old = self.values[idx];
        self.steps.push(SortStep::Write { idx, old, new });
        self.values[idx] = new;
    }

    /// Mark a slot settled. Idempotent, because a second mark would make the
    /// undo ambiguous.
    pub fn mark_sorted(&mut self, idx: usize) {
        if !self.sorted[idx] {
            self.sorted[idx] = true;
            self.steps.push(SortStep::MarkSorted { idx });
        }
    }

    /// Mark every remaining slot settled, for the tail of an early exit.
    pub fn mark_all_sorted(&mut self) {
        for i in 0..self.len() {
            self.mark_sorted(i);
        }
    }

    pub fn finish(self, name: &'static str, complexity: Complexity) -> Trace<SortStep> {
        Trace {
            initial: SortState::new(self.initial),
            steps: self.steps,
            name,
            complexity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replaying a whole trace has to land on the sorted array. This is the
    /// cheap version of the property test in `tests/properties.rs`, kept here
    /// for the edge cases that generators rarely produce.
    fn check(algorithm: Algorithm, input: &[u32]) {
        let trace = algorithm.run(input.to_vec());
        let state = trace.state_at(trace.len());

        let mut want = input.to_vec();
        want.sort_unstable();

        assert_eq!(state.values, want, "{} mis-sorted {input:?}", trace.name);
        assert!(
            state.sorted.iter().all(|&s| s),
            "{} left slots unmarked on {input:?}",
            trace.name
        );
    }

    #[test]
    fn sorts_edge_cases() {
        let cases: &[&[u32]] = &[
            &[],
            &[1],
            &[2, 1],
            &[1, 2, 3, 4, 5],
            &[5, 4, 3, 2, 1],
            &[3, 3, 3, 3],
            &[2, 1, 2, 1, 2],
            &[u32::MAX, 0, u32::MAX],
        ];

        for algorithm in Algorithm::ALL {
            for case in cases {
                check(algorithm, case);
            }
        }
    }

    #[test]
    fn bubble_exits_early_when_already_sorted() {
        // One clean sweep of n-1 comparisons, and no swaps.
        let trace = bubble(vec![1, 2, 3, 4, 5]);
        let state = trace.state_at(trace.len());
        assert_eq!(state.comparisons, 4);
        assert_eq!(state.writes, 0);
    }

    #[test]
    fn selection_comparison_count_is_input_independent() {
        // n(n-1)/2 = 10 for n = 5, whatever the order.
        for input in [vec![1, 2, 3, 4, 5], vec![5, 4, 3, 2, 1], vec![3, 1, 4, 1, 5]] {
            let trace = selection(input);
            assert_eq!(trace.state_at(trace.len()).comparisons, 10);
        }
    }

    #[test]
    fn algorithm_cycles_both_ways() {
        for a in Algorithm::ALL {
            assert_eq!(a.next().prev(), a);
            assert_eq!(a.prev().next(), a);
        }
    }
}
