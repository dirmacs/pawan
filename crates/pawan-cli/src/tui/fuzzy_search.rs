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
        self.selected = 0;
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
    }

    /// Move selection down (wraps past the last row).
    pub fn next(&mut self) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1) % self.results.len();
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
    super::default_slash_fuzzy_lines()
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

    #[test]
    fn test_filter_updates_results() {
        let items = vec!["apple".to_string(), "banana".to_string(), "apricot".to_string()];
        let mut search = FuzzySearchState::new(items);
        search.filter("ap");
        assert!(search.results.iter().all(|r| r.contains("ap")));
    }

    #[test]
    fn test_next_wraps_around() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut search = FuzzySearchState::new(items);
        search.selected = 2;
        search.next();
        assert_eq!(search.selected, 0);
    }
}
