//! Session list filtering + session browser rendering.

#![allow(unused_imports)]

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pawan::agent::session::{RetentionPolicy, SearchResult, Session, SessionSummary};
use pawan::agent::{AgentResponse, Message, PawanAgent, Role, ToolCallRecord, ToolCallRequest};
use pawan::config::TuiConfig;
use pawan::{PawanError, Result};
use ratatui::style::Style;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use ratatui_textarea::{Input, TextArea};
use regex::Regex;
use std::io::{self, Stdout};
use std::sync::OnceLock;
use std::time::Instant;
use tokio::sync::mpsc;

use super::app::App;
use super::types::*;

impl<'a> App<'a> {
    pub(crate) fn filtered_sessions(&self) -> Vec<SessionSummary> {
        let mut sessions = match Session::list() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let query = self.session_browser_query.to_lowercase();
        if !query.is_empty() {
            if let Some(tag) = query.strip_prefix("tag:") {
                let tag = tag.trim();
                sessions.retain(|s| s.tags.iter().any(|t| t.to_lowercase() == tag));
            } else {
                sessions.retain(|s| {
                    s.id.to_lowercase().contains(&query) || s.model.to_lowercase().contains(&query)
                });
            }
        }
        // Apply sorting based on session_sort_mode
        match self.session_sort_mode {
            SessionSortMode::NewestFirst => {
                sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            }
            SessionSortMode::Alphabetical => {
                sessions.sort_by(|a, b| a.id.cmp(&b.id));
            }
            SessionSortMode::MostUsed => {
                sessions.sort_by(|a, b| b.message_count.cmp(&a.message_count));
            }
        }
        sessions
    }

    pub(crate) fn render_session_browser(&self, f: &mut Frame) {
        let area = f.area();
        let sessions: Vec<SessionSummary> = self.filtered_sessions();
        let selected = self
            .session_browser_selected
            .min(sessions.len().saturating_sub(1));

        let w = (area.width * 70 / 100).max(50);
        let h = (sessions.len() as u16 + 4).min(15);
        let x = (area.width.saturating_sub(w)) / 2;
        let y = area.height / 4;
        let browser_area = Rect::new(x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green))
            .title(" Session Browser ");

        f.render_widget(ratatui::widgets::Clear, browser_area);
        f.render_widget(block.clone(), browser_area);

        let inner = block.inner(browser_area);

        // Search input
        let search_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Green)),
            Span::styled(
                &self.session_browser_query,
                Style::default().fg(Color::White),
            ),
            Span::styled(
                "▌",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]);
        f.render_widget(
            Paragraph::new(search_line),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        // Session list with viewport scrolling
        let list_area = Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(1),
        );
        let list_height = list_area.height as usize;

        // Calculate scroll offset to keep selected item in view
        let offset = if selected < list_height {
            0
        } else {
            selected - list_height + 1
        };

        let visible_items: Vec<ListItem> = sessions
            .iter()
            .skip(offset)
            .take(list_height)
            .enumerate()
            .map(|(i, session)| {
                let actual_idx = i + offset;
                let style = if actual_idx == selected {
                    Style::default().fg(Color::Black).bg(Color::Green)
                } else {
                    Style::default()
                };
                let indicator = if session.message_count > 0 {
                    "●"
                } else {
                    "○"
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(
                            "{} {} ({} msg)",
                            indicator, session.id, session.message_count
                        ),
                        style.add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" [{}]", session.model), style.fg(Color::DarkGray)),
                ]))
            })
            .collect();
        f.render_widget(List::new(visible_items), list_area);
    }
}
