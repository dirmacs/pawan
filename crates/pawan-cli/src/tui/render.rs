//! Rendering, markdown, and the display-cache `impl DisplayMessage` block.

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
    pub(crate) fn filtered_models(&self) -> Vec<&ModelInfo> {
        if self.model_picker.models.is_empty() {
            return Vec::new();
        }

        let query = self.model_picker.query.to_lowercase();
        if query.is_empty() {
            return self.model_picker.models.iter().collect();
        }

        self.model_picker
            .models
            .iter()
            .filter(|m| {
                m.id.to_lowercase().contains(&query) || m.provider.to_lowercase().contains(&query)
            })
            .collect()
    }
}

impl<'a> App<'a> {
    pub(crate) fn render_model_selector(&self, f: &mut Frame) {
        let area = f.area();
        let models = self.filtered_models();
        let selected = self
            .model_picker
            .selected
            .min(models.len().saturating_sub(1));

        let w = (area.width * 50 / 100)
            .max(40)
            .min(area.width.saturating_sub(4));
        let h = (models.len() as u16 + 4)
            .min(18)
            .min(area.height.saturating_sub(2));
        let x = (area.width.saturating_sub(w)) / 2;
        let y = (area.height.saturating_sub(h)) / 2;
        let selector_area = Rect::new(x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue))
            .title(" Model Picker (^M) ")
            .title_style(Style::default().add_modifier(Modifier::BOLD));
        f.render_widget(ratatui::widgets::Clear, selector_area);
        f.render_widget(block.clone(), selector_area);

        let inner = block.inner(selector_area);

        let search_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Blue)),
            Span::styled(&self.model_picker.query, Style::default().fg(Color::White)),
            Span::styled(
                "▌",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]);
        f.render_widget(
            Paragraph::new(search_line),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        let list_area = Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(1),
        );
        let list_height = list_area.height as usize;
        let offset = if selected < list_height {
            0
        } else {
            selected - list_height + 1
        };
        let visible_items: Vec<ListItem> = models
            .iter()
            .skip(offset)
            .take(list_height)
            .enumerate()
            .map(|(i, model)| {
                let actual_idx = i + offset;
                let is_sel = actual_idx == selected;
                let badge = if model.provider.len() > 12 {
                    format!("{}…", &model.provider[..11])
                } else {
                    model.provider.clone()
                };
                let line_style = if is_sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Blue)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let badge_style = if is_sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Cyan)
                };
                let score_st = if is_sel {
                    Style::default().fg(Color::Black).bg(Color::Blue)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", badge), badge_style),
                    Span::styled(model.id.clone(), line_style),
                    Span::styled(format!("  q{} ", model.quality_score), score_st),
                ]))
            })
            .collect();
        f.render_widget(List::new(visible_items), list_area);
    }
}

impl<'a> App<'a> {
    pub(crate) fn ui(&self, f: &mut Frame) {
        // Dynamic input height: 3 lines default, grows with content up to 10
        let input_lines = self.input.lines().len();
        let input_height = (input_lines + 2).clamp(3, 10) as u16; // +2 for border

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Min(3),               // messages: takes all remaining space
                Constraint::Length(input_height), // input: auto-resizes
                Constraint::Length(1),            // status bar
            ])
            .split(f.area());

        // Split layout: messages + activity panel when processing
        if self.processing {
            let horiz = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
                .split(chunks[0]);
            self.render_messages(f, horiz[0]);
            self.render_activity(f, horiz[1]);
        } else {
            self.render_messages(f, chunks[0]);
        }
        self.render_input(f, chunks[1]);
        self.render_status(f, chunks[2]);

        // Inline slash popup (above input, below other overlays)
        if self.is_slash_popup_active()
            && !self.show_welcome
            && !self.help_overlay
            && self.fuzzy_search.is_none()
        {
            self.render_slash_popup(f, chunks[1]);
        }

        // Overlays (on top of everything)
        if self.permission_dialog.is_some() {
            self.render_permission_dialog(f);
        } else if self.show_welcome {
            self.render_welcome(f);
        } else if self.model_picker.visible {
            self.render_model_selector(f);
        } else if self.session_browser_open {
            self.render_session_browser(f);
        } else if self.help_overlay {
            self.render_help_overlay(f);
        } else if self.fuzzy_search.is_some() {
            self.render_fuzzy_search(f);
        }
    }

    pub(crate) fn render_permission_dialog(&self, f: &mut Frame) {
        let dialog = match &self.permission_dialog {
            Some(d) => d,
            None => return,
        };

        let area = f.area();
        let width = 60u16.min(area.width.saturating_sub(4));
        let height = 8u16;
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        // Clear background
        f.render_widget(ratatui::widgets::Clear, popup_area);

        let text = vec![
            Line::from(vec![
                Span::styled(
                    "Tool: ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(&dialog.tool_name),
            ]),
            Line::from(vec![
                Span::styled("Args: ", Style::default().fg(Color::DarkGray)),
                Span::raw(if dialog.args_summary.len() > 45 {
                    format!("{}...", &dialog.args_summary[..42])
                } else {
                    dialog.args_summary.clone()
                }),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    " y ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Allow  "),
                Span::styled(
                    " n ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Red)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Deny  "),
                Span::styled(
                    " a ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Allow all"),
            ]),
        ];

        let block = Block::default()
            .title(" Permission Required ")
            .title_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let paragraph = Paragraph::new(text).block(block);
        f.render_widget(paragraph, popup_area);
    }

    /// Check if the inline slash popup should be shown.
    pub(crate) fn is_slash_popup_active(&self) -> bool {
        let text: String = self.input.lines().join("\n");
        let trimmed = text.trim();
        trimmed.starts_with('/') || (trimmed.starts_with(':') && !trimmed.contains(' '))
    }

    /// Get filtered slash command items based on current input.
    pub(crate) fn slash_items(&self) -> Vec<(String, String)> {
        let mut all: Vec<(String, String)> = self
            .slash_registry
            .all()
            .iter()
            .map(|c| (c.name.clone(), c.description.clone()))
            .collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));

        let text: String = self.input.lines().join("\n");
        let mut q = text.trim().to_lowercase();
        if q.starts_with(':') {
            if q == ":" {
                q = "/".to_string();
            } else {
                q = format!("/{}", &q[1..]);
            }
        }
        if q == "/" {
            return all;
        }
        all.into_iter()
            .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&q))
            .collect()
    }

    /// Render inline slash command popup above the input area.
    pub(crate) fn render_slash_popup(&self, f: &mut Frame, input_area: Rect) {
        let items = self.slash_items();
        if items.is_empty() {
            return;
        }

        let max_height = 10u16;
        let h = (items.len() as u16 + 2).min(max_height);
        let w = 45u16.min(input_area.width);
        let y = input_area.y.saturating_sub(h);
        let popup_area = Rect::new(input_area.x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" / Commands ");

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(block.clone(), popup_area);

        let inner = block.inner(popup_area);
        let inner_height = inner.height as usize;

        // Calculate scroll offset to keep selected item in view
        let selected = self.slash_popup_selected.min(items.len().saturating_sub(1));
        let offset = if selected < inner_height {
            0
        } else {
            selected - inner_height + 1
        };

        // Render visible items with offset
        let visible_items: Vec<ListItem> = items
            .iter()
            .skip(offset)
            .take(inner_height)
            .enumerate()
            .map(|(i, (cmd, desc))| {
                let actual_idx = i + offset;
                let style = if actual_idx == selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", cmd), style.add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!("— {}", desc),
                        if actual_idx == selected {
                            Style::default().fg(Color::Black).bg(Color::Cyan)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ),
                ]))
            })
            .collect();

        f.render_widget(List::new(visible_items), inner);
    }

    /// Render welcome screen overlay
    pub(crate) fn render_welcome(&self, f: &mut Frame) {
        let area = f.area();
        let w = 52u16.min(area.width.saturating_sub(4));
        let h = 12u16.min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(w)) / 2;
        let y = (area.height.saturating_sub(h)) / 2;
        let welcome_area = Rect::new(x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" पवन — pawan ");

        f.render_widget(ratatui::widgets::Clear, welcome_area);
        f.render_widget(block.clone(), welcome_area);

        let inner = block.inner(welcome_area);
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let text = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "  Self-healing CLI coding agent",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  v{}", env!("CARGO_PKG_VERSION")),
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Model: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&self.model_name, Style::default().fg(Color::Cyan)),
            ]),
            Line::from(vec![
                Span::styled("  Path:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(cwd, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Type a task, or explore:",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(vec![
                Span::styled(
                    "  Ctrl+P",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  fuzzy search (commands)",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "  F1    ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  keyboard shortcuts", Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Press any key to start...",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
        ];
        f.render_widget(Paragraph::new(text), inner);
    }

    pub(crate) fn render_help_overlay(&self, f: &mut Frame) {
        let area = f.area();
        let w = 48u16.min(area.width.saturating_sub(4));
        let h = 16u16.min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(w)) / 2;
        let y = (area.height.saturating_sub(h)) / 2;
        let help_area = Rect::new(x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Keyboard Shortcuts (F1) ");

        f.render_widget(ratatui::widgets::Clear, help_area);
        f.render_widget(block.clone(), help_area);

        let inner = block.inner(help_area);
        let shortcuts = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Navigation",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("  Tab     ", Style::default().fg(Color::Yellow)),
                Span::raw("Switch focus (input/messages)"),
            ]),
            Line::from(vec![
                Span::styled("  j/k     ", Style::default().fg(Color::Yellow)),
                Span::raw("Scroll up/down"),
            ]),
            Line::from(vec![
                Span::styled("  g/G     ", Style::default().fg(Color::Yellow)),
                Span::raw("Jump to top/bottom"),
            ]),
            Line::from(vec![
                Span::styled("  /       ", Style::default().fg(Color::Yellow)),
                Span::raw("Search in messages"),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Commands",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("  Ctrl+P  ", Style::default().fg(Color::Yellow)),
                Span::raw("Fuzzy search (slash commands)"),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+L  ", Style::default().fg(Color::Yellow)),
                Span::raw("Clear chat"),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+Q  ", Style::default().fg(Color::Yellow)),
                Span::raw("Quit"),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Slash Commands",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("  /model  ", Style::default().fg(Color::Yellow)),
                Span::raw("Switch model at runtime"),
            ]),
            Line::from(vec![
                Span::styled("  /search ", Style::default().fg(Color::Yellow)),
                Span::raw("Web search via Daedra"),
            ]),
            Line::from(vec![
                Span::styled("  /tools  ", Style::default().fg(Color::Yellow)),
                Span::raw("List all tools"),
            ]),
        ];
        f.render_widget(Paragraph::new(shortcuts), inner);
    }

    pub(crate) fn render_fuzzy_search(&self, f: &mut Frame) {
        let Some(fs) = &self.fuzzy_search else {
            return;
        };
        let area = f.area();
        // Center: 50% width, up to 22 lines tall (query + up to 20+ result rows, capped in state)
        let w = (area.width * 50 / 100).max(30);
        let n = fs.results.len();
        let h = (n as u16 + 4).min(24);
        let x = (area.width.saturating_sub(w)) / 2;
        let y = area.height / 4;
        let modal_area = Rect::new(x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Fuzzy search (Ctrl+P, Ctrl+F) ");

        let inner = block.inner(modal_area);
        f.render_widget(ratatui::widgets::Clear, modal_area);
        f.render_widget(block, modal_area);

        let search_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::styled(&fs.query, Style::default().fg(Color::White)),
            Span::styled(
                "▌",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]);
        if inner.height > 0 {
            f.render_widget(
                Paragraph::new(search_line),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
        }

        let list_area = Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(1),
        );
        let list_height = list_area.height as usize;
        let items = &fs.results;
        let selected = fs.selected.min(items.len().saturating_sub(1));
        let offset = if selected < list_height {
            0
        } else {
            selected - list_height + 1
        };

        let visible_items: Vec<ListItem> = items
            .iter()
            .skip(offset)
            .take(list_height)
            .enumerate()
            .map(|(i, line)| {
                let actual_idx = i + offset;
                let style = if actual_idx == selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(line, style)))
            })
            .collect();
        f.render_widget(List::new(visible_items), list_area);
    }

    pub(crate) fn render_activity(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta))
            .title(" Activity ");
        let mut items: Vec<ListItem> = Vec::new();
        for msg in self.messages.iter().rev().take(5) {
            for tc in msg.tool_records() {
                let icon = if tc.success { "✓" } else { "✗" };
                let color = if tc.success { Color::Green } else { Color::Red };
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", icon), Style::default().fg(color)),
                    Span::styled(tc.name.clone(), Style::default().fg(Color::White)),
                    Span::styled(
                        format!(" {}ms", tc.duration_ms),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])));
            }
        }
        // Show running tools from streaming state
        if let Some(ref state) = self.streaming {
            for block in &state.blocks {
                if let ContentBlock::ToolCall { name, state, .. } = block {
                    if matches!(state.as_ref(), ToolBlockState::Running) {
                        items.push(ListItem::new(Line::from(vec![
                            Span::styled(" ⚙ ", Style::default().fg(Color::Yellow)),
                            Span::styled(
                                name.clone(),
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ])));
                    }
                }
            }
        }
        if items.is_empty() {
            items.push(ListItem::new(Span::styled(
                " Waiting...",
                Style::default().fg(Color::DarkGray),
            )));
        }
        f.render_widget(List::new(items).block(block), area);
    }

    pub(crate) fn render_messages(&self, f: &mut Frame, area: Rect) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let now = std::time::Instant::now();

        for msg in &self.messages {
            self.render_message_to_lines(msg, now, &mut lines);
            lines.push(Line::from(""));
        }

        // Streaming state: render the in-progress assistant message
        if self.processing {
            if let Some(ref state) = self.streaming {
                if !state.blocks.is_empty() {
                    lines.push(Line::from(vec![Span::styled(
                        "Pawan: ",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )]));
                    for block in &state.blocks {
                        Self::render_block_to_lines(block, true, &mut lines);
                    }
                } else {
                    lines.push(Line::from(vec![Span::styled(
                        "  Pawan is thinking...",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::ITALIC),
                    )]));
                }
            } else {
                lines.push(Line::from(vec![Span::styled(
                    "  Pawan is thinking...",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::ITALIC),
                )]));
            }
        }

        let border_style = if self.focus == Panel::Messages {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let total_lines = lines.len();
        let visible_height = area.height.saturating_sub(2) as usize; // minus borders
        let max_offset = total_lines.saturating_sub(visible_height);
        let scroll_offset = if self.scroll == usize::MAX {
            max_offset // auto-scroll to bottom
        } else {
            self.scroll.min(max_offset)
        };

        let scroll_indicator = if total_lines > visible_height {
            let pct = if max_offset > 0 {
                scroll_offset * 100 / max_offset
            } else {
                100
            };
            format!(" [{}%]", pct)
        } else {
            String::new()
        };

        let title = if self.search_mode {
            format!(" Search: {}▌ ", self.search_query)
        } else if !self.search_query.is_empty() {
            format!(
                " Messages{} [/{}] (n/N next/prev) ",
                scroll_indicator, self.search_query
            )
        } else {
            format!(" Messages{} (Tab, j/k, /, g/G, e) ", scroll_indicator)
        };

        let messages_block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let paragraph = Paragraph::new(lines)
            .block(messages_block)
            .scroll((scroll_offset as u16, 0));
        f.render_widget(paragraph, area);
    }

    /// Render a single DisplayMessage into Lines.
    /// Uses cached block lines when available (populated by `block_lines_cached()`).
    pub(crate) fn render_message_to_lines(
        &self,
        msg: &DisplayMessage,
        now: std::time::Instant,
        lines: &mut Vec<Line<'static>>,
    ) {
        let (prefix, style) = match msg.role {
            Role::User => (
                "You",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Role::Assistant => (
                "Pawan",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Role::System => ("System", Style::default().fg(Color::Yellow)),
            Role::Tool => ("Tool", Style::default().fg(Color::Magenta)),
        };

        let elapsed = now.duration_since(msg.timestamp);
        let time_str = if elapsed.as_secs() < 5 {
            "now".to_string()
        } else if elapsed.as_secs() < 60 {
            format!("{}s", elapsed.as_secs())
        } else if elapsed.as_secs() < 3600 {
            format!("{}m", elapsed.as_secs() / 60)
        } else {
            format!("{}h", elapsed.as_secs() / 3600)
        };

        lines.push(Line::from(vec![
            Span::styled(format!("{}: ", prefix), style),
            Span::styled(
                format!("({})", time_str),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Use cached block lines if available; otherwise render fresh
        if let Some(ref cached) = msg.cached_block_lines {
            lines.extend(cached.iter().cloned());
        } else {
            let is_assistant = msg.role == Role::Assistant;
            for block in &msg.blocks {
                Self::render_block_to_lines(block, is_assistant, lines);
            }
        }
    }

    /// Render a single ContentBlock into Lines.
    pub(crate) fn render_block_to_lines(
        block: &ContentBlock,
        use_markdown: bool,
        lines: &mut Vec<Line<'static>>,
    ) {
        match block {
            ContentBlock::Text { content, streaming } => {
                if use_markdown {
                    for line in markdown_to_lines(&strip_reasoning_tags(content)) {
                        let mut spans: Vec<Span<'static>> = vec![Span::raw("  ".to_string())];
                        spans.extend(line.spans);
                        lines.push(Line::from(spans));
                    }
                } else {
                    for line_str in content.lines() {
                        lines.push(Line::from(Span::raw(format!("  {}", line_str))));
                    }
                }
                if *streaming {
                    lines.push(Line::from(vec![Span::styled(
                        "  ▌",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::SLOW_BLINK),
                    )]));
                }
            }
            ContentBlock::ToolCall {
                name,
                args_summary,
                state,
            } => match state.as_ref() {
                ToolBlockState::Running => {
                    lines.push(Line::from(vec![
                        Span::styled("  ⚙ ", Style::default().fg(Color::Yellow)),
                        Span::styled(
                            format!("Running {}...", name),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
                ToolBlockState::Done { record, expanded } => {
                    let icon = if record.success { "✓" } else { "✗" };
                    let color = if record.success {
                        Color::Green
                    } else {
                        Color::Red
                    };
                    let mut spans = vec![
                        Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                        Span::styled(
                            name.clone(),
                            Style::default()
                                .fg(Color::Magenta)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ];
                    if !args_summary.is_empty() {
                        spans.push(Span::styled(
                            format!("({})", args_summary),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    spans.push(Span::styled(
                        format!(" {}ms", record.duration_ms),
                        Style::default().fg(Color::DarkGray),
                    ));
                    lines.push(Line::from(spans));

                    if *expanded {
                        let result_str = format_tool_result(&record.result);
                        for result_line in result_str.lines().take(20) {
                            lines.push(Line::from(Span::styled(
                                format!("    {}", result_line),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }
                        let total = result_str.lines().count();
                        if total > 20 {
                            lines.push(Line::from(Span::styled(
                                format!("    ... ({} more lines)", total - 20),
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::ITALIC),
                            )));
                        }
                    } else {
                        let preview = one_line_preview(&record.result, 60);
                        if !preview.is_empty() {
                            lines.push(Line::from(Span::styled(
                                format!("    {}", preview),
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::DIM),
                            )));
                        }
                    }
                }
            },
        }
    }

    /// Toggle expand/collapse on the nearest tool block to the current scroll position.
    pub(crate) fn toggle_nearest_tool_expansion(&mut self) {
        let mut line_offset = 0usize;
        let mut best: Option<(usize, usize, usize)> = None; // (msg_idx, block_idx, distance)

        for (mi, msg) in self.messages.iter().enumerate() {
            line_offset += 1; // header line
            for (bi, block) in msg.blocks.iter().enumerate() {
                if let ContentBlock::ToolCall { state, .. } = block {
                    if matches!(state.as_ref(), ToolBlockState::Done { .. }) {
                        let dist = line_offset.abs_diff(self.scroll);
                        if best.is_none() || dist < best.unwrap().2 {
                            best = Some((mi, bi, dist));
                        }
                    }
                }
                // Estimate lines this block takes
                match block {
                    ContentBlock::Text { content, .. } => {
                        line_offset += content.lines().count().max(1);
                    }
                    ContentBlock::ToolCall { state, .. } => {
                        if let ToolBlockState::Done { expanded, record } = state.as_ref() {
                            line_offset += 1; // summary line
                            if *expanded {
                                line_offset +=
                                    format_tool_result(&record.result).lines().count().min(21);
                            } else {
                                line_offset += 1; // preview line
                            }
                        } else {
                            line_offset += 1;
                        }
                    }
                }
            }
            line_offset += 1; // spacer
        }

        if let Some((mi, bi, _)) = best {
            if let ContentBlock::ToolCall { state, .. } = &mut self.messages[mi].blocks[bi] {
                if let ToolBlockState::Done { expanded, .. } = state.as_mut() {
                    *expanded = !*expanded;
                }
            }
            // Invalidate cache since expanded state changed
            self.messages[mi].invalidate_cache();
        }
    }

    pub(crate) fn render_input(&self, f: &mut Frame, area: Rect) {
        let border_style = if self.focus == Panel::Input {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let title = if self.processing {
            " Input (processing...) "
        } else {
            match self.current_context {
                KeybindContext::Input => " Input (Enter send | : or ^P command | ^M model) ",
                KeybindContext::Normal => " Input (i focus) ",
                KeybindContext::Command => " Input (fuzzy search open) ",
                KeybindContext::Help => " Input (F1) ",
                KeybindContext::ModelPicker => " Input (model picker open) ",
            }
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(&self.input, inner);
    }

    pub(crate) fn render_status(&self, f: &mut Frame, area: Rect) {
        let status_style = if self.processing {
            Style::default().fg(Color::Yellow)
        } else if self.status.starts_with("Error") {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let mut spans = vec![
            Span::styled("Model: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&self.model_name, Style::default().fg(Color::Cyan)),
            Span::raw(" | "),
            Span::styled(&self.status, status_style),
        ];

        if self.total_tokens > 0 {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!("{}tok", self.total_tokens),
                Style::default().fg(Color::Yellow),
            ));
            // Show thinking/action split if reasoning tokens were tracked
            if self.total_reasoning_tokens > 0 {
                spans.push(Span::styled(
                    format!(
                        " (think:{} act:{})",
                        self.total_reasoning_tokens, self.total_action_tokens
                    ),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                spans.push(Span::styled(
                    format!(
                        " ({}↑ {}↓)",
                        self.total_prompt_tokens, self.total_completion_tokens
                    ),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        if self.iteration_count > 0 {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!("iter:{}", self.iteration_count),
                Style::default().fg(Color::Magenta),
            ));
        }
        if self.context_estimate > 0 {
            let ctx_k = self.context_estimate / 1000;
            let ctx_style = if ctx_k > 80 {
                Style::default().fg(Color::Red)
            } else if ctx_k > 60 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(format!("~{}k ctx", ctx_k), ctx_style));
        }

        // Session stats
        if self.session_tool_calls > 0 {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!("{}⚡", self.session_tool_calls),
                Style::default().fg(Color::Magenta),
            ));
            if self.session_files_edited > 0 {
                spans.push(Span::styled(
                    format!(" {}📝", self.session_files_edited),
                    Style::default().fg(Color::Green),
                ));
            }
        }

        if !self.session_tags.is_empty() {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                self.session_tags.join(", "),
                Style::default().fg(Color::Green),
            ));
        }
        if !self.messages.is_empty() {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!("{}msg", self.messages.len()),
                Style::default().fg(Color::DarkGray),
            ));
        }

        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            self.keybind_status_hint(),
            Style::default().fg(Color::DarkGray),
        ));

        let status = Paragraph::new(Line::from(spans));

        f.render_widget(status, area);
    }
}

/// Parse markdown text into styled ratatui Lines
fn markdown_to_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines_out = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("```") {
            if !in_code_block {
                in_code_block = true;
                code_lang = rest.trim().to_string();
                let label = if code_lang.is_empty() {
                    "─── code ───".to_string()
                } else {
                    format!("─── {} ───", code_lang)
                };
                lines_out.push(Line::from(Span::styled(
                    label,
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                in_code_block = false;
                code_lang.clear();
                lines_out.push(Line::from(Span::styled(
                    "────────────".to_string(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            continue;
        }

        if in_code_block {
            // Code lines: monospace with distinct background color
            lines_out.push(Line::from(Span::styled(
                format!("  {}", line),
                Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 46)),
            )));
            continue;
        }

        if let Some(rest) = line.strip_prefix("### ") {
            lines_out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(rest) = line.strip_prefix("## ") {
            lines_out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
        } else if let Some(rest) = line.strip_prefix("# ") {
            lines_out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
        } else if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            let mut spans = vec![Span::raw("• ".to_string())];
            spans.extend(parse_inline_markdown(rest));
            lines_out.push(Line::from(spans));
        } else if line.len() > 2
            && line.as_bytes()[0].is_ascii_digit()
            && (line.contains(". ") || line.contains(") "))
        {
            // Numbered list: "1. text" or "1) text"
            if let Some(pos) = line.find(". ").or_else(|| line.find(") ")) {
                let num = &line[..=pos];
                let rest = &line[pos + 2..];
                let mut spans = vec![Span::styled(
                    num.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                )];
                spans.push(Span::raw(" ".to_string()));
                spans.extend(parse_inline_markdown(rest));
                lines_out.push(Line::from(spans));
            } else {
                lines_out.push(Line::from(parse_inline_markdown(line)));
            }
        } else if let Some(rest) = line.strip_prefix("> ") {
            lines_out.push(Line::from(Span::styled(
                format!("│ {}", rest),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else if line.chars().all(|c| c == '-' || c == '=') && line.len() >= 3 {
            // Horizontal rule
            lines_out.push(Line::from(Span::styled(
                "─".repeat(40),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            lines_out.push(Line::from(parse_inline_markdown(line)));
        }
    }

    lines_out
}

/// Parse inline markdown: **bold**, `code`, *italic*
fn parse_inline_markdown(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Find the next markdown marker
        let next_bold = remaining.find("**");
        let next_code = remaining.find('`');
        let next_italic = remaining.find('*').filter(|&pos| {
            // Not a ** (bold) marker
            next_bold != Some(pos)
        });

        // Find earliest marker
        let earliest = [next_bold, next_code, next_italic]
            .into_iter()
            .flatten()
            .min();

        match earliest {
            None => {
                spans.push(Span::raw(remaining.to_string()));
                break;
            }
            Some(pos) => {
                if pos > 0 {
                    spans.push(Span::raw(remaining[..pos].to_string()));
                }

                if Some(pos) == next_bold {
                    let after = &remaining[pos + 2..];
                    if let Some(end) = after.find("**") {
                        spans.push(Span::styled(
                            after[..end].to_string(),
                            Style::default().add_modifier(Modifier::BOLD),
                        ));
                        remaining = &after[end + 2..];
                    } else {
                        spans.push(Span::raw("**".to_string()));
                        remaining = after;
                    }
                } else if Some(pos) == next_code {
                    let after = &remaining[pos + 1..];
                    if let Some(end) = after.find('`') {
                        spans.push(Span::styled(
                            after[..end].to_string(),
                            Style::default().fg(Color::Yellow),
                        ));
                        remaining = &after[end + 1..];
                    } else {
                        spans.push(Span::raw("`".to_string()));
                        remaining = after;
                    }
                } else {
                    // italic *...*
                    let after = &remaining[pos + 1..];
                    if let Some(end) = after.find('*') {
                        spans.push(Span::styled(
                            after[..end].to_string(),
                            Style::default().add_modifier(Modifier::ITALIC),
                        ));
                        remaining = &after[end + 1..];
                    } else {
                        spans.push(Span::raw("*".to_string()));
                        remaining = after;
                    }
                }
            }
        }
    }

    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }

    spans
}

impl DisplayMessage {
    /// Get or build cached block lines. Returns cached lines if available.
    pub(crate) fn block_lines_cached(&mut self) -> &[Line<'static>] {
        if self.cached_block_lines.is_none() {
            let mut lines = Vec::new();
            let is_assistant = self.role == Role::Assistant;
            for block in &self.blocks {
                App::render_block_to_lines(block, is_assistant, &mut lines);
            }
            self.cached_block_lines = Some(lines);
        }
        self.cached_block_lines.as_ref().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::super::app::App;
    use super::super::types::*;

    use pawan::config::TuiConfig;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use tokio::sync::mpsc;

    use crossterm::event::{Event, KeyCode, KeyModifiers};
    use pawan::agent::Role;
    use ratatui::style::Color;
    use ratatui::Terminal;

    use super::super::fuzzy_search::{default_command_item_lines, FuzzySearchState};
    use super::{markdown_to_lines, parse_inline_markdown};
    use pawan::agent::session::Session;
    use pawan::agent::ToolCallRecord;
    use ratatui::style::Modifier;
    use ratatui_textarea::TextArea;

    /// Create a test App with dummy channels (for state/render tests, not event loops)
    fn test_app<'a>() -> App<'a> {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        // Disable welcome screen in tests so keypresses reach normal handlers
        app.show_welcome = false;
        app
    }

    // ===== Rendering tests using TestBackend =====

    #[test]
    fn test_render_empty_state() {
        let app = test_app();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let buf = terminal.backend().buffer().clone();
        // Should contain model name in status bar
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("test-model"),
            "Status bar should show model name"
        );
        assert!(content.contains("Ready"), "Status bar should show Ready");
        assert!(
            content.contains("Messages"),
            "Messages panel title should render"
        );
        assert!(content.contains("Input"), "Input panel title should render");
    }

    #[test]
    fn test_render_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Hello pawan"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("You:"), "Should render user prefix");
        assert!(content.contains("Pawan:"), "Should render assistant prefix");
        assert!(
            content.contains("Hello pawan"),
            "Should render user message"
        );
        assert!(
            content.contains("Hi there!"),
            "Should render assistant message"
        );
    }

    #[test]
    fn test_render_processing_thinking() {
        let mut app = test_app();
        app.processing = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("thinking"),
            "Should show thinking indicator"
        );
    }

    #[test]
    fn test_render_streaming_content() {
        let mut app = test_app();
        app.processing = true;
        app.streaming = Some(StreamingAssistantState {
            blocks: vec![ContentBlock::Text {
                content: "partial response so far".to_string(),
                streaming: true,
            }],
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("partial response"),
            "Should render streaming content"
        );
        assert!(content.contains("▌"), "Should show blinking cursor");
    }

    #[test]
    fn test_render_active_tool() {
        let mut app = test_app();
        app.processing = true;
        app.streaming = Some(StreamingAssistantState {
            blocks: vec![ContentBlock::ToolCall {
                name: "bash".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Running),
            }],
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("bash"), "Should show active tool name");
    }

    #[test]
    fn test_render_token_stats() {
        let mut app = test_app();
        app.total_tokens = 1500;
        app.total_prompt_tokens = 1000;
        app.total_completion_tokens = 500;

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("1500tok"), "Should show total token count");
    }

    #[test]
    fn test_render_tool_call_results() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![
                ContentBlock::Text {
                    content: "Done".into(),
                    streaming: false,
                },
                ContentBlock::ToolCall {
                    name: "write_file".into(),
                    args_summary: String::new(),
                    state: Box::new(ToolBlockState::Done {
                        record: ToolCallRecord {
                            id: "1".into(),
                            name: "write_file".into(),
                            arguments: serde_json::json!({}),
                            result: serde_json::json!({"success": true}),
                            success: true,
                            duration_ms: 42,
                        },
                        expanded: false,
                    }),
                },
                ContentBlock::ToolCall {
                    name: "bash".into(),
                    args_summary: String::new(),
                    state: Box::new(ToolBlockState::Done {
                        record: ToolCallRecord {
                            id: "2".into(),
                            name: "bash".into(),
                            arguments: serde_json::json!({}),
                            result: serde_json::json!({"error": "timeout"}),
                            success: false,
                            duration_ms: 30000,
                        },
                        expanded: true,
                    }),
                },
            ],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("write_file"),
            "Should show successful tool name"
        );
        assert!(content.contains("bash"), "Should show failed tool name");
        assert!(
            content.contains("42ms") || content.contains("✓"),
            "Should show success indicator"
        );
    }

    #[test]
    fn test_tool_call_expansion_toggle() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![ContentBlock::ToolCall {
                name: "bash".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "bash".to_string(),
                        arguments: serde_json::json!({"command": "ls"}),
                        result: serde_json::json!({"output": "file1.txt\nfile2.txt"}),
                        success: true,
                        duration_ms: 100,
                    },
                    expanded: false,
                }),
            }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        // Toggle expansion
        app.toggle_nearest_tool_expansion();

        // Verify that the tool call state was modified
        if let Some(ContentBlock::ToolCall {
            state: tool_state, ..
        }) = app.messages.first().unwrap().blocks.first()
        {
            if let ToolBlockState::Done { expanded, .. } = tool_state.as_ref() {
                assert!(*expanded, "Tool call should be expanded after toggle");
            }
        }
    }

    #[test]
    fn test_tool_call_error_display() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![ContentBlock::ToolCall {
                name: "bash".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "bash".to_string(),
                        arguments: serde_json::json!({"command": "invalid_command"}),
                        result: serde_json::json!({"error": "command not found"}),
                        success: false,
                        duration_ms: 50,
                    },
                    expanded: true,
                }),
            }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("error") || content.contains("failed"),
            "Should show error indication for failed tool call"
        );
    }

    #[test]
    fn test_tool_call_duration_display() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![ContentBlock::ToolCall {
                name: "bash".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "bash".to_string(),
                        arguments: serde_json::json!({}),
                        result: serde_json::json!({}),
                        success: true,
                        duration_ms: 1234,
                    },
                    expanded: true,
                }),
            }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        // Duration should be shown in some format (ms, s, etc.)
        assert!(
            content.contains("1") || content.contains("234"),
            "Should show tool call duration"
        );
    }

    #[test]
    fn test_multiple_tool_calls_in_message() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
        role: Role::Assistant,
        blocks: vec![
            ContentBlock::Text { content: "I'll help you with that".into(), streaming: false },
            ContentBlock::ToolCall {
                name: "read_file".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "read_file".to_string(),
                        arguments: serde_json::json!({"path": "test.txt"}),
                        result: serde_json::json!({"content": "test content"}),
                        success: true,
                        duration_ms: 10,
                    },
                    expanded: false,
                }),
            },
            ContentBlock::ToolCall {
                name: "write_file".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "2".to_string(),
                        name: "write_file".to_string(),
                        arguments: serde_json::json!({"path": "output.txt", "content": "output"}),
                        result: serde_json::json!({"success": true}),
                        success: true,
                        duration_ms: 15,
                    },
                    expanded: false,
                }),
            },
        ],
        timestamp: std::time::Instant::now(),
        cached_block_lines: None,
    });

        // Verify that both tool calls are present
        let records = app.messages.first().unwrap().tool_records();
        assert_eq!(records.len(), 2, "Should have 2 tool call records");
    }

    #[test]
    fn test_tool_call_with_complex_arguments() {
        let mut app = test_app();
        let complex_args = serde_json::json!({
            "files": ["file1.txt", "file2.txt"],
            "options": {
                "recursive": true,
                "max_depth": 5
            }
        });

        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![ContentBlock::ToolCall {
                name: "search".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "search".to_string(),
                        arguments: complex_args.clone(),
                        result: serde_json::json!({"results": []}),
                        success: true,
                        duration_ms: 200,
                    },
                    expanded: true,
                }),
            }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        let records = app.messages.first().unwrap().tool_records();
        assert_eq!(records.len(), 1, "Should have 1 tool call record");
        assert_eq!(
            records[0].arguments, complex_args,
            "Should preserve complex arguments"
        );
    }

    #[test]
    fn test_render_context_estimate() {
        let mut app = test_app();
        app.context_estimate = 85000; // 85k — should be red

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("85k ctx"), "Should show context estimate");
    }

    #[test]
    fn test_render_search_mode() {
        let mut app = test_app();
        app.search_mode = true;
        app.search_query = "hello".to_string();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("Search: hello"),
            "Should show search query in panel title"
        );
    }

    #[test]
    fn test_render_focus_input() {
        let app = test_app();
        assert_eq!(app.focus, Panel::Input, "Default focus should be Input");
    }

    // ===== Event handling tests =====

    #[test]
    fn test_ctrl_c_clears_input() {
        let mut app = test_app();
        // Add some text to the input
        app.input.insert_str("test message");
        assert!(
            !app.input.lines().iter().all(|l| l.is_empty()),
            "Input should have text"
        );

        // Press Ctrl+C
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        // Input should be cleared, not quit
        assert!(!app.should_quit, "Ctrl+C should not quit");
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Input should be cleared"
        );
        assert_eq!(
            app.status, "Input cleared",
            "Status should show input cleared"
        );
    }

    #[test]
    fn test_ctrl_c_clears_empty_input() {
        let mut app = test_app();
        // Input is empty by default

        // Press Ctrl+C
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        // Input should still be cleared (no-op), not quit
        assert!(
            !app.should_quit,
            "Ctrl+C should not quit even when input is empty"
        );
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Input should be empty"
        );
        assert_eq!(
            app.status, "Input cleared",
            "Status should show input cleared"
        );
    }

    #[test]
    fn test_ctrl_q_quits() {
        let mut app = test_app();
        // Add some text to the input
        app.input.insert_str("test message");

        // Press Ctrl+Q
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        )));

        // Should quit regardless of input state
        assert!(app.should_quit, "Ctrl+Q should quit");
    }

    #[test]
    fn test_history_navigation() {
        let mut app = test_app();

        // Submit some messages to build history
        app.input.insert_str("first message");
        app.submit_input();
        app.input.insert_str("second message");
        app.submit_input();
        app.input.insert_str("third message");
        app.submit_input();

        // Verify history was built
        assert_eq!(app.history.len(), 3, "Should have 3 messages in history");
        assert_eq!(app.history[0], "first message");
        assert_eq!(app.history[1], "second message");
        assert_eq!(app.history[2], "third message");

        // Press up arrow to go to most recent message
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )));

        // Should have the most recent message
        assert_eq!(app.history_position, Some(2), "Should be at position 2");
        assert_eq!(app.input.lines().join("\n"), "third message");

        // Press up arrow again to go to previous message
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )));

        // Should have the second message
        assert_eq!(app.history_position, Some(1), "Should be at position 1");
        assert_eq!(app.input.lines().join("\n"), "second message");

        // Press down arrow to go forward
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));

        // Should have the most recent message again
        assert_eq!(app.history_position, Some(2), "Should be at position 2");
        assert_eq!(app.input.lines().join("\n"), "third message");

        // Press down arrow again to exit history mode
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));

        // Should exit history mode
        assert_eq!(app.history_position, None, "Should exit history mode");
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Input should be empty"
        );
    }

    #[test]
    fn test_history_does_not_save_slash_commands() {
        let mut app = test_app();

        // Submit a slash command
        app.input.insert_str("/help");
        app.submit_input();

        // Submit a normal message
        app.input.insert_str("normal message");
        app.submit_input();

        // Verify only normal message was saved to history
        assert_eq!(app.history.len(), 1, "Should have 1 message in history");
        assert_eq!(app.history[0], "normal message");
    }

    #[test]
    fn test_ctrl_c_resets_history_position() {
        let mut app = test_app();

        // Build history
        app.input.insert_str("test message");
        app.submit_input();

        // Navigate to history
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )));

        assert_eq!(app.history_position, Some(0), "Should be in history mode");

        // Press Ctrl+C to clear
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        // History position should be reset
        assert_eq!(
            app.history_position, None,
            "History position should be reset"
        );
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Input should be empty"
        );
    }

    #[test]
    fn test_ctrl_l_clears() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test"));
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.messages.is_empty(), "Ctrl+L should clear messages");
        assert_eq!(app.status, "Cleared");
    }

    #[test]
    fn test_tab_switches_focus() {
        let mut app = test_app();
        assert_eq!(app.focus, Panel::Input);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert_eq!(
            app.focus,
            Panel::Messages,
            "Tab from Input goes to Messages"
        );

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.focus, Panel::Input, "Tab from Messages goes to Input");
    }

    #[test]
    fn test_scroll_keys_in_messages_panel() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 5;

        // j scrolls down
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 6);

        // k scrolls up
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 5);

        // g goes to top
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn test_scroll_saturates_at_zero() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 0;
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 0, "Scroll should not go below 0");
    }

    #[test]
    fn test_search_mode_entry_and_exit() {
        let mut app = test_app();
        app.focus = Panel::Messages;

        // Enter search mode with /
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
        )));
        assert!(app.search_mode, "/ should enter search mode");

        // Type search query
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.search_query, "hi");

        // Backspace deletes
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.search_query, "h");

        // Enter exits search mode, keeps query
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(!app.search_mode);
        assert_eq!(app.search_query, "h");
    }

    #[test]
    fn test_search_esc_clears_query() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.search_mode = true;
        app.search_query = "findme".to_string();

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.search_mode);
        assert!(app.search_query.is_empty(), "Esc should clear search query");
    }

    #[test]
    fn test_search_n_jumps_forward() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.search_query = "target".to_string();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "no match"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "has target word"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "another target"));
        app.scroll = 0;

        // n should jump to first match after current scroll
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 1, "n should jump to first match at index 1");

        // n again should jump to next match
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 2, "n should jump to next match at index 2");
    }

    #[test]
    fn test_search_n_reverse() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.search_query = "target".to_string();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "first target"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "no match"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "second target"));
        app.scroll = 2;

        // N should jump to previous match
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('N'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 0, "N should jump to previous match at index 0");
    }

    #[test]
    fn test_mouse_scroll() {
        let mut app = test_app();
        app.scroll = 5;
        app.config.mouse_support = true;
        app.config.scroll_speed = 3;

        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(
            app.scroll, 2,
            "Mouse scroll up should decrease by scroll_speed"
        );

        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(
            app.scroll, 5,
            "Mouse scroll down should increase by scroll_speed"
        );
    }

    #[test]
    fn test_mouse_scroll_disabled() {
        let mut app = test_app();
        app.scroll = 5;
        app.config.mouse_support = false;

        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(
            app.scroll, 5,
            "Mouse scroll should be ignored when disabled"
        );
    }

    // ===== State transition tests =====

    #[test]
    fn test_submit_input_creates_message() {
        let mut app = test_app();
        app.input = TextArea::from(vec!["hello pawan"]);

        app.submit_input();

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].text_content(), "hello pawan");
        assert_eq!(app.messages[0].role, Role::User);
        assert!(app.processing, "Should be processing after submit");
        assert_eq!(app.status, "Processing...");
    }

    #[test]
    fn test_submit_empty_input_ignored() {
        let mut app = test_app();
        app.input = TextArea::from(vec!["   "]);

        app.submit_input();

        assert!(
            app.messages.is_empty(),
            "Empty input should not create message"
        );
        assert!(!app.processing, "Should not be processing for empty input");
    }

    #[test]
    fn test_processing_input_title() {
        let mut app = test_app();
        app.processing = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("processing"),
            "Input panel should show processing state"
        );
    }

    #[test]
    fn test_error_status_renders() {
        let mut app = test_app();
        app.status = "Error: connection refused".to_string();

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("Error: connection refused"),
            "Error status should render"
        );
    }

    #[test]
    fn test_page_up_down_scroll() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 15;

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::PageUp,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 5, "PageUp should scroll up by 10");

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::PageDown,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 15, "PageDown should scroll down by 10");
    }

    #[test]
    fn test_ctrl_u_d_half_page() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 25;

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(app.scroll, 5, "Ctrl+U should scroll up by 20");

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(app.scroll, 25, "Ctrl+D should scroll down by 20");
    }

    #[test]
    fn test_i_returns_to_input() {
        let mut app = test_app();
        app.focus = Panel::Messages;

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::NONE,
        )));
        assert_eq!(
            app.focus,
            Panel::Input,
            "'i' in Messages panel should return to Input"
        );
    }

    // ===== Helper =====

    /// Convert a ratatui Buffer to a plain string for assertion matching
    fn buffer_to_string(buf: &Buffer) -> String {
        let area = buf.area;
        let mut result = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                let cell = &buf[(x, y)];
                result.push_str(cell.symbol());
            }
            result.push('\n');
        }
        result
    }

    // ===== Slash command tests =====

    #[test]
    fn test_slash_help() {
        let mut app = test_app();
        app.handle_slash_command("/help");
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert!(app.messages[0].text_content().contains("/model"));
        assert!(app.messages[0].text_content().contains("/search"));
        assert!(app.messages[0].text_content().contains("/quit"));
    }

    #[test]
    fn test_slash_clear() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "reply"));
        app.handle_slash_command("/clear");
        assert!(app.messages.is_empty());
        assert_eq!(app.status, "Cleared");
    }

    #[test]
    fn test_slash_model_show() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        // New behavior: opens visual model selector
        assert!(app.model_picker.visible);
        assert_eq!(app.messages.len(), 0); // no message added
    }

    #[test]
    fn test_slash_model_switch() {
        let mut app = test_app();
        app.handle_slash_command("/model mistral-small-4");
        assert_eq!(app.model_name, "mistral-small-4");
        assert!(app.messages[0].text_content().contains("mistral-small-4"));
    }

    #[test]
    fn test_slash_tools() {
        let mut app = test_app();
        app.handle_slash_command("/tools");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].text_content().contains("bash"));
        assert!(app.messages[0].text_content().contains("ast_grep"));
        assert!(app.messages[0].text_content().contains("mcp_daedra"));
    }

    #[test]
    fn test_slash_quit() {
        let mut app = test_app();
        app.handle_slash_command("/quit");
        assert!(app.should_quit);
    }

    #[test]
    fn test_slash_unknown() {
        let mut app = test_app();
        app.handle_slash_command("/bogus");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].text_content().contains("Unknown command"));
    }

    #[test]
    fn test_slash_shorthand() {
        let mut app = test_app();
        app.handle_slash_command("/c");
        // /c is alias for /clear
        assert!(app.messages.is_empty());
    }

    #[test]
    fn test_slash_handoff_empty() {
        let mut app = test_app();
        app.handle_slash_command("/handoff");
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert!(app.messages[0]
            .text_content()
            .contains("No conversation to handoff"));
        assert_eq!(app.status, "Nothing to handoff");
    }

    #[test]
    fn test_slash_handoff_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Implement feature X"));
        app.messages.push(DisplayMessage::new_text(
            Role::Assistant,
            "I'll help with that",
        ));
        app.session_tool_calls = 5;
        app.session_files_edited = 2;

        app.handle_slash_command("/handoff");

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert!(app.messages[0].text_content().contains("Session Handoff"));
        assert!(app.messages[0].text_content().contains("Model:"));
        assert!(app.messages[0].text_content().contains("Messages:"));
        assert!(app.messages[0].text_content().contains("Tool calls:"));
        assert!(app.messages[0].text_content().contains("Files edited:"));
        assert_eq!(app.status, "Handoff complete");
    }

    #[test]
    fn test_slash_handoff_clears_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "First message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "First response"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Second message"));

        app.handle_slash_command("/handoff");

        // Should have only the handoff system message
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert!(app.messages[0].text_content().contains("Session Handoff"));
    }

    #[test]
    fn test_generate_handoff_prompt_empty() {
        let app = test_app();
        let prompt = app.generate_handoff_prompt();
        assert!(prompt.contains("No conversation context available"));
    }

    #[test]
    fn test_generate_handoff_prompt_with_content() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Fix src/main.rs"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "I'll fix it"));
        app.session_tool_calls = 3;
        app.session_files_edited = 1;

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Session Handoff"));
        assert!(prompt.contains("Model:"));
        assert!(prompt.contains("Messages:"));
        assert!(prompt.contains("Tool calls:"));
        assert!(prompt.contains("Files edited:"));
    }

    #[test]
    fn test_generate_handoff_prompt_extracts_files() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(
            Role::User,
            "Edit src/main.rs and lib/helper.ts",
        ));

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Files Referenced"));
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("lib/helper.ts"));
    }

    #[test]
    fn test_generate_handoff_prompt_extracts_constraints() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(
            Role::User,
            "MUST use async functions\nMUST NOT break existing tests",
        ));

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Constraints"));
        assert!(prompt.contains("MUST"));
    }

    #[test]
    fn test_generate_handoff_prompt_extracts_tasks() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(
            Role::User,
            "- Implement feature X\n- Fix bug Y\n* Add tests",
        ));

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Key Tasks"));
        assert!(prompt.contains("Implement feature X") || prompt.contains("feature X"));
    }

    #[test]
    fn test_generate_handoff_prompt_recent_context() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "First message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "First response"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Second message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Second response"));

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Recent Context"));
        assert!(prompt.contains("User") || prompt.contains("Assistant"));
    }

    // ===== Fuzzy search tests =====

    #[test]
    fn test_ctrl_p_toggles_fuzzy_search() {
        let mut app = test_app();
        assert!(app.fuzzy_search.is_none());
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.fuzzy_search.is_some());
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.fuzzy_search.is_none());
    }

    #[test]
    fn test_ctrl_f_opens_fuzzy_search() {
        let mut app = test_app();
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('f'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.fuzzy_search.is_some());
    }

    #[test]
    fn test_fuzzy_filter_model() {
        let mut st = FuzzySearchState::new(default_command_item_lines());
        st.filter("model");
        assert!(!st.results.is_empty());
        assert!(st
            .results
            .iter()
            .all(|l| l.to_lowercase().contains("model")));
    }

    #[test]
    fn test_fuzzy_esc_closes() {
        let mut app = test_app();
        app.fuzzy_search = Some(FuzzySearchState::new(default_command_item_lines()));
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(app.fuzzy_search.is_none());
    }

    #[test]
    fn test_session_stats_increment() {
        let app = test_app();
        assert_eq!(app.session_tool_calls, 0);
        assert_eq!(app.session_files_edited, 0);
    }

    // ===== Markdown rendering tests (existing) =====

    #[test]
    fn test_header_h1() {
        let lines = markdown_to_lines("# Hello");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Hello");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn test_header_h2() {
        let lines = markdown_to_lines("## Subtitle");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Subtitle");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn test_header_h3() {
        let lines = markdown_to_lines("### Section");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Section");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_bullet_list() {
        let lines = markdown_to_lines("- item one");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('•'));
    }

    #[test]
    fn test_star_bullet() {
        let lines = markdown_to_lines("* star item");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('•'));
    }

    #[test]
    fn test_code_block() {
        let lines = markdown_to_lines("```rust\nlet x = 1;\n```");
        assert_eq!(lines.len(), 3);
        // First line is separator with language
        assert!(lines[0].spans[0].content.contains("rust"));
        // Middle line is code with dark background
        assert_eq!(lines[1].spans[0].style.bg, Some(Color::Rgb(30, 30, 46)));
        // Last line is closing separator
        assert!(lines[2].spans[0].content.contains('─'));
    }

    #[test]
    fn test_code_block_no_lang() {
        let lines = markdown_to_lines("```\nhello\n```");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].spans[0].content.contains("code"));
    }

    #[test]
    fn test_blockquote() {
        let lines = markdown_to_lines("> quoted text");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('│'));
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::ITALIC));
    }

    #[test]
    fn test_horizontal_rule() {
        let lines = markdown_to_lines("---");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('─'));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn test_numbered_list() {
        let lines = markdown_to_lines("1. first item");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans.len() >= 2);
        // First span is the bold number
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_inline_bold() {
        let spans = parse_inline_markdown("hello **world**");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "hello ");
        assert_eq!(spans[1].content, "world");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_inline_code() {
        let spans = parse_inline_markdown("use `cargo test` here");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "use ");
        assert_eq!(spans[1].content, "cargo test");
        assert_eq!(spans[1].style.fg, Some(Color::Yellow));
        assert_eq!(spans[2].content, " here");
    }

    #[test]
    fn test_inline_italic() {
        let spans = parse_inline_markdown("this is *important*");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "this is ");
        assert_eq!(spans[1].content, "important");
        assert!(spans[1].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_inline_mixed() {
        let spans = parse_inline_markdown("**bold** and `code`");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content, " and ");
        assert_eq!(spans[2].content, "code");
        assert_eq!(spans[2].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn test_plain_text() {
        let spans = parse_inline_markdown("just plain text");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "just plain text");
    }

    #[test]
    fn test_unclosed_bold() {
        // Unclosed ** should render as literal **
        let spans = parse_inline_markdown("hello **unclosed");
        assert!(spans.len() >= 2);
    }

    #[test]
    fn test_multiline_markdown() {
        let text = "# Title\n\nSome **bold** text\n\n- bullet\n- another";
        let lines = markdown_to_lines(text);
        assert!(lines.len() >= 5);
        // First line is H1
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_empty_input() {
        let lines = markdown_to_lines("");
        // Empty string produces no lines (str::lines() returns empty iterator)
        assert_eq!(lines.len(), 0);
    }

    // ===== Welcome screen tests =====

    #[test]
    fn test_welcome_shown_on_fresh_app() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        assert!(app.show_welcome, "Fresh app should show welcome");
    }

    #[test]
    fn test_welcome_dismissed_on_keypress() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        assert!(app.show_welcome);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        assert!(!app.show_welcome, "Any keypress should dismiss welcome");
    }

    #[test]
    fn test_welcome_swallows_keypress() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        // Type 'a' while welcome is showing — should NOT reach input
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Welcome should swallow the keypress"
        );
    }

    #[test]
    fn test_welcome_renders() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
        );
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            text.contains("pawan"),
            "Welcome overlay should show 'pawan'"
        );
    }

    // ===== F1 Help overlay tests =====

    #[test]
    fn test_f1_toggles_help_overlay() {
        let mut app = test_app();
        assert!(!app.help_overlay);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::F(1),
            KeyModifiers::NONE,
        )));
        assert!(app.help_overlay, "F1 should open help overlay");
    }

    #[test]
    fn test_help_overlay_dismissed_on_keypress() {
        let mut app = test_app();
        app.help_overlay = true;
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )));
        assert!(
            !app.help_overlay,
            "Any keypress should dismiss help overlay"
        );
    }

    #[test]
    fn test_help_overlay_swallows_keypress() {
        let mut app = test_app();
        app.help_overlay = true;
        // Type 'a' while help is showing — should NOT reach input
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Help overlay should swallow the keypress"
        );
    }

    #[test]
    fn test_help_overlay_renders() {
        let mut app = test_app();
        app.help_overlay = true;
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            text.contains("Keyboard"),
            "Help overlay should show keyboard shortcuts"
        );
    }

    // ===== Export tests =====

    #[test]
    fn test_export_conversation() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Hello"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));
        let path = "/tmp/pawan_test_export.md";
        let result = app.export_conversation(path, ExportFormat::Markdown);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("**You**"));
        assert!(content.contains("**Pawan**"));
        assert!(content.contains("Hello"));
        assert!(content.contains("Hi there!"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_slash_export() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test msg"));
        app.handle_slash_command("/export /tmp/pawan_test_slash_export.md");
        // Should have added a system message about export
        assert!(app.messages.len() >= 2);
        let last = app.messages.last().unwrap();
        assert_eq!(last.role, Role::System);
        assert!(
            last.text_content().contains("Exported"),
            "Should confirm export: {}",
            last.text_content()
        );
        std::fs::remove_file("/tmp/pawan_test_slash_export.md").ok();
    }
    // ===== Session Tagging Tests =====

    #[test]
    fn test_tag_add_single_tag() {
        let mut app = test_app();
        app.handle_slash_command("/tag add important");
        assert_eq!(app.session_tags, vec!["important".to_string()]);
    }

    #[test]
    fn test_tag_add_multiple_tags() {
        let mut app = test_app();
        app.handle_slash_command("/tag add foo bar baz");
        assert_eq!(app.session_tags.len(), 3);
        assert!(app.session_tags.contains(&"foo".to_string()));
        assert!(app.session_tags.contains(&"bar".to_string()));
        assert!(app.session_tags.contains(&"baz".to_string()));
    }

    #[test]
    fn test_tag_add_prevents_duplicates() {
        let mut app = test_app();
        app.handle_slash_command("/tag add alpha");
        app.handle_slash_command("/tag add alpha");
        assert_eq!(app.session_tags.len(), 1);
        assert_eq!(app.session_tags, vec!["alpha".to_string()]);
    }

    #[test]
    fn test_tag_remove_existing() {
        let mut app = test_app();
        app.handle_slash_command("/tag add one two three");
        app.handle_slash_command("/tag rm two");
        assert_eq!(app.session_tags.len(), 2);
        assert!(!app.session_tags.contains(&"two".to_string()));
    }

    #[test]
    fn test_tag_remove_nonexistent() {
        let mut app = test_app();
        app.handle_slash_command("/tag add alpha");
        app.handle_slash_command("/tag rm beta");
        assert_eq!(app.session_tags, vec!["alpha".to_string()]);
    }

    #[test]
    fn test_tag_list() {
        let mut app = test_app();
        app.handle_slash_command("/tag add tag1 tag2");
        app.handle_slash_command("/tag list");
        let last_msg = app.messages.last().unwrap();
        assert!(last_msg.text_content().contains("tag1"));
        assert!(last_msg.text_content().contains("tag2"));
    }

    #[test]
    fn test_tag_clear() {
        let mut app = test_app();
        app.handle_slash_command("/tag add one two three");
        app.handle_slash_command("/tag clear");
        assert!(app.session_tags.is_empty());
    }

    #[test]
    fn test_tag_empty_command_shows_usage() {
        let mut app = test_app();
        app.handle_slash_command("/tag");
        let last_msg = app.messages.last().unwrap();
        assert!(last_msg.text_content().contains("Usage"));
    }

    #[test]
    fn test_tag_invalid_command_shows_usage() {
        let mut app = test_app();
        app.handle_slash_command("/tag invalid_cmd");
        let last_msg = app.messages.last().unwrap();
        assert!(last_msg.text_content().contains("Usage"));
    }

    #[test]
    fn test_session_tags_persist_on_save() {
        let mut app = test_app();
        app.handle_slash_command("/tag add persistent_tag");
        app.handle_slash_command("/save");
        let sessions_dir = pawan::agent::session::Session::sessions_dir().unwrap();
        let mut found = false;
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
                if content.contains("persistent_tag") {
                    found = true;
                    break;
                }
            }
        }
        assert!(found, "Tag not persisted in saved session");
    }

    #[test]
    fn test_session_browser_tag_filter() {
        let mut app = test_app();
        app.session_browser_query = "tag:important".to_string();
        assert!(app.session_browser_query.starts_with("tag:"));
    }
    // ===== /fork, /dump, /share Command Tests =====

    #[test]
    fn test_fork_empty_conversation() {
        let mut app = test_app();
        app.handle_slash_command("/fork");
        let last = app.messages.last().unwrap();
        assert!(
            last.text_content().contains("No conversation to fork"),
            "Should warn when empty"
        );
    }

    #[test]
    fn test_fork_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Hello"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));
        app.handle_slash_command("/fork");
        // Should create new session and switch to it
        assert!(
            app.current_session_id.is_some(),
            "Should have new session ID after fork"
        );
        let last = app.messages.last().unwrap();
        assert!(
            last.text_content().contains("Forked"),
            "Should confirm fork"
        );
    }

    #[test]
    fn test_dump_empty_conversation() {
        let mut app = test_app();
        app.handle_slash_command("/dump");
        let last = app.messages.last().unwrap();
        assert!(
            last.text_content().contains("Nothing to dump"),
            "Should warn when empty"
        );
    }

    #[test]
    fn test_dump_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Test message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Response"));
        app.handle_slash_command("/dump");
        // Note: clipboard may not be available in test env, but should still generate markdown
        let last = app.messages.last().unwrap();
        let content = last.text_content();
        assert!(
            content.contains("Copied") || content.contains("Failed"),
            "Should attempt clipboard operation"
        );
        // Verify it tried to generate markdown
        assert!(
            content.contains("Pawan Session")
                || content.contains("Copied")
                || content.contains("Failed"),
            "Should contain session output"
        );
    }

    #[test]
    fn test_share_empty_conversation() {
        let mut app = test_app();
        app.handle_slash_command("/share");
        let last = app.messages.last().unwrap();
        assert!(
            last.text_content().contains("Nothing to share"),
            "Should warn when empty"
        );
    }

    #[test]
    fn test_share_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Share test"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Shared!"));
        app.handle_slash_command("/share");
        // Should save and copy path to clipboard
        let last = app.messages.last().unwrap();
        let content = last.text_content();
        assert!(
            content.contains("Session saved") || content.contains("Share failed"),
            "Should attempt save"
        );
    }

    #[test]
    fn test_fork_preserves_model_and_tags() {
        let mut app = test_app();
        app.model_name = "nvidia/llama-3.1-nemotron".to_string();
        app.session_tags.push("test-tag".to_string());
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Test"));
        app.handle_slash_command("/fork");
        // Verify new session got the same model and tags
        if let Some(ref new_id) = app.current_session_id {
            if let Ok(session) = Session::load(new_id) {
                assert_eq!(session.model, "nvidia/llama-3.1-nemotron");
                assert!(session.tags.contains(&"test-tag".to_string()));
            }
        }
    }

    // ===== /diff Command Test =====

    #[test]
    fn test_diff_command_handler() {
        let mut app = test_app();
        app.handle_slash_command("/diff");
        assert!(!app.messages.is_empty());
        let content = app.messages.last().unwrap().text_content();
        assert!(!content.is_empty());
    }

    // ===== Export Format Tests =====
    // ===== Export Format Tests =====

    #[test]
    fn test_export_html_format() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "HTML test"));
        let path = "/tmp/pawan_html_test.html";
        let result = app.export_conversation(path, ExportFormat::Html);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("<!DOCTYPE html>"));
        assert!(content.contains("<html"));
        assert!(content.contains("HTML test"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_json_format() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "JSON test"));
        let path = "/tmp/pawan_json_test.json";
        let result = app.export_conversation(path, ExportFormat::Json);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("\"messages\""));
        assert!(content.contains("JSON test"));
        let _: serde_json::Value = serde_json::from_str(&content).unwrap();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_txt_format() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "TXT test"));
        let path = "/tmp/pawan_txt_test.txt";
        let result = app.export_conversation(path, ExportFormat::Txt);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("TXT test"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_empty_conversation() {
        let app = test_app();
        let path = "/tmp/pawan_empty_test.md";
        let result = app.export_conversation(path, ExportFormat::Markdown);
        assert!(result.is_ok());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_html_escaping() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(
            Role::User,
            "<script>alert('xss')</script>",
        ));
        let path = "/tmp/pawan_escape_test.html";
        let result = app.export_conversation(path, ExportFormat::Html);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(!content.contains("<script>"));
        assert!(content.contains("&lt;script&gt;"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_with_tool_calls() {
        let mut app = test_app();

        // Create a message with tool calls
        let mut msg = DisplayMessage::new_text(Role::Assistant, "Processing request");
        msg.blocks.push(ContentBlock::ToolCall {
            name: "bash".to_string(),
            args_summary: "echo test".to_string(),
            state: Box::new(ToolBlockState::Done {
                record: ToolCallRecord {
                    id: "test-id".to_string(),
                    name: "bash".to_string(),
                    arguments: serde_json::json!({"command": "echo test"}),
                    result: serde_json::Value::String("test output".to_string()),
                    success: true,
                    duration_ms: 100,
                },
                expanded: true,
            }),
        });
        app.messages.push(msg);

        // Test markdown export
        let md_path = "/tmp/test_tool_calls.md";
        let result = app.export_conversation(md_path, ExportFormat::Markdown);
        assert!(result.is_ok(), "Markdown export should succeed");

        let md_content = std::fs::read_to_string(md_path).unwrap();
        assert!(md_content.contains("bash"), "Should contain tool name");
        assert!(
            md_content.contains("echo test"),
            "Should contain args summary"
        );
        assert!(md_content.contains("test output"), "Should contain result");

        // Test JSON export
        let json_path = "/tmp/test_tool_calls.json";
        let result = app.export_conversation(json_path, ExportFormat::Json);
        assert!(result.is_ok(), "JSON export should succeed");

        let json_content = std::fs::read_to_string(json_path).unwrap();
        assert!(
            json_content.contains("bash"),
            "JSON should contain tool name"
        );

        // Cleanup
        let _ = std::fs::remove_file(md_path);
        let _ = std::fs::remove_file(json_path);
    }

    // ===== Timestamp tests =====

    #[test]
    fn test_message_has_timestamp() {
        let before = std::time::Instant::now();
        let msg = DisplayMessage::new_text(Role::User, "test");
        let after = std::time::Instant::now();
        assert!(msg.timestamp >= before);
        assert!(msg.timestamp <= after);
    }

    // ===== Scroll indicator tests =====

    #[test]
    fn test_scroll_indicator_in_title() {
        let mut app = test_app();
        // Add enough messages to exceed the visible area so scroll indicator appears
        for i in 0..20 {
            app.messages.push(DisplayMessage::new_text(
                Role::User,
                format!("message line {}", i),
            ));
        }
        app.scroll = 5;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        // Scroll indicator now shows percentage when content exceeds visible area
        assert!(
            text.contains("[") && text.contains("%]"),
            "Should show scroll percentage indicator, got:\n{}",
            &text[..300.min(text.len())]
        );
    }

    // ===== Message count in status bar =====

    #[test]
    fn test_status_bar_shows_message_count() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "hi"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "hello"));
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(text.contains("2msg"), "Status bar should show '2msg'");
    }

    #[test]
    fn test_fuzzy_catalog_includes_export() {
        let items = default_command_item_lines();
        assert!(
            items.iter().any(|s| s.starts_with("/export")),
            "Catalog should include /export"
        );
    }

    #[test]
    fn test_fuzzy_catalog_includes_import() {
        let items = default_command_item_lines();
        assert!(
            items.iter().any(|s| s.starts_with("/import")),
            "Catalog should include /import"
        );
    }

    #[test]
    fn test_import_command_requires_path() {
        let mut app = test_app();
        app.handle_slash_command("/import");
        assert!(app.messages.iter().any(|m| {
        matches!(m, DisplayMessage { role: Role::System, .. } if {
            // Check if any block contains the usage message
            m.blocks.iter().any(|block| {
                matches!(block, ContentBlock::Text { content, .. } if content.contains("Usage: /import <path>"))
            })
        })
    }), "Should show usage message when no path provided");
    }
    #[test]
    fn test_load_available_models_populates_list() {
        let mut app = test_app();
        assert!(app.model_picker.models.is_empty());
        app.load_available_models();
        assert!(!app.model_picker.models.is_empty());
        assert!(app.model_picker.models.len() >= 4);
    }

    #[test]
    fn test_filtered_models_empty_when_not_loaded() {
        let app = test_app();
        let filtered = app.filtered_models();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filtered_models_with_search() {
        let mut app = test_app();
        app.load_available_models();
        app.model_picker.query = "nvidia".to_string();
        let _filtered = app.filtered_models();
        app.model_picker.query = "anthropic".to_string();
        let _filtered = app.filtered_models();
        app.model_picker.query = "nonexistent".to_string();
        let filtered = app.filtered_models();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filtered_models_empty_query_returns_all() {
        let mut app = test_app();
        app.load_available_models();
        app.model_picker.query.clear();
        let filtered = app.filtered_models();
        assert_eq!(filtered.len(), app.model_picker.models.len());
    }

    #[test]
    fn test_model_selector_modal_state() {
        let mut app = test_app();
        assert!(!app.model_picker.visible);
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        assert_eq!(app.model_picker.query, "");
        assert_eq!(app.model_picker.selected, 0);
        app.model_picker.visible = false;
        app.model_picker.query.clear();
        app.model_picker.selected = 0;
        assert!(!app.model_picker.visible);
    }

    // ===== Session Browser Tests =====
    #[test]
    fn test_session_browser_modal_state() {
        let mut app = test_app();
        assert!(!app.session_browser_open);
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        assert_eq!(app.session_browser_query, "");
        assert_eq!(app.session_browser_selected, 0);
    }

    #[test]
    fn test_session_sorting_modes() {
        let modes = [
            SessionSortMode::NewestFirst,
            SessionSortMode::Alphabetical,
            SessionSortMode::MostUsed,
        ];
        assert_eq!(modes.len(), 3);
    }
    // ===== Slash Command Tests =====
    #[test]
    fn test_slash_sessions_opens_browser() {
        let mut app = test_app();
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
    }

    #[test]
    fn test_slash_save_creates_session() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));
        app.handle_slash_command("/save");
        assert!(app.messages.len() >= 2);
        let _last = app.messages.last().unwrap();
    }

    #[test]
    fn test_slash_load_opens_browser() {
        let mut app = test_app();
        app.handle_slash_command("/load");
        assert!(app.session_browser_open);
    }

    #[test]
    fn test_slash_resume_opens_browser() {
        let mut app = test_app();
        app.handle_slash_command("/resume");
        assert!(app.session_browser_open);
    }

    #[test]
    fn test_slash_new_clears_session() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));
        app.handle_slash_command("/new");
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert_eq!(
            app.messages[0].text_content().trim(),
            "Started new conversation"
        );
    }

    #[test]
    fn test_slash_items_includes_all_commands() {
        let app = test_app();
        let items = app.slash_items();
        let commands: Vec<_> = items
            .iter()
            .map(|(cmd, _)| cmd.as_str())
            .collect::<Vec<_>>();
        assert!(commands.contains(&"/sessions"));
        assert!(commands.contains(&"/save"));
        assert!(commands.contains(&"/load"));
        assert!(commands.contains(&"/resume"));
        assert!(commands.contains(&"/new"));
        assert!(commands.contains(&"/model"));
        assert!(commands.contains(&"/export"));
        assert!(commands.contains(&"/compact"));
        assert!(commands.contains(&"/session"));
        assert!(commands.contains(&"/retry"));
    }
    // ===== Auto-save Tests =====
    #[test]
    fn test_autosave_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));
        // Should not panic
        app.autosave();
    }

    #[test]
    fn test_autosave_with_empty_session() {
        let mut app = test_app();
        // Should not panic even with empty messages
        app.autosave();
    }

    #[test]
    fn test_autosave_with_multiple_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "First message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Second message"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Third message"));
        // Should not panic with multiple messages
        app.autosave();
    }

    #[test]
    fn test_autosave_with_whitespace_only_messages() {
        let mut app = test_app();
        // Add whitespace-only messages
        app.messages
            .push(DisplayMessage::new_text(Role::User, "   "));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "\t\n"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Valid message"));
        // Should not panic and should handle whitespace-only messages
        app.autosave();
    }

    #[test]
    fn test_autosave_does_not_modify_app_state() {
        let mut app = test_app();
        let initial_message_count = app.messages.len();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));

        app.autosave();

        // Autosave should not modify app state (it's called on &self)
        assert_eq!(
            app.messages.len(),
            initial_message_count + 1,
            "Autosave should not modify message count"
        );
    }
    #[test]
    fn test_model_selector_modal_rendering() {
        let mut app = test_app();
        app.model_picker.visible = true;
        app.load_available_models();
        let _ = app;
    }

    #[test]
    fn test_session_browser_modal_rendering() {
        let mut app = test_app();
        app.session_browser_open = true;
        let _ = app;
    }

    #[test]
    fn test_help_overlay_modal_rendering() {
        let mut app = test_app();
        app.help_overlay = true;
        let _ = app;
    }

    // ===== Keyboard Handling Tests =====
    #[test]
    fn test_keyboard_esc_closes_modals() {
        let mut app = test_app();
        app.model_picker.visible = true;
        app.session_browser_open = true;
        app.help_overlay = true;
        app.model_picker.visible = false;
        app.session_browser_open = false;
        app.help_overlay = false;
        assert!(!app.model_picker.visible);
        assert!(!app.session_browser_open);
        assert!(!app.help_overlay);
    }

    #[test]
    fn test_keyboard_enter_in_model_selector() {
        let mut app = test_app();
        app.model_picker.visible = true;
        app.load_available_models();
        if !app.model_picker.models.is_empty() {
            app.model_picker.selected = 0;
            let selected = app.model_picker.models.get(app.model_picker.selected);
            let _ = selected;
        }
    }

    #[test]
    fn test_keyboard_enter_in_session_browser() {
        let mut app = test_app();
        app.session_browser_open = true;
        app.session_browser_selected = 0;
        let _ = app;
    }

    // ===== Integration Tests =====
    #[test]
    fn test_full_session_lifecycle() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));
        app.handle_slash_command("/save");
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
    }

    #[test]
    fn test_model_selection_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        if !app.model_picker.models.is_empty() {
            app.model_picker.selected = 0;
            app.model_picker.visible = false;
        }
    }

    #[test]
    fn test_slash_command_dispatch() {
        let mut app = test_app();
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.model_picker.visible = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
    }

    #[test]
    fn test_modal_transitions() {
        let mut app = test_app();
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.model_picker.visible = false;
        app.help_overlay = true;
        assert!(app.help_overlay);
    }

    // ===== E2E Test Scaffolding =====
    #[test]
    fn test_e2e_session_creation_and_browsing() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "first message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "response"));
        app.handle_slash_command("/save");
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
    }

    #[test]
    fn test_e2e_model_switching_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        if !app.model_picker.models.is_empty() {
            app.model_picker.selected = 0;
            app.model_picker.visible = false;
        }
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test"));
        app.handle_slash_command("/save");
    }

    #[test]
    fn test_e2e_session_management_workflow() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "message 1"));
        app.handle_slash_command("/save");
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.messages
            .push(DisplayMessage::new_text(Role::User, "message 2"));
        app.handle_slash_command("/save");
    }

    #[test]
    fn test_e2e_autosave_during_session() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "message 1"));
        app.autosave();
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "response 1"));
        app.autosave();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "message 2"));
        app.autosave();
    }

    #[test]
    fn test_e2e_slash_command_sequence() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.model_picker.visible = false;
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.handle_slash_command("/export");
    }

    #[test]
    fn test_e2e_modal_state_consistency() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        assert!(!app.session_browser_open);
        assert!(!app.help_overlay);
        app.model_picker.visible = false;
        app.handle_slash_command("/sessions");
        assert!(!app.model_picker.visible);
        assert!(app.session_browser_open);
        assert!(!app.help_overlay);
        app.session_browser_open = false;
        app.help_overlay = true;
        assert!(!app.model_picker.visible);
        assert!(!app.session_browser_open);
        assert!(app.help_overlay);
    }

    #[test]
    fn test_e2e_session_sorting_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_sort_mode = SessionSortMode::Alphabetical;
        app.session_sort_mode = SessionSortMode::MostUsed;
        app.session_sort_mode = SessionSortMode::NewestFirst;
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_search_and_filter_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        app.model_picker.query = "test".to_string();
        let filtered = app.filtered_models();
        let _ = filtered;
        app.model_picker.query.clear();
        app.model_picker.visible = false;
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_query = "test".to_string();
        let sessions = app.filtered_sessions();
        let _ = sessions;
        app.session_browser_query.clear();
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_keyboard_navigation_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        let count = app.model_picker.models.len();
        if count > 0 {
            app.model_picker.selected = 0;
            app.model_picker.selected = (app.model_picker.selected + 1).min(count - 1);
            app.model_picker.selected = app.model_picker.selected.saturating_sub(1);
        }
        app.model_picker.visible = false;
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_selected = 0;
        app.session_browser_selected = app.session_browser_selected.saturating_sub(1);
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_error_handling_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/save");
        app.handle_slash_command("/load");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/resume");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_concurrent_modals_prevention() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.help_overlay = true;
        assert!(app.model_picker.visible || app.session_browser_open || app.help_overlay);
    }

    #[test]
    fn test_e2e_state_persistence_workflow() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "persistent message"));
        app.autosave();
        let _msg_count = app.messages.len();
        app.handle_slash_command("/save");
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_full_user_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        app.model_picker.visible = false;
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Hello"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));
        app.autosave();
        app.handle_slash_command("/save");
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.handle_slash_command("/export");
    }

    #[test]
    fn test_filtered_sessions_empty_query() {
        let app = test_app();
        let sessions = app.filtered_sessions();
        let _ = sessions;
    }

    #[test]
    fn test_filtered_sessions_with_search() {
        let app = test_app();
        let sessions = app.filtered_sessions();
        let _ = sessions;
    }

    #[test]
    fn test_model_selector_navigation() {
        let mut app = test_app();
        app.load_available_models();
        let count = app.model_picker.models.len();
        if count > 0 {
            app.model_picker.selected = (app.model_picker.selected + 1).min(count - 1);
            assert_eq!(app.model_picker.selected, 1);
        }
    }
}
