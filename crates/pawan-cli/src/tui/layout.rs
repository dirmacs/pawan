//! Terminal layout helpers for the interactive UI.
//!
//! Regions follow a top status bar, a flexible message area, an optional
//! activity column, and a bottom stack for the task queue plus input.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Height of the top status strip (model, mode, tokens).
pub const STATUS_BAR_HEIGHT: u16 = 1;
/// Default height reserved for the multiline input widget (including borders).
pub const INPUT_BASE_HEIGHT: u16 = 3;
/// Height of a single queue row.
pub const QUEUE_ITEM_HEIGHT: u16 = 1;
/// Minimum width for the optional activity column.
pub const ACTIVITY_PANEL_MIN_WIDTH: u16 = 30;
/// Preferred default width for the activity column when laying out manually.
pub const ACTIVITY_PANEL_DEFAULT_WIDTH: u16 = 40;

/// Primary layout regions for the main chat view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewLayout {
    /// Top strip — model, mode, tokens.
    pub status_area: Rect,
    /// Main transcript / message list.
    pub msg_area: Rect,
    /// Sub-agent task queue (may be zero-height).
    pub queue_area: Rect,
    /// Bottom text input.
    pub input_area: Rect,
    /// Optional tool-activity column (zero `Rect` when disabled).
    pub activity_area: Rect,
}

/// Compute the main five-region layout for the terminal.
///
/// Vertical stack:
/// - `Constraint::Length(1)` status bar
/// - `Constraint::Min(1)` messages (optionally split with activity)
/// - `Constraint::Length(queue_height + input_height)` for queue + input
///
/// When `show_activity` is true, the middle band is split **70 / 30** between
/// messages (left) and activity (right).
pub fn compute_layout(
    full_area: Rect,
    queue_height: u16,
    input_height: u16,
    show_activity: bool,
) -> ViewLayout {
    let bottom_h = queue_height.saturating_add(input_height);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(STATUS_BAR_HEIGHT),
            Constraint::Min(1),
            Constraint::Length(bottom_h),
        ])
        .split(full_area);

    let status_area = vertical[0];
    let middle = vertical[1];
    let bottom = vertical[2];

    let (msg_area, activity_area) = if show_activity {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(middle);
        (cols[0], cols[1])
    } else {
        (middle, Rect::default())
    };

    let bottom_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(queue_height),
            Constraint::Length(input_height),
        ])
        .split(bottom);

    ViewLayout {
        status_area,
        msg_area,
        queue_area: bottom_split[0],
        input_area: bottom_split[1],
        activity_area,
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
        let layout = compute_layout(area, 2, 3, false);
        assert_eq!(layout.status_area.height, 1);
        assert_eq!(layout.queue_area.height, 2);
        assert_eq!(layout.input_area.height, 3);
        assert_eq!(layout.activity_area, Rect::default());
        let used = layout.status_area.height
            + layout.msg_area.height
            + layout.queue_area.height
            + layout.input_area.height;
        assert_eq!(used, area.height);
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
