//! Sorting algorithms, written against [`Tracer`] rather than a bare slice.
//!
//! Each one is a textbook implementation with its reads and writes routed
//! through the recorder. None of them knows that a UI exists, and none of them
//! can pause: they run to completion and hand back a [`Trace`].
//!
//! The three here are deliberately contrasting in the shape of work they do.
//! Bubble sort is almost all swaps, selection sort is almost all comparisons
//! with exactly one swap per pass, and insertion sort shifts with
//! [`Step::Write`] rather than exchanging — which is what exercises the
//! reversibility of writes.
//!
//! [`Step::Write`]: crate::algo::Step::Write

use std::cmp::Ordering;

use super::step::{Complexity, Trace, Tracer};

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
    pub fn run(self, values: Vec<u32>) -> Trace {
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
    stable: true,
};

pub const INSERTION: Complexity = Complexity {
    best: "O(n)",
    average: "O(n²)",
    worst: "O(n²)",
    space: "O(1)",
    stable: true,
};

pub const SELECTION: Complexity = Complexity {
    best: "O(n²)",
    average: "O(n²)",
    worst: "O(n²)",
    space: "O(1)",
    stable: false,
};

/// Bubble sort: repeatedly sweep, exchanging adjacent slots that are out of
/// order.
///
/// Each sweep settles one more element at the top, so the swept range shrinks.
/// The `swapped` flag is what gives bubble sort its O(n) best case: a clean
/// sweep means the whole array is ordered and the remaining passes would do
/// nothing.
pub fn bubble(values: Vec<u32>) -> Trace {
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
/// instead of a swap's two, and it is the reason [`super::Step::Write`] has to
/// carry the overwritten value.
pub fn insertion(values: Vec<u32>) -> Trace {
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
pub fn selection(values: Vec<u32>) -> Trace {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Replaying a whole trace has to land on the sorted array. This is the
    /// cheap version of the property test in `tests/properties.rs`, kept here
    /// for the edge cases that generators rarely produce.
    fn check(algorithm: Algorithm, input: &[u32]) {
        let trace = algorithm.run(input.to_vec());
        let frame = trace.frame_at(trace.len());

        let mut want = input.to_vec();
        want.sort_unstable();

        assert_eq!(frame.values, want, "{} mis-sorted {input:?}", trace.name);
        assert!(
            frame.sorted.iter().all(|&s| s),
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
        let frame = trace.frame_at(trace.len());
        assert_eq!(frame.comparisons, 4);
        assert_eq!(frame.writes, 0);
    }

    #[test]
    fn selection_comparison_count_is_input_independent() {
        // n(n-1)/2 = 10 for n = 5, whatever the order.
        for input in [vec![1, 2, 3, 4, 5], vec![5, 4, 3, 2, 1], vec![3, 1, 4, 1, 5]] {
            let trace = selection(input);
            assert_eq!(trace.frame_at(trace.len()).comparisons, 10);
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
