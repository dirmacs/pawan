//! Rich formatting for tool call summaries embedded in message views.

#![allow(dead_code)] // integrated by upcoming shell layout wiring

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const PREVIEW_MAX: usize = 200;

/// Compact display of a tool invocation result.
#[derive(Debug, Clone)]
pub struct ToolDisplay {
    pub tool_name: String,
    pub success: bool,
    pub output_preview: String,
    pub duration_ms: u64,
}

impl ToolDisplay {
    /// Builds one or two lines suitable for embedding in a `Paragraph` / message bubble.
    pub fn format_result(&self) -> Vec<Line<'static>> {
        let preview: String = self
            .output_preview
            .chars()
            .take(PREVIEW_MAX)
            .collect::<String>()
            .replace('\n', " ")
            .trim()
            .to_owned();

        let (icon, icon_color, status_word) = if self.success {
            ("✓", Color::Green, "ok")
        } else {
            ("✗", Color::Red, "failed")
        };

        let header = Line::from(vec![
            Span::styled(
                format!("{icon} "),
                Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} ", self.tool_name),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({status_word}, {} ms) ", self.duration_ms),
                Style::default().fg(Color::Gray),
            ),
        ]);

        if preview.is_empty() {
            return vec![header];
        }

        let preview_line = Line::from(vec![Span::styled(
            preview,
            Style::default().fg(Color::Gray),
        )]);

        vec![header, preview_line]
    }
}
