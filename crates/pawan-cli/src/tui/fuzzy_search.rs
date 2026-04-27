//! Fuzzy search modal: substring match over command entries (no external fuzzy matcher).

/// Maximum rows shown for a non-empty filter query.
pub const FUZZY_MAX_RESULTS: usize = 100;
/// When the query is empty, show this many items from the full list.
pub const FUZZY_EMPTY_ALL_LIMIT: usize = 20;

/// TUI state for a searchable, selectable overlay list.
pub struct FuzzySearchState {
    /// Whether the dialog is present (`App` also uses `Option<FuzzySearchState>` for open/close).
    #[allow(dead_code)]
    pub visible: bool,
    /// Current input filter.
    pub query: String,
    /// Filtered (or unfiltered cap) list shown to the user.
    pub results: Vec<String>,
    /// Index into `results`.
    pub selected: usize,
    /// Full catalog of display lines (command plus description text).
    pub all_items: Vec<String>,
}

impl FuzzySearchState {
    /// Opens a new search dialog with a fresh filter pass over `items`.
    pub fn new(items: Vec<String>) -> Self {
        let mut s = Self {
            visible: true,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            all_items: items,
        };
        s.filter("");
        s
    }

    /// Recompute `results` for `query` (case-insensitive `contains` on the display line).
    pub fn filter(&mut self, query: &str) {
        self.query = query.to_string();
        let q = query.to_lowercase();
        self.results = if q.is_empty() {
            self.all_items
                .iter()
                .take(FUZZY_EMPTY_ALL_LIMIT)
                .cloned()
                .collect()
        } else {
            self.all_items
                .iter()
                .filter(|s| s.to_lowercase().contains(&q))
                .take(FUZZY_MAX_RESULTS)
                .cloned()
                .collect()
        };
        if self.selected >= self.results.len() {
            self.selected = self.results.len().saturating_sub(1);
        }
    }

    /// Move selection down.
    pub fn next(&mut self) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1).min(self.results.len() - 1);
        }
    }

    /// Move selection up.
    pub fn prev(&mut self) {
        if !self.results.is_empty() {
            self.selected = self.selected.saturating_sub(1);
        }
    }
}

/// Strips the description after `" — "` so we can run the slash command.
pub fn command_prefix(line: &str) -> &str {
    line.splitn(2, " — ")
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(line)
}

/// Default command palette lines (order matches prior static palette).
pub fn default_command_item_lines() -> Vec<String> {
    [
        ("/help", "Show available commands"),
        ("/model", "Show or switch LLM model"),
        (
            "/model qwen/qwen3.5-122b-a10b",
            "Qwen 3.5 122B (S tier, fast)",
        ),
        ("/model minimaxai/minimax-m2.5", "MiniMax M2.5 (SWE 80.2%)"),
        (
            "/model stepfun-ai/step-3.5-flash",
            "Step 3.5 Flash (S+ tier)",
        ),
        (
            "/model mistralai/mistral-small-4-119b-2603",
            "Mistral Small 4 119B",
        ),
        ("/search", "Web search via Daedra"),
        ("/tools", "List available tools"),
        ("/heal", "Auto-fix build errors"),
        ("/export", "Export conversation to markdown"),
        ("/diff", "Show git diff (use --cached for staged changes)"),
        ("/import", "Import conversation from JSON file"),
        ("/fork", "Clone current session to a new one"),
        ("/dump", "Copy conversation to clipboard"),
        ("/share", "Export session and print shareable path"),
        (
            "/compact",
            "Compact session (default/aggressive/conservative)",
        ),
        ("/clear", "Clear chat history"),
        ("/quit", "Exit pawan"),
        ("/exit", "Exit pawan (alias for /quit)"),
    ]
    .into_iter()
    .map(|(c, d)| format!("{c} — {d}"))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_lines_include_export() {
        let v = default_command_item_lines();
        assert!(v.iter().any(|s| s.starts_with("/export")));
    }

    #[test]
    fn command_prefix_takes_left_side() {
        assert_eq!(command_prefix("/export — x"), "/export");
    }
}
