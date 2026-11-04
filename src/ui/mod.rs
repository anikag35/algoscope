//! Rendering: the layout, and the widgets that fill it.
//!
//! Every piece of chrome is optional. The sidebar is dropped on a narrow
//! terminal and the scrubber on a short one, in that order, because the bars
//! are the point and everything else is commentary. Nothing here resizes the
//! array to fit the terminal, so a resize mid-run never disturbs the sort.

pub mod bars;
pub mod panel;
pub mod theme;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Clear, Paragraph};

use self::bars::Bars;
use self::panel::Scrubber;
use self::theme::Theme;
use crate::app::App;

/// Width given to the metrics sidebar.
const SIDEBAR_WIDTH: u16 = 32;

/// Total width below which the sidebar is dropped, so the bars keep enough
/// room to be legible.
const SIDEBAR_MIN_TOTAL_WIDTH: u16 = 76;

/// Height below which the scrubber is dropped.
const SCRUBBER_MIN_HEIGHT: u16 = 8;

/// Below this there is no useful rendering to do.
const MIN_WIDTH: u16 = 16;
const MIN_HEIGHT: u16 = 2;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        // Say so rather than drawing something unreadable.
        frame.render_widget(
            Paragraph::new("too small").style(theme.muted()),
            area,
        );
        return;
    }

    let show_scrubber = area.height >= SCRUBBER_MIN_HEIGHT;

    let rows = if show_scrubber {
        Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area)
    } else {
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area)
    };

    let content = rows[0];
    let status_row = rows[rows.len() - 1];

    let chart = if area.width >= SIDEBAR_MIN_TOTAL_WIDTH {
        let columns = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(SIDEBAR_WIDTH),
        ])
        .split(content);
        frame.render_widget(panel::metrics(app, theme), columns[1]);
        columns[0]
    } else {
        content
    };

    frame.render_widget(Bars::new(app.playback().frame(), theme), chart);

    if show_scrubber {
        frame.render_widget(Scrubber::new(app.playback().progress(), theme), rows[1]);
    }

    frame.render_widget(panel::status(app, theme), status_row);

    if app.show_help() {
        let overlay = centered(area, 50, 22);
        // Clear a column either side as well as the box itself. Without the
        // gutter the border sits flush against whatever is behind it, and
        // `┌ keys ─┐le sort` reads as corruption rather than as a panel
        // floating above the chart.
        frame.render_widget(Clear, with_gutter(overlay, area));
        frame.render_widget(panel::help(theme), overlay);
    }
}

/// Widen a rect by one column on each side, clamped to `bounds`.
fn with_gutter(rect: Rect, bounds: Rect) -> Rect {
    let left = rect.x.saturating_sub(1).max(bounds.x);
    let right = rect.x.saturating_add(rect.width).saturating_add(1);
    let right = right.min(bounds.x.saturating_add(bounds.width));
    Rect {
        x: left,
        y: rect.y,
        width: right.saturating_sub(left),
        height: rect.height,
    }
}

/// Centre a `width` by `height` box in `area`, shrinking it to fit.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::algo::sort::Algorithm;

    /// Render at a given size and return the screen as lines of text.
    fn render_at(width: u16, height: u16, help: bool) -> Vec<String> {
        let mut app = App::new(Algorithm::Bubble, 24, 3);
        // Part-way through, so bars, counters and the scrubber all have
        // something non-trivial to show.
        for _ in 0..40 {
            app.handle_key(ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Right,
                ratatui::crossterm::event::KeyModifiers::NONE,
            ));
        }
        if help {
            app.handle_key(ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char('?'),
                ratatui::crossterm::event::KeyModifiers::NONE,
            ));
        }

        let theme = Theme::minimal();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, &app, &theme))
            .unwrap();

        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn wide_terminal_shows_the_sidebar() {
        let screen = render_at(120, 30, false).join("\n");
        assert!(screen.contains("COMPLEXITY"), "sidebar missing:\n{screen}");
        assert!(screen.contains("Bubble sort"));
        assert!(screen.contains("comparisons"));
    }

    #[test]
    fn narrow_terminal_drops_the_sidebar_but_keeps_the_name() {
        let screen = render_at(48, 20, false).join("\n");
        assert!(
            !screen.contains("COMPLEXITY"),
            "sidebar should be dropped at 48 columns:\n{screen}"
        );
        assert!(
            screen.contains("Bubble sort"),
            "the status line should still name the algorithm:\n{screen}"
        );
    }

    #[test]
    fn short_terminal_drops_the_scrubber() {
        // Four rows: bars plus a status line, and nothing else.
        let screen = render_at(100, 4, false);
        assert!(screen.last().unwrap().contains("Bubble sort"));
    }

    #[test]
    fn help_overlay_covers_the_bars() {
        let screen = render_at(100, 30, true).join("\n");
        assert!(screen.contains("play or pause"), "help missing:\n{screen}");
        assert!(screen.contains("keys"));
    }

    #[test]
    fn extreme_sizes_do_not_panic() {
        for (w, h) in [(1, 1), (2, 2), (16, 2), (300, 2), (20, 100), (1, 100)] {
            let _ = render_at(w, h, false);
            let _ = render_at(w, h, true);
        }
    }

    #[test]
    fn ascii_theme_renders_no_wide_or_non_ascii_glyphs() {
        // A terminal that is not in a UTF-8 locale must receive only ASCII,
        // since anything else risks mojibake or a width mismatch that
        // misaligns every column after it.
        let screen = render_at(100, 30, false);
        for line in &screen {
            assert!(
                line.is_ascii(),
                "non-ASCII output from the minimal theme: {line:?}"
            );
        }
    }
}
