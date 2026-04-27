//! Syntax highlighting for fenced code blocks using `syntect` + Ratatui spans.

use std::sync::{Arc, OnceLock};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SyntectStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use syntect::Error as SyntectError;

static SYNTAX_SET: OnceLock<Arc<SyntaxSet>> = OnceLock::new();
static THEME_SET: OnceLock<Arc<ThemeSet>> = OnceLock::new();

fn syntax_set() -> Arc<SyntaxSet> {
    SYNTAX_SET
        .get_or_init(|| Arc::new(SyntaxSet::load_defaults_newlines()))
        .clone()
}

fn theme_set() -> Arc<ThemeSet> {
    THEME_SET
        .get_or_init(|| Arc::new(ThemeSet::load_defaults()))
        .clone()
}

/// Map friendly editor palette names onto themes bundled with `syntect`.
///
/// `ThemeSet::load_defaults()` only ships a small curated set (`base16-*`,
/// `InspiredGitHub`, `Solarized (light|dark)`). We pick the closest bundled
/// sibling for popular names and fall back to `base16-mocha.dark` when needed.
fn map_user_theme_name(name: &str) -> Option<&'static str> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "dracula" | "catppuccin-mocha" | "tokyo-night" | "one-dark" => "base16-mocha.dark",
        "nord" => "base16-ocean.dark",
        "gruvbox-dark" => "base16-eighties.dark",
        "monokai" => "Solarized (dark)",
        "github-dark" => "InspiredGitHub",
        _ => return None,
    })
}

fn resolve_theme(theme_name: &str) -> Arc<Theme> {
    let ts = theme_set();
    let trimmed = theme_name.trim();

    if let Some(theme) = ts.themes.get(trimmed) {
        return Arc::new(theme.clone());
    }

    if let Some(mapped) = map_user_theme_name(trimmed) {
        if let Some(theme) = ts.themes.get(mapped) {
            return Arc::new(theme.clone());
        }
        tracing::debug!(
            requested = trimmed,
            mapped,
            "mapped syntect theme missing from bundled defaults"
        );
    } else {
        tracing::debug!(
            requested = trimmed,
            "unknown theme label; falling back to bundled dark palette"
        );
    }

    for key in [
        "base16-mocha.dark",
        "base16-ocean.dark",
        "InspiredGitHub",
        "Solarized (dark)",
    ] {
        if let Some(theme) = ts.themes.get(key) {
            return Arc::new(theme.clone());
        }
    }

    Arc::new(
        ts.themes
            .values()
            .next()
            .cloned()
            .unwrap_or_default(),
    )
}

fn syntect_to_rat_color(color: syntect::highlighting::Color) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}

fn syntect_style_to_ratatui(style: SyntectStyle, theme: &Theme) -> Style {
    let mut out = Style::default().fg(syntect_to_rat_color(style.foreground));

    let default_bg = theme.settings.background;
    let apply_bg = default_bg
        .map(|bg| bg != style.background)
        .unwrap_or(style.background.a > 0);
    if apply_bg {
        out = out.bg(syntect_to_rat_color(style.background));
    }

    let mut mods = Modifier::empty();
    if style.font_style.contains(FontStyle::BOLD) {
        mods |= Modifier::BOLD;
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        mods |= Modifier::ITALIC;
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        mods |= Modifier::UNDERLINED;
    }
    out.add_modifier(mods)
}

const MAX_LINE_CHARS: usize = 500;
const CONTINUATION: &str = "...";

fn push_truncated_spans(
    ranges: Vec<(SyntectStyle, String)>,
    theme: &Theme,
    out: &mut Vec<Line<'static>>,
) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut written = 0usize;

    for (st, text) in ranges {
        if written >= MAX_LINE_CHARS {
            break;
        }
        let room = MAX_LINE_CHARS.saturating_sub(written);
        let style = syntect_style_to_ratatui(st, theme);

        if text.len() <= room {
            written = written.saturating_add(text.len());
            spans.push(Span::styled(text, style));
            continue;
        }

        let cont_len = CONTINUATION.len();
        if room == 0 {
            break;
        }
        if room <= cont_len {
            let clipped: String = CONTINUATION.chars().take(room).collect();
            spans.push(Span::styled(clipped, style));
            break;
        }

        let keep = room.saturating_sub(cont_len);
        let prefix: String = text.chars().take(keep).collect();
        spans.push(Span::styled(prefix, style));
        spans.push(Span::styled(CONTINUATION.to_owned(), style));
        break;
    }

    if spans.is_empty() {
        out.push(Line::default());
    } else {
        out.push(Line::from(spans));
    }
}

fn plain_lines(code: &str) -> Vec<Line<'static>> {
    if code.is_empty() {
        return Vec::new();
    }
    code.lines()
        .map(|l| Line::from(vec![Span::raw(l.to_owned())]))
        .collect()
}

fn resolve_syntax<'a>(
    syntax_set: &'a SyntaxSet,
    language_hint: &str,
    code: &str,
) -> &'a syntect::parsing::SyntaxReference {
    let hint = language_hint.trim();
    let lowered = hint.to_ascii_lowercase();

    if hint.is_empty() {
        return syntax_set
            .find_syntax_by_first_line(code)
            .unwrap_or_else(|| syntax_set.find_syntax_plain_text());
    }

    let ext = match lowered.as_str() {
        "rust" | "rs" => "rs",
        "js" | "javascript" => "js",
        "ts" | "typescript" => "ts",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "py" | "python" => "py",
        "sh" | "bash" | "zsh" => "sh",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "go" | "golang" => "go",
        "css" => "css",
        "html" | "htm" => "html",
        other => other,
    };

    if let Some(syntax) = syntax_set.find_syntax_by_extension(ext) {
        return syntax;
    }

    syntax_set
        .find_syntax_by_token(hint)
        .or_else(|| syntax_set.find_syntax_by_first_line(code))
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text())
}

/// Stateful highlighter backed by the bundled `syntect` assets.
pub struct SyntaxHighlighter {
    pub theme: Arc<Theme>,
    pub syntax_set: Arc<SyntaxSet>,
}

impl SyntaxHighlighter {
    /// Build a highlighter using a friendly theme name (`dracula`, `nord`, …).
    pub fn new(theme_name: &str) -> Result<Self, String> {
        let theme = resolve_theme(theme_name);
        Ok(Self {
            theme,
            syntax_set: syntax_set(),
        })
    }

    /// Construct from an already-resolved `syntect` [`Theme`].
    pub fn with_theme(theme: Theme) -> Self {
        Self {
            theme: Arc::new(theme),
            syntax_set: syntax_set(),
        }
    }

    /// Replace the active theme while keeping syntax definitions cached.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = Arc::new(theme);
    }

    /// Highlight `code` for `language_hint`, producing owned Ratatui lines.
    pub fn highlight(&self, code: &str, language_hint: &str) -> Vec<Line<'static>> {
        if code.is_empty() {
            return Vec::new();
        }

        let lowered = language_hint.trim().to_ascii_lowercase();
        if matches!(lowered.as_str(), "md" | "markdown") {
            return plain_lines(code);
        }

        let syntax = resolve_syntax(self.syntax_set.as_ref(), language_hint, code);
        let mut highlighter = HighlightLines::new(syntax, self.theme.as_ref());
        let theme_ref = self.theme.as_ref();

        let mut lines = Vec::new();

        for raw_line in LinesWithEndings::from(code) {
            match highlighter.highlight_line(raw_line, self.syntax_set.as_ref()) {
                Ok(ranges) => {
                    let owned: Vec<(SyntectStyle, String)> = ranges
                        .into_iter()
                        .map(|(style, text)| (style, text.to_owned()))
                        .collect();
                    push_truncated_spans(owned, theme_ref, &mut lines);
                }
                Err(err) => {
                    log_highlight_error(err);
                    lines.push(Line::from(vec![Span::raw(raw_line.to_owned())]));
                }
            }
        }

        lines
    }
}

fn log_highlight_error(err: SyntectError) {
    tracing::debug!(error = %err, "syntect error while highlighting line");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_rust_produces_lines() {
        let hi = SyntaxHighlighter::new("dracula").expect("theme resolves");
        let lines = hi.highlight("fn main() {\n    let x = 1;\n}\n", "rs");
        assert!(lines.len() >= 3);
    }

    #[test]
    fn markdown_skips_colorization() {
        let hi = SyntaxHighlighter::new("nord").expect("theme resolves");
        let lines = hi.highlight("# Title\n", "md");
        assert_eq!(lines.len(), 1);
    }
}
