use serde::{Deserialize, Serialize};

/// Configuration for the TUI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    /// Enable syntax highlighting
    pub syntax_highlighting: bool,

    /// Theme for syntax highlighting
    pub theme: String,

    /// Show line numbers in code blocks
    pub line_numbers: bool,

    /// Enable mouse support
    pub mouse_support: bool,

    /// Scroll speed (lines per scroll event)
    pub scroll_speed: usize,

    /// Maximum history entries to keep
    pub max_history: usize,

    /// Auto-save enabled (default: true)
    pub auto_save_enabled: bool,
    /// Auto-save interval in minutes
    pub auto_save_interval_minutes: u32,
    /// Custom save directory for auto-saves (defaults to ~/.pawan/sessions/)
    pub auto_save_dir: Option<std::path::PathBuf>,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            syntax_highlighting: true,
            theme: "base16-ocean.dark".to_string(),
            line_numbers: true,
            mouse_support: true,
            scroll_speed: 3,
            max_history: 1000,
            auto_save_enabled: true,
            auto_save_interval_minutes: 5,
            auto_save_dir: None,
        }
    }
}
