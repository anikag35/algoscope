//! Application state, and the keymap over it.
//!
//! The app holds both modes at once and switches between them, so leaving the
//! grid to look at a sort and coming back finds the search where it was left.
//! Playback advances only in [`App::tick`], driven by elapsed wall time rather
//! than by frame count, so the speed setting means the same thing whatever the
//! render rate turns out to be.
//!
//! The two modes have nothing in common in their state and everything in
//! common in their controls, which is what the [`Session`] trait is for: the
//! keymap is written once against it, and `active_mut` resolves the mode in one
//! place instead of matching in every handler.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

use crate::algo::grid::search as search_algo;
use crate::algo::grid::{Cell, Grid, GridState, GridStep, Terrain};
use crate::algo::sort::{self, SortState, SortStep};
use crate::algo::{Complexity, Playback};

pub const MIN_SIZE: usize = 4;
pub const MAX_SIZE: usize = 512;
pub const DEFAULT_SIZE: usize = 48;

pub const MIN_GRID: u16 = 5;
pub const MAX_GRID: u16 = 200;
pub const DEFAULT_GRID_WIDTH: u16 = 48;
pub const DEFAULT_GRID_HEIGHT: u16 = 20;

/// Playback rates in steps per second.
///
/// Geometric rather than linear: the interesting range spans three orders of
/// magnitude, from single-stepping a comparison at a time to watching a large
/// array resolve in a second.
const SPEEDS: [f64; 7] = [2.0, 8.0, 24.0, 64.0, 160.0, 420.0, 1200.0];

const DEFAULT_SPEED: usize = 2;

/// Most steps applied in a single tick.
///
/// Without a ceiling, a long stall (a suspended process, a resize storm) would
/// hand the next tick a huge `dt` and freeze the UI while it replayed the
/// backlog.
const STEP_BUDGET: usize = 50_000;

/// Which visualizer is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Sort,
    Grid,
}

impl Mode {
    pub fn other(self) -> Self {
        match self {
            Self::Sort => Self::Grid,
            Self::Grid => Self::Sort,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Sort => "sorting",
            Self::Grid => "pathfinding",
        }
    }
}

/// What the renderer needs, without it having to know about modes.
pub enum View<'a> {
    Sort(&'a SortState),
    Grid {
        state: &'a GridState,
        /// `Some` only while editing terrain.
        cursor: Option<Cell>,
    },
}

/// Startup configuration.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub mode: Mode,
    pub sort_algorithm: sort::Algorithm,
    pub grid_algorithm: search_algo::Algorithm,
    pub size: usize,
    pub grid_width: u16,
    pub grid_height: u16,
    pub seed: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            sort_algorithm: sort::Algorithm::default(),
            grid_algorithm: search_algo::Algorithm::default(),
            size: DEFAULT_SIZE,
            grid_width: DEFAULT_GRID_WIDTH,
            grid_height: DEFAULT_GRID_HEIGHT,
            seed: 1,
        }
    }
}

/// The controls both modes share.
///
/// Implemented once per mode so the keymap can be written against the trait
/// rather than branching on the mode in every handler.
trait Session {
    fn name(&self) -> &'static str;
    fn complexity(&self) -> Complexity;
    fn cursor(&self) -> usize;
    fn len(&self) -> usize;
    fn progress(&self) -> f64;
    fn at_end(&self) -> bool;

    fn step_forward(&mut self) -> bool;
    fn step_back(&mut self);
    fn seek(&mut self, target: usize);
    fn rewind(&mut self);
    fn fast_forward(&mut self);

    /// Re-run the current algorithm on the current input.
    fn rebuild(&mut self);
    fn next_algorithm(&mut self);
    fn prev_algorithm(&mut self);
    /// Pick by zero-based position, ignoring anything out of range.
    fn select_algorithm(&mut self, index: usize);
    /// Draw a fresh input.
    fn reshuffle(&mut self);
    fn resize(&mut self, grow: bool);
    /// A one-line description of the input, for the status bar.
    fn extent(&self) -> String;
}

struct SortSession {
    playback: Playback<SortStep>,
    algorithm: sort::Algorithm,
    size: usize,
    seed: u64,
}

impl SortSession {
    fn new(algorithm: sort::Algorithm, size: usize, seed: u64) -> Self {
        let size = size.clamp(MIN_SIZE, MAX_SIZE);
        Self {
            playback: Playback::new(algorithm.run(shuffled(size, seed))),
            algorithm,
            size,
            seed,
        }
    }
}

impl Session for SortSession {
    fn name(&self) -> &'static str {
        self.playback.trace().name
    }

    fn complexity(&self) -> Complexity {
        self.playback.trace().complexity
    }

    fn cursor(&self) -> usize {
        self.playback.cursor()
    }

    fn len(&self) -> usize {
        self.playback.len()
    }

    fn progress(&self) -> f64 {
        self.playback.progress()
    }

    fn at_end(&self) -> bool {
        self.playback.at_end()
    }

    fn step_forward(&mut self) -> bool {
        self.playback.step_forward()
    }

    fn step_back(&mut self) {
        self.playback.step_back();
    }

    fn seek(&mut self, target: usize) {
        self.playback.seek(target);
    }

    fn rewind(&mut self) {
        self.playback.rewind();
    }

    fn fast_forward(&mut self) {
        self.playback.fast_forward();
    }

    fn rebuild(&mut self) {
        self.playback = Playback::new(self.algorithm.run(shuffled(self.size, self.seed)));
    }

    fn next_algorithm(&mut self) {
        self.algorithm = self.algorithm.next();
        self.rebuild();
    }

    fn prev_algorithm(&mut self) {
        self.algorithm = self.algorithm.prev();
        self.rebuild();
    }

    fn select_algorithm(&mut self, index: usize) {
        if let Some(&algorithm) = sort::Algorithm::ALL.get(index) {
            if algorithm != self.algorithm {
                self.algorithm = algorithm;
                self.rebuild();
            }
        }
    }

    fn reshuffle(&mut self) {
        self.seed = self.seed.wrapping_add(1);
        self.rebuild();
    }

    fn resize(&mut self, grow: bool) {
        let next = if grow {
            (self.size + self.size / 4 + 1).min(MAX_SIZE)
        } else {
            (self.size - self.size / 5).max(MIN_SIZE)
        };
        if next != self.size {
            self.size = next;
            self.rebuild();
        }
    }

    fn extent(&self) -> String {
        format!("{} items", self.size)
    }
}

struct GridSession {
    playback: Playback<GridStep>,
    algorithm: search_algo::Algorithm,
    /// The editable map. The playback holds its own copy, so this is the
    /// master that edits apply to and re-runs are built from.
    grid: Grid,
    seed: u64,
    cursor: Cell,
    editing: bool,
}

impl GridSession {
    fn new(algorithm: search_algo::Algorithm, width: u16, height: u16, seed: u64) -> Self {
        let grid = Grid::generate(
            width.clamp(MIN_GRID, MAX_GRID),
            height.clamp(MIN_GRID, MAX_GRID),
            seed,
        );
        let cursor = grid.start;
        Self {
            playback: Playback::new(algorithm.run(grid.clone())),
            algorithm,
            grid,
            seed,
            cursor,
            editing: false,
        }
    }

    fn state(&self) -> &GridState {
        self.playback.state()
    }

    /// Move the edit cursor, stopping at the edges.
    fn move_cursor(&mut self, dx: i32, dy: i32) {
        let (x, y) = self.grid.xy(self.cursor);
        let nx = (i32::from(x) + dx).clamp(0, i32::from(self.grid.width()) - 1) as u16;
        let ny = (i32::from(y) + dy).clamp(0, i32::from(self.grid.height()) - 1) as u16;
        self.cursor = self.grid.cell(nx, ny);
    }

    /// Re-run and jump to the finished state.
    ///
    /// Used after an edit, so painting a wall shows its effect on the route at
    /// once. Watching the search itself is `home` then play.
    fn rebuild_and_show_result(&mut self) {
        self.rebuild();
        self.fast_forward();
    }

    /// Cycle the terrain under the cursor.
    fn cycle_terrain(&mut self) {
        // The endpoints must stay passable, so they are not paintable.
        if self.cursor == self.grid.start || self.cursor == self.grid.goal {
            return;
        }
        let next = self.grid.terrain(self.cursor).next();
        self.grid.set(self.cursor, next);
        self.rebuild_and_show_result();
    }

    /// Move the start or the goal to the cursor.
    fn place_endpoint(&mut self, start: bool) {
        if start && self.cursor == self.grid.goal || !start && self.cursor == self.grid.start {
            return;
        }
        // An endpoint on a wall would make every run fail instantly.
        self.grid.set(self.cursor, Terrain::Open);
        if start {
            self.grid.start = self.cursor;
        } else {
            self.grid.goal = self.cursor;
        }
        self.rebuild_and_show_result();
    }
}

impl Session for GridSession {
    fn name(&self) -> &'static str {
        self.playback.trace().name
    }

    fn complexity(&self) -> Complexity {
        self.playback.trace().complexity
    }

    fn cursor(&self) -> usize {
        self.playback.cursor()
    }

    fn len(&self) -> usize {
        self.playback.len()
    }

    fn progress(&self) -> f64 {
        self.playback.progress()
    }

    fn at_end(&self) -> bool {
        self.playback.at_end()
    }

    fn step_forward(&mut self) -> bool {
        self.playback.step_forward()
    }

    fn step_back(&mut self) {
        self.playback.step_back();
    }

    fn seek(&mut self, target: usize) {
        self.playback.seek(target);
    }

    fn rewind(&mut self) {
        self.playback.rewind();
    }

    fn fast_forward(&mut self) {
        self.playback.fast_forward();
    }

    fn rebuild(&mut self) {
        self.playback = Playback::new(self.algorithm.run(self.grid.clone()));
    }

    fn next_algorithm(&mut self) {
        self.algorithm = self.algorithm.next();
        self.rebuild();
    }

    fn prev_algorithm(&mut self) {
        self.algorithm = self.algorithm.prev();
        self.rebuild();
    }

    fn select_algorithm(&mut self, index: usize) {
        if let Some(&algorithm) = search_algo::Algorithm::ALL.get(index) {
            if algorithm != self.algorithm {
                self.algorithm = algorithm;
                self.rebuild();
            }
        }
    }

    fn reshuffle(&mut self) {
        self.seed = self.seed.wrapping_add(1);
        self.grid = Grid::generate(self.grid.width(), self.grid.height(), self.seed);
        self.cursor = self.grid.start;
        self.rebuild();
    }

    fn resize(&mut self, grow: bool) {
        let (width, height) = (self.grid.width(), self.grid.height());
        let scale = |value: u16| {
            if grow {
                (value + value / 5 + 1).min(MAX_GRID)
            } else {
                (value - value / 6).max(MIN_GRID)
            }
        };
        let (next_width, next_height) = (scale(width), scale(height));
        if (next_width, next_height) != (width, height) {
            self.grid = Grid::generate(next_width, next_height, self.seed);
            self.cursor = self.grid.start;
            self.rebuild();
        }
    }

    fn extent(&self) -> String {
        format!("{}x{}", self.grid.width(), self.grid.height())
    }
}

pub struct App {
    mode: Mode,
    sort: SortSession,
    grid: GridSession,
    speed: usize,
    playing: bool,
    show_help: bool,
    should_quit: bool,
    /// Fractional steps carried between ticks, so slow rates stay smooth
    /// instead of quantising to the tick interval.
    accumulator: f64,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            mode: config.mode,
            sort: SortSession::new(config.sort_algorithm, config.size, config.seed),
            grid: GridSession::new(
                config.grid_algorithm,
                config.grid_width,
                config.grid_height,
                config.seed,
            ),
            speed: DEFAULT_SPEED,
            playing: false,
            show_help: false,
            should_quit: false,
            accumulator: 0.0,
        }
    }

    fn active(&self) -> &dyn Session {
        match self.mode {
            Mode::Sort => &self.sort,
            Mode::Grid => &self.grid,
        }
    }

    fn active_mut(&mut self) -> &mut dyn Session {
        match self.mode {
            Mode::Sort => &mut self.sort,
            Mode::Grid => &mut self.grid,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// What to draw.
    pub fn view(&self) -> View<'_> {
        match self.mode {
            Mode::Sort => View::Sort(self.sort.playback.state()),
            Mode::Grid => View::Grid {
                state: self.grid.state(),
                cursor: self.grid.editing.then_some(self.grid.cursor),
            },
        }
    }

    pub fn algorithm_name(&self) -> &'static str {
        self.active().name()
    }

    pub fn complexity(&self) -> Complexity {
        self.active().complexity()
    }

    pub fn cursor(&self) -> usize {
        self.active().cursor()
    }

    pub fn len(&self) -> usize {
        self.active().len()
    }

    pub fn is_empty(&self) -> bool {
        self.active().len() == 0
    }

    pub fn progress(&self) -> f64 {
        self.active().progress()
    }

    pub fn at_end(&self) -> bool {
        self.active().at_end()
    }

    /// A description of the current input, for the status bar.
    pub fn extent(&self) -> String {
        self.active().extent()
    }

    /// Whether terrain editing is on.
    pub fn editing(&self) -> bool {
        self.mode == Mode::Grid && self.grid.editing
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn show_help(&self) -> bool {
        self.show_help
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn seed(&self) -> u64 {
        match self.mode {
            Mode::Sort => self.sort.seed,
            Mode::Grid => self.grid.seed,
        }
    }

    pub fn steps_per_second(&self) -> f64 {
        SPEEDS[self.speed]
    }

    pub fn speed_level(&self) -> usize {
        self.speed + 1
    }

    pub fn speed_levels(&self) -> usize {
        SPEEDS.len()
    }

    /// How many algorithms the current mode offers.
    pub fn algorithm_count(&self) -> usize {
        match self.mode {
            Mode::Sort => sort::Algorithm::ALL.len(),
            Mode::Grid => search_algo::Algorithm::ALL.len(),
        }
    }

    /// Advance playback by however much time has passed.
    pub fn tick(&mut self, dt: Duration) {
        if !self.playing {
            return;
        }
        if self.active().at_end() {
            self.playing = false;
            self.accumulator = 0.0;
            return;
        }

        self.accumulator += dt.as_secs_f64() * SPEEDS[self.speed];

        let mut applied = 0;
        while self.accumulator >= 1.0 && applied < STEP_BUDGET {
            self.accumulator -= 1.0;
            applied += 1;
            if !self.active_mut().step_forward() {
                // Reaching the end pauses rather than looping, so the final
                // state stays on screen to be read.
                self.playing = false;
                self.accumulator = 0.0;
                return;
            }
        }

        if applied >= STEP_BUDGET {
            self.accumulator = 0.0;
        }
    }

    pub fn toggle_play(&mut self) {
        if self.active().at_end() {
            // Replaying from the end would otherwise look like a dead key.
            self.active_mut().rewind();
        }
        self.playing = !self.playing;
        self.accumulator = 0.0;
    }

    /// Switch between sorting and pathfinding, leaving both where they were.
    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.other();
        self.playing = false;
        self.accumulator = 0.0;
    }

    fn pause(&mut self) {
        self.playing = false;
        self.accumulator = 0.0;
    }

    /// Step by hand, which implies pausing.
    fn nudge(&mut self, forward: bool) {
        self.pause();
        if forward {
            self.active_mut().step_forward();
        } else {
            self.active_mut().step_back();
        }
    }

    /// Jump by a fraction of the whole run, for coarse scrubbing.
    fn scrub(&mut self, fraction: f64) {
        self.pause();
        let delta = (self.active().len() as f64 * fraction).round() as i64;
        let target = (self.active().cursor() as i64 + delta).max(0) as usize;
        self.active_mut().seek(target);
    }

    /// Handle a key while editing terrain. Returns whether it was consumed.
    fn handle_edit_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.grid.move_cursor(0, -1),
            KeyCode::Down | KeyCode::Char('j') => self.grid.move_cursor(0, 1),
            KeyCode::Left | KeyCode::Char('h') => self.grid.move_cursor(-1, 0),
            KeyCode::Right | KeyCode::Char('l') => self.grid.move_cursor(1, 0),
            KeyCode::Char(' ') | KeyCode::Enter => self.grid.cycle_terrain(),
            KeyCode::Char('s') => self.grid.place_endpoint(true),
            KeyCode::Char('g') => self.grid.place_endpoint(false),
            KeyCode::Char('e') | KeyCode::Esc => self.grid.editing = false,
            // Everything else falls through to the normal keymap, so quitting
            // and changing algorithm still work while editing.
            _ => return false,
        }
        true
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Windows reports both press and release; acting on both would double
        // every keystroke.
        if key.kind != KeyEventKind::Press {
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d')) {
            self.should_quit = true;
            return;
        }

        // Any key dismisses help, so it cannot become a trap.
        if self.show_help {
            self.show_help = false;
            if matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
                return;
            }
        }

        if self.editing() && self.handle_edit_key(key) {
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = true,
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_play(),
            KeyCode::Char('m') => self.toggle_mode(),

            KeyCode::Char('e') => {
                if self.mode == Mode::Grid {
                    self.grid.editing = true;
                    self.pause();
                }
            }

            KeyCode::Right | KeyCode::Char('l') => self.nudge(true),
            KeyCode::Left | KeyCode::Char('h') => self.nudge(false),
            KeyCode::PageDown => self.scrub(0.1),
            KeyCode::PageUp => self.scrub(-0.1),

            KeyCode::Up | KeyCode::Char('k') => {
                self.speed = (self.speed + 1).min(SPEEDS.len() - 1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.speed = self.speed.saturating_sub(1);
            }

            KeyCode::Home | KeyCode::Char('g') => {
                self.pause();
                self.active_mut().rewind();
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.pause();
                self.active_mut().fast_forward();
            }

            KeyCode::Char('r') => {
                self.pause();
                self.active_mut().reshuffle();
            }
            KeyCode::Tab => {
                self.pause();
                self.active_mut().next_algorithm();
            }
            KeyCode::BackTab => {
                self.pause();
                self.active_mut().prev_algorithm();
            }

            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.pause();
                self.active_mut().resize(true);
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.pause();
                self.active_mut().resize(false);
            }

            KeyCode::Char(c @ '1'..='9') => {
                self.pause();
                self.active_mut().select_algorithm(c as usize - '1' as usize);
            }

            _ => {}
        }
    }
}

/// A shuffled permutation of `1..=size`, deterministic in `seed`.
///
/// A permutation rather than random values: every bar height is distinct, so
/// the array is readable as a shape and any remaining disorder is visible. The
/// seed keeps runs reproducible, which is what lets the same input be replayed
/// through a different algorithm.
fn shuffled(size: usize, seed: u64) -> Vec<u32> {
    let mut values: Vec<u32> = (1..=size as u32).collect();
    values.shuffle(&mut StdRng::seed_from_u64(seed));
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn app() -> App {
        App::new(Config {
            size: 16,
            seed: 7,
            grid_width: 14,
            grid_height: 9,
            ..Config::default()
        })
    }

    fn grid_app() -> App {
        let mut app = app();
        app.toggle_mode();
        assert_eq!(app.mode(), Mode::Grid);
        app
    }

    fn sort_values(app: &App) -> Vec<u32> {
        match app.view() {
            View::Sort(state) => state.values.clone(),
            View::Grid { .. } => panic!("expected sort mode"),
        }
    }

    fn grid_state(app: &App) -> &GridState {
        match app.view() {
            View::Grid { state, .. } => state,
            View::Sort(_) => panic!("expected grid mode"),
        }
    }

    #[test]
    fn shuffled_is_a_deterministic_permutation() {
        let a = shuffled(32, 99);
        assert_eq!(a, shuffled(32, 99), "same seed must give the same array");

        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (1..=32).collect::<Vec<u32>>());
        assert_ne!(shuffled(32, 100), a, "a new seed should reshuffle");
    }

    #[test]
    fn stepping_by_hand_pauses_playback() {
        let mut app = app();
        app.handle_key(press(KeyCode::Char(' ')));
        assert!(app.playing());

        app.handle_key(press(KeyCode::Right));
        assert!(!app.playing(), "a manual step should pause");
        assert_eq!(app.cursor(), 1);

        app.handle_key(press(KeyCode::Left));
        assert_eq!(app.cursor(), 0);
    }

    #[test]
    fn stepping_back_at_the_start_is_a_no_op() {
        let mut app = app();
        for _ in 0..5 {
            app.handle_key(press(KeyCode::Left));
        }
        assert_eq!(app.cursor(), 0);
    }

    #[test]
    fn tick_advances_in_proportion_to_elapsed_time() {
        let mut app = app();
        app.toggle_play();
        let rate = app.steps_per_second();

        app.tick(Duration::from_secs_f64(10.0 / rate));
        assert_eq!(app.cursor(), 10);
    }

    #[test]
    fn fractional_steps_accumulate_rather_than_rounding_away() {
        let mut app = app();
        app.toggle_play();
        let rate = app.steps_per_second();

        for _ in 0..10 {
            app.tick(Duration::from_secs_f64(0.1 / rate));
        }
        assert_eq!(app.cursor(), 1);
    }

    #[test]
    fn a_paused_app_does_not_advance() {
        let mut app = app();
        app.tick(Duration::from_secs(10));
        assert_eq!(app.cursor(), 0);
    }

    #[test]
    fn reaching_the_end_pauses_and_stays_there() {
        let mut app = app();
        app.toggle_play();
        app.tick(Duration::from_secs(600));

        assert!(app.at_end());
        assert!(!app.playing(), "finishing should pause, not loop");
        match app.view() {
            View::Sort(state) => assert!(state.is_sorted()),
            View::Grid { .. } => unreachable!(),
        }
    }

    #[test]
    fn play_from_the_end_restarts() {
        let mut app = app();
        app.handle_key(press(KeyCode::End));
        assert!(app.at_end());

        app.toggle_play();
        assert_eq!(app.cursor(), 0, "should rewind, not sit dead");
        assert!(app.playing());
    }

    #[test]
    fn switching_algorithm_keeps_the_array() {
        let mut app = app();
        let before = sort_values(&app);

        app.handle_key(press(KeyCode::Tab));
        assert_eq!(app.algorithm_name(), "Insertion sort");
        assert_eq!(
            sort_values(&app),
            before,
            "the input must be held fixed so the counters stay comparable"
        );
    }

    #[test]
    fn shuffling_changes_the_array_and_rewinds() {
        let mut app = app();
        app.handle_key(press(KeyCode::End));
        let before = sort_values(&app);

        app.handle_key(press(KeyCode::Char('r')));
        assert_ne!(sort_values(&app), before);
        assert!(app.cursor() == 0);
    }

    #[test]
    fn resize_stays_within_bounds() {
        let mut app = app();
        for _ in 0..100 {
            app.handle_key(press(KeyCode::Char('-')));
        }
        assert_eq!(app.sort.size, MIN_SIZE);

        for _ in 0..200 {
            app.handle_key(press(KeyCode::Char('+')));
        }
        assert_eq!(app.sort.size, MAX_SIZE);
    }

    #[test]
    fn speed_saturates_at_both_ends() {
        let mut app = app();
        for _ in 0..50 {
            app.handle_key(press(KeyCode::Up));
        }
        assert_eq!(app.speed_level(), app.speed_levels());

        for _ in 0..50 {
            app.handle_key(press(KeyCode::Down));
        }
        assert_eq!(app.speed_level(), 1);
    }

    #[test]
    fn number_keys_select_an_algorithm_and_ignore_the_rest() {
        let mut app = app();
        app.handle_key(press(KeyCode::Char('3')));
        assert_eq!(app.algorithm_name(), "Selection sort");

        app.handle_key(press(KeyCode::Char('9')));
        assert_eq!(app.algorithm_name(), "Selection sort");
    }

    #[test]
    fn help_is_dismissed_by_any_key() {
        let mut app = app();
        app.handle_key(press(KeyCode::Char('?')));
        assert!(app.show_help());

        app.handle_key(press(KeyCode::Char(' ')));
        assert!(!app.show_help(), "help must not be a trap");
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut app = app();
        let mut release = press(KeyCode::Right);
        release.kind = KeyEventKind::Release;

        app.handle_key(release);
        assert_eq!(
            app.cursor(),
            0,
            "acting on a release would double every keystroke on Windows"
        );
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit());
    }

    #[test]
    fn mode_switching_preserves_both_sides() {
        let mut app = app();
        app.handle_key(press(KeyCode::End));
        let sort_cursor = app.cursor();
        assert!(sort_cursor > 0);

        app.handle_key(press(KeyCode::Char('m')));
        assert_eq!(app.mode(), Mode::Grid);
        assert_eq!(app.cursor(), 0, "the grid search has not been run yet");

        app.handle_key(press(KeyCode::Right));
        assert_eq!(app.cursor(), 1);

        app.handle_key(press(KeyCode::Char('m')));
        assert_eq!(app.mode(), Mode::Sort);
        assert_eq!(
            app.cursor(),
            sort_cursor,
            "the sort should be where it was left"
        );

        app.handle_key(press(KeyCode::Char('m')));
        assert_eq!(app.cursor(), 1, "and so should the search");
    }

    #[test]
    fn switching_mode_pauses() {
        let mut app = app();
        app.toggle_play();
        app.handle_key(press(KeyCode::Char('m')));
        assert!(!app.playing());
    }

    #[test]
    fn grid_mode_offers_four_algorithms() {
        let mut app = grid_app();
        assert_eq!(app.algorithm_count(), 4);

        app.handle_key(press(KeyCode::Char('4')));
        assert_eq!(app.algorithm_name(), "A* search");

        app.handle_key(press(KeyCode::Char('1')));
        assert_eq!(app.algorithm_name(), "Breadth-first");
    }

    #[test]
    fn grid_search_runs_to_a_route() {
        let mut app = grid_app();
        app.toggle_play();
        app.tick(Duration::from_secs(600));

        assert!(app.at_end());
        assert!(grid_state(&app).has_path(), "no route found");
    }

    #[test]
    fn switching_search_keeps_the_map() {
        let mut app = grid_app();
        let before = grid_state(&app).grid.clone();

        app.handle_key(press(KeyCode::Tab));
        assert_eq!(
            grid_state(&app).grid,
            before,
            "the map must be held fixed so expansion counts stay comparable"
        );
    }

    #[test]
    fn editing_is_grid_only() {
        let mut app = app();
        app.handle_key(press(KeyCode::Char('e')));
        assert!(!app.editing(), "sorting has no terrain to edit");

        app.handle_key(press(KeyCode::Char('m')));
        app.handle_key(press(KeyCode::Char('e')));
        assert!(app.editing());

        app.handle_key(press(KeyCode::Esc));
        assert!(!app.editing());
        assert!(!app.should_quit(), "esc should leave editing, not quit");
    }

    #[test]
    fn the_edit_cursor_moves_and_stops_at_the_edges() {
        let mut app = grid_app();
        app.handle_key(press(KeyCode::Char('e')));

        let start = app.grid.cursor;
        app.handle_key(press(KeyCode::Right));
        assert_ne!(app.grid.cursor, start, "cursor should have moved");

        for _ in 0..50 {
            app.handle_key(press(KeyCode::Left));
        }
        let (x, _) = app.grid.grid.xy(app.grid.cursor);
        assert_eq!(x, 0, "cursor should stop at the left edge");
    }

    #[test]
    fn painting_terrain_reruns_the_search() {
        let mut app = grid_app();
        app.handle_key(press(KeyCode::Char('e')));
        // Somewhere that is not an endpoint.
        app.handle_key(press(KeyCode::Down));
        app.handle_key(press(KeyCode::Right));

        let cell = app.grid.cursor;
        let before = app.grid.grid.terrain(cell);
        app.handle_key(press(KeyCode::Char(' ')));

        assert_ne!(app.grid.grid.terrain(cell), before, "terrain should cycle");
        assert!(
            app.at_end(),
            "an edit should show its result rather than resetting to the start"
        );
    }

    #[test]
    fn endpoints_cannot_be_painted_over() {
        let mut app = grid_app();
        app.handle_key(press(KeyCode::Char('e')));
        // The cursor starts on the start cell.
        let start = app.grid.grid.start;
        assert_eq!(app.grid.cursor, start);

        app.handle_key(press(KeyCode::Char(' ')));
        assert_eq!(
            app.grid.grid.terrain(start),
            Terrain::Open,
            "walling the start would make every run fail instantly"
        );
    }

    #[test]
    fn moving_an_endpoint_clears_the_cell_under_it() {
        let mut app = grid_app();
        app.handle_key(press(KeyCode::Char('e')));

        // Find a wall to drop the goal onto, if generation made one.
        let target = (0..app.grid.grid.len() as Cell)
            .find(|&c| !app.grid.grid.passable(c))
            .unwrap_or(app.grid.grid.cell(1, 1));
        app.grid.cursor = target;
        app.handle_key(press(KeyCode::Char('g')));

        assert_eq!(app.grid.grid.goal, target);
        assert!(
            app.grid.grid.passable(target),
            "an endpoint must never sit on a wall"
        );
    }

    #[test]
    fn editing_still_allows_quitting_and_algorithm_changes() {
        let mut app = grid_app();
        app.handle_key(press(KeyCode::Char('e')));

        // Unhandled-in-edit-mode keys fall through to the normal keymap.
        app.handle_key(press(KeyCode::Tab));
        assert_eq!(app.algorithm_name(), "Depth-first");

        app.handle_key(press(KeyCode::Char('q')));
        assert!(app.should_quit());
    }

    #[test]
    fn grid_resize_regenerates_within_bounds() {
        let mut app = grid_app();
        for _ in 0..100 {
            app.handle_key(press(KeyCode::Char('-')));
        }
        assert_eq!(app.grid.grid.width(), MIN_GRID);
        assert_eq!(app.grid.grid.height(), MIN_GRID);

        for _ in 0..200 {
            app.handle_key(press(KeyCode::Char('+')));
        }
        assert_eq!(app.grid.grid.width(), MAX_GRID);
    }
}
