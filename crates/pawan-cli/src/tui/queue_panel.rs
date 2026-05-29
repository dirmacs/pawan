//! Compact queue / subagent status strip.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Queue row for a future multi-agent scheduler.
#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub task_name: String,
    pub status: TaskStatus,
}

/// Lifecycle for a queued unit of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    #[allow(dead_code)]
    Pending,
    Running,
    Done,
    Failed,
}

/// Renders a small multi-line badge row for queued work.
#[derive(Debug, Clone, Default)]
pub struct QueuePanel {
    entries: Vec<QueueEntry>,
}

impl QueuePanel {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn set_entries(&mut self, entries: Vec<QueueEntry>) {
        self.entries = entries;
    }

    pub fn height_hint(&self) -> u16 {
        if self.entries.is_empty() {
            return 0;
        }
        if self.entries.len() <= 6 {
            1
        } else if self.entries.len() <= 12 {
            2
        } else {
            3
        }
    }

    pub fn view(&self, frame: &mut Frame, area: Rect) {
        if self.entries.is_empty() || area.height == 0 || area.width == 0 {
            return;
        }

        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as usize)
            .unwrap_or(0);
        let spin_idx = (ms / 80) % SPINNER_FRAMES.len();
        let spinner = SPINNER_FRAMES[spin_idx];

        let pieces: Vec<String> = self
            .entries
            .iter()
            .map(|e| format_piece(e, spinner))
            .collect();

        let mut lines: Vec<Vec<String>> = vec![Vec::new()];
        let max_lines = (area.height as usize).min(3).max(1);
        let budget = area.width as usize;

        for piece in pieces {
            let Some(cur) = lines.last_mut() else {
                tracing::debug!("queue_panel: line bucket missing");
                break;
            };

            let used = line_used_width(cur);

            let needs_wrap =
                !cur.is_empty() && used.saturating_add(1).saturating_add(piece.len()) > budget;
            if needs_wrap {
                if lines.len() >= max_lines {
                    break;
                }
                lines.push(vec![piece]);
                continue;
            }

            cur.push(piece);
        }

        let mut y = area.y;
        for row in lines.into_iter().take(max_lines) {
            if y >= area.y + area.height {
                break;
            }
            let text = row.join(" ");
            let line = Line::from(Span::styled(text, Style::default().fg(Color::Gray)));
            let row_area = Rect::new(area.x, y, area.width, 1);
            frame.render_widget(Paragraph::new(line), row_area);
            y = y.saturating_add(1);
        }
    }
}

fn line_used_width(cur: &[String]) -> usize {
    if cur.is_empty() {
        return 0;
    }
    cur.iter()
        .map(|s| s.len().saturating_add(1))
        .sum::<usize>()
        .saturating_sub(1)
}

fn format_piece(entry: &QueueEntry, spinner: char) -> String {
    match entry.status {
        TaskStatus::Pending => format!("○ {}", entry.task_name),
        TaskStatus::Running => format!("{spinner} {} ●", entry.task_name),
        TaskStatus::Done => format!("✓ {}", entry.task_name),
        TaskStatus::Failed => format!("✗ {}", entry.task_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn entry(name: &str, status: TaskStatus) -> QueueEntry {
        QueueEntry {
            task_name: name.to_string(),
            status,
        }
    }

    fn buffer_to_string(buf: &Buffer) -> String {
        let area = buf.area;
        let mut result = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                result.push_str(buf[(x, y)].symbol());
            }
            result.push('\n');
        }
        result
    }

    fn render_panel(panel: &QueuePanel, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| panel.view(f, Rect::new(0, 0, width, height)))
            .unwrap();
        buffer_to_string(terminal.backend().buffer())
    }

    #[test]
    fn height_hint_scales_with_entry_count() {
        let mut panel = QueuePanel::new();
        assert_eq!(panel.height_hint(), 0);

        panel.set_entries(vec![entry("a", TaskStatus::Pending)]);
        assert_eq!(panel.height_hint(), 1);

        panel.set_entries((0..6).map(|i| entry(&format!("t{i}"), TaskStatus::Running)).collect());
        assert_eq!(panel.height_hint(), 1);

        panel.set_entries((0..7).map(|i| entry(&format!("t{i}"), TaskStatus::Running)).collect());
        assert_eq!(panel.height_hint(), 2);

        panel.set_entries((0..13).map(|i| entry(&format!("t{i}"), TaskStatus::Done)).collect());
        assert_eq!(panel.height_hint(), 3);
    }

    #[test]
    fn format_piece_styles_each_status() {
        assert_eq!(
            format_piece(&entry("auth", TaskStatus::Pending), '⠋'),
            "○ auth"
        );
        assert_eq!(
            format_piece(&entry("auth", TaskStatus::Running), '⠙'),
            "⠙ auth ●"
        );
        assert_eq!(
            format_piece(&entry("auth", TaskStatus::Done), '⠋'),
            "✓ auth"
        );
        assert_eq!(
            format_piece(&entry("auth", TaskStatus::Failed), '⠋'),
            "✗ auth"
        );
    }

    #[test]
    fn line_used_width_accounts_for_separators() {
        assert_eq!(line_used_width(&[]), 0);
        assert_eq!(line_used_width(&["a".into()]), 1);
        assert_eq!(line_used_width(&["ab".into(), "c".into()]), 4);
    }

    #[test]
    fn view_renders_running_entry() {
        let mut panel = QueuePanel::new();
        panel.set_entries(vec![entry("worker", TaskStatus::Running)]);
        let text = render_panel(&panel, 40, 3);
        assert!(text.contains("worker"));
    }

    #[test]
    fn view_noops_on_empty_panel() {
        let panel = QueuePanel::new();
        let text = render_panel(&panel, 20, 2);
        assert!(text.chars().all(char::is_whitespace));
    }

    #[test]
    fn height_hint_empty_is_0() {
        assert_eq!(QueuePanel::new().height_hint(), 0);
    }

    #[test]
    fn height_hint_6_entries_is_1() {
        let mut panel = QueuePanel::new();
        panel.set_entries(
            (0..6)
                .map(|i| entry(&format!("t{i}"), TaskStatus::Running))
                .collect(),
        );
        assert_eq!(panel.height_hint(), 1);
    }

    #[test]
    fn height_hint_13_entries_is_3() {
        let mut panel = QueuePanel::new();
        panel.set_entries(
            (0..13)
                .map(|i| entry(&format!("t{i}"), TaskStatus::Done))
                .collect(),
        );
        assert_eq!(panel.height_hint(), 3);
    }

    #[test]
    fn new_has_no_entries() {
        let panel = QueuePanel::new();
        assert_eq!(panel.height_hint(), 0);
        let text = render_panel(&panel, 30, 2);
        assert!(text.chars().all(char::is_whitespace));
    }

    #[test]
    fn set_entries_updates_view() {
        let mut panel = QueuePanel::new();
        panel.set_entries(vec![entry("alpha", TaskStatus::Pending)]);
        let first = render_panel(&panel, 40, 2);
        assert!(first.contains("alpha"));

        panel.set_entries(vec![entry("beta", TaskStatus::Failed)]);
        let second = render_panel(&panel, 40, 2);
        assert!(!second.contains("alpha"));
        assert!(second.contains("beta"));
    }

    #[test]
    fn view_renders_pending_and_done_entries() {
        let mut panel = QueuePanel::new();
        panel.set_entries(vec![
            entry("wait", TaskStatus::Pending),
            entry("ok", TaskStatus::Done),
        ]);
        let text = render_panel(&panel, 60, 3);
        assert!(text.contains("wait"));
        assert!(text.contains("ok"));
    }
}
