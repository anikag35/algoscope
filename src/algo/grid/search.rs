//! The four searches.
//!
//! All of them are the same loop — take a cell off the frontier, relax its
//! neighbours, put it back — and differ only in what "the frontier" means. A
//! queue gives breadth-first, a stack gives depth-first, a priority queue on
//! accumulated cost gives Dijkstra, and the same queue on cost plus a
//! heuristic gives A\*. Seeing that the four are one algorithm with four
//! orderings is most of the point of putting them side by side.
//!
//! Wall neighbours are skipped without being recorded. Counting them would
//! inflate `examined` with work that is rejected on sight, and more to the
//! point it would add steps to the trace that change nothing on screen, so
//! stepping through would show stalls where nothing happened.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use super::{Grid, GridStep, Status, Tracer};
use crate::algo::trace::{Complexity, Trace};

/// Which search to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Algorithm {
    #[default]
    Bfs,
    Dfs,
    Dijkstra,
    AStar,
}

impl Algorithm {
    pub const ALL: [Self; 4] = [Self::Bfs, Self::Dfs, Self::Dijkstra, Self::AStar];

    pub fn run(self, grid: Grid) -> Trace<GridStep> {
        match self {
            Self::Bfs => bfs(grid),
            Self::Dfs => dfs(grid),
            Self::Dijkstra => dijkstra(grid),
            Self::AStar => astar(grid),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Bfs => "Breadth-first",
            Self::Dfs => "Depth-first",
            Self::Dijkstra => "Dijkstra",
            Self::AStar => "A* search",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&a| a == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|&a| a == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

pub const BFS: Complexity = Complexity {
    best: "O(1)",
    average: "O(V)",
    worst: "O(V)",
    space: "O(V)",
    note: "fewest tiles",
};

pub const DFS: Complexity = Complexity {
    best: "O(1)",
    average: "O(V)",
    worst: "O(V)",
    space: "O(V)",
    note: "no guarantee at all",
};

pub const DIJKSTRA: Complexity = Complexity {
    best: "O(1)",
    average: "O(V log V)",
    worst: "O(V log V)",
    space: "O(V)",
    note: "cheapest route, blind",
};

pub const ASTAR: Complexity = Complexity {
    best: "O(d log d)",
    average: "O(V log V)",
    worst: "O(V log V)",
    space: "O(V)",
    note: "cheapest, goal-directed",
};

/// Breadth-first search: expand the frontier in order of hop count.
///
/// The queue is the whole algorithm. Because every cell is reached in the
/// fewest possible steps, the route it finds has the fewest tiles — but it
/// pays no attention to terrain, so on a weighted grid that route is usually
/// *not* the cheapest. Comparing its route cost against Dijkstra's on the same
/// map is the clearest demonstration of what the priority queue is buying.
pub fn bfs(grid: Grid) -> Trace<GridStep> {
    let mut t = Tracer::new(grid);
    let (start, goal) = (t.start(), t.goal());
    let mut queue = VecDeque::new();

    t.relax(start, 0, start);
    queue.push_back(start);

    while let Some(cell) = queue.pop_front() {
        t.expand(cell);
        if cell == goal {
            t.settle(cell);
            break;
        }

        let hops = t.cost_of(cell).unwrap_or(0);
        for neighbour in t.grid().neighbours(cell).into_iter().flatten() {
            if !t.grid().passable(neighbour) {
                continue;
            }
            t.examine(neighbour);
            if t.cost_of(neighbour).is_none() {
                t.relax(neighbour, hops + 1, cell);
                queue.push_back(neighbour);
            }
        }

        t.settle(cell);
    }

    t.mark_path();
    t.finish(Algorithm::Bfs.name(), BFS)
}

/// Depth-first search: follow one corridor to its end before trying another.
///
/// The only change from breadth-first is `pop` instead of `pop_front`, and the
/// result is unrecognisable: it commits to a direction and wanders, often
/// crossing most of the map to reach a goal a few tiles away. It offers no
/// optimality guarantee of any kind, which is exactly why it is worth watching
/// next to the other three.
pub fn dfs(grid: Grid) -> Trace<GridStep> {
    let mut t = Tracer::new(grid);
    let (start, goal) = (t.start(), t.goal());
    let mut stack = vec![start];

    t.relax(start, 0, start);

    while let Some(cell) = stack.pop() {
        // A cell is only ever pushed once, when first seen, so this is
        // defensive rather than load-bearing.
        if matches!(t.status_of(cell), Status::Settled | Status::Expanding) {
            continue;
        }

        t.expand(cell);
        if cell == goal {
            t.settle(cell);
            break;
        }

        let depth = t.cost_of(cell).unwrap_or(0);
        for neighbour in t.grid().neighbours(cell).into_iter().flatten() {
            if !t.grid().passable(neighbour) {
                continue;
            }
            t.examine(neighbour);
            if t.cost_of(neighbour).is_none() {
                t.relax(neighbour, depth + 1, cell);
                stack.push(neighbour);
            }
        }

        t.settle(cell);
    }

    t.mark_path();
    t.finish(Algorithm::Dfs.name(), DFS)
}

/// Dijkstra's algorithm: expand the frontier in order of accumulated cost.
///
/// `cost` now holds real terrain cost rather than hop count, and the frontier
/// is a min-heap on it. That is enough to guarantee the cheapest route. The
/// price is that it still searches blindly in every direction — the frontier
/// grows as a rough circle around the start, because nothing tells it which
/// way the goal lies.
pub fn dijkstra(grid: Grid) -> Trace<GridStep> {
    let mut t = Tracer::new(grid);
    let (start, goal) = (t.start(), t.goal());
    let mut heap = BinaryHeap::new();

    t.relax(start, 0, start);
    heap.push((Reverse(0u32), start));

    while let Some((Reverse(cost), cell)) = heap.pop() {
        // A cheaper route to this cell may have been found after this entry
        // was queued, leaving it stale. Skipping stale entries is cheaper than
        // supporting decrease-key.
        if t.status_of(cell) == Status::Settled {
            continue;
        }
        if t.cost_of(cell).is_some_and(|best| cost > best) {
            continue;
        }

        t.expand(cell);
        if cell == goal {
            t.settle(cell);
            break;
        }

        for neighbour in t.grid().neighbours(cell).into_iter().flatten() {
            let Some(step_cost) = t.grid().cost(neighbour) else {
                continue;
            };
            t.examine(neighbour);
            let candidate = cost + step_cost;

            // A settled cell's cost is already optimal, so it fails this test
            // too and needs no separate check.
            if t.cost_of(neighbour).is_none_or(|best| candidate < best) {
                t.relax(neighbour, candidate, cell);
                heap.push((Reverse(candidate), neighbour));
            }
        }

        t.settle(cell);
    }

    t.mark_path();
    t.finish(Algorithm::Dijkstra.name(), DIJKSTRA)
}

/// A\*: Dijkstra, ordered by cost-so-far plus an estimate of what remains.
///
/// The only difference from [`dijkstra`] is the heap key — `g + h` instead of
/// `g`, where `h` is Manhattan distance to the goal. Because that estimate
/// never overstates the true remaining cost, the route found is still the
/// cheapest; but the frontier now stretches toward the goal instead of
/// spreading evenly, and it typically expands a fraction of the cells. The
/// `expanded` counter next to Dijkstra's on the same map is the whole argument
/// for heuristic search in one number.
pub fn astar(grid: Grid) -> Trace<GridStep> {
    let mut t = Tracer::new(grid);
    let (start, goal) = (t.start(), t.goal());
    let mut heap = BinaryHeap::new();

    t.relax(start, 0, start);
    heap.push((Reverse(t.grid().manhattan(start, goal)), start));

    while let Some((Reverse(_), cell)) = heap.pop() {
        if t.status_of(cell) == Status::Settled {
            continue;
        }

        t.expand(cell);
        if cell == goal {
            t.settle(cell);
            break;
        }

        let g = t.cost_of(cell).unwrap_or(0);
        for neighbour in t.grid().neighbours(cell).into_iter().flatten() {
            let Some(step_cost) = t.grid().cost(neighbour) else {
                continue;
            };
            t.examine(neighbour);
            let candidate = g + step_cost;

            if t.cost_of(neighbour).is_none_or(|best| candidate < best) {
                t.relax(neighbour, candidate, cell);
                let priority = candidate + t.grid().manhattan(neighbour, goal);
                heap.push((Reverse(priority), neighbour));
            }
        }

        t.settle(cell);
    }

    t.mark_path();
    t.finish(Algorithm::AStar.name(), ASTAR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::grid::{Cell, Terrain};

    /// Walk the marked route and check it is a real, legal path.
    fn check_route(algorithm: Algorithm, grid: Grid) -> (u32, u64) {
        let start = grid.start;
        let goal = grid.goal;
        let trace = algorithm.run(grid);
        let state = trace.state_at(trace.len());

        assert!(
            state.has_path(),
            "{} found no route on a connected grid",
            trace.name
        );

        // Rebuild the route from the predecessor chain and verify every hop.
        let mut route = vec![goal];
        let mut cell = goal;
        while cell != start {
            cell = state.from[cell as usize].expect("route must be connected");
            route.push(cell);
        }

        for pair in route.windows(2) {
            assert_eq!(
                state.grid.manhattan(pair[0], pair[1]),
                1,
                "{} produced a non-adjacent hop",
                trace.name
            );
            assert!(
                state.grid.passable(pair[0]) && state.grid.passable(pair[1]),
                "{} routed through a wall",
                trace.name
            );
        }

        assert_eq!(
            state.path_cells as usize,
            route.len(),
            "{} marked a different number of cells than the route has",
            trace.name
        );

        (state.path_cost, state.expanded)
    }

    #[test]
    fn all_four_find_a_legal_route() {
        for seed in 0..12 {
            for algorithm in Algorithm::ALL {
                check_route(algorithm, Grid::generate(30, 16, seed));
            }
        }
    }

    #[test]
    fn dijkstra_and_astar_agree_on_the_cheapest_cost() {
        // Both are optimal, so on the same map they must find routes of equal
        // cost -- though not necessarily the same route, since ties break
        // differently.
        for seed in 0..25 {
            let grid = Grid::generate(34, 18, seed);
            let (dijkstra_cost, _) = check_route(Algorithm::Dijkstra, grid.clone());
            let (astar_cost, _) = check_route(Algorithm::AStar, grid);
            assert_eq!(
                dijkstra_cost, astar_cost,
                "seed {seed}: two optimal searches disagreed"
            );
        }
    }

    #[test]
    fn neither_optimal_search_is_beaten_by_the_others() {
        for seed in 0..25 {
            let grid = Grid::generate(34, 18, seed);
            let (optimal, _) = check_route(Algorithm::Dijkstra, grid.clone());
            for algorithm in [Algorithm::Bfs, Algorithm::Dfs] {
                let (cost, _) = check_route(algorithm, grid.clone());
                assert!(
                    cost >= optimal,
                    "seed {seed}: {} beat the optimal cost",
                    algorithm.name()
                );
            }
        }
    }

    #[test]
    fn astar_expands_no_more_than_dijkstra_on_average() {
        // Not a per-seed guarantee: an unhelpful heuristic can make A* expand
        // as many cells as Dijkstra, and tie-breaking can push it one or two
        // over. Across a spread of maps the saving should be clear, and this
        // is the claim the metrics panel makes.
        let mut dijkstra_total = 0;
        let mut astar_total = 0;
        for seed in 0..25 {
            let grid = Grid::generate(40, 20, seed);
            dijkstra_total += check_route(Algorithm::Dijkstra, grid.clone()).1;
            astar_total += check_route(Algorithm::AStar, grid).1;
        }
        assert!(
            astar_total < dijkstra_total,
            "A* expanded {astar_total} cells against Dijkstra's {dijkstra_total}"
        );
    }

    #[test]
    fn breadth_first_finds_the_fewest_tiles() {
        // Its own guarantee, which Dijkstra does not share.
        for seed in 0..20 {
            let grid = Grid::generate(30, 16, seed);
            let bfs_tiles = {
                let trace = Algorithm::Bfs.run(grid.clone());
                trace.state_at(trace.len()).path_cells
            };
            for algorithm in [Algorithm::Dijkstra, Algorithm::AStar, Algorithm::Dfs] {
                let trace = algorithm.run(grid.clone());
                let tiles = trace.state_at(trace.len()).path_cells;
                assert!(
                    tiles >= bfs_tiles,
                    "seed {seed}: {} used fewer tiles than breadth-first",
                    algorithm.name()
                );
            }
        }
    }

    #[test]
    fn an_unreachable_goal_is_reported_rather_than_faked() {
        // Wall the goal off completely. Exhausting the reachable area and
        // finding nothing is a legitimate result, and the trace must say so
        // instead of inventing a route.
        let mut grid = Grid::open(12, 9);
        let goal = grid.goal;
        for neighbour in grid.neighbours(goal).into_iter().flatten() {
            grid.set(neighbour, Terrain::Wall);
        }

        for algorithm in Algorithm::ALL {
            let trace = algorithm.run(grid.clone());
            let state = trace.state_at(trace.len());
            assert!(
                !state.has_path(),
                "{} claimed a route to a walled-off goal",
                trace.name
            );
            assert_eq!(state.goal_cost(), None);
            assert!(state.expanded > 0, "{} did not search at all", trace.name);
        }
    }

    #[test]
    fn a_goal_on_the_start_terminates_immediately() {
        let mut grid = Grid::open(10, 8);
        grid.goal = grid.start;

        for algorithm in Algorithm::ALL {
            let trace = algorithm.run(grid.clone());
            let state = trace.state_at(trace.len());
            assert_eq!(state.path_cells, 1, "{}", trace.name);
            assert_eq!(state.path_cost, 0, "{} charged for standing still", trace.name);
        }
    }

    #[test]
    fn rough_ground_diverts_the_optimal_searches() {
        // A corridor where the direct line is rough and a detour is open.
        // Breadth-first takes the short expensive line; Dijkstra pays more
        // tiles to avoid it. This is the behaviour the weighted terrain exists
        // to show, so it is worth pinning down explicitly.
        let mut grid = Grid::open(9, 5);
        grid.start = grid.cell(0, 2);
        grid.goal = grid.cell(8, 2);
        for x in 1..8 {
            grid.set(grid.cell(x, 2), Terrain::Rough);
        }

        let bfs_trace = Algorithm::Bfs.run(grid.clone());
        let bfs_state = bfs_trace.state_at(bfs_trace.len());
        let dijkstra_trace = Algorithm::Dijkstra.run(grid.clone());
        let dijkstra_state = dijkstra_trace.state_at(dijkstra_trace.len());

        assert!(
            dijkstra_state.path_cost < bfs_state.path_cost,
            "Dijkstra {} should undercut breadth-first {}",
            dijkstra_state.path_cost,
            bfs_state.path_cost
        );
        assert!(
            dijkstra_state.path_cells > bfs_state.path_cells,
            "the cheaper route should be the longer one here"
        );
    }

    #[test]
    fn examined_counts_only_passable_neighbours() {
        // Walls are skipped silently, so a grid that is almost all wall should
        // produce very little recorded work.
        let mut grid = Grid::open(5, 5);
        for cell in 0..grid.len() as Cell {
            if cell != grid.start && cell != grid.goal {
                grid.set(cell, Terrain::Wall);
            }
        }
        let trace = Algorithm::Bfs.run(grid);
        let state = trace.state_at(trace.len());
        assert_eq!(state.examined, 0, "wall checks should not be recorded");
    }
}
