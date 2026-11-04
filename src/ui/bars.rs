//! The sorting bar chart.
//!
//! Bars are drawn with vertical eighth-blocks, which gives eight times the
//! vertical resolution of a plain full-block chart. In a 20-row pane that is
//! 160 distinguishable heights instead of 20, so adjacent values stay visually
//! distinct instead of collapsing into equal-looking columns.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use super::theme::{BarState, Theme};
use crate::algo::Frame;

/// Bar chart of a [`Frame`]'s array.
pub struct Bars<'a> {
    frame: &'a Frame,
    theme: &'a Theme,
}

impl<'a> Bars<'a> {
    pub fn new(frame: &'a Frame, theme: &'a Theme) -> Self {
        Self { frame, theme }
    }

    fn state_of(&self, idx: usize) -> BarState {
        // `active` holds at most two entries, so the scan is cheaper than the
        // set it would take to avoid it.
        if self.frame.active.contains(&idx) {
            BarState::Active
        } else if self.frame.sorted[idx] {
            BarState::Sorted
        } else {
            BarState::Idle
        }
    }
}

impl Widget for Bars<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let n = self.frame.values.len();
        let width = area.width as usize;
        let height = area.height as usize;
        if n == 0 || width == 0 || height == 0 {
            return;
        }

        let max = self.frame.values.iter().copied().max().unwrap_or(0);

        // `idx = col * n / width` maps columns to values in both directions at
        // once: with room to spare, consecutive columns land on the same value
        // and bars come out thick; squeezed, some values are skipped. Either
        // way the array is never resized to fit the terminal, so a resize mid
        // sort cannot disturb the run.
        let bar_width = width / n;
        let gapped = bar_width >= 3;

        for col in 0..width {
            let idx = col * n / width;

            // Blank the last column of each run so wide bars read as separate
            // rather than as one solid mass.
            if gapped && (col + 1) * n / width != idx {
                continue;
            }

            let value = self.frame.values[idx];
            let style = self.theme.bar(value, max, self.state_of(idx));
            let eighths = fill_eighths(value, max, height);
            let x = area.x + col as u16;

            for row in 0..height {
                // Bars grow upward, so the bottom row is the last one.
                let from_bottom = height - 1 - row;
                let filled = eighths.saturating_sub(from_bottom * 8);
                if filled == 0 {
                    continue;
                }
                if let Some(cell) = buf.cell_mut((x, area.y + row as u16)) {
                    cell.set_char(self.theme.eighth(filled.min(8)))
                        .set_style(style);
                }
            }
        }
    }
}

/// Height of a bar in eighths of a cell.
///
/// A non-zero value is always given at least one eighth: a short bar that
/// rounds to nothing would read as a hole in the array rather than as a small
/// element.
fn fill_eighths(value: u32, max: u32, height: usize) -> usize {
    if value == 0 || max == 0 {
        return 0;
    }
    let capacity = height * 8;
    let exact = f64::from(value) / f64::from(max) * capacity as f64;
    (exact.round() as usize).clamp(1, capacity)
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;
    use crate::algo::Frame;
    use crate::ui::theme::Theme;

    fn render(values: Vec<u32>, width: u16, height: u16) -> Buffer {
        let frame = Frame::new(values);
        let theme = Theme::minimal();
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        Bars::new(&frame, &theme).render(area, &mut buf);
        buf
    }

    /// How many rows of column `x` are non-blank.
    fn column_height(buf: &Buffer, x: u16, height: u16) -> usize {
        (0..height)
            .filter(|&y| buf[(x, y)].symbol() != " ")
            .count()
    }

    #[test]
    fn tallest_value_fills_the_pane() {
        let buf = render(vec![1, 10], 2, 8);
        assert_eq!(column_height(&buf, 1, 8), 8);
    }

    #[test]
    fn bar_heights_are_ordered_like_their_values() {
        let buf = render(vec![1, 5, 10], 3, 10);
        let heights: Vec<_> = (0..3).map(|x| column_height(&buf, x, 10)).collect();
        assert!(
            heights[0] <= heights[1] && heights[1] <= heights[2],
            "heights {heights:?} do not follow the values"
        );
    }

    #[test]
    fn small_values_still_show() {
        // 1 out of 10_000 rounds to zero eighths, but it is a real element and
        // has to be drawn, or the array looks like it has a gap in it.
        let buf = render(vec![1, 10_000], 2, 10);
        assert_eq!(column_height(&buf, 0, 10), 1);
    }

    #[test]
    fn narrower_than_the_array_does_not_panic() {
        // 100 values into 10 columns: values get skipped, nothing overflows.
        let buf = render((1..=100).collect(), 10, 5);
        assert!((0..10).any(|x| column_height(&buf, x, 5) > 0));
    }

    #[test]
    fn wide_bars_are_separated_by_a_gap() {
        // 2 values across 20 columns is a bar width of 10, so every tenth
        // column is left blank.
        let buf = render(vec![10, 10], 20, 4);
        assert_eq!(column_height(&buf, 9, 4), 0, "expected a gap at column 9");
        assert!(column_height(&buf, 8, 4) > 0, "column 8 should be drawn");
    }

    #[test]
    fn degenerate_areas_are_safe() {
        for (w, h) in [(0, 0), (0, 5), (5, 0), (1, 1)] {
            let _ = render(vec![3, 1, 2], w, h);
        }
        let _ = render(vec![], 10, 10);
        let _ = render(vec![0, 0, 0], 10, 10);
    }
}
