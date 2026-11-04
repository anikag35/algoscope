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

use crate::algo::grid::{Status, Terrain};

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

/// What a grid cell is, where that overrides its terrain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellRole {
    Plain,
    Start,
    Goal,
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

    /// The two-column glyph pair and style for one grid cell.
    ///
    /// Cells are two columns wide so that they read as roughly square: a
    /// terminal cell is about twice as tall as it is wide, so one column per
    /// grid cell would render every map squashed horizontally.
    ///
    /// Where there is colour, both columns carry the terrain glyph and the
    /// colour carries the search status — so a rough cell stays visibly rough
    /// even after the frontier has passed over it, which is what shows *why*
    /// Dijkstra routed around it. With no colour available that trick is gone,
    /// so the second column becomes a status marker instead; terrain in the
    /// first column, progress in the second.
    pub fn grid_cell(
        &self,
        terrain: Terrain,
        status: Status,
        role: CellRole,
    ) -> ([char; 2], Style) {
        let base = match role {
            CellRole::Start => 'S',
            CellRole::Goal => 'G',
            CellRole::Plain => match terrain {
                Terrain::Wall => if self.unicode { '\u{2588}' } else { '#' },
                Terrain::Rough => if self.unicode { '\u{2592}' } else { ':' },
                Terrain::Open => if self.unicode { '\u{00b7}' } else { '.' },
            },
        };

        let glyphs = if self.depth == ColorDepth::Mono {
            [base, status_marker(status)]
        } else if role == CellRole::Plain {
            [base, base]
        } else {
            [base, ' ']
        };

        (glyphs, self.grid_style(terrain, status))
    }

    fn grid_style(&self, terrain: Terrain, status: Status) -> Style {
        match self.depth {
            // Glyphs carry the state here, so only the untouched map is
            // pushed into the background.
            ColorDepth::Mono => match status {
                Status::Unseen => Style::new().add_modifier(Modifier::DIM),
                Status::Path => Style::new().add_modifier(Modifier::BOLD),
                _ => Style::new(),
            },
            ColorDepth::Ansi16 => {
                let color = match status {
                    Status::Unseen => match terrain {
                        Terrain::Wall | Terrain::Open => Color::DarkGray,
                        Terrain::Rough => Color::Yellow,
                    },
                    Status::Frontier => Color::Cyan,
                    Status::Expanding => Color::Yellow,
                    Status::Settled => Color::Blue,
                    Status::Path => Color::Green,
                };
                let mut style = Style::new().fg(color);
                if matches!(status, Status::Path | Status::Expanding) {
                    style = style.add_modifier(Modifier::BOLD);
                }
                // Walls stay solid; open and rough ground recede.
                if status == Status::Unseen && terrain != Terrain::Wall {
                    style = style.add_modifier(Modifier::DIM);
                }
                style
            }
            ColorDepth::Full => {
                let color = match status {
                    Status::Unseen => match terrain {
                        Terrain::Wall => Color::Rgb(76, 80, 92),
                        Terrain::Rough => Color::Rgb(124, 100, 58),
                        Terrain::Open => Color::Rgb(54, 58, 68),
                    },
                    Status::Frontier => Color::Rgb(110, 200, 225),
                    Status::Expanding => Color::Rgb(240, 186, 70),
                    Status::Settled => Color::Rgb(72, 104, 140),
                    Status::Path => Color::Rgb(96, 206, 132),
                };
                let mut style = Style::new().fg(color);
                if matches!(status, Status::Path | Status::Expanding) {
                    style = style.add_modifier(Modifier::BOLD);
                }
                style
            }
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

/// The second-column marker that stands in for colour in monochrome.
fn status_marker(status: Status) -> char {
    match status {
        Status::Unseen => ' ',
        Status::Frontier => '+',
        Status::Expanding => '>',
        Status::Settled => '-',
        Status::Path => '*',
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
    let no_color = env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    depth_from(
        no_color,
        &env::var("TERM").unwrap_or_default(),
        &env::var("COLORTERM").unwrap_or_default(),
    )
}

/// The colour decision, separated from the environment so it can be tested
/// without mutating process-wide state from several threads at once.
fn depth_from(no_color: bool, term: &str, colorterm: &str) -> ColorDepth {
    if no_color {
        return ColorDepth::Mono;
    }
    if term.is_empty() || term == "dumb" {
        return ColorDepth::Mono;
    }
    if colorterm.eq_ignore_ascii_case("truecolor") || colorterm.eq_ignore_ascii_case("24bit") {
        return ColorDepth::Full;
    }
    if term.contains("256color") || term.contains("direct") {
        return ColorDepth::Full;
    }
    ColorDepth::Ansi16
}

fn detect_unicode() -> bool {
    // Checked in the precedence order the C library uses.
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|key| env::var(key).ok())
        .find(|value| !value.is_empty());

    unicode_from(
        &env::var("TERM").unwrap_or_default(),
        locale.as_deref(),
    )
}

/// The glyph decision, separated from the environment for the same reason as
/// [`depth_from`].
///
/// Assumes UTF-8 and looks only for positive evidence against it. Requiring a
/// UTF-8 locale looks more careful but is wrong in the most common case: macOS
/// routinely leaves `LANG` unset while running a perfectly capable
/// Terminal.app, so demanding it would hand every default Mac shell the ASCII
/// fallback. The locale is in any case a poor proxy for what the terminal can
/// draw — it describes what the C library should assume about byte encoding,
/// while we emit UTF-8 bytes either way and the emulator renders them on their
/// own merits.
fn unicode_from(term: &str, locale: Option<&str>) -> bool {
    // No terminal, or one that has said it can do nothing. The Linux virtual
    // console is here because its default font has no block-element glyphs,
    // whatever its encoding.
    if term.is_empty() || term == "dumb" || term == "linux" {
        return false;
    }

    // An explicit C or POSIX locale, or one naming a charset that is not
    // UTF-8, is a deliberate statement and is honoured.
    if let Some(locale) = locale {
        let locale = locale.to_ascii_lowercase();
        if locale == "c" || locale == "posix" {
            return false;
        }
        if let Some((_, charset)) = locale.split_once('.') {
            return charset.contains("utf-8") || charset.contains("utf8");
        }
        // A locale with no charset suffix, such as `en_US`, says nothing
        // either way, so it does not override the default.
    }

    true
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
    fn unicode_is_assumed_unless_something_says_otherwise() {
        // The case that matters most: a stock macOS shell exports no locale at
        // all, and it must still get the real glyphs.
        assert!(unicode_from("xterm-256color", None));
        assert!(unicode_from("xterm-256color", Some("en_US.UTF-8")));
        assert!(unicode_from("screen", Some("en_US.utf8")));
        // A locale with no charset suffix says nothing either way.
        assert!(unicode_from("xterm", Some("en_US")));
        assert!(unicode_from("xterm", Some("")));
    }

    #[test]
    fn unicode_yields_to_positive_evidence_against_it() {
        assert!(!unicode_from("", None), "no TERM at all");
        assert!(!unicode_from("dumb", Some("en_US.UTF-8")), "TERM=dumb");
        assert!(!unicode_from("linux", None), "no block glyphs in that font");
        assert!(!unicode_from("xterm-256color", Some("C")));
        assert!(!unicode_from("xterm-256color", Some("POSIX")));
        assert!(!unicode_from("xterm-256color", Some("en_US.ISO8859-1")));
    }

    #[test]
    fn colour_depth_reads_the_usual_signals() {
        assert_eq!(
            depth_from(false, "xterm-256color", ""),
            ColorDepth::Full,
            "256color in TERM"
        );
        assert_eq!(
            depth_from(false, "xterm", "truecolor"),
            ColorDepth::Full,
            "COLORTERM"
        );
        assert_eq!(depth_from(false, "xterm", "24bit"), ColorDepth::Full);
        assert_eq!(depth_from(false, "xterm", ""), ColorDepth::Ansi16);
        assert_eq!(depth_from(false, "vt100", ""), ColorDepth::Ansi16);
    }

    #[test]
    fn no_color_wins_over_everything() {
        // https://no-color.org is explicit that the variable's presence is the
        // signal, and it must beat even a truecolor COLORTERM.
        assert_eq!(
            depth_from(true, "xterm-256color", "truecolor"),
            ColorDepth::Mono
        );
        assert_eq!(depth_from(false, "dumb", "truecolor"), ColorDepth::Mono);
        assert_eq!(depth_from(false, "", ""), ColorDepth::Mono);
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
