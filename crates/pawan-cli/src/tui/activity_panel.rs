//! Recent tool activity feed for a side column.

#![allow(dead_code)] // integrated by upcoming shell layout wiring

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

const MAX_ENTRIES: usize = 20;

/// One line in the activity feed.
#[derive(Debug, Clone)]
pub struct ActivityEntry {
    pub tool_name: String,
    pub success: bool,
    pub timestamp: String,
    pub detail: Option<String>,
}

/// Scrollable-ish feed (shows the newest rows that fit).
#[derive(Debug, Clone)]
pub struct ActivityPanel {
    entries: Vec<ActivityEntry>,
    accent_color: Color,
}

impl ActivityPanel {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            accent_color: Color::Cyan,
        }
    }

    pub fn push(&mut self, entry: ActivityEntry) {
        self.entries.push(entry);
        while self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn view(&self, frame: &mut Frame, area: Rect) {
        if area.width < 25 {
            let line = Line::from(Span::styled("…", Style::default().fg(Color::DarkGray)));
            frame.render_widget(Paragraph::new(line), area);
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.accent_color))
            .title(" Activity ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let visible = inner.height as usize;
        let start = self.entries.len().saturating_sub(visible);
        let slice = &self.entries[start..];

        let mut y = inner.y;
        for entry in slice {
            if y >= inner.y + inner.height {
                break;
            }

            let (icon, icon_color) = if entry.success {
                ("✓", Color::Green)
            } else {
                ("✗", Color::Red)
            };

            let mut spans = vec![
                Span::styled(
                    format!("{icon} "),
                    Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} ", entry.tool_name),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ];

            if let Some(detail) = &entry.detail {
                let detail = truncate_to_width(detail, inner.width.saturating_sub(12) as usize);
                spans.push(Span::styled(
                    format!("{detail} "),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            spans.push(Span::styled(
                entry.timestamp.clone(),
                Style::default().fg(Color::Gray),
            ));

            let row = Rect::new(inner.x, y, inner.width, 1);
            frame.render_widget(Paragraph::new(Line::from(spans)), row);
            y = y.saturating_add(1);
        }
    }
}

impl Default for ActivityPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate_to_width(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return "…".to_owned();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    if out.chars().count() == max_chars && s.chars().count() > max_chars {
        out.push('…');
    }
    out
}
