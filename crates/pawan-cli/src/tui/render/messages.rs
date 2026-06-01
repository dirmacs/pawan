//! Message and markdown rendering.

#![allow(unused_imports)]

use crate::tui::app::App;
use crate::tui::highlight::SyntaxHighlighter;
use crate::tui::theme::{self, current as theme_current};
use crate::tui::types::*;
use pawan::agent::Role;
use ratatui::layout::{Position, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

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
            let has_blocks = self
                .streaming
                .as_ref()
                .map(|s| !s.blocks.is_empty())
                .unwrap_or(false);
            if has_blocks {
                lines.push(Line::from(vec![Span::styled(
                    " Pawan ",
                    Style::default()
                        .fg(theme.background)
                        .bg(theme.success)
                        .add_modifier(Modifier::BOLD),
                )]));
                if let Some(state) = self.streaming.as_ref() {
                    for block in &state.blocks {
                        Self::render_block_to_lines(block, true, &mut lines);
                    }
                }
            } else {
                // Animated spinner (ratatui-cheese) replaces the static label.
                lines.push(Line::from(vec![Span::styled(
                    format!("  {} Pawan is thinking…", self.spinner.frame_str()),
                    Style::default()
                        .fg(theme.warning)
                        .add_modifier(Modifier::ITALIC),
                )]));
            }
        }

        let content_h = lines.len().max(1).min(u16::MAX as usize) as u16;
        let content_w = area.width.max(1);

        // Search indicator: subtle top overlay
        let search_hint = if self.search_mode {
            format!(" Search: {}\u{258c}", self.search_query)
        } else if !self.search_query.is_empty() {
            format!(" [/{}] n/N", self.search_query)
        } else {
            String::new()
        };

        // tui-scrollview owns offset clamping + scrollbar rendering, replacing the
        // previous manual `Paragraph::scroll` math and bespoke percentage overlay.
        let mut scroll_view = ScrollView::new(Size::new(content_w, content_h))
            .horizontal_scrollbar_visibility(ScrollbarVisibility::Never)
            .vertical_scrollbar_visibility(ScrollbarVisibility::Automatic);
        scroll_view.render_widget(
            Paragraph::new(lines).style(Style::default().fg(theme.foreground).bg(theme.surface)),
            Rect::new(0, 0, content_w, content_h),
        );

        let mut sv_state = ScrollViewState::new();
        if self.scroll == usize::MAX {
            sv_state.scroll_to_bottom();
        } else {
            sv_state.set_offset(Position::new(0, self.scroll.min(u16::MAX as usize) as u16));
        }
        f.render_stateful_widget(scroll_view, area, &mut sv_state);

        // Render search hint as a subtle top overlay
        if !search_hint.is_empty() {
            let hint_line = Line::from(vec![Span::styled(
                search_hint,
                Style::default().fg(Color::Yellow),
            )]);
            let hint_area = Rect::new(area.x, area.y, area.width.min(40), 1);
            f.render_widget(Paragraph::new(hint_line), hint_area);
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
        let (label, badge_bg) = match msg.role {
            Role::User => (" You ", theme.accent),
            Role::Assistant => (" Pawan ", theme.success),
            Role::System => (" System ", theme.warning),
            Role::Tool => (" Tool ", theme.accent_dim),
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
            Span::styled(
                label,
                Style::default()
                    .fg(theme.background)
                    .bg(badge_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {}", time_str), Style::default().fg(theme.muted)),
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
            } => {
                // Leading spacer gives each tool call breathing room in the flow.
                lines.push(Line::from(""));
                match state.as_ref() {
                    ToolBlockState::Running => {
                        lines.push(Line::from(vec![
                            Span::styled("  ╭ ", Style::default().fg(theme.accent_dim)),
                            Span::styled("⚙ ", Style::default().fg(theme.warning)),
                            Span::styled(
                                format!("running {}", name),
                                Style::default()
                                    .fg(theme.warning)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(" …", Style::default().fg(theme.muted)),
                        ]));
                    }
                    ToolBlockState::Done { record, expanded } => {
                        let (icon, color) = if record.success {
                            ("✓", theme.tool_success)
                        } else {
                            ("✗", theme.tool_error)
                        };
                        // Header row: icon + tool name + args + duration.
                        let mut header = vec![
                            Span::styled("  ╭─ ", Style::default().fg(theme.accent_dim)),
                            Span::styled(
                                format!("{} ", icon),
                                Style::default().fg(color).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                name.clone(),
                                Style::default()
                                    .fg(theme.accent)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ];
                        if !args_summary.is_empty() {
                            header.push(Span::styled(
                                format!(" {}", args_summary),
                                Style::default().fg(theme.muted),
                            ));
                        }
                        header.push(Span::styled(
                            format!("  · {}ms", record.duration_ms),
                            Style::default().fg(theme.subtle),
                        ));
                        lines.push(Line::from(header));

                        // Body: result framed with a left accent bar. Collapsed
                        // still shows a useful window (6 lines); expanded shows up
                        // to 40 so users can actually read what executed.
                        let result_str = format_tool_result(&record.result);
                        let total = result_str.lines().count();
                        let max_lines = if *expanded { 40 } else { 6 };
                        let mut shown = 0usize;
                        for result_line in result_str.lines().take(max_lines) {
                            lines.push(Line::from(vec![
                                Span::styled("  │ ", Style::default().fg(theme.accent_dim)),
                                Span::styled(
                                    result_line.to_string(),
                                    Style::default().fg(theme.foreground),
                                ),
                            ]));
                            shown += 1;
                        }
                        if total > shown {
                            lines.push(Line::from(vec![
                                Span::styled("  │ ", Style::default().fg(theme.accent_dim)),
                                Span::styled(
                                    format!("… {} more lines — press e to expand", total - shown),
                                    Style::default()
                                        .fg(theme.muted)
                                        .add_modifier(Modifier::ITALIC),
                                ),
                            ]));
                        }
                        lines.push(Line::from(Span::styled(
                            "  ╰─",
                            Style::default().fg(theme.accent_dim),
                        )));
                    }
                }
            }
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
                        // Mirror render layout: leading blank + header (or running
                        // line), then for completed calls the framed body + footer.
                        line_offset += 2;
                        if let ToolBlockState::Done { expanded, record } = state.as_ref() {
                            let total = format_tool_result(&record.result).lines().count();
                            let max_lines = if *expanded { 40 } else { 6 };
                            let shown = total.min(max_lines);
                            line_offset += shown;
                            if total > shown {
                                line_offset += 1; // "more lines" hint
                            }
                            line_offset += 1; // footer
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
