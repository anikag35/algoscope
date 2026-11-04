//! The pathfinding grid view.
//!
//! Each grid cell occupies two terminal columns, so maps read as roughly
//! square rather than squashed. All the colour and glyph decisions live in
//! [`Theme::grid_cell`]; this widget only works out where each cell goes.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::Widget;

use super::theme::{CellRole, Theme};
use crate::algo::grid::{Cell, GridState};

/// Terminal columns per grid cell.
const CELL_WIDTH: u16 = 2;

/// A rendered search in progress.
pub struct GridView<'a> {
    state: &'a GridState,
    theme: &'a Theme,
    /// The edit cursor, drawn reversed when editing terrain.
    cursor: Option<Cell>,
}

impl<'a> GridView<'a> {
    pub fn new(state: &'a GridState, theme: &'a Theme) -> Self {
        Self {
            state,
            theme,
            cursor: None,
        }
    }

    /// Show the edit cursor at `cell`.
    pub fn cursor(mut self, cell: Option<Cell>) -> Self {
        self.cursor = cell;
        self
    }

    /// How many whole cells fit in `area`, which is what the app sizes new
    /// grids to.
    pub fn capacity(area: Rect) -> (u16, u16) {
        (area.width / CELL_WIDTH, area.height)
    }

    fn role_of(&self, cell: Cell) -> CellRole {
        let grid = &self.state.grid;
        if cell == grid.start {
            CellRole::Start
        } else if cell == grid.goal {
            CellRole::Goal
        } else {
            CellRole::Plain
        }
    }
}

impl Widget for GridView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let grid = &self.state.grid;
        if area.width < CELL_WIDTH || area.height == 0 || grid.is_empty() {
            return;
        }

        // Clip rather than scale when the pane is too small. The grid is never
        // resized to fit the terminal, for the same reason the sorting array is
        // not: a resize mid-run would invalidate the trace being replayed.
        let (fits_wide, fits_high) = Self::capacity(area);
        let columns = fits_wide.min(grid.width());
        let rows = fits_high.min(grid.height());
        if columns == 0 || rows == 0 {
            return;
        }

        // Centre whatever is shown in the pane.
        let origin_x = area.x + (area.width - columns * CELL_WIDTH) / 2;
        let origin_y = area.y + (area.height - rows) / 2;

        for row in 0..rows {
            for column in 0..columns {
                let cell = grid.cell(column, row);
                let (glyphs, mut style) = self.theme.grid_cell(
                    grid.terrain(cell),
                    self.state.status(cell),
                    self.role_of(cell),
                );

                if self.cursor == Some(cell) {
                    style = style.add_modifier(Modifier::REVERSED);
                }

                for (offset, &glyph) in glyphs.iter().enumerate() {
                    let x = origin_x + column * CELL_WIDTH + offset as u16;
                    if let Some(target) = buf.cell_mut((x, origin_y + row)) {
                        target.set_char(glyph).set_style(style);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::grid::search::Algorithm;
    use crate::algo::grid::{Grid, Status, Terrain};
    use crate::ui::theme::{ColorDepth, Theme};

    fn render(state: &GridState, theme: &Theme, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        GridView::new(state, theme).render(area, &mut buf);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// A finished search, which exercises every status at once.
    fn finished() -> GridState {
        let trace = Algorithm::AStar.run(Grid::generate(20, 10, 4));
        trace.state_at(trace.len())
    }

    #[test]
    fn start_and_goal_are_labelled() {
        let state = finished();
        let theme = Theme::minimal();
        let screen = render(&state, &theme, 40, 10).join("\n");
        assert!(screen.contains('S'), "start not drawn:\n{screen}");
        assert!(screen.contains('G'), "goal not drawn:\n{screen}");
    }

    #[test]
    fn monochrome_marks_status_in_the_second_column() {
        // With no colour, the marker column is the only thing distinguishing a
        // settled cell from an untouched one.
        let state = finished();
        let screen = render(&state, &Theme::minimal(), 40, 10).join("\n");
        assert!(screen.contains('*'), "no path marker:\n{screen}");
        assert!(
            screen.contains('-') || screen.contains('+'),
            "no search progress markers:\n{screen}"
        );
    }

    #[test]
    fn colour_themes_keep_terrain_visible_under_the_search() {
        // Both columns stay the terrain glyph, so a rough cell still reads as
        // rough after the frontier has crossed it.
        let mut grid = Grid::open(8, 5);
        let rough = grid.cell(3, 2);
        grid.set(rough, Terrain::Rough);
        let mut state = GridState::new(grid);
        state.status[rough as usize] = Status::Settled;

        let theme = Theme {
            depth: ColorDepth::Full,
            unicode: true,
        };
        let screen = render(&state, &theme, 16, 5).join("\n");
        assert!(
            screen.contains('\u{2592}'),
            "rough terrain lost under the search:\n{screen}"
        );
    }

    #[test]
    fn ascii_theme_emits_only_ascii() {
        let state = finished();
        for line in render(&state, &Theme::minimal(), 40, 10) {
            assert!(line.is_ascii(), "non-ASCII from the minimal theme: {line:?}");
        }
    }

    #[test]
    fn cells_are_two_columns_wide() {
        let grid = Grid::open(4, 3);
        let state = GridState::new(grid);
        let theme = Theme {
            depth: ColorDepth::Full,
            unicode: false,
        };
        // 4 cells at 2 columns each fills exactly 8 of the 8 available.
        let screen = render(&state, &theme, 8, 3);
        assert_eq!(screen[0].trim_end().len(), 8);
    }

    #[test]
    fn capacity_reports_whole_cells_only() {
        assert_eq!(GridView::capacity(Rect::new(0, 0, 80, 20)), (40, 20));
        // An odd width cannot host half a cell.
        assert_eq!(GridView::capacity(Rect::new(0, 0, 81, 20)), (40, 20));
    }

    #[test]
    fn a_pane_smaller_than_the_grid_clips_without_panicking() {
        let state = finished();
        let theme = Theme::minimal();
        for (w, h) in [(1, 1), (2, 1), (6, 3), (39, 9), (41, 11)] {
            let _ = render(&state, &theme, w, h);
        }
    }

    #[test]
    fn the_edit_cursor_is_drawn() {
        let grid = Grid::open(6, 4);
        let state = GridState::new(grid);
        let theme = Theme::minimal();
        let area = Rect::new(0, 0, 12, 4);

        let target = state.grid.cell(2, 1);
        let mut plain = Buffer::empty(area);
        GridView::new(&state, &theme).render(area, &mut plain);
        let mut with_cursor = Buffer::empty(area);
        GridView::new(&state, &theme)
            .cursor(Some(target))
            .render(area, &mut with_cursor);

        assert_ne!(
            plain[(4, 1)].style(),
            with_cursor[(4, 1)].style(),
            "the cursor cell should be styled differently"
        );
        assert_eq!(
            plain[(0, 0)].style(),
            with_cursor[(0, 0)].style(),
            "other cells should be untouched"
        );
    }
}
