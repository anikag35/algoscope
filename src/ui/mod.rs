//! Rendering: the layout, and the widgets that fill it.
//!
//! Every piece of chrome is optional. The sidebar is dropped on a narrow
//! terminal and the scrubber on a short one, in that order, because the
//! visualization is the point and everything else is commentary. Nothing here
//! resizes the data to fit the terminal, so a resize mid-run never disturbs a
//! run in progress.

pub mod bars;
pub mod grid;
pub mod panel;
pub mod theme;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Clear, Paragraph};

use self::bars::Bars;
use self::grid::GridView;
use self::panel::Scrubber;
use self::theme::Theme;
use crate::app::{App, View};

/// Width given to the metrics sidebar.
const SIDEBAR_WIDTH: u16 = 32;

/// Total width below which the sidebar is dropped, so the visualization keeps
/// enough room to be legible.
const SIDEBAR_MIN_TOTAL_WIDTH: u16 = 76;

/// Height below which the scrubber is dropped.
const SCRUBBER_MIN_HEIGHT: u16 = 8;

/// Width of the help overlay.
const HELP_WIDTH: u16 = 52;

/// Below this there is no useful rendering to do.
const MIN_WIDTH: u16 = 16;
const MIN_HEIGHT: u16 = 2;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        // Say so rather than drawing something unreadable.
        frame.render_widget(Paragraph::new("too small").style(theme.muted()), area);
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

    let main = if area.width >= SIDEBAR_MIN_TOTAL_WIDTH {
        let columns = Layout::horizontal([Constraint::Min(1), Constraint::Length(SIDEBAR_WIDTH)])
            .split(content);
        frame.render_widget(panel::metrics(app, theme), columns[1]);
        columns[0]
    } else {
        content
    };

    match app.view() {
        View::Sort(state) => frame.render_widget(Bars::new(state, theme), main),
        View::Grid { state, cursor } => {
            frame.render_widget(GridView::new(state, theme).cursor(cursor), main);
        }
    }

    if show_scrubber {
        frame.render_widget(Scrubber::new(app.progress(), theme), rows[1]);
    }

    frame.render_widget(panel::status(app, theme), status_row);

    if app.show_help() {
        let overlay = centered(area, HELP_WIDTH, panel::help_height(app));
        // Clear a column either side as well as the box itself. Without the
        // gutter the border sits flush against whatever is behind it, and the
        // title row reads as corruption rather than as a panel floating above
        // the view.
        frame.render_widget(Clear, with_gutter(overlay, area));
        frame.render_widget(panel::help(app, theme), overlay);
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

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::app::{Config, Mode};

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn config() -> Config {
        Config {
            size: 24,
            seed: 3,
            grid_width: 20,
            grid_height: 10,
            ..Config::default()
        }
    }

    fn screen_of(app: &App, width: u16, height: u16) -> Vec<String> {
        let theme = Theme::minimal();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app, &theme)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// Render part-way through a run and return the screen as lines of text.
    fn render_at(mode: Mode, width: u16, height: u16, help: bool) -> Vec<String> {
        let mut app = App::new(Config { mode, ..config() });
        for _ in 0..40 {
            press(&mut app, KeyCode::Right);
        }
        if help {
            press(&mut app, KeyCode::Char('?'));
        }
        screen_of(&app, width, height)
    }

    #[test]
    fn wide_terminal_shows_the_sidebar() {
        let screen = render_at(Mode::Sort, 120, 30, false).join("\n");
        assert!(screen.contains("COMPLEXITY"), "sidebar missing:\n{screen}");
        assert!(screen.contains("Bubble sort"));
        assert!(screen.contains("comparisons"));
    }

    #[test]
    fn narrow_terminal_drops_the_sidebar_but_keeps_the_name() {
        let screen = render_at(Mode::Sort, 48, 20, false).join("\n");
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
        let screen = render_at(Mode::Sort, 100, 4, false);
        assert!(screen.last().unwrap().contains("Bubble sort"));
    }

    #[test]
    fn grid_mode_renders_the_map_and_its_own_metrics() {
        let screen = render_at(Mode::Grid, 120, 30, false).join("\n");
        assert!(screen.contains("Breadth-first"), "wrong mode:\n{screen}");
        assert!(screen.contains("SEARCH"), "grid metrics missing:\n{screen}");
        assert!(screen.contains("expanded"));
        assert!(screen.contains('S'), "start not drawn");
        assert!(screen.contains('G'), "goal not drawn");
        // Sorting metrics must not leak into grid mode.
        assert!(!screen.contains("comparisons"));
    }

    #[test]
    fn help_overlay_covers_the_view() {
        let screen = render_at(Mode::Sort, 100, 34, true).join("\n");
        assert!(screen.contains("play or pause"), "help missing:\n{screen}");
        assert!(screen.contains("keys"));
    }

    #[test]
    fn help_lists_grid_keys_only_in_grid_mode() {
        let sorting = render_at(Mode::Sort, 100, 40, true).join("\n");
        assert!(!sorting.contains("edit the terrain"));

        let pathfinding = render_at(Mode::Grid, 100, 40, true).join("\n");
        assert!(
            pathfinding.contains("edit the terrain"),
            "grid help missing:\n{pathfinding}"
        );
    }

    #[test]
    fn help_lists_the_right_number_of_algorithms_per_mode() {
        let sorting = render_at(Mode::Sort, 100, 40, true).join("\n");
        assert!(sorting.contains("1 .. 3"), "{sorting}");
        let pathfinding = render_at(Mode::Grid, 100, 40, true).join("\n");
        assert!(pathfinding.contains("1 .. 4"), "{pathfinding}");
    }

    #[test]
    fn edit_mode_help_replaces_the_playback_keys() {
        let mut app = App::new(Config {
            mode: Mode::Grid,
            ..config()
        });
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('?'));

        let screen = screen_of(&app, 100, 40).join("\n");
        assert!(screen.contains("put the start here"), "{screen}");
        assert!(screen.contains("open / rough / wall"));
    }

    #[test]
    fn extreme_sizes_do_not_panic() {
        for mode in [Mode::Sort, Mode::Grid] {
            for (w, h) in [(1, 1), (2, 2), (16, 2), (300, 2), (20, 100), (1, 100)] {
                let _ = render_at(mode, w, h, false);
                let _ = render_at(mode, w, h, true);
            }
        }
    }

    #[test]
    fn ascii_theme_renders_no_non_ascii_glyphs() {
        // A terminal that is not in a UTF-8 locale must receive only ASCII,
        // since anything else risks mojibake or a width mismatch that
        // misaligns every column after it.
        for mode in [Mode::Sort, Mode::Grid] {
            for help in [false, true] {
                for line in render_at(mode, 100, 30, help) {
                    assert!(
                        line.is_ascii(),
                        "non-ASCII output from the minimal theme: {line:?}"
                    );
                }
            }
        }
    }
}
