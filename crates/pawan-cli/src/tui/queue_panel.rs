//! Compact queue / subagent status strip.

#![allow(dead_code)] // integrated by upcoming shell layout wiring

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
        Self { entries: Vec::new() }
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

            let needs_wrap = !cur.is_empty() && used.saturating_add(1).saturating_add(piece.len()) > budget;
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
            let line = Line::from(Span::styled(
                text,
                Style::default().fg(Color::Gray),
            ));
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
    cur.iter().map(|s| s.len().saturating_add(1)).sum::<usize>().saturating_sub(1)
}

fn format_piece(entry: &QueueEntry, spinner: char) -> String {
    match entry.status {
        TaskStatus::Pending => format!("○ {}", entry.task_name),
        TaskStatus::Running => format!("{spinner} {} ●", entry.task_name),
        TaskStatus::Done => format!("✓ {}", entry.task_name),
        TaskStatus::Failed => format!("✗ {}", entry.task_name),
    }
}
