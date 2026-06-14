//! Popups, dialogs, and selector overlays.

#![allow(unused_imports)]

use crate::tui::app::App;
use crate::tui::theme::current as theme_current;
use crate::tui::types::*;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

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
    pub(crate) fn render_model_selector(&self, f: &mut Frame) -> Rect {
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

        let catalog_label = format!(
            "{} / {} shown",
            self.model_picker.source.label(),
            models.len()
        );
        let search_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Blue)),
            Span::styled(&self.model_picker.query, Style::default().fg(Color::White)),
            Span::styled(
                "▌",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(catalog_label, Style::default().fg(theme_current().muted)),
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
                    Style::default().fg(theme_current().muted)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", badge), badge_style),
                    Span::styled(model.id.clone(), line_style),
                    Span::styled(format!("  q{} ", model.quality_score), score_st),
                ]))
            })
            .collect();
        f.render_widget(List::new(visible_items), list_area);
        selector_area
    }
    pub(crate) fn render_permission_dialog(&self, f: &mut Frame) -> Rect {
        let dialog = match &self.permission_dialog {
            Some(d) => d,
            None => return Rect::new(0, 0, 0, 0),
        };
        let theme = theme_current();
        let area = f.area();

        let width = 66u16.min(area.width.saturating_sub(4)).max(40);
        // Inner text width: border (2) + horizontal padding (2*2).
        let inner_w = width.saturating_sub(6).max(8) as usize;

        // Wrap the argument summary across up to three lines (char-based so it
        // never panics on long unbroken tokens like file paths).
        let args = dialog.args_summary.trim();
        let arg_lines: Vec<String> = if args.is_empty() {
            Vec::new()
        } else {
            let chars: Vec<char> = args.chars().collect();
            let mut rows: Vec<String> = chars
                .chunks(inner_w)
                .take(3)
                .map(|c| c.iter().collect())
                .collect();
            if chars.len() > inner_w * 3 {
                if let Some(last) = rows.last_mut() {
                    last.pop();
                    last.push('…');
                }
            }
            rows
        };

        let mut text: Vec<Line> = Vec::new();
        text.push(Line::from(vec![
            Span::styled("tool  ", Style::default().fg(theme.muted)),
            Span::styled(
                dialog.tool_name.clone(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        text.push(Line::from(""));
        text.push(Line::from(Span::styled(
            "arguments",
            Style::default().fg(theme.muted),
        )));
        if arg_lines.is_empty() {
            text.push(Line::from(Span::styled(
                "(none)",
                Style::default()
                    .fg(theme.subtle)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else {
            for l in &arg_lines {
                text.push(Line::from(Span::styled(
                    l.clone(),
                    Style::default().fg(theme.foreground),
                )));
            }
        }
        text.push(Line::from(""));

        let key = |k: &str, bg: Color| {
            Span::styled(
                format!(" {k} "),
                Style::default()
                    .fg(theme.background)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            )
        };
        text.push(Line::from(vec![
            key("Y", theme.success),
            Span::styled(" Allow   ", Style::default().fg(theme.foreground)),
            key("N", theme.error),
            Span::styled(" Deny   ", Style::default().fg(theme.foreground)),
            key("A", theme.accent),
            Span::styled(" Allow all", Style::default().fg(theme.foreground)),
        ]));

        let content_h = text.len() as u16;
        let height = (content_h + 4).min(area.height.saturating_sub(2)); // padding(2) + border(2)
        let popup_area = area.centered(Constraint::Length(width), Constraint::Length(height));

        // Drop shadow for an elevated feel.
        if popup_area.x + width < area.width && popup_area.y + height < area.height {
            let shadow =
                ratatui::layout::Rect::new(popup_area.x + 1, popup_area.y + 1, width, height);
            f.render_widget(ratatui::widgets::Clear, shadow);
            f.render_widget(
                Block::default().style(Style::default().bg(theme.background)),
                shadow,
            );
        }

        f.render_widget(ratatui::widgets::Clear, popup_area);

        let block = Block::default()
            .title(Span::styled(
                " ⚠  Permission Required ",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focused))
            .padding(ratatui::widgets::Padding::new(2, 2, 1, 1))
            .style(Style::default().bg(theme.surface_elevated));
        f.render_widget(Paragraph::new(text).block(block), popup_area);
        popup_area
    }

    /// Check if the inline slash popup should be shown.
    pub(crate) fn is_slash_popup_active(&self) -> bool {
        let text: String = self.input.lines().join("\n");
        let trimmed = text.trim();
        (trimmed.starts_with('/') || trimmed.starts_with(':')) && !trimmed.contains(' ')
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
                            Style::default().fg(theme_current().muted)
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

    pub(crate) fn render_help_overlay(&self, f: &mut Frame) -> Rect {
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
        help_area
    }

    pub(crate) fn render_fuzzy_search(&self, f: &mut Frame) -> Rect {
        let Some(fs) = &self.fuzzy_search else {
            return Rect::new(0, 0, 0, 0);
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
        modal_area
    }
}
