//! Top-of-screen status strip (model, mode, tokens, git branch, clock).

#![allow(dead_code)] // integrated by upcoming shell layout wiring

use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Transient overlay message for the status strip.
#[derive(Debug, Clone)]
pub struct StatusBar {
    flash_message: Option<String>,
    flash_until: Option<Instant>,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            flash_message: None,
            flash_until: None,
        }
    }

    pub fn flash(&mut self, msg: String) {
        self.flash_message = Some(msg);
        self.flash_until = Some(Instant::now() + Duration::from_secs(3));
    }

    pub fn clear_flash(&mut self) {
        self.flash_message = None;
        self.flash_until = None;
    }

    pub fn view(&self, frame: &mut Frame, area: Rect, ctx: &StatusBarContext) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let flash_active = match (self.flash_message.as_ref(), self.flash_until) {
            (Some(msg), Some(until)) if Instant::now() < until => Some(msg.as_str()),
            _ => None,
        };

        if let Some(msg) = flash_active {
            let line = Line::from(vec![Span::styled(
                msg,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]);
            let p = Paragraph::new(line).centered();
            frame.render_widget(p, area);
            return;
        }

        let narrow = area.width < 60;
        let model = abbrev_model(&ctx.model_name, narrow);
        let model_icon = "◉";

        let mode_label = format!(" {} ", ctx.mode);
        let mode_badge = Span::styled(
            mode_label,
            ctx
                .mode_style
                .add_modifier(Modifier::BOLD),
        );

        let thinking = ctx
            .thinking_label
            .as_ref()
            .map(|t| {
                Span::styled(
                    format!(" {t} "),
                    Style::default().fg(Color::Yellow),
                )
            });

        let tok = format_tokens(ctx.total_tokens);
        let pct = (ctx.context_pct.clamp(0.0, 1.0) * 100.0).round() as u32;
        let bar_w = if area.width >= 80 { 10u16 } else { 6u16 };
        let bar = context_bar(ctx.context_pct, bar_w);
        let token_summary = format!("{tok} | {pct}% {bar}");

        let iter_txt = format!("iter {}", ctx.iteration);

        let branch_txt = ctx.branch.as_ref().and_then(|b| {
            let b = b.trim();
            if b.is_empty() {
                None
            } else {
                Some(format!("⎇ {b}"))
            }
        });

        let mut spans: Vec<Span> = vec![
            Span::styled(
                format!("{model_icon} {model}"),
                Style::default().fg(Color::Cyan),
            ),
            sep(),
            mode_badge,
        ];

        if let Some(span) = thinking {
            spans.push(span);
        }

        spans.push(sep());
        spans.push(Span::styled(
            token_summary,
            Style::default().fg(Color::Gray),
        ));
        spans.push(sep());
        spans.push(Span::styled(
            iter_txt,
            Style::default().fg(Color::DarkGray),
        ));

        if let Some(b) = branch_txt {
            spans.push(sep());
            spans.push(Span::styled(b, Style::default().fg(Color::Magenta)));
        }

        spans.push(sep());
        spans.push(Span::styled(
            ctx.timestamp.clone(),
            Style::default().fg(Color::DarkGray),
        ));

        let mut line = Line::from(spans);
        line = truncate_line(line, area.width as usize);

        let row = Rect::new(area.x, area.y, area.width, 1.min(area.height));
        frame.render_widget(Paragraph::new(line), row);
    }
}

fn sep() -> Span<'static> {
    Span::styled(" │ ", Style::default().fg(Color::DarkGray))
}

fn abbrev_model(name: &str, narrow: bool) -> String {
    if !narrow {
        return name.to_owned();
    }
    let mut chars: Vec<char> = name.chars().collect();
    if chars.len() <= 20 {
        return name.to_owned();
    }
    chars = chars.split_off(chars.len() - 20);
    let mut s = String::with_capacity(chars.len() + 1);
    s.push('…');
    s.extend(chars);
    s
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M tok", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k tok", n as f64 / 1000.0)
    } else {
        format!("{n} tok")
    }
}

fn context_bar(pct: f32, width: u16) -> String {
    let w = width.max(3) as usize;
    let filled = ((pct.clamp(0.0, 1.0)) * w as f32).round() as usize;
    let filled = filled.min(w);
    let mut out = String::new();
    for i in 0..w {
        out.push(if i < filled { '█' } else { '░' });
    }
    out
}

fn truncate_line(line: Line<'_>, max_width: usize) -> Line<'_> {
    let w = line.width();
    if w <= max_width {
        return line;
    }
    let mut out: Vec<Span> = Vec::new();
    let mut used = 0usize;
    for sp in line.spans {
        let sw = sp.width();
        if used + sw <= max_width.saturating_sub(1) {
            used += sw;
            out.push(sp);
        } else {
            let budget = max_width.saturating_sub(used).saturating_sub(1);
            if budget == 0 {
                break;
            }
            let mut content = sp.content.to_string();
            while content.chars().count() > budget && !content.is_empty() {
                content.pop();
            }
            if !content.is_empty() {
                out.push(Span::styled(content, sp.style));
            }
            out.push(Span::styled("…", Style::default()));
            break;
        }
    }
    Line::from(out)
}

/// Data supplied by the host each frame for the status strip.
#[derive(Debug, Clone)]
pub struct StatusBarContext {
    pub model_name: String,
    pub mode: &'static str,
    pub mode_style: Style,
    pub total_tokens: u64,
    pub context_pct: f32,
    pub iteration: u32,
    pub branch: Option<String>,
    pub timestamp: String,
    pub thinking_label: Option<String>,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

