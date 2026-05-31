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
    line.split(" — ")
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
        let items = vec![
            "apple".to_string(),
            "banana".to_string(),
            "apricot".to_string(),
        ];
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
    #[test]
    fn test_prev_moves_up() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut search = FuzzySearchState::new(items);
        search.selected = 1;
        search.prev();
        assert_eq!(search.selected, 0);
    }
    #[test]
    fn test_prev_saturate_at_zero() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut search = FuzzySearchState::new(items);
        search.selected = 0;
        search.prev();
        assert_eq!(search.selected, 0);
    }
    #[test]
    fn test_filter_empty_query_shows_limited_results() {
        let items: Vec<String> = (0..30).map(|i| format!("item{i}")).collect();
        let search = FuzzySearchState::new(items);
        assert_eq!(search.results.len(), FUZZY_EMPTY_ALL_LIMIT);
    }
    #[test]
    fn test_filter_limits_results_to_max() {
        let items: Vec<String> = (0..150).map(|i| format!("item{i}")).collect();
        let mut search = FuzzySearchState::new(items);
        search.filter("item");
        assert!(search.results.len() <= FUZZY_MAX_RESULTS);
    }
    #[test]
    fn test_filter_case_insensitive() {
        let items = vec!["Apple".to_string(), "BANANA".to_string(), "Cherry".to_string()];
        let mut search = FuzzySearchState::new(items);
        search.filter("BAN");
        assert_eq!(search.results.len(), 1);
        assert_eq!(search.results[0], "BANANA");
    }
    #[test]
    fn test_filter_sets_selected_to_zero() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut search = FuzzySearchState::new(items);
        search.selected = 2;
        search.filter("a");
        assert_eq!(search.selected, 0);
    }
    #[test]
    fn test_filter_no_matches_returns_empty() {
        let items = vec!["apple".to_string(), "banana".to_string()];
        let mut search = FuzzySearchState::new(items);
        search.filter("xyz");
        assert!(search.results.is_empty());
    }
    #[test]
    fn command_prefix_returns_full_string_when_no_separator() {
        assert_eq!(command_prefix("/help"), "/help");
    }
    #[test]
    fn command_prefix_handles_trailing_separator() {
        assert_eq!(command_prefix("/export — "), "/export");
    }
    #[test]
    fn command_prefix_handles_multiple_separators() {
        assert_eq!(command_prefix("/cmd — arg — more"), "/cmd");
    }
    #[test]
    fn next_noop_on_empty_results() {
        let mut search = FuzzySearchState {
            visible: true,
            query: String::new(),
            results: vec![],
            selected: 0,
            all_items: vec![],
        };
        search.next();
        assert_eq!(search.selected, 0);
    }
    #[test]
    fn prev_noop_on_empty_results() {
        let mut search = FuzzySearchState {
            visible: true,
            query: String::new(),
            results: vec![],
            selected: 0,
            all_items: vec![],
        };
        search.prev();
        assert_eq!(search.selected, 0);
    }
}
