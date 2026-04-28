//! Bottom status strip (mode, thinking, model, tokens, clock).
//!
//! Layout: left side shows mode + thinking label + git branch;
//! right side shows model name, token usage bar, iteration, and timestamp.
//! Inspired by maki-ui's left/right split status bar.

use std::time::{Duration, Instant};

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
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

        // Flash messages take over the entire bar
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

        // Left side: mode badge + thinking label + branch
        let mut left_spans: Vec<Span> = vec![Span::styled(
            format!(" {} ", ctx.mode),
            ctx.mode_style.add_modifier(Modifier::BOLD),
        )];

        if let Some(t) = &ctx.thinking_label {
            left_spans.push(Span::styled(
                format!(" {t}"),
                Style::default().fg(Color::Yellow),
            ));
        }

        if let Some(b) = &ctx.branch {
            let b = b.trim();
            if !b.is_empty() {
                left_spans.push(Span::styled(
                    format!(" ⎇{b}"),
                    Style::default().fg(Color::Magenta),
                ));
            }
        }

        // Right side: model + token context bar + iteration + timestamp
        let mut right_spans: Vec<Span> = Vec::new();
        right_spans.push(Span::styled(
            model,
            Style::default().fg(Color::Cyan),
        ));

        let tok = format_tokens(ctx.total_tokens);
        let pct = (ctx.context_pct.clamp(0.0, 1.0) * 100.0).round() as u32;
        let bar_w = if area.width >= 80 { 10u16 } else { 6u16 };
        let bar = context_bar(ctx.context_pct, bar_w);

        right_spans.push(Span::raw("  "));
        right_spans.push(Span::styled(
            format!("{tok} {pct}%{bar}"),
            Style::default().fg(Color::Gray),
        ));

        if ctx.iteration > 0 {
            right_spans.push(Span::styled(
                format!("  iter {}", ctx.iteration),
                Style::default().fg(Color::DarkGray),
            ));
        }

        right_spans.push(Span::styled(
            format!("  {} ", ctx.timestamp),
            Style::default().fg(Color::DarkGray),
        ));

        // Split layout: left gets Min, right gets exact width
        let right_width: u16 = right_spans.iter().map(|s| s.width() as u16).sum();
        let [left_area, right_area] = Layout::horizontal([
            Constraint::Min(0),
            Constraint::Length(right_width),
        ])
        .areas(area);

        frame.render_widget(Paragraph::new(Line::from(left_spans)), left_area);
        frame.render_widget(
            Paragraph::new(Line::from(right_spans)).alignment(Alignment::Right),
            right_area,
        );
    }
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
