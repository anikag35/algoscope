//! Terminal capability detection, and the palette built on top of it.
//!
//! The visualizer should look right on a modern terminal and still be readable
//! over SSH on something ancient, so nothing here assumes truecolor, a Nerd
//! Font, or even that the terminal can draw box-drawing characters. Capability
//! is detected once at startup and every colour and glyph decision routes
//! through [`Theme`].
//!
//! Colour choices avoid the red/green pairing, which is the one combination a
//! red-green colourblind viewer cannot separate: settled bars are green and
//! active bars are amber, which stay distinct under every common form of colour
//! vision deficiency. The magnitude ramp is a single blue-to-cyan hue for the
//! same reason — it reads as a gradient by lightness, not by hue alone.

use std::borrow::Cow;
use std::env;

use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;

/// How much colour the terminal can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    /// No colour at all: `NO_COLOR` is set, or `TERM` says `dumb`. Shape and
    /// text attributes have to carry the whole signal.
    Mono,
    /// The 16 ANSI colours, whose actual values are the user's theme. Safe
    /// essentially everywhere.
    Ansi16,
    /// 24-bit colour, so the magnitude ramp can be used.
    Full,
}

/// What a bar means right now, which decides how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarState {
    /// Unsorted and untouched by the current step.
    Idle,
    /// Touched by the step just applied.
    Active,
    /// Settled by the algorithm.
    Sorted,
}

/// Vertical eighth-blocks, indexed by eighths filled.
const EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// ASCII stand-in for [`EIGHTHS`], for terminals not in a UTF-8 locale.
///
/// Sub-cell precision is not really available here, so this is a coarse ramp
/// that at least keeps relative heights legible.
const EIGHTHS_ASCII: [char; 9] = [' ', '.', '.', ':', ':', '=', '=', '#', '#'];

/// Resolved terminal capabilities plus the palette derived from them.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub depth: ColorDepth,
    /// Whether non-ASCII glyphs are safe to emit.
    pub unicode: bool,
}

impl Theme {
    /// Inspect the environment once.
    ///
    /// Deliberately conservative: anything unrecognised degrades rather than
    /// producing mojibake or invisible text.
    pub fn detect() -> Self {
        Self {
            depth: detect_depth(),
            unicode: detect_unicode(),
        }
    }

    /// A theme that assumes nothing, for tests and for `--ascii`.
    pub fn minimal() -> Self {
        Self {
            depth: ColorDepth::Mono,
            unicode: false,
        }
    }

    /// The glyph for `n` eighths of a cell filled, `n` in `0..=8`.
    pub fn eighth(&self, n: usize) -> char {
        let table = if self.unicode { &EIGHTHS } else { &EIGHTHS_ASCII };
        table[n.min(8)]
    }

    /// A horizontal line glyph, for rules and the scrubber track.
    pub fn line(&self) -> char {
        if self.unicode { '─' } else { '-' }
    }

    /// The scrubber's filled portion.
    pub fn track_full(&self) -> char {
        if self.unicode { '━' } else { '=' }
    }

    /// Border glyphs for blocks.
    ///
    /// ratatui's default set is box-drawing, so this has to be overridden
    /// rather than merely avoided: a `Block` left to itself would emit `│` on a
    /// terminal that cannot show it.
    pub fn border_set(&self) -> border::Set<'static> {
        if self.unicode {
            border::PLAIN
        } else {
            border::Set {
                top_left: "+",
                top_right: "+",
                bottom_left: "+",
                bottom_right: "+",
                vertical_left: "|",
                vertical_right: "|",
                horizontal_top: "-",
                horizontal_bottom: "-",
            }
        }
    }

    /// Transliterate label text for terminals that cannot render non-ASCII.
    ///
    /// Complexity labels are the reason this exists: `O(n²)` is worth having
    /// where it renders, but it must degrade to `O(n^2)` rather than to a
    /// replacement box. Borrows in the common case, so the usual path costs
    /// nothing.
    pub fn text<'a>(&self, s: &'a str) -> Cow<'a, str> {
        if self.unicode || s.is_ascii() {
            return Cow::Borrowed(s);
        }

        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '²' => out.push_str("^2"),
                '³' => out.push_str("^3"),
                '·' => out.push('*'),
                '≈' => out.push('~'),
                '─' | '━' => out.push('-'),
                '│' => out.push('|'),
                '▶' => out.push('>'),
                '→' => out.push_str("->"),
                '←' => out.push_str("<-"),
                '…' => out.push_str("..."),
                c if c.is_ascii() => out.push(c),
                // Anything unanticipated is dropped rather than risking a
                // replacement glyph of unknown width, which would misalign
                // every column after it.
                _ => {}
            }
        }
        Cow::Owned(out)
    }

    /// Style for a bar of `value` out of `max`, in the given state.
    ///
    /// In [`ColorDepth::Mono`] the state has to come through as a text
    /// attribute, since there is no colour to carry it: active reverses, and
    /// settled dims to read as "done".
    pub fn bar(&self, value: u32, max: u32, state: BarState) -> Style {
        match self.depth {
            ColorDepth::Mono => match state {
                BarState::Idle => Style::new(),
                BarState::Active => Style::new().add_modifier(Modifier::REVERSED),
                BarState::Sorted => Style::new().add_modifier(Modifier::DIM),
            },
            ColorDepth::Ansi16 => match state {
                BarState::Idle => Style::new().fg(Color::Blue),
                BarState::Active => Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                BarState::Sorted => Style::new().fg(Color::Green),
            },
            ColorDepth::Full => match state {
                BarState::Idle => Style::new().fg(magnitude(value, max)),
                BarState::Active => Style::new()
                    .fg(Color::Rgb(238, 185, 70))
                    .add_modifier(Modifier::BOLD),
                BarState::Sorted => Style::new().fg(Color::Rgb(90, 190, 125)),
            },
        }
    }

    /// Dimmed style for labels and chrome.
    pub fn muted(&self) -> Style {
        match self.depth {
            ColorDepth::Mono => Style::new().add_modifier(Modifier::DIM),
            ColorDepth::Ansi16 => Style::new().fg(Color::DarkGray),
            ColorDepth::Full => Style::new().fg(Color::Rgb(128, 136, 150)),
        }
    }

    /// Style for headings and emphasised values.
    pub fn strong(&self) -> Style {
        match self.depth {
            ColorDepth::Mono => Style::new().add_modifier(Modifier::BOLD),
            ColorDepth::Ansi16 => Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            ColorDepth::Full => Style::new()
                .fg(Color::Rgb(228, 232, 240))
                .add_modifier(Modifier::BOLD),
        }
    }

    /// Accent style, for the current algorithm name and active keybinds.
    pub fn accent(&self) -> Style {
        match self.depth {
            ColorDepth::Mono => Style::new().add_modifier(Modifier::BOLD),
            ColorDepth::Ansi16 => Style::new().fg(Color::Cyan),
            ColorDepth::Full => Style::new().fg(Color::Rgb(110, 200, 225)),
        }
    }
}

/// Blue-to-cyan ramp encoding magnitude.
///
/// Having colour track value makes the sort visibly converge: a sorted array is
/// a smooth gradient, and any remaining disorder shows up as a colour
/// discontinuity even where the height difference is a single cell.
fn magnitude(value: u32, max: u32) -> Color {
    let t = if max == 0 {
        0.0
    } else {
        f64::from(value) / f64::from(max)
    };
    let lerp = |a: f64, b: f64| (a + (b - a) * t).round() as u8;
    Color::Rgb(lerp(48.0, 112.0), lerp(88.0, 214.0), lerp(172.0, 228.0))
}

fn detect_depth() -> ColorDepth {
    // https://no-color.org: any non-empty value means no colour.
    if env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return ColorDepth::Mono;
    }

    let term = env::var("TERM").unwrap_or_default();
    if term.is_empty() || term == "dumb" {
        return ColorDepth::Mono;
    }

    let colorterm = env::var("COLORTERM").unwrap_or_default();
    if colorterm.eq_ignore_ascii_case("truecolor") || colorterm.eq_ignore_ascii_case("24bit") {
        return ColorDepth::Full;
    }
    if term.contains("256color") || term.contains("direct") {
        return ColorDepth::Full;
    }

    ColorDepth::Ansi16
}

fn detect_unicode() -> bool {
    // The locale variables are the only portable hint available. Checked in the
    // precedence order the C library uses.
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|key| env::var(key).ok())
        .find(|value| !value.is_empty())
        .is_some_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("utf-8") || value.contains("utf8")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eighth_is_total_and_clamps() {
        let theme = Theme {
            depth: ColorDepth::Full,
            unicode: true,
        };
        assert_eq!(theme.eighth(0), ' ');
        assert_eq!(theme.eighth(8), '█');
        // Out-of-range input saturates rather than panicking, so a rounding
        // slip in the renderer cannot take the whole app down.
        assert_eq!(theme.eighth(99), '█');
    }

    #[test]
    fn ascii_theme_emits_only_ascii() {
        let theme = Theme::minimal();
        for n in 0..=8 {
            assert!(theme.eighth(n).is_ascii(), "eighth({n}) was not ASCII");
        }
        assert!(theme.line().is_ascii());
        assert!(theme.track_full().is_ascii());
    }

    #[test]
    fn magnitude_handles_zero_max() {
        // An all-zero array must not divide by zero.
        assert_eq!(magnitude(0, 0), Color::Rgb(48, 88, 172));
    }

    #[test]
    fn magnitude_spans_the_ramp() {
        assert_eq!(magnitude(0, 10), Color::Rgb(48, 88, 172));
        assert_eq!(magnitude(10, 10), Color::Rgb(112, 214, 228));
    }

    #[test]
    fn mono_distinguishes_states_without_colour() {
        let theme = Theme::minimal();
        let idle = theme.bar(5, 10, BarState::Idle);
        let active = theme.bar(5, 10, BarState::Active);
        let sorted = theme.bar(5, 10, BarState::Sorted);

        assert_ne!(idle, active);
        assert_ne!(idle, sorted);
        assert_ne!(active, sorted);
        assert!(idle.fg.is_none(), "mono theme must not set a colour");
        assert!(active.fg.is_none(), "mono theme must not set a colour");
        assert!(sorted.fg.is_none(), "mono theme must not set a colour");
    }
}
