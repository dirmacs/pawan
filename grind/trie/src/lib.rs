//! Trie (prefix tree) — O(m) insert/search/delete where m = key length.
use std::collections::HashMap;

#[derive(Default)]
struct TrieNode {
    children: HashMap<char, TrieNode>,
    is_end: bool,
}

#[derive(Default)]
pub struct Trie {
    root: TrieNode,
}

impl Trie {
    pub fn new() -> Self { Self::default() }

    pub fn insert(&mut self, word: &str) {
        let mut node = &mut self.root;
        for ch in word.chars() {
            node = node.children.entry(ch).or_default();
        }
        node.is_end = true;
    }

    pub fn search(&self, word: &str) -> bool {
        self.get_node(word).map(|n| n.is_end).unwrap_or(false)
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        self.get_node(prefix).is_some()
    }

    pub fn delete(&mut self, word: &str) -> bool {
        let mut found = false;
        Self::delete_rec(&mut self.root, word, 0, &mut found);
        found
    }

    /// Returns true if the node can be pruned from its parent.
    fn delete_rec(node: &mut TrieNode, word: &str, depth: usize, found: &mut bool) -> bool {
        let chars: Vec<char> = word.chars().collect();
        if depth == chars.len() {
            if !node.is_end { return false; }
            *found = true;
            node.is_end = false;
            return node.children.is_empty();
        }
        let ch = chars[depth];
        if let Some(child) = node.children.get_mut(&ch) {
            if Self::delete_rec(child, word, depth + 1, found) {
                node.children.remove(&ch);
                return !node.is_end && node.children.is_empty();
            }
        }
        false
    }

    fn get_node(&self, prefix: &str) -> Option<&TrieNode> {
        let mut node = &self.root;
        for ch in prefix.chars() {
            node = node.children.get(&ch)?;
        }
        Some(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_insert_search() {
        let mut t = Trie::new();
        t.insert("apple");
        assert!(t.search("apple"));
        assert!(!t.search("app"));
    }
    #[test] fn test_starts_with() {
        let mut t = Trie::new();
        t.insert("apple");
        assert!(t.starts_with("app"));
        assert!(!t.starts_with("ban"));
    }
    #[test] fn test_delete() {
        let mut t = Trie::new();
        t.insert("rust");
        t.insert("rusty");
        assert!(t.delete("rust"));
        assert!(!t.search("rust"));
        assert!(t.search("rusty"));
    }
    #[test] fn test_delete_nonexistent() {
        let mut t = Trie::new(); t.insert("hi");
        assert!(!t.delete("nope"));
    }
    #[test] fn test_empty_string() {
        let mut t = Trie::new();
        t.insert("");
        assert!(t.search(""));
        assert!(t.delete(""));
        assert!(!t.search(""));
    }
    #[test] fn test_unicode() {
        let mut t = Trie::new();
        t.insert("héllo");
        assert!(t.search("héllo"));
        assert!(t.starts_with("hél"));
    }
    #[test] fn test_many_words() {
        let mut t = Trie::new();
        let words = ["cat","car","card","care","careful","ego","edge"];
        for w in &words { t.insert(w); }
        for w in &words { assert!(t.search(w), "{w}"); }
        assert!(!t.search("ca"));
        assert!(t.starts_with("ca"));
    }
}
