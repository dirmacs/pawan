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
            track_color: Color::DarkGray,
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

        let col = area
            .x
            .saturating_add(area.width.saturating_sub(1));
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
