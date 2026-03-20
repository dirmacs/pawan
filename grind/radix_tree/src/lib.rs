//! Radix Tree (compressed trie) — shared prefixes collapsed into single edges.
//!
//! O(m) insert/search/starts_with where m = key length.

#[derive(Debug, Clone, Default)]
struct RadixNode {
    children: Vec<(String, RadixNode)>,
    is_end: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RadixTree {
    root: RadixNode,
}

impl RadixTree {
    pub fn new() -> Self { Self::default() }

    pub fn insert(&mut self, word: &str) {
        Self::insert_at(&mut self.root, word);
    }

    fn insert_at(node: &mut RadixNode, word: &str) {
        if word.is_empty() {
            node.is_end = true;
            return;
        }

        // Find a child edge that shares a prefix with `word`
        for i in 0..node.children.len() {
            let common = common_prefix(&node.children[i].0, word);
            if common == 0 { continue; }

            let edge = &node.children[i].0;
            let edge_len = edge.len();

            if common == edge_len {
                // Edge fully matched — recurse into child with remainder
                let rest = &word[common..];
                Self::insert_at(&mut node.children[i].1, rest);
                return;
            }

            // Partial match — split the edge
            let old_edge = node.children[i].0.clone();
            let old_child = std::mem::take(&mut node.children[i].1);

            // Replace current edge with the common prefix
            node.children[i].0 = old_edge[..common].to_string();
            let mut split_node = RadixNode::default();

            // Old suffix becomes a child of the split node
            split_node.children.push((old_edge[common..].to_string(), old_child));

            // New word suffix becomes another child (or marks split_node as end)
            let new_rest = &word[common..];
            if new_rest.is_empty() {
                split_node.is_end = true;
            } else {
                let mut new_child = RadixNode::default();
                new_child.is_end = true;
                split_node.children.push((new_rest.to_string(), new_child));
            }

            node.children[i].1 = split_node;
            return;
        }

        // No matching edge — add new child
        let mut new_child = RadixNode::default();
        new_child.is_end = true;
        node.children.push((word.to_string(), new_child));
    }

    pub fn search(&self, word: &str) -> bool {
        Self::search_at(&self.root, word)
    }

    fn search_at(node: &RadixNode, word: &str) -> bool {
        if word.is_empty() {
            return node.is_end;
        }
        for (edge, child) in &node.children {
            let common = common_prefix(edge, word);
            if common == edge.len() {
                return Self::search_at(child, &word[common..]);
            }
        }
        false
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        Self::prefix_at(&self.root, prefix)
    }

    fn prefix_at(node: &RadixNode, prefix: &str) -> bool {
        if prefix.is_empty() {
            return true; // We've consumed the entire prefix
        }
        for (edge, child) in &node.children {
            let common = common_prefix(edge, prefix);
            if common == 0 { continue; }
            if common == prefix.len() {
                // Prefix fully consumed (edge may be longer)
                return true;
            }
            if common == edge.len() {
                // Edge fully consumed, continue with rest of prefix
                return Self::prefix_at(child, &prefix[common..]);
            }
            // Partial match — prefix diverges midway through edge
            return false;
        }
        false
    }
}

/// Length of common byte prefix between two strings.
fn common_prefix(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_search() {
        let mut tree = RadixTree::new();
        tree.insert("hello");
        assert!(tree.search("hello"));
        assert!(!tree.search("hell"));
    }

    #[test]
    fn test_starts_with() {
        let mut tree = RadixTree::new();
        tree.insert("hello");
        assert!(tree.starts_with("hel"));
        assert!(!tree.starts_with("xyz"));
    }

    #[test]
    fn test_shared_prefix() {
        let mut tree = RadixTree::new();
        tree.insert("test");
        tree.insert("testing");
        tree.insert("team");
        assert!(tree.search("test"));
        assert!(tree.search("testing"));
        assert!(tree.search("team"));
        assert!(!tree.search("te"));
    }

    #[test]
    fn test_empty() {
        let tree = RadixTree::new();
        assert!(!tree.search(""));
        assert!(tree.starts_with(""));
    }

    #[test]
    fn test_single_char() {
        let mut tree = RadixTree::new();
        tree.insert("a");
        assert!(tree.search("a"));
        assert!(!tree.search("b"));
    }

    #[test]
    fn test_many_words() {
        let mut tree = RadixTree::new();
        let words = vec![
            "apple", "apply", "apples", "apricot",
            "banana", "bandana", "band",
            "cat", "caterpillar", "cattle",
            "dog", "do", "dove",
        ];
        for word in &words {
            tree.insert(word);
        }
        for word in &words {
            assert!(tree.search(word), "missing: {word}");
        }
    }

    #[test]
    fn test_no_match() {
        let mut tree = RadixTree::new();
        tree.insert("hello");
        assert!(!tree.search("nonexistent"));
    }

    #[test]
    fn test_prefix_of_existing() {
        let mut tree = RadixTree::new();
        tree.insert("testing");
        tree.insert("test");
        assert!(tree.search("test"));
        assert!(tree.search("testing"));
        assert!(tree.starts_with("tes"));
    }
}
