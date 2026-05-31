//! Message and markdown rendering.

#![allow(unused_imports)]

use crate::tui::app::App;
use crate::tui::highlight::SyntaxHighlighter;
use crate::tui::theme::{self, current as theme_current};
use crate::tui::types::*;
use pawan::agent::Role;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

impl<'a> App<'a> {
    pub(crate) fn render_messages(&self, f: &mut Frame, area: Rect) {
        let theme = theme_current();
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

        let total_lines = lines.len();
        let visible_height = area.height as usize;
        let max_offset = total_lines.saturating_sub(visible_height);
        let scroll_offset = if self.scroll == usize::MAX {
            max_offset // auto-scroll to bottom
        } else {
            self.scroll.min(max_offset)
        };

        // Subtle scroll indicator: bottom-right percentage
        let scroll_pct = if total_lines > visible_height {
            let pct = (scroll_offset * 100).checked_div(max_offset).unwrap_or(100);
            format!("[{}%]", pct)
        } else {
            String::new()
        };

        // Search indicator: subtle top overlay
        let search_hint = if self.search_mode {
            format!(" Search: {}\u{258c}", self.search_query)
        } else if !self.search_query.is_empty() {
            format!(" [/{}] n/N", self.search_query)
        } else {
            String::new()
        };

        let paragraph = Paragraph::new(lines)
            .style(Style::default().fg(theme.foreground).bg(theme.surface))
            .scroll((scroll_offset as u16, 0));
        f.render_widget(paragraph, area);

        // Render search hint as a subtle top overlay
        if !search_hint.is_empty() {
            let hint_line = Line::from(vec![Span::styled(
                search_hint,
                Style::default().fg(Color::Yellow),
            )]);
            let hint_area = Rect::new(area.x, area.y, area.width.min(40), 1);
            f.render_widget(Paragraph::new(hint_line), hint_area);
        }

        // Render scroll % as a subtle bottom-right overlay
        if !scroll_pct.is_empty() {
            let pct_w = scroll_pct.len() as u16 + 1;
            let pct_area = Rect::new(
                area.x + area.width.saturating_sub(pct_w),
                area.y + area.height.saturating_sub(1),
                pct_w,
                1,
            );
            let pct_line = Line::from(vec![Span::styled(
                scroll_pct,
                Style::default().fg(theme.muted),
            )]);
            f.render_widget(Paragraph::new(pct_line), pct_area);
        }
    }

    /// Render a single DisplayMessage into Lines.
    /// Uses cached block lines when available (populated by `block_lines_cached()`).
    pub(crate) fn render_message_to_lines(
        &self,
        msg: &DisplayMessage,
        now: std::time::Instant,
        lines: &mut Vec<Line<'static>>,
    ) {
        let theme = theme_current();
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
            Span::styled(format!("({})", time_str), Style::default().fg(theme.muted)),
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
        let theme = theme_current();
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
                            Style::default().fg(theme.muted),
                        ));
                    }
                    spans.push(Span::styled(
                        format!(" {}ms", record.duration_ms),
                        Style::default().fg(theme.muted),
                    ));
                    lines.push(Line::from(spans));

                    if *expanded {
                        let result_str = format_tool_result(&record.result);
                        for result_line in result_str.lines().take(20) {
                            lines.push(Line::from(Span::styled(
                                format!("    {}", result_line),
                                Style::default().fg(theme.muted),
                            )));
                        }
                        let total = result_str.lines().count();
                        if total > 20 {
                            lines.push(Line::from(Span::styled(
                                format!("    ... ({} more lines)", total - 20),
                                Style::default()
                                    .fg(theme.muted)
                                    .add_modifier(Modifier::ITALIC),
                            )));
                        }
                    } else {
                        let preview = one_line_preview(&record.result, 60);
                        if !preview.is_empty() {
                            lines.push(Line::from(Span::styled(
                                format!("    {}", preview),
                                Style::default().fg(theme.muted).add_modifier(Modifier::DIM),
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
}

/// Highlight a single line of fenced code using the active UI palette name.
pub(super) fn highlight_markdown_code_line(line: &str, lang: &str) -> Vec<Line<'static>> {
    static HL: OnceLock<Mutex<(String, SyntaxHighlighter)>> = OnceLock::new();
    let lock = HL.get_or_init(|| {
        let name = theme::current().name.to_string();
        let hi = SyntaxHighlighter::new(&name)
            .unwrap_or_else(|_| SyntaxHighlighter::new("dracula").expect("dracula theme resolves"));
        Mutex::new((name, hi))
    });

    let mut guard = lock.lock().unwrap();
    let cur = theme::current().name.to_string();
    if guard.0 != cur {
        match SyntaxHighlighter::new(&cur) {
            Ok(h) => {
                guard.0 = cur;
                guard.1 = h;
            }
            Err(_) => {
                let t = theme_current();
                return vec![Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(t.foreground).bg(t.code_bg),
                ))];
            }
        }
    }

    guard.1.highlight(line, lang)
}

/// Parse markdown text into styled ratatui Lines
pub(super) fn markdown_to_lines(text: &str) -> Vec<Line<'static>> {
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
                    Style::default().fg(theme_current().muted),
                )));
            } else {
                in_code_block = false;
                code_lang.clear();
                lines_out.push(Line::from(Span::styled(
                    "────────────".to_string(),
                    Style::default().fg(theme_current().muted),
                )));
            }
            continue;
        }

        if in_code_block {
            let hl_lines = highlight_markdown_code_line(line, &code_lang);
            if hl_lines.is_empty() {
                lines_out.push(Line::default());
            } else {
                for hl_line in hl_lines {
                    lines_out.push(hl_line);
                }
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("### ") {
            lines_out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(theme_current().foreground)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(rest) = line.strip_prefix("## ") {
            lines_out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(theme_current().accent_dim)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(rest) = line.strip_prefix("# ") {
            lines_out.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(theme_current().accent)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
        } else if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            let mut spans = vec![Span::styled(
                "• ".to_string(),
                Style::default().fg(theme_current().muted),
            )];
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
                    Style::default().fg(theme_current().muted),
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
                    .fg(theme_current().muted)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else if line.chars().all(|c| c == '-' || c == '=') && line.len() >= 3 {
            // Horizontal rule
            lines_out.push(Line::from(Span::styled(
                "─".repeat(40),
                Style::default().fg(theme_current().muted),
            )));
        } else {
            lines_out.push(Line::from(parse_inline_markdown(line)));
        }
    }

    lines_out
}

/// Parse inline markdown: **bold**, `code`, *italic*
pub(super) fn parse_inline_markdown(text: &str) -> Vec<Span<'static>> {
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
                        let t = theme_current();
                        spans.push(Span::styled(
                            after[..end].to_string(),
                            Style::default().fg(t.accent).bg(t.code_bg),
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
