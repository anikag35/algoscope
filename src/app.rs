//! Application state, and the keymap over it.
//!
//! The app owns a [`Playback`] and does nothing to it that the user did not
//! ask for. Playback advances only in [`App::tick`], driven by elapsed wall
//! time rather than by frame count, so the speed setting means the same thing
//! whatever the render rate turns out to be.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

use crate::algo::Playback;
use crate::algo::sort::{Algorithm, SortStep};

pub const MIN_SIZE: usize = 4;
pub const MAX_SIZE: usize = 512;
pub const DEFAULT_SIZE: usize = 48;

/// Playback rates in steps per second.
///
/// Geometric rather than linear: the interesting range spans three orders of
/// magnitude, from single-stepping a comparison at a time to watching a large
/// array resolve in a second.
const SPEEDS: [f64; 7] = [2.0, 8.0, 24.0, 64.0, 160.0, 420.0, 1200.0];

/// Index into [`SPEEDS`] that playback starts at.
const DEFAULT_SPEED: usize = 2;

/// Most steps applied in a single tick.
///
/// Without a ceiling, a long stall (a suspended process, a resize storm) would
/// hand the next tick a huge `dt` and freeze the UI while it replayed the
/// backlog.
const STEP_BUDGET: usize = 50_000;

pub struct App {
    playback: Playback<SortStep>,
    algorithm: Algorithm,
    size: usize,
    seed: u64,
    speed: usize,
    playing: bool,
    show_help: bool,
    should_quit: bool,
    /// Fractional steps carried between ticks, so slow rates stay smooth
    /// instead of quantising to the tick interval.
    accumulator: f64,
}

impl App {
    pub fn new(algorithm: Algorithm, size: usize, seed: u64) -> Self {
        let size = size.clamp(MIN_SIZE, MAX_SIZE);
        let playback = Playback::new(algorithm.run(generate(size, seed)));
        Self {
            playback,
            algorithm,
            size,
            seed,
            speed: DEFAULT_SPEED,
            playing: false,
            show_help: false,
            should_quit: false,
            accumulator: 0.0,
        }
    }

    pub fn playback(&self) -> &Playback<SortStep> {
        &self.playback
    }

    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn seed(&self) -> u64 {
        self.seed
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

    /// Current rate in steps per second.
    pub fn steps_per_second(&self) -> f64 {
        SPEEDS[self.speed]
    }

    /// Speed as a 1-based position, for display.
    pub fn speed_level(&self) -> usize {
        self.speed + 1
    }

    pub fn speed_levels(&self) -> usize {
        SPEEDS.len()
    }

    /// Advance playback by however much time has passed.
    pub fn tick(&mut self, dt: Duration) {
        if !self.playing {
            return;
        }
        if self.playback.at_end() {
            self.playing = false;
            self.accumulator = 0.0;
            return;
        }

        self.accumulator += dt.as_secs_f64() * self.steps_per_second();

        let mut applied = 0;
        while self.accumulator >= 1.0 && applied < STEP_BUDGET {
            self.accumulator -= 1.0;
            applied += 1;
            if !self.playback.step_forward() {
                // Reaching the end pauses rather than looping, so the final
                // state stays on screen to be read.
                self.playing = false;
                self.accumulator = 0.0;
                return;
            }
        }

        // Drop any remaining backlog instead of carrying it into the next tick.
        if applied >= STEP_BUDGET {
            self.accumulator = 0.0;
        }
    }

    /// Re-run the current algorithm on the current array.
    fn rebuild(&mut self) {
        self.playback = Playback::new(self.algorithm.run(generate(self.size, self.seed)));
        self.accumulator = 0.0;
        self.playing = false;
    }

    /// Switch algorithm, keeping the same array.
    ///
    /// Holding the input fixed is what makes the counters comparable: the same
    /// 48 values through bubble and through insertion sort is a direct
    /// measurement, not two unrelated runs.
    pub fn set_algorithm(&mut self, algorithm: Algorithm) {
        if algorithm != self.algorithm {
            self.algorithm = algorithm;
            self.rebuild();
        }
    }

    /// Draw a new array.
    pub fn shuffle(&mut self) {
        self.seed = self.seed.wrapping_add(1);
        self.rebuild();
    }

    /// Change the array length, in proportional steps.
    pub fn resize(&mut self, grow: bool) {
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

    pub fn toggle_play(&mut self) {
        if self.playback.at_end() {
            // Replaying from the end would otherwise look like a dead key.
            self.playback.rewind();
        }
        self.playing = !self.playing;
        self.accumulator = 0.0;
    }

    /// Step by hand, which implies pausing.
    fn nudge(&mut self, forward: bool) {
        self.playing = false;
        self.accumulator = 0.0;
        if forward {
            self.playback.step_forward();
        } else {
            self.playback.step_back();
        }
    }

    /// Jump by a fraction of the whole run, for coarse scrubbing.
    fn scrub(&mut self, fraction: f64) {
        self.playing = false;
        self.accumulator = 0.0;
        let delta = (self.playback.len() as f64 * fraction).round() as i64;
        let target = (self.playback.cursor() as i64 + delta).max(0) as usize;
        self.playback.seek(target);
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

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = true,
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_play(),

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
                self.playing = false;
                self.accumulator = 0.0;
                self.playback.rewind();
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.playing = false;
                self.accumulator = 0.0;
                self.playback.fast_forward();
            }

            KeyCode::Char('r') => self.shuffle(),
            KeyCode::Tab => self.set_algorithm(self.algorithm.next()),
            KeyCode::BackTab => self.set_algorithm(self.algorithm.prev()),

            KeyCode::Char('+') | KeyCode::Char('=') => self.resize(true),
            KeyCode::Char('-') | KeyCode::Char('_') => self.resize(false),

            KeyCode::Char(c @ '1'..='9') => {
                let i = c as usize - '1' as usize;
                if let Some(&algorithm) = Algorithm::ALL.get(i) {
                    self.set_algorithm(algorithm);
                }
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
fn generate(size: usize, seed: u64) -> Vec<u32> {
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
        App::new(Algorithm::Bubble, 16, 7)
    }

    #[test]
    fn generate_is_a_deterministic_permutation() {
        let a = generate(32, 99);
        let b = generate(32, 99);
        assert_eq!(a, b, "same seed must give the same array");

        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (1..=32).collect::<Vec<u32>>());

        assert_ne!(generate(32, 100), a, "a new seed should reshuffle");
    }

    #[test]
    fn stepping_by_hand_pauses_playback() {
        let mut app = app();
        app.handle_key(press(KeyCode::Char(' ')));
        assert!(app.playing());

        app.handle_key(press(KeyCode::Right));
        assert!(!app.playing(), "a manual step should pause");
        assert_eq!(app.playback().cursor(), 1);

        app.handle_key(press(KeyCode::Left));
        assert_eq!(app.playback().cursor(), 0);
    }

    #[test]
    fn stepping_back_at_the_start_is_a_no_op() {
        let mut app = app();
        for _ in 0..5 {
            app.handle_key(press(KeyCode::Left));
        }
        assert_eq!(app.playback().cursor(), 0);
    }

    #[test]
    fn tick_advances_in_proportion_to_elapsed_time() {
        let mut app = app();
        app.toggle_play();
        let rate = app.steps_per_second();

        app.tick(Duration::from_secs_f64(10.0 / rate));
        assert_eq!(app.playback().cursor(), 10);
    }

    #[test]
    fn fractional_steps_accumulate_rather_than_rounding_away() {
        let mut app = app();
        app.toggle_play();
        let rate = app.steps_per_second();

        // Ten ticks of a tenth of a step each should add up to one step, not
        // to zero.
        for _ in 0..10 {
            app.tick(Duration::from_secs_f64(0.1 / rate));
        }
        assert_eq!(app.playback().cursor(), 1);
    }

    #[test]
    fn a_paused_app_does_not_advance() {
        let mut app = app();
        app.tick(Duration::from_secs(10));
        assert_eq!(app.playback().cursor(), 0);
    }

    #[test]
    fn reaching_the_end_pauses_and_stays_there() {
        let mut app = app();
        app.toggle_play();
        app.tick(Duration::from_secs(600));

        assert!(app.playback().at_end());
        assert!(!app.playing(), "finishing should pause, not loop");
        assert!(app.playback().state().is_sorted());
    }

    #[test]
    fn play_from_the_end_restarts() {
        let mut app = app();
        app.handle_key(press(KeyCode::End));
        assert!(app.playback().at_end());

        app.toggle_play();
        assert_eq!(app.playback().cursor(), 0, "should rewind, not sit dead");
        assert!(app.playing());
    }

    #[test]
    fn switching_algorithm_keeps_the_array() {
        let mut app = app();
        let before = app.playback().trace().initial.values.clone();

        app.handle_key(press(KeyCode::Tab));
        assert_eq!(app.algorithm(), Algorithm::Insertion);
        assert_eq!(
            app.playback().trace().initial.values,
            before,
            "the input must be held fixed so the counters stay comparable"
        );
    }

    #[test]
    fn shuffling_changes_the_array_and_rewinds() {
        let mut app = app();
        app.handle_key(press(KeyCode::End));
        let before = app.playback().trace().initial.values.clone();

        app.handle_key(press(KeyCode::Char('r')));
        assert_ne!(app.playback().trace().initial.values, before);
        assert!(app.playback().at_start());
    }

    #[test]
    fn resize_stays_within_bounds() {
        let mut app = app();
        for _ in 0..100 {
            app.resize(false);
        }
        assert_eq!(app.size(), MIN_SIZE);

        for _ in 0..200 {
            app.resize(true);
        }
        assert_eq!(app.size(), MAX_SIZE);
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
        assert_eq!(app.algorithm(), Algorithm::Selection);

        // Out of range: should be ignored, not panic or wrap.
        app.handle_key(press(KeyCode::Char('9')));
        assert_eq!(app.algorithm(), Algorithm::Selection);
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
            app.playback().cursor(),
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
}
