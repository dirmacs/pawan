//! Vertical scrollbar widget for scrollable TUI regions.

#![allow(dead_code)] // integrated by upcoming shell layout wiring

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

const THUMB_CHAR: char = '█';
const TRACK_CHAR: char = '░';

/// Custom vertical scrollbar (track + proportional thumb).
#[derive(Debug, Clone, Copy)]
pub struct Scrollbar {
    pub total_items: usize,
    pub visible_items: usize,
    pub scroll_offset: usize,
    pub thumb_color: Color,
    pub track_color: Color,
}

impl Scrollbar {
    pub fn new(total: usize, visible: usize, offset: usize) -> Self {
        Self {
            total_items: total,
            visible_items: visible,
            scroll_offset: offset,
            thumb_color: Color::Cyan,
            track_color: Color::Gray,
        }
    }

    pub fn with_colors(mut self, thumb: Color, track: Color) -> Self {
        self.thumb_color = thumb;
        self.track_color = track;
        self
    }
}

impl Widget for Scrollbar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.total_items == 0 || self.total_items <= self.visible_items {
            return;
        }
        if area.width == 0 || area.height == 0 {
            return;
        }

        let col = area.x.saturating_add(area.width.saturating_sub(1));
        let area_height = area.height as usize;
        if area_height == 0 {
            return;
        }

        let thumb_h = ((self.visible_items.saturating_mul(area_height))
            .saturating_div(self.total_items))
        .max(1)
        .min(area_height);

        let thumb_y_rel = self
            .scroll_offset
            .saturating_mul(area_height)
            .saturating_div(self.total_items);
        let thumb_y_rel = thumb_y_rel.min(area_height.saturating_sub(thumb_h));

        for row in 0..area_height {
            let y = area.y.saturating_add(row as u16);
            let ch = if row >= thumb_y_rel && row < thumb_y_rel + thumb_h {
                THUMB_CHAR
            } else {
                TRACK_CHAR
            };
            if let Some(cell) = buf.cell_mut((col, y)) {
                if ch == THUMB_CHAR {
                    cell.set_char(ch).set_fg(self.thumb_color);
                } else {
                    cell.set_char(ch).set_fg(self.track_color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_scrollbar_with_defaults() {
        let sb = Scrollbar::new(100, 10, 0);
        assert_eq!(sb.total_items, 100);
        assert_eq!(sb.visible_items, 10);
        assert_eq!(sb.scroll_offset, 0);
        assert_eq!(sb.thumb_color, Color::Cyan);
        assert_eq!(sb.track_color, Color::Gray);
    }

    #[test]
    fn with_colors_sets_custom_colors() {
        let sb = Scrollbar::new(100, 10, 0).with_colors(Color::Red, Color::Blue);
        assert_eq!(sb.thumb_color, Color::Red);
        assert_eq!(sb.track_color, Color::Blue);
    }

    #[test]
    fn render_noop_when_total_zero() {
        let sb = Scrollbar::new(0, 10, 0);
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);

        sb.render(area, &mut buf);

        assert!(buf.content().iter().all(|cell| cell.symbol() == " "));
    }

    #[test]
    fn render_noop_when_total_equals_visible() {
        let sb = Scrollbar::new(10, 10, 0);
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);

        sb.render(area, &mut buf);

        assert!(buf.content().iter().all(|cell| cell.symbol() == " "));
    }

    #[test]
    fn render_noop_when_area_has_zero_width() {
        let sb = Scrollbar::new(100, 10, 0);
        let area = Rect::new(0, 0, 0, 10);
        let mut buf = Buffer::empty(area);

        sb.render(area, &mut buf);

        assert!(buf.content().is_empty());
    }

    #[test]
    fn render_thumb_at_top_with_zero_offset() {
        let sb = Scrollbar::new(100, 10, 0);
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        sb.render(area, &mut buf);
        // thumb_y_rel = (0 * 10) / 100 = 0
        // thumb_h = floor(10 * 10 / 100) = 1
        // First row should be THUMB_CHAR
        assert_eq!(buf[(9, 0)].symbol(), THUMB_CHAR.to_string());
    }

    #[test]
    fn render_uses_track_and_thumb_chars() {
        let sb = Scrollbar::new(100, 10, 50);
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        sb.render(area, &mut buf);

        let thumb_count = (0..area.height)
            .filter(|&y| buf[(9, y)].symbol() == THUMB_CHAR.to_string())
            .count();
        let track_count = (0..area.height)
            .filter(|&y| buf[(9, y)].symbol() == TRACK_CHAR.to_string())
            .count();

        assert!(thumb_count > 0, "Should have thumb cells");
        assert!(track_count > 0, "Should have track cells");
    }

    #[test]
    fn render_produces_correct_thumb_height() {
        let sb = Scrollbar::new(20, 5, 0);
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        sb.render(area, &mut buf);

        let thumb_count = (0..area.height)
            .filter(|&y| buf[(9, y)].symbol() == THUMB_CHAR.to_string())
            .count();

        assert_eq!(thumb_count, 2);
    }

    #[test]
    fn render_thumb_position_calculated_correctly() {
        // At offset 50 with total 100, thumb should be at position 5
        let sb = Scrollbar::new(100, 10, 50);
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        sb.render(area, &mut buf);
        // thumb_y_rel = (50 * 10) / 100 = 5
        // thumb starts at row 5
        assert_eq!(buf[(9, 5)].symbol(), THUMB_CHAR.to_string());
    }
}