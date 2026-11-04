//! Property tests for the event-log model.
//!
//! The unit tests next to each algorithm check that it sorts. These check the
//! thing the whole UI is built on: that a trace can be walked in either
//! direction without drifting. If `round_trip` ever fails, scrubbing backwards
//! in the app is silently corrupting the array.

use algoscope::algo::Playback;
use algoscope::algo::sort::{Algorithm, SortState};
use proptest::prelude::*;

/// Compare the parts of a frame that must be exact.
///
/// `active` is excluded on purpose: it is the highlight, and stepping backwards
/// deliberately clears it rather than keeping a stack of previous highlights
/// around to restore. Everything else has to match to the bit.
fn assert_state_eq(left: &SortState, right: &SortState, ctx: &str) {
    assert_eq!(left.values, right.values, "values differ {ctx}");
    assert_eq!(left.sorted, right.sorted, "sorted flags differ {ctx}");
    assert_eq!(
        left.comparisons, right.comparisons,
        "comparison count differs {ctx}"
    );
    assert_eq!(left.writes, right.writes, "write count differs {ctx}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Replaying a whole trace lands on the sorted array, with every slot
    /// marked settled.
    #[test]
    fn replay_sorts(input in prop::collection::vec(0u32..1000, 0..64)) {
        let mut want = input.clone();
        want.sort_unstable();

        for algorithm in Algorithm::ALL {
            let trace = algorithm.run(input.clone());
            let state = trace.state_at(trace.len());

            prop_assert_eq!(&state.values, &want, "{} mis-sorted", trace.name);
            prop_assert!(state.is_sorted(), "{} left disorder", trace.name);
            prop_assert!(
                state.sorted.iter().all(|&s| s),
                "{} left slots unmarked",
                trace.name
            );
        }
    }

    /// Forward through every step, then backward through every step, returns
    /// exactly to the starting state.
    ///
    /// This is the test that keeps `Step` honest about carrying its own
    /// inverse. A narrow value range is used so duplicates are common, since
    /// equal elements are where comparison-ordering mistakes hide.
    #[test]
    fn round_trip(input in prop::collection::vec(0u32..8, 0..48)) {
        for algorithm in Algorithm::ALL {
            let trace = algorithm.run(input.clone());
            let start = trace.initial.clone();
            let name = trace.name;
            let mut playback = Playback::new(trace);

            while playback.step_forward() {}
            prop_assert!(playback.at_end(), "{name} did not reach the end");

            while playback.step_back() {}
            prop_assert!(playback.at_start(), "{name} did not reach the start");

            assert_state_eq(playback.state(), &start, &format!("after rewinding {name}"));
        }
    }

    /// Incremental seeking agrees with replaying from scratch, arrived at from
    /// either direction.
    ///
    /// `Playback` walks from wherever the cursor already is, so this is what
    /// catches an `apply`/`unapply` pair that is subtly asymmetric: reaching
    /// the same cursor by stepping up to it and by stepping down to it must
    /// produce the same state.
    #[test]
    fn seek_matches_replay(
        input in prop::collection::vec(0u32..100, 1..40),
        probe in 0.0f64..1.0,
    ) {
        for algorithm in Algorithm::ALL {
            let trace = algorithm.run(input.clone());
            let name = trace.name;
            let target = (probe * trace.len() as f64) as usize;
            let expected = trace.state_at(target);

            let mut forward = Playback::new(trace.clone());
            forward.seek(target);
            prop_assert_eq!(forward.cursor(), target);
            assert_state_eq(forward.state(), &expected, &format!("seeking up in {name}"));

            let mut backward = Playback::new(trace);
            backward.fast_forward();
            backward.seek(target);
            prop_assert_eq!(backward.cursor(), target);
            assert_state_eq(backward.state(), &expected, &format!("seeking down in {name}"));
        }
    }
}
