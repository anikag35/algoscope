//! The metrics sidebar, the status line, the scrubber, and the help overlay.
//!
//! The sidebar is the part that makes this more than an animation: it shows the
//! work actually performed next to the textbook bound for the same input, so
//! the gap between `O(n²)` as a phrase and 1,128 comparisons as a number is
//! visible while the algorithm runs. In grid mode the equivalent is A\*'s
//! expansion count sitting next to Dijkstra's on the same map.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Widget};

use super::theme::Theme;
use crate::algo::grid::GridState;
use crate::algo::sort::SortState;
use crate::app::{App, Mode, View};

/// The metrics sidebar.
pub fn metrics(app: &App, theme: &Theme) -> Paragraph<'static> {
    let complexity = app.complexity();

    let mut lines = vec![
        heading("ALGORITHM", theme),
        Line::from(Span::styled(
            app.algorithm_name().to_string(),
            theme.accent(),
        )),
        // The step counter sits at the top because the sidebar is the first
        // thing a short terminal clips, and position is the one field that
        // changes every frame.
        field(
            "step",
            &format!(
                "{} / {}",
                commas(app.cursor() as u64),
                commas(app.len() as u64)
            ),
            theme,
        ),
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
    // The note is prose, not a measurement, so it gets the panel's full width
    // rather than being squeezed into the value column and truncated.
    lines.push(Line::from(Span::styled(
        theme.text(complexity.note).into_owned(),
        theme.muted(),
    )));
    lines.push(Line::default());

    match app.view() {
        View::Sort(state) => sort_metrics(&mut lines, state, theme),
        View::Grid { state, .. } => grid_metrics(&mut lines, state, app.at_end(), theme),
    }

    Paragraph::new(lines).block(
        Block::new()
            .borders(Borders::LEFT)
            .border_set(theme.border_set())
            .border_style(theme.muted())
            .padding(Padding::horizontal(2)),
    )
}

fn sort_metrics(lines: &mut Vec<Line<'static>>, state: &SortState, theme: &Theme) {
    let n = state.values.len();

    lines.push(heading("WORK DONE", theme));
    lines.push(field("comparisons", &commas(state.comparisons), theme));
    lines.push(field("writes", &commas(state.writes), theme));

    lines.push(Line::default());
    lines.push(heading(&format!("BOUNDS (n = {n})"), theme));
    lines.push(field(&theme.text("n log n ≈"), &commas(n_log_n(n)), theme));
    lines.push(field("n(n-1)/2 =", &commas(pairs(n)), theme));
}

fn grid_metrics(
    lines: &mut Vec<Line<'static>>,
    state: &GridState,
    finished: bool,
    theme: &Theme,
) {
    let cells = state.grid.len();

    lines.push(heading("SEARCH", theme));
    lines.push(field("expanded", &commas(state.expanded), theme));
    lines.push(field("examined", &commas(state.examined), theme));
    lines.push(field("relaxed", &commas(state.relaxations), theme));
    lines.push(field(
        "of cells",
        &format!("{} / {}", commas(state.expanded), commas(cells as u64)),
        theme,
    ));

    lines.push(Line::default());
    lines.push(heading("ROUTE", theme));
    if state.has_path() {
        lines.push(field("tiles", &commas(u64::from(state.path_cells)), theme));
        lines.push(field("cost", &commas(u64::from(state.path_cost)), theme));
    } else if !finished {
        // Mid-search there is simply no answer yet, which is a different thing
        // from there being no route at all.
        lines.push(Line::from(Span::styled(
            "searching".to_string(),
            theme.muted(),
        )));
    } else {
        // An unreachable goal is a real result, not an error, and the panel
        // says so rather than showing a misleading zero.
        lines.push(Line::from(Span::styled(
            "no route".to_string(),
            theme.muted(),
        )));
    }
}

/// The one-line status bar.
pub fn status(app: &App, theme: &Theme) -> Line<'static> {
    let (marker, label) = if app.editing() {
        ("#".to_string(), "editing")
    } else if app.playing() {
        (theme.text("▶").into_owned(), "playing")
    } else if app.at_end() {
        ("=".to_string(), "done")
    } else {
        ("|".to_string(), "paused")
    };

    let separator = || Span::styled(theme.text("  ·  ").into_owned(), theme.muted());

    // The algorithm name leads, because on a narrow terminal the sidebar is
    // dropped and this line is the only place it appears.
    Line::from(vec![
        Span::styled(app.algorithm_name().to_string(), theme.accent()),
        separator(),
        Span::raw(format!("{marker} {label}")),
        separator(),
        Span::raw(app.extent()),
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
pub fn help(app: &App, theme: &Theme) -> Paragraph<'static> {
    const SHARED: &[(&str, &str)] = &[
        ("space / enter", "play or pause"),
        ("→ / l", "one step forward"),
        ("← / h", "one step back"),
        ("pgup / pgdn", "scrub by a tenth"),
        ("home / g", "jump to the start"),
        ("end / G", "jump to the end"),
        ("↑ / k", "faster"),
        ("↓ / j", "slower"),
        ("tab", "next algorithm"),
        ("r", "new input"),
        ("+ / -", "grow or shrink"),
        ("m", "sorting / pathfinding"),
    ];

    const EDIT: &[(&str, &str)] = &[
        ("e", "leave edit mode"),
        ("arrows / hjkl", "move the cursor"),
        ("space", "open / rough / wall"),
        ("s", "put the start here"),
        ("g", "put the goal here"),
    ];

    const GRID: &[(&str, &str)] = &[("e", "edit the terrain")];

    let intro = if app.editing() {
        "Painting re-runs the search and shows the result."
    } else {
        "Changing algorithm keeps the same input, so the counts stay comparable."
    };

    let mut lines = vec![
        Line::from(Span::styled(intro.to_string(), theme.muted())),
        Line::default(),
    ];

    let extra: &[(&str, &str)] = if app.editing() {
        EDIT
    } else if app.mode() == Mode::Grid {
        GRID
    } else {
        &[]
    };

    let count = format!("1 .. {}", app.algorithm_count());
    let binds = SHARED
        .iter()
        .copied()
        .chain(extra.iter().copied())
        .chain([
            (count.as_str(), "pick an algorithm"),
            ("?", "close this"),
            ("q", "quit"),
        ]);

    for (keys, description) in binds {
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

/// How many rows [`help`] wants, so the layout can size its box.
pub fn help_height(app: &App) -> u16 {
    let extra = if app.editing() {
        5
    } else if app.mode() == Mode::Grid {
        1
    } else {
        0
    };
    // 12 shared binds + 3 trailing + intro, blank line, and the border.
    12 + 3 + extra + 5
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
        assert_eq!(trace.state_at(trace.len()).comparisons, pairs(n));
    }
}
