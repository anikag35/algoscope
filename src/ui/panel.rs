//! The metrics sidebar, the status line, the scrubber, and the help overlay.
//!
//! The sidebar is the part that makes this more than an animation: it shows the
//! work actually performed next to the textbook bound for the same `n`, so the
//! gap between `O(n²)` as a phrase and 1,128 comparisons as a number is visible
//! while the sort runs.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Widget};

use super::theme::Theme;
use crate::app::App;

/// The metrics sidebar.
pub fn metrics(app: &App, theme: &Theme) -> Paragraph<'static> {
    let playback = app.playback();
    let frame = playback.frame();
    let trace = playback.trace();
    let complexity = &trace.complexity;
    let n = app.size();

    let mut lines = vec![
        heading("ALGORITHM", theme),
        Line::from(Span::styled(trace.name.to_string(), theme.accent())),
        Line::default(),
        heading("COMPLEXITY", theme),
    ];

    for (label, value) in [
        ("best", complexity.best),
        ("average", complexity.average),
        ("worst", complexity.worst),
        ("space", complexity.space),
    ] {
        lines.push(field(label, &theme.text(value), theme));
    }
    lines.push(field(
        "stable",
        if complexity.stable { "yes" } else { "no" },
        theme,
    ));

    lines.push(Line::default());
    lines.push(heading("WORK DONE", theme));
    lines.push(field("comparisons", &commas(frame.comparisons), theme));
    lines.push(field("writes", &commas(frame.writes), theme));
    lines.push(field(
        "step",
        &format!(
            "{} / {}",
            commas(playback.cursor() as u64),
            commas(playback.len() as u64)
        ),
        theme,
    ));

    lines.push(Line::default());
    lines.push(heading(&format!("BOUNDS (n = {n})"), theme));
    lines.push(field(
        &theme.text("n log n ≈"),
        &commas(n_log_n(n)),
        theme,
    ));
    lines.push(field("n(n-1)/2 =", &commas(pairs(n)), theme));

    Paragraph::new(lines).block(
        Block::new()
            .borders(Borders::LEFT)
            .border_set(theme.border_set())
            .border_style(theme.muted())
            .padding(Padding::horizontal(2)),
    )
}

/// The one-line status bar.
pub fn status(app: &App, theme: &Theme) -> Line<'static> {
    let playback = app.playback();

    let (marker, label) = if app.playing() {
        (theme.text("▶").into_owned(), "playing")
    } else if playback.at_end() {
        ("=".to_string(), "done")
    } else {
        ("|".to_string(), "paused")
    };

    let separator = || Span::styled(theme.text("  ·  ").into_owned(), theme.muted());

    // The algorithm name leads, because on a narrow terminal the sidebar is
    // dropped and this line is the only place it appears.
    Line::from(vec![
        Span::styled(playback.trace().name.to_string(), theme.accent()),
        separator(),
        Span::raw(format!("{marker} {label}")),
        separator(),
        Span::raw(format!("{} items", app.size())),
        separator(),
        Span::raw(format!(
            "speed {}/{}",
            app.speed_level(),
            app.speed_levels()
        )),
        separator(),
        Span::styled(format!("seed {}", app.seed()), theme.muted()),
        separator(),
        Span::styled("? keys".to_string(), theme.muted()),
    ])
}

/// A one-row progress track for the position in the trace.
pub struct Scrubber<'a> {
    progress: f64,
    theme: &'a Theme,
}

impl<'a> Scrubber<'a> {
    pub fn new(progress: f64, theme: &'a Theme) -> Self {
        Self { progress, theme }
    }
}

impl Widget for Scrubber<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = area.width as usize;
        let filled = (self.progress.clamp(0.0, 1.0) * width as f64).round() as usize;

        for x in 0..width {
            let (symbol, style) = if x < filled {
                (self.theme.track_full(), self.theme.accent())
            } else {
                (self.theme.line(), self.theme.muted())
            };
            if let Some(cell) = buf.cell_mut((area.x + x as u16, area.y)) {
                cell.set_char(symbol).set_style(style);
            }
        }
    }
}

/// The keybind overlay.
pub fn help(theme: &Theme) -> Paragraph<'static> {
    const BINDS: &[(&str, &str)] = &[
        ("space / enter", "play or pause"),
        ("→ / l", "one step forward"),
        ("← / h", "one step back"),
        ("pgup / pgdn", "scrub by a tenth of the run"),
        ("home / g", "jump to the start"),
        ("end / G", "jump to the end"),
        ("↑ / k", "faster"),
        ("↓ / j", "slower"),
        ("tab", "next algorithm, same array"),
        ("1 .. 3", "pick an algorithm"),
        ("r", "reshuffle"),
        ("+ / -", "grow or shrink the array"),
        ("?", "close this"),
        ("q / esc", "quit"),
    ];

    let mut lines = vec![
        Line::from(Span::styled(
            "Switching algorithm keeps the same array, so the".to_string(),
            theme.muted(),
        )),
        Line::from(Span::styled(
            "comparison counts are directly comparable.".to_string(),
            theme.muted(),
        )),
        Line::default(),
    ];

    for (keys, description) in BINDS {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<16}", theme.text(keys).into_owned()),
                theme.accent(),
            ),
            Span::raw(description.to_string()),
        ]));
    }

    Paragraph::new(lines).block(
        Block::bordered()
            .title(" keys ")
            .border_set(theme.border_set())
            .border_style(theme.muted())
            .title_style(theme.strong())
            .padding(Padding::new(2, 2, 1, 1)),
    )
}

fn heading(text: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), theme.strong()))
}

fn field(label: &str, value: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<13}"), theme.muted()),
        Span::raw(value.to_string()),
    ])
}

/// `n log2 n`, the comparison bound a good sort would hit.
fn n_log_n(n: usize) -> u64 {
    if n < 2 {
        return 0;
    }
    let n = n as f64;
    (n * n.log2()).round() as u64
}

/// `n(n-1)/2`, the number of distinct pairs, which the quadratic sorts
/// approach.
fn pairs(n: usize) -> u64 {
    let n = n as u64;
    n * n.saturating_sub(1) / 2
}

/// Group digits in threes.
///
/// The counters reach six figures on a few hundred elements, and unseparated
/// digits at that length are genuinely hard to compare at a glance.
fn commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commas_group_digits() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(7), "7");
        assert_eq!(commas(999), "999");
        assert_eq!(commas(1_000), "1,000");
        assert_eq!(commas(12_345), "12,345");
        assert_eq!(commas(1_234_567), "1,234,567");
        assert_eq!(commas(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn bounds_are_right_for_known_n() {
        assert_eq!(pairs(0), 0);
        assert_eq!(pairs(1), 0);
        assert_eq!(pairs(5), 10);
        assert_eq!(pairs(48), 1_128);

        assert_eq!(n_log_n(0), 0);
        assert_eq!(n_log_n(1), 0);
        // 8 * 3 = 24
        assert_eq!(n_log_n(8), 24);
    }

    #[test]
    fn selection_sort_hits_the_pair_bound_exactly() {
        // The panel claims n(n-1)/2 is what the quadratic sorts approach.
        // Selection sort should land on it precisely, for any input.
        use crate::algo::sort;
        let n = 24;
        let trace = sort::selection((1..=n as u32).rev().collect());
        assert_eq!(trace.frame_at(trace.len()).comparisons, pairs(n));
    }
}
