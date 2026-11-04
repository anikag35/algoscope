//! The pathfinding grid: terrain, generation, and the search event vocabulary.
//!
//! A weighted grid rather than an arbitrary node-link graph. That choice is
//! deliberate: it covers the same algorithmic ground — breadth-first, depth-
//! first, Dijkstra and A\* are all just different orderings of the same
//! frontier — with no node-layout code at all, and it reads far better in a
//! terminal than edges drawn between scattered vertices.
//!
//! Terrain weights are what make the four algorithms diverge visibly. On a
//! grid of uniform cost Dijkstra degenerates into breadth-first search and the
//! comparison is pointless; with rough ground costing three times open ground,
//! breadth-first finds the route with the fewest tiles while Dijkstra finds the
//! genuinely cheapest one, and they are not the same route.


pub mod search;

use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::{RngExt, SeedableRng};

use super::trace::{Step, Trace};

/// An index into [`Grid`], as `y * width + x`.
///
/// Deliberately a plain `u32` rather than a struct: a trace holds tens of
/// thousands of steps and every one of them carries at least one.
pub type Cell = u32;

/// What occupies a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Terrain {
    #[default]
    Open,
    /// Passable, but three times the cost. The reason Dijkstra and
    /// breadth-first search disagree.
    Rough,
    Wall,
}

impl Terrain {
    /// Cost of entering a cell of this terrain, or `None` if impassable.
    pub fn cost(self) -> Option<u32> {
        match self {
            Self::Open => Some(1),
            Self::Rough => Some(3),
            Self::Wall => None,
        }
    }

    pub fn passable(self) -> bool {
        self.cost().is_some()
    }

    /// The next terrain, for cycling a cell while editing.
    pub fn next(self) -> Self {
        match self {
            Self::Open => Self::Rough,
            Self::Rough => Self::Wall,
            Self::Wall => Self::Open,
        }
    }
}

/// How far a search has got with a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// Never reached.
    #[default]
    Unseen,
    /// Reached, with a tentative cost, and waiting on the frontier.
    Frontier,
    /// Taken off the frontier and currently being expanded.
    Expanding,
    /// Finished with: for Dijkstra and A\* its cost is now final.
    Settled,
    /// On the reconstructed route.
    Path,
}

/// The map: fixed terrain, a start and a goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    width: u16,
    height: u16,
    terrain: Vec<Terrain>,
    pub start: Cell,
    pub goal: Cell,
}

impl Grid {
    /// An empty open grid with start and goal at the left and right edges.
    pub fn open(width: u16, height: u16) -> Self {
        let width = width.max(3);
        let height = height.max(3);
        let mid = height / 2;
        let mut grid = Self {
            width,
            height,
            terrain: vec![Terrain::Open; usize::from(width) * usize::from(height)],
            start: 0,
            goal: 0,
        };
        grid.start = grid.cell(1, mid);
        grid.goal = grid.cell(width - 2, mid);
        grid
    }

    /// A map of scattered obstacles and rough patches, deterministic in `seed`.
    ///
    /// Blobs grown by short random walks rather than independent random cells:
    /// uniform noise produces a grey mush that every algorithm crosses in
    /// much the same way, whereas contiguous obstacles create the detours and
    /// dead ends that make the orderings visibly differ.
    pub fn generate(width: u16, height: u16, seed: u64) -> Self {
        let mut grid = Self::open(width, height);
        let mut rng = StdRng::seed_from_u64(seed);
        let area = grid.len();

        grid.scatter_blobs(&mut rng, Terrain::Wall, area / 26, 14);
        grid.scatter_blobs(&mut rng, Terrain::Rough, area / 30, 18);

        // Keep the endpoints and their surroundings clear, so a run never
        // begins boxed in by an accident of generation.
        for cell in [grid.start, grid.goal] {
            grid.clear_around(cell);
        }

        grid.ensure_connected();
        grid
    }

    /// Grow `count` patches of `terrain`, each up to `size` cells.
    fn scatter_blobs(&mut self, rng: &mut StdRng, terrain: Terrain, count: usize, size: usize) {
        for _ in 0..count {
            let mut cell = self.random_cell(rng);
            for _ in 0..size {
                self.set(cell, terrain);
                let options = self.neighbours(cell);
                let open: Vec<Cell> = options.into_iter().flatten().collect();
                match open.choose(rng) {
                    Some(&next) => cell = next,
                    None => break,
                }
            }
        }
    }

    fn random_cell(&self, rng: &mut StdRng) -> Cell {
        rng.random_range(0..self.len() as Cell)
    }

    /// Open a cell and its four neighbours.
    fn clear_around(&mut self, cell: Cell) {
        self.set(cell, Terrain::Open);
        for neighbour in self.neighbours(cell).into_iter().flatten() {
            self.set(neighbour, Terrain::Open);
        }
    }

    /// Guarantee the goal is reachable from the start.
    ///
    /// Generation is random, so some seeds wall the goal off entirely. An
    /// unreachable goal is a legitimate thing for a search to discover, and the
    /// visualizer handles it, but it makes a poor default: the frontier floods
    /// the whole reachable area and stops. So the connectivity is checked and,
    /// if it is missing, a corridor is carved along the L-shaped route between
    /// the two points. Only walls actually in the way are removed.
    fn ensure_connected(&mut self) {
        if self.reachable(self.start, self.goal) {
            return;
        }

        let (sx, sy) = self.xy(self.start);
        let (gx, gy) = self.xy(self.goal);

        for x in sx.min(gx)..=sx.max(gx) {
            let cell = self.cell(x, sy);
            if !self.terrain(cell).passable() {
                self.set(cell, Terrain::Open);
            }
        }
        for y in sy.min(gy)..=sy.max(gy) {
            let cell = self.cell(gx, y);
            if !self.terrain(cell).passable() {
                self.set(cell, Terrain::Open);
            }
        }
    }

    /// Plain flood fill, used only by [`Grid::ensure_connected`]. Records
    /// nothing; the traced searches are in [`search`].
    fn reachable(&self, from: Cell, to: Cell) -> bool {
        let mut seen = vec![false; self.len()];
        let mut stack = vec![from];
        seen[from as usize] = true;

        while let Some(cell) = stack.pop() {
            if cell == to {
                return true;
            }
            for neighbour in self.neighbours(cell).into_iter().flatten() {
                if !seen[neighbour as usize] && self.terrain(neighbour).passable() {
                    seen[neighbour as usize] = true;
                    stack.push(neighbour);
                }
            }
        }
        false
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn len(&self) -> usize {
        self.terrain.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terrain.is_empty()
    }

    pub fn terrain(&self, cell: Cell) -> Terrain {
        self.terrain[cell as usize]
    }

    pub fn set(&mut self, cell: Cell, terrain: Terrain) {
        self.terrain[cell as usize] = terrain;
    }

    pub fn passable(&self, cell: Cell) -> bool {
        self.terrain(cell).passable()
    }

    /// Cost of entering `cell`, or `None` if it is a wall.
    pub fn cost(&self, cell: Cell) -> Option<u32> {
        self.terrain(cell).cost()
    }

    pub fn cell(&self, x: u16, y: u16) -> Cell {
        Cell::from(y) * Cell::from(self.width) + Cell::from(x)
    }

    pub fn xy(&self, cell: Cell) -> (u16, u16) {
        let width = Cell::from(self.width);
        ((cell % width) as u16, (cell / width) as u16)
    }

    /// The four orthogonal neighbours, `None` where the grid ends.
    ///
    /// Four-way rather than eight-way movement, which keeps Manhattan distance
    /// an admissible heuristic for A\* without having to reason about diagonal
    /// costs.
    pub fn neighbours(&self, cell: Cell) -> [Option<Cell>; 4] {
        let (x, y) = self.xy(cell);
        [
            (y > 0).then(|| self.cell(x, y - 1)),
            (x + 1 < self.width).then(|| self.cell(x + 1, y)),
            (y + 1 < self.height).then(|| self.cell(x, y + 1)),
            (x > 0).then(|| self.cell(x - 1, y)),
        ]
    }

    /// Manhattan distance, the A\* heuristic.
    ///
    /// Admissible because movement is four-way and the cheapest terrain costs
    /// one, so no route can beat the straight-line tile count.
    pub fn manhattan(&self, a: Cell, b: Cell) -> u32 {
        let (ax, ay) = self.xy(a);
        let (bx, by) = self.xy(b);
        u32::from(ax.abs_diff(bx)) + u32::from(ay.abs_diff(by))
    }
}

/// A single reversible event in a graph search.
///
/// Every variant that changes a cell's status carries the status it replaced.
/// That is what makes the log walkable backwards, exactly as [`SortStep`]
/// records the value a write overwrote.
///
/// [`SortStep`]: crate::algo::sort::SortStep
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridStep {
    /// A neighbour was looked at but not improved: already settled, or reached
    /// no more cheaply than before.
    Examine { cell: Cell },
    /// A neighbour was reached for the first time, or more cheaply than before.
    Relax {
        cell: Cell,
        cost: u32,
        from: Cell,
        old_status: Status,
        old_cost: Option<u32>,
        old_from: Option<Cell>,
    },
    /// A cell came off the frontier and is being expanded.
    Expand { cell: Cell, old_status: Status },
    /// A cell is finished with.
    Settle { cell: Cell, old_status: Status },
    /// A cell is on the reconstructed route.
    ///
    /// `cost` is the terrain cost of entering it, accumulated into
    /// [`GridState::path_cost`]. The algorithms disagree about what their own
    /// `cost` array means -- breadth-first and depth-first store hop counts,
    /// Dijkstra and A* store terrain cost -- so the route's true price is
    /// totalled here instead, where it is comparable across all four.
    MarkPath {
        cell: Cell,
        cost: u32,
        old_status: Status,
    },
}

/// The visible state of a search at one cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridState {
    /// The map. Fixed for the whole run, so editing terrain starts a new one.
    pub grid: Grid,
    pub status: Vec<Status>,
    /// Best known cost to each cell.
    pub cost: Vec<Option<u32>>,
    /// Predecessor on the best known route to each cell.
    pub from: Vec<Option<Cell>>,
    /// The cell currently being expanded, for the highlight. Cleared when
    /// stepping backwards, as the sorting highlight is.
    pub active: Option<Cell>,
    /// Passable neighbours inspected. The headline measure of work: this is
    /// where A\* beats Dijkstra.
    ///
    /// Counts inspections only, so seeding the start does not register and
    /// wall neighbours -- rejected on sight -- are not recorded at all.
    pub examined: u64,
    /// Inspections that improved a cell's known cost.
    pub relaxations: u64,
    /// Cells taken off the frontier.
    pub expanded: u64,
    /// Cells on the route, once one has been found.
    pub path_cells: u32,
    /// Total terrain cost of the route, comparable across all four algorithms.
    pub path_cost: u32,
}

impl GridState {
    pub fn new(grid: Grid) -> Self {
        let n = grid.len();
        Self {
            grid,
            status: vec![Status::Unseen; n],
            cost: vec![None; n],
            from: vec![None; n],
            active: None,
            examined: 0,
            relaxations: 0,
            expanded: 0,
            path_cells: 0,
            path_cost: 0,
        }
    }

    pub fn status(&self, cell: Cell) -> Status {
        self.status[cell as usize]
    }

    pub fn cost(&self, cell: Cell) -> Option<u32> {
        self.cost[cell as usize]
    }

    /// Total cost of the route to the goal, once it has been reached.
    pub fn goal_cost(&self) -> Option<u32> {
        self.cost(self.grid.goal)
    }

    /// Whether the route has been drawn.
    pub fn has_path(&self) -> bool {
        self.path_cells > 0
    }
}

impl Step for GridStep {
    type State = GridState;

    fn apply(self, state: &mut GridState) {
        match self {
            Self::Examine { cell } => {
                state.examined += 1;
                let _ = cell;
            }
            Self::Relax {
                cell, cost, from, ..
            } => {
                state.relaxations += 1;
                state.status[cell as usize] = Status::Frontier;
                state.cost[cell as usize] = Some(cost);
                state.from[cell as usize] = Some(from);
            }
            Self::Expand { cell, .. } => {
                state.expanded += 1;
                state.status[cell as usize] = Status::Expanding;
                state.active = Some(cell);
            }
            Self::Settle { cell, .. } => {
                state.status[cell as usize] = Status::Settled;
            }
            Self::MarkPath { cell, cost, .. } => {
                state.status[cell as usize] = Status::Path;
                state.path_cells += 1;
                state.path_cost += cost;
            }
        }
    }

    fn unapply(self, state: &mut GridState) {
        state.active = None;
        match self {
            Self::Examine { .. } => state.examined -= 1,
            Self::Relax {
                cell,
                old_status,
                old_cost,
                old_from,
                ..
            } => {
                state.relaxations -= 1;
                state.status[cell as usize] = old_status;
                state.cost[cell as usize] = old_cost;
                state.from[cell as usize] = old_from;
            }
            Self::Expand { cell, old_status } => {
                state.expanded -= 1;
                state.status[cell as usize] = old_status;
            }
            Self::Settle { cell, old_status } => {
                state.status[cell as usize] = old_status;
            }
            Self::MarkPath {
                cell,
                cost,
                old_status,
            } => {
                state.status[cell as usize] = old_status;
                state.path_cells -= 1;
                state.path_cost -= cost;
            }
        }
    }
}

/// Recorder the search algorithms write through.
pub struct Tracer {
    initial: Grid,
    state: GridState,
    steps: Vec<GridStep>,
}

impl Tracer {
    pub fn new(grid: Grid) -> Self {
        Self {
            initial: grid.clone(),
            state: GridState::new(grid),
            steps: Vec::new(),
        }
    }

    pub fn grid(&self) -> &Grid {
        &self.state.grid
    }

    pub fn start(&self) -> Cell {
        self.state.grid.start
    }

    pub fn goal(&self) -> Cell {
        self.state.grid.goal
    }

    pub fn cost_of(&self, cell: Cell) -> Option<u32> {
        self.state.cost(cell)
    }

    pub fn status_of(&self, cell: Cell) -> Status {
        self.state.status(cell)
    }

    fn push(&mut self, step: GridStep) {
        step.apply(&mut self.state);
        self.steps.push(step);
    }

    /// Record inspecting a passable neighbour.
    ///
    /// Called for every passable neighbour considered, whether or not it then
    /// improves, so `examined` is exactly the number of inspections.
    pub fn examine(&mut self, cell: Cell) {
        self.push(GridStep::Examine { cell });
    }

    /// Record reaching `cell` from `from` at `cost`.
    ///
    /// Purely the state change; the inspection that led here is recorded
    /// separately by [`Tracer::examine`]. Seeding the start therefore does not
    /// inflate the inspection count.
    pub fn relax(&mut self, cell: Cell, cost: u32, from: Cell) {
        self.push(GridStep::Relax {
            cell,
            cost,
            from,
            old_status: self.state.status(cell),
            old_cost: self.state.cost(cell),
            old_from: self.state.from[cell as usize],
        });
    }

    pub fn expand(&mut self, cell: Cell) {
        self.push(GridStep::Expand {
            cell,
            old_status: self.state.status(cell),
        });
    }

    pub fn settle(&mut self, cell: Cell) {
        self.push(GridStep::Settle {
            cell,
            old_status: self.state.status(cell),
        });
    }

    /// Walk the predecessor chain back from the goal, marking the route.
    ///
    /// Marked from the goal backwards so that scrubbing through the tail of the
    /// trace draws the path in the direction the search proved it.
    pub fn mark_path(&mut self) {
        if self.state.cost(self.goal()).is_none() {
            // No route: the search exhausted the reachable area. Leaving the
            // trace without a path is the honest outcome, and the UI says so.
            return;
        }

        let mut route = Vec::new();
        let mut cell = self.goal();
        route.push(cell);
        while cell != self.start() {
            match self.state.from[cell as usize] {
                Some(previous) => {
                    cell = previous;
                    route.push(cell);
                }
                // Defensive: a broken chain would otherwise loop forever.
                None => return,
            }
        }

        for cell in route {
            // Entering the start costs nothing; you begin there.
            let cost = if cell == self.start() {
                0
            } else {
                self.state.grid.cost(cell).unwrap_or(0)
            };
            self.push(GridStep::MarkPath {
                cell,
                cost,
                old_status: self.state.status(cell),
            });
        }
    }

    pub fn finish(
        self,
        name: &'static str,
        complexity: super::trace::Complexity,
    ) -> Trace<GridStep> {
        Trace {
            initial: GridState::new(self.initial),
            steps: self.steps,
            name,
            complexity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_round_trip() {
        let grid = Grid::open(13, 7);
        for cell in 0..grid.len() as Cell {
            let (x, y) = grid.xy(cell);
            assert_eq!(grid.cell(x, y), cell);
        }
    }

    #[test]
    fn neighbours_stop_at_the_edges() {
        let grid = Grid::open(5, 5);

        let corner = grid.cell(0, 0);
        assert_eq!(
            grid.neighbours(corner).into_iter().flatten().count(),
            2,
            "a corner has two neighbours"
        );

        let edge = grid.cell(2, 0);
        assert_eq!(grid.neighbours(edge).into_iter().flatten().count(), 3);

        let middle = grid.cell(2, 2);
        assert_eq!(grid.neighbours(middle).into_iter().flatten().count(), 4);
    }

    #[test]
    fn neighbours_are_adjacent_and_distinct() {
        let grid = Grid::open(9, 9);
        let cell = grid.cell(4, 4);
        let neighbours: Vec<Cell> = grid.neighbours(cell).into_iter().flatten().collect();

        assert_eq!(neighbours.len(), 4);
        for &n in &neighbours {
            assert_eq!(grid.manhattan(cell, n), 1, "neighbour must be one step away");
        }
        let mut sorted = neighbours.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 4, "neighbours must be distinct");
    }

    #[test]
    fn generation_is_deterministic_in_the_seed() {
        assert_eq!(Grid::generate(30, 16, 5), Grid::generate(30, 16, 5));
        assert_ne!(Grid::generate(30, 16, 5), Grid::generate(30, 16, 6));
    }

    #[test]
    fn generated_grids_always_connect_start_to_goal() {
        // Generation is random, so this is the invariant that keeps a seed
        // from producing an immediately boxed-in run.
        for seed in 0..60 {
            let grid = Grid::generate(40, 20, seed);
            assert!(grid.passable(grid.start), "seed {seed} walled the start");
            assert!(grid.passable(grid.goal), "seed {seed} walled the goal");
            assert!(
                grid.reachable(grid.start, grid.goal),
                "seed {seed} left the goal unreachable"
            );
        }
    }

    #[test]
    fn generation_produces_a_mix_of_terrain() {
        let grid = Grid::generate(60, 30, 11);
        let walls = (0..grid.len() as Cell)
            .filter(|&c| grid.terrain(c) == Terrain::Wall)
            .count();
        let rough = (0..grid.len() as Cell)
            .filter(|&c| grid.terrain(c) == Terrain::Rough)
            .count();

        assert!(walls > 0, "no obstacles at all");
        assert!(rough > 0, "no rough ground, so Dijkstra has nothing to find");
        assert!(
            walls < grid.len() / 2,
            "more than half walls would leave nothing to search"
        );
    }

    #[test]
    fn tiny_grids_are_clamped_not_rejected() {
        let grid = Grid::generate(1, 1, 0);
        assert!(grid.width() >= 3 && grid.height() >= 3);
        assert!(grid.reachable(grid.start, grid.goal));
    }

    #[test]
    fn terrain_cycles_through_all_three() {
        assert_eq!(Terrain::Open.next(), Terrain::Rough);
        assert_eq!(Terrain::Rough.next(), Terrain::Wall);
        assert_eq!(Terrain::Wall.next(), Terrain::Open);
        assert!(!Terrain::Wall.passable());
        assert!(Terrain::Rough.cost() > Terrain::Open.cost());
    }
}
