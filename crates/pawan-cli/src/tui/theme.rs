//! Centralized UI palette for the Ratatui front-end (inspired by maki-ui).

use std::sync::{Arc, LazyLock};

use arc_swap::{ArcSwap, Guard};
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub background: Color,
    pub surface: Color,
    pub surface_elevated: Color,
    pub foreground: Color,
    pub accent: Color,
    pub accent_dim: Color,
    pub user_bubble: Color,
    pub assistant_bubble: Color,
    pub code_bg: Color,
    pub border: Color,
    pub border_focused: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub tool_success: Color,
    pub tool_error: Color,
    pub muted: Color,
    pub subtle: Color,
    pub selection_bg: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxTheme {
    pub name: &'static str,
    pub fg: Color,
    pub bg: Color,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

const DRACULA: Theme = Theme {
    name: "dracula",
    background: rgb(0x28, 0x2a, 0x36),
    surface: rgb(0x38, 0x3a, 0x4a),
    surface_elevated: rgb(0x44, 0x47, 0x5a),
    foreground: rgb(0xf8, 0xf8, 0xf2),
    accent: rgb(0xbd, 0x93, 0xf9),
    accent_dim: rgb(0x95, 0x80, 0xff),
    user_bubble: rgb(0x44, 0x47, 0x5a),
    assistant_bubble: rgb(0x38, 0x3a, 0x4a),
    code_bg: rgb(0x21, 0x22, 0x2c),
    border: rgb(0x62, 0x72, 0xa4),
    border_focused: rgb(0xbd, 0x93, 0xf9),
    error: rgb(0xff, 0x55, 0x55),
    warning: rgb(0xff, 0xb8, 0x6c),
    success: rgb(0x50, 0xfa, 0x7b),
    tool_success: rgb(0x50, 0xfa, 0x7b),
    tool_error: rgb(0xff, 0x55, 0x55),
    muted: rgb(0xb7, 0xc0, 0xd8),
    subtle: rgb(0x44, 0x47, 0x5a),
    selection_bg: rgb(0x44, 0x47, 0x5a),
};

const CATPPUCCIN_MOCHA: Theme = Theme {
    name: "catppuccin_mocha",
    background: rgb(0x1e, 0x1e, 0x2e),
    surface: rgb(0x31, 0x32, 0x44),
    surface_elevated: rgb(0x45, 0x47, 0x5a),
    foreground: rgb(0xcd, 0xd6, 0xf4),
    accent: rgb(0xcb, 0xa6, 0xf7),
    accent_dim: rgb(0x93, 0x99, 0xb2),
    user_bubble: rgb(0x31, 0x32, 0x44),
    assistant_bubble: rgb(0x1e, 0x1e, 0x2e),
    code_bg: rgb(0x11, 0x11, 0x1b),
    border: rgb(0x6c, 0x70, 0x86),
    border_focused: rgb(0xcb, 0xa6, 0xf7),
    error: rgb(0xf3, 0x8b, 0xa8),
    warning: rgb(0xfa, 0xb3, 0x87),
    success: rgb(0xa6, 0xe3, 0xa1),
    tool_success: rgb(0xa6, 0xe3, 0xa1),
    tool_error: rgb(0xf3, 0x8b, 0xa8),
    muted: rgb(0xa6, 0xad, 0xc8),
    subtle: rgb(0x31, 0x32, 0x44),
    selection_bg: rgb(0x31, 0x32, 0x44),
};

const NORD: Theme = Theme {
    name: "nord",
    background: rgb(0x2e, 0x34, 0x40),
    surface: rgb(0x3b, 0x42, 0x52),
    surface_elevated: rgb(0x43, 0x4c, 0x5e),
    foreground: rgb(0xec, 0xef, 0xf4),
    accent: rgb(0x88, 0xc0, 0xd0),
    accent_dim: rgb(0x81, 0xa1, 0xc1),
    user_bubble: rgb(0x3b, 0x42, 0x52),
    assistant_bubble: rgb(0x2e, 0x34, 0x40),
    code_bg: rgb(0x3b, 0x42, 0x52),
    border: rgb(0x4c, 0x56, 0x6a),
    border_focused: rgb(0x88, 0xc0, 0xd0),
    error: rgb(0xbf, 0x61, 0x6a),
    warning: rgb(0xeb, 0xcb, 0x8b),
    success: rgb(0xa3, 0xbe, 0x8c),
    tool_success: rgb(0xa3, 0xbe, 0x8c),
    tool_error: rgb(0xbf, 0x61, 0x6a),
    muted: rgb(0xd8, 0xde, 0xe9),
    subtle: rgb(0x3b, 0x42, 0x52),
    selection_bg: rgb(0x3b, 0x42, 0x52),
};

const TOKYONIGHT: Theme = Theme {
    name: "tokyonight",
    background: rgb(0x1a, 0x1b, 0x26),
    surface: rgb(0x24, 0x28, 0x3b),
    surface_elevated: rgb(0x41, 0x48, 0x68),
    foreground: rgb(0xc0, 0xca, 0xf5),
    accent: rgb(0x7a, 0xa2, 0xf7),
    accent_dim: rgb(0x56, 0x5f, 0x89),
    user_bubble: rgb(0x41, 0x48, 0x68),
    assistant_bubble: rgb(0x1a, 0x1b, 0x26),
    code_bg: rgb(0x1f, 0x23, 0x35),
    border: rgb(0x41, 0x48, 0x68),
    border_focused: rgb(0x7a, 0xa2, 0xf7),
    error: rgb(0xf7, 0x76, 0x8e),
    warning: rgb(0xe0, 0xaf, 0x68),
    success: rgb(0x9e, 0xce, 0x6a),
    tool_success: rgb(0x9e, 0xce, 0x6a),
    tool_error: rgb(0xf7, 0x76, 0x8e),
    muted: rgb(0xa9, 0xb1, 0xd6),
    subtle: rgb(0x41, 0x48, 0x68),
    selection_bg: rgb(0x41, 0x48, 0x68),
};

const GRUVBOX: Theme = Theme {
    name: "gruvbox",
    background: rgb(0x28, 0x28, 0x28),
    surface: rgb(0x3c, 0x38, 0x36),
    surface_elevated: rgb(0x50, 0x49, 0x45),
    foreground: rgb(0xeb, 0xdb, 0xb2),
    accent: rgb(0xfa, 0xbd, 0x2f),
    accent_dim: rgb(0xd7, 0x99, 0x21),
    user_bubble: rgb(0x3c, 0x38, 0x36),
    assistant_bubble: rgb(0x28, 0x28, 0x28),
    code_bg: rgb(0x32, 0x30, 0x2f),
    border: rgb(0x92, 0x83, 0x74),
    border_focused: rgb(0xfa, 0xbd, 0x2f),
    error: rgb(0xfb, 0x49, 0x34),
    warning: rgb(0xfa, 0xbd, 0x2f),
    success: rgb(0xb8, 0xbb, 0x26),
    tool_success: rgb(0xb8, 0xbb, 0x26),
    tool_error: rgb(0xfb, 0x49, 0x34),
    muted: rgb(0xa8, 0x99, 0x84),
    subtle: rgb(0x3c, 0x38, 0x36),
    selection_bg: rgb(0x3c, 0x38, 0x36),
};

const MONOKAI_PRO: Theme = Theme {
    name: "monokai_pro",
    background: rgb(0x2d, 0x2a, 0x2e),
    surface: rgb(0x3d, 0x3a, 0x40),
    surface_elevated: rgb(0x50, 0x49, 0x45),
    foreground: rgb(0xf2, 0xe6, 0xdc),
    accent: rgb(0xff, 0x61, 0x88),
    accent_dim: rgb(0xa5, 0x69, 0xbd),
    user_bubble: rgb(0x3d, 0x3a, 0x40),
    assistant_bubble: rgb(0x2d, 0x2a, 0x2e),
    code_bg: rgb(0x2d, 0x2a, 0x2e),
    border: rgb(0x6e, 0x6e, 0x6e),
    border_focused: rgb(0xff, 0x61, 0x88),
    error: rgb(0xff, 0x61, 0x88),
    warning: rgb(0xff, 0xd8, 0x66),
    success: rgb(0xa9, 0xdc, 0x76),
    tool_success: rgb(0xa9, 0xdc, 0x76),
    tool_error: rgb(0xff, 0x61, 0x88),
    muted: rgb(0xc1, 0xb9, 0xc7),
    subtle: rgb(0x3d, 0x3a, 0x40),
    selection_bg: rgb(0x3d, 0x3a, 0x40),
};

const EVERFOREST: Theme = Theme {
    name: "everforest",
    background: rgb(0x2d, 0x38, 0x3b),
    surface: rgb(0x3b, 0x4d, 0x52),
    surface_elevated: rgb(0x4c, 0x59, 0x55),
    foreground: rgb(0xd3, 0xc6, 0xaa),
    accent: rgb(0xa7, 0xc0, 0x80),
    accent_dim: rgb(0x83, 0xc0, 0x78),
    user_bubble: rgb(0x3b, 0x4d, 0x52),
    assistant_bubble: rgb(0x2d, 0x38, 0x3b),
    code_bg: rgb(0x2d, 0x38, 0x3b),
    border: rgb(0x7a, 0x89, 0x87),
    border_focused: rgb(0xa7, 0xc0, 0x80),
    error: rgb(0xe6, 0x7e, 0x80),
    warning: rgb(0xe5, 0xa0, 0x6a),
    success: rgb(0xa7, 0xc0, 0x80),
    tool_success: rgb(0xa7, 0xc0, 0x80),
    tool_error: rgb(0xe6, 0x7e, 0x80),
    muted: rgb(0xb8, 0xc6, 0xb0),
    subtle: rgb(0x3b, 0x4d, 0x52),
    selection_bg: rgb(0x3b, 0x4d, 0x52),
};

const ONEDARK: Theme = Theme {
    name: "onedark",
    background: rgb(0x28, 0x2c, 0x34),
    surface: rgb(0x35, 0x3b, 0x45),
    surface_elevated: rgb(0x3e, 0x44, 0x51),
    foreground: rgb(0xab, 0xb2, 0xbf),
    accent: rgb(0x61, 0xaf, 0xef),
    accent_dim: rgb(0x56, 0xb6, 0xc2),
    user_bubble: rgb(0x35, 0x3b, 0x45),
    assistant_bubble: rgb(0x28, 0x2c, 0x34),
    code_bg: rgb(0x21, 0x25, 0x2b),
    border: rgb(0x4b, 0x52, 0x63),
    border_focused: rgb(0x61, 0xaf, 0xef),
    error: rgb(0xe0, 0x6c, 0x75),
    warning: rgb(0xd1, 0x9a, 0x66),
    success: rgb(0x98, 0xc3, 0x79),
    tool_success: rgb(0x98, 0xc3, 0x79),
    tool_error: rgb(0xe0, 0x6c, 0x75),
    muted: rgb(0x9a, 0xa5, 0xb8),
    subtle: rgb(0x35, 0x3b, 0x45),
    selection_bg: rgb(0x35, 0x3b, 0x45),
};

static ALL_THEMES: &[Theme] = &[
    DRACULA,
    CATPPUCCIN_MOCHA,
    NORD,
    TOKYONIGHT,
    GRUVBOX,
    MONOKAI_PRO,
    EVERFOREST,
    ONEDARK,
];

static THEME_STATE: LazyLock<ArcSwap<Theme>> = LazyLock::new(|| ArcSwap::from_pointee(DRACULA));

/// Thread-safe handle to the active palette (clone-on-read).
pub fn current() -> Guard<Arc<Theme>> {
    THEME_STATE.load()
}

pub fn set_theme(name: &str) -> Result<(), String> {
    let key = name.trim();
    let found = ALL_THEMES
        .iter()
        .find(|t| t.name == key)
        .copied()
        .ok_or_else(|| format!("unknown theme: {key}"))?;
    THEME_STATE.store(Arc::new(found));
    Ok(())
}

pub fn available_themes() -> Vec<&'static str> {
    ALL_THEMES.iter().map(|t| t.name).collect()
}

#[inline]
pub fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    let af = f32::from(a);
    let bf = f32::from(b);
    (af + (bf - af) * t).round() as u8
}

pub fn extract_rgb(color: Color, fallback: (u8, u8, u8)) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => fallback,
    }
}

const DEFAULT_ACCENT_FALLBACK: (u8, u8, u8) = (100, 140, 255);

/// Smooth accent transitions when switching palettes.
#[derive(Debug, Clone)]
pub struct ColorTransition {
    from: (u8, u8, u8),
    to: (u8, u8, u8),
    start: std::time::Instant,
    duration_secs: f32,
}

impl ColorTransition {
    pub fn new(color: Color) -> Self {
        let rgb = extract_rgb(color, DEFAULT_ACCENT_FALLBACK);
        Self {
            from: rgb,
            to: rgb,
            start: std::time::Instant::now()
                - std::time::Duration::from_secs_f32(Self::DEFAULT_DURATION_SECS),
            duration_secs: Self::DEFAULT_DURATION_SECS,
        }
    }

    const DEFAULT_DURATION_SECS: f32 = 0.4;

    pub fn set(&mut self, color: Color) {
        let rgb = extract_rgb(color, DEFAULT_ACCENT_FALLBACK);
        if rgb == self.to {
            return;
        }
        let now = std::time::Instant::now();
        self.from = self.resolve_rgb(now);
        self.to = rgb;
        self.start = now;
    }

    pub fn resolve(&self) -> Color {
        let (r, g, b) = self.resolve_rgb(std::time::Instant::now());
        Color::Rgb(r, g, b)
    }

    pub fn is_animating(&self) -> bool {
        std::time::Instant::now()
            .duration_since(self.start)
            .as_secs_f32()
            < self.duration_secs
    }

    fn resolve_rgb(&self, now: std::time::Instant) -> (u8, u8, u8) {
        let t = (now.duration_since(self.start).as_secs_f32() / self.duration_secs).min(1.0);
        let p = ease_out_cubic(t);
        (
            lerp_u8(self.from.0, self.to.0, p),
            lerp_u8(self.from.1, self.to.1, p),
            lerp_u8(self.from.2, self.to.2, p),
        )
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_theme_roundtrip() {
        set_theme("nord").unwrap();
        assert_eq!(current().name, "nord");
        set_theme("dracula").unwrap();
        assert_eq!(current().name, "dracula");
    }
}
