//! Terminal layout helpers for the interactive UI.
//!
//! Layout: full-width chat on top, queue + input in the middle, status bar at
//! the bottom. No side activity panel — tool activity is shown inline in the
//! chat stream, matching the maki-ui design language.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Height of the bottom status strip (model, mode, tokens).
pub const STATUS_BAR_HEIGHT: u16 = 1;
/// Default height reserved for the input widget (borderless, 1-line min + 2 padding).
pub const INPUT_BASE_HEIGHT: u16 = 3;
/// Height of a single queue row.
pub const QUEUE_ITEM_HEIGHT: u16 = 1;

/// Primary layout regions for the main chat view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewLayout {
    /// Main transcript / message list (full width).
    pub msg_area: Rect,
    /// Sub-agent task queue (may be zero-height).
    pub queue_area: Rect,
    /// Text input area.
    pub input_area: Rect,
    /// Bottom status strip — model, mode, tokens, clock.
    pub status_area: Rect,
}

/// Compute the four-region layout for the terminal.
///
/// Vertical stack (bottom to top gravity):
/// - `Constraint::Min(1)` messages (full width)
/// - `Constraint::Length(queue_height)` task queue
/// - `Constraint::Length(input_height)` input
/// - `Constraint::Length(1)` status bar at bottom
pub fn compute_layout(full_area: Rect, queue_height: u16, input_height: u16) -> ViewLayout {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),                    // messages
            Constraint::Length(queue_height),      // queue
            Constraint::Length(input_height),      // input
            Constraint::Length(STATUS_BAR_HEIGHT), // status bar
        ])
        .split(full_area);

    ViewLayout {
        msg_area: vertical[0],
        queue_area: vertical[1],
        input_area: vertical[2],
        status_area: vertical[3],
    }
}

/// Shrink `area` by one cell on every side (for bordered widgets).
pub fn inset_border(area: Rect) -> Rect {
    if area.width <= 2 || area.height <= 2 {
        return Rect::default();
    }
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Split `area` into `(left, right)` where `right` is `width` columns wide.
pub fn split_right(area: Rect, width: u16) -> (Rect, Rect) {
    let w = width.min(area.width);
    let left_w = area.width.saturating_sub(w);
    let left = Rect {
        x: area.x,
        y: area.y,
        width: left_w,
        height: area.height,
    };
    let right = Rect {
        x: area.x.saturating_add(left_w),
        y: area.y,
        width: w,
        height: area.height,
    };
    (left, right)
}

/// How many logical lines can fit in a viewport of `area_height` rows.
pub fn visible_height(lines: usize, area_height: u16) -> usize {
    let cap = usize::from(area_height);
    lines.min(cap)
}

/// Clamp a scroll offset so the viewport stays within `total` lines.
pub fn scroll_offset(total: usize, visible: usize, scroll: usize) -> usize {
    if total == 0 || visible == 0 {
        return 0;
    }
    if visible >= total {
        return 0;
    }
    let max_start = total.saturating_sub(visible);
    scroll.min(max_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_layout_splits_vertically() {
        let area = Rect::new(0, 0, 100, 30);
        let layout = compute_layout(area, 2, 3);
        assert_eq!(layout.status_area.height, 1);
        assert_eq!(layout.queue_area.height, 2);
        assert_eq!(layout.input_area.height, 3);
        let used = layout.msg_area.height
            + layout.queue_area.height
            + layout.input_area.height
            + layout.status_area.height;
        assert_eq!(used, area.height);
    }

    #[test]
    fn compute_layout_no_queue() {
        let area = Rect::new(0, 0, 80, 24);
        let layout = compute_layout(area, 0, 3);
        assert_eq!(layout.queue_area.height, 0);
        assert_eq!(layout.msg_area.height, 20);
    }

    #[test]
    fn inset_border_shrinks_by_one() {
        let r = Rect::new(0, 0, 10, 8);
        let i = inset_border(r);
        assert_eq!(i.x, 1);
        assert_eq!(i.y, 1);
        assert_eq!(i.width, 8);
        assert_eq!(i.height, 6);
    }
}
