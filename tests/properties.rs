//! Property tests for the event-log model.
//!
//! The unit tests next to each algorithm check that it produces the right
//! answer. These check the thing the whole UI is built on: that a trace can be
//! walked in either direction without drifting. If `round_trip` ever fails,
//! scrubbing backwards in the app is silently corrupting state.
//!
//! Both modes are covered through the same generic helpers, which is the
//! payoff for `Trace` and `Playback` being generic over the step type: the
//! invariant is stated once and applied to sorting and pathfinding alike.

use algoscope::algo::grid::search::Algorithm as Search;
use algoscope::algo::grid::{Grid, GridState};
use algoscope::algo::sort::{Algorithm as Sort, SortState};
use algoscope::algo::{Playback, Step, Trace};
use proptest::prelude::*;

/// Walk a trace fully forward, then fully back.
///
/// Returns the state it started from and the state it ended at, which must
/// agree in everything but the cosmetic highlight.
fn round_trip<S: Step>(trace: Trace<S>) -> (S::State, S::State) {
    let expected = trace.initial.clone();
    let mut playback = Playback::new(trace);

    while playback.step_forward() {}
    assert!(playback.at_end(), "forward walk did not reach the end");

    while playback.step_back() {}
    assert!(playback.at_start(), "backward walk did not reach the start");

    (expected, playback.state().clone())
}

/// Reach `target` three ways: replayed from scratch, stepped up to from the
/// start, and stepped down to from the end.
///
/// All three must agree. This is what catches an `apply`/`unapply` pair that is
/// subtly asymmetric, since `Playback` walks from wherever the cursor already
/// is rather than replaying.
fn three_ways<S: Step>(trace: &Trace<S>, target: usize) -> (S::State, S::State, S::State) {
    let replayed = trace.state_at(target);

    let mut up = Playback::new(trace.clone());
    up.seek(target);
    assert_eq!(up.cursor(), target);

    let mut down = Playback::new(trace.clone());
    down.fast_forward();
    down.seek(target);
    assert_eq!(down.cursor(), target);

    (replayed, up.state().clone(), down.state().clone())
}

/// Compare the parts of a sort state that must be exact.
///
/// `active` is excluded on purpose: it is the highlight, and stepping
/// backwards deliberately clears it rather than keeping a stack of previous
/// highlights around to restore. Everything else has to match to the bit.
fn assert_sort_eq(left: &SortState, right: &SortState, ctx: &str) {
    assert_eq!(left.values, right.values, "values differ {ctx}");
    assert_eq!(left.sorted, right.sorted, "sorted flags differ {ctx}");
    assert_eq!(
        left.comparisons, right.comparisons,
        "comparison count differs {ctx}"
    );
    assert_eq!(left.writes, right.writes, "write count differs {ctx}");
}

/// The same, for a search. `active` is excluded for the same reason.
fn assert_grid_eq(left: &GridState, right: &GridState, ctx: &str) {
    assert_eq!(left.grid, right.grid, "the map changed {ctx}");
    assert_eq!(left.status, right.status, "cell statuses differ {ctx}");
    assert_eq!(left.cost, right.cost, "known costs differ {ctx}");
    assert_eq!(left.from, right.from, "predecessors differ {ctx}");
    assert_eq!(left.examined, right.examined, "examined count differs {ctx}");
    assert_eq!(
        left.relaxations, right.relaxations,
        "relaxation count differs {ctx}"
    );
    assert_eq!(left.expanded, right.expanded, "expanded count differs {ctx}");
    assert_eq!(left.path_cells, right.path_cells, "path length differs {ctx}");
    assert_eq!(left.path_cost, right.path_cost, "path cost differs {ctx}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    /// Replaying a whole sort trace lands on the sorted array, with every slot
    /// marked settled.
    #[test]
    fn sort_replay_sorts(input in prop::collection::vec(0u32..1000, 0..64)) {
        let mut want = input.clone();
        want.sort_unstable();

        for algorithm in Sort::ALL {
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

    /// All the way forward then all the way back returns exactly to the start.
    ///
    /// A narrow value range is used so duplicates are common, since equal
    /// elements are where comparison-ordering mistakes hide.
    #[test]
    fn sort_round_trip(input in prop::collection::vec(0u32..8, 0..48)) {
        for algorithm in Sort::ALL {
            let trace = algorithm.run(input.clone());
            let name = trace.name;
            let (expected, actual) = round_trip(trace);
            assert_sort_eq(&actual, &expected, &format!("after rewinding {name}"));
        }
    }

    /// Incremental seeking agrees with replaying from scratch, from either
    /// direction.
    #[test]
    fn sort_seek_matches_replay(
        input in prop::collection::vec(0u32..100, 1..40),
        probe in 0.0f64..1.0,
    ) {
        for algorithm in Sort::ALL {
            let trace = algorithm.run(input.clone());
            let name = trace.name;
            let target = (probe * trace.len() as f64) as usize;

            let (replayed, up, down) = three_ways(&trace, target);
            assert_sort_eq(&up, &replayed, &format!("seeking up in {name}"));
            assert_sort_eq(&down, &replayed, &format!("seeking down in {name}"));
        }
    }

    /// The same round trip, for the four searches.
    ///
    /// Grid steps carry more to restore than sorting steps do -- a status, a
    /// cost and a predecessor each -- so this is the stricter of the two.
    #[test]
    fn grid_round_trip(
        width in 5u16..28,
        height in 5u16..16,
        seed in any::<u64>(),
    ) {
        let grid = Grid::generate(width, height, seed);
        for algorithm in Search::ALL {
            let trace = algorithm.run(grid.clone());
            let name = trace.name;
            let (expected, actual) = round_trip(trace);
            assert_grid_eq(&actual, &expected, &format!("after rewinding {name}"));
        }
    }

    #[test]
    fn grid_seek_matches_replay(
        width in 5u16..28,
        height in 5u16..16,
        seed in any::<u64>(),
        probe in 0.0f64..1.0,
    ) {
        let grid = Grid::generate(width, height, seed);
        for algorithm in Search::ALL {
            let trace = algorithm.run(grid.clone());
            let name = trace.name;
            let target = (probe * trace.len() as f64) as usize;

            let (replayed, up, down) = three_ways(&trace, target);
            assert_grid_eq(&up, &replayed, &format!("seeking up in {name}"));
            assert_grid_eq(&down, &replayed, &format!("seeking down in {name}"));
        }
    }

    /// Whatever the map, the two optimal searches agree on the cheapest cost
    /// and the other two never beat it.
    ///
    /// This is the claim the metrics panel makes, checked against arbitrary
    /// generated maps rather than a handful of fixed seeds.
    #[test]
    fn optimal_searches_agree_and_are_never_beaten(
        width in 6u16..30,
        height in 6u16..18,
        seed in any::<u64>(),
    ) {
        let grid = Grid::generate(width, height, seed);

        let cost_of = |algorithm: Search| {
            let trace = algorithm.run(grid.clone());
            trace.state_at(trace.len()).path_cost
        };

        let dijkstra = cost_of(Search::Dijkstra);
        prop_assert_eq!(dijkstra, cost_of(Search::AStar), "optimal searches disagreed");
        prop_assert!(cost_of(Search::Bfs) >= dijkstra, "breadth-first beat the optimum");
        prop_assert!(cost_of(Search::Dfs) >= dijkstra, "depth-first beat the optimum");
    }

    /// Breadth-first search finds the route with the fewest tiles, which is
    /// its own guarantee and not one Dijkstra shares.
    #[test]
    fn breadth_first_uses_the_fewest_tiles(
        width in 6u16..30,
        height in 6u16..18,
        seed in any::<u64>(),
    ) {
        let grid = Grid::generate(width, height, seed);

        let tiles_of = |algorithm: Search| {
            let trace = algorithm.run(grid.clone());
            trace.state_at(trace.len()).path_cells
        };

        let fewest = tiles_of(Search::Bfs);
        for algorithm in [Search::Dfs, Search::Dijkstra, Search::AStar] {
            prop_assert!(
                tiles_of(algorithm) >= fewest,
                "{} found a shorter route than breadth-first",
                algorithm.name()
            );
        }
    }
}
