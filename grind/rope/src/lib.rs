//! Rope — efficient string with O(log n) concat/split.
//!
//! Leaves hold short strings, branches hold two sub-ropes with cached length.

use std::fmt;

enum RopeNode {
    Leaf(String),
    Branch {
        left: Box<RopeNode>,
        right: Box<RopeNode>,
        len: usize,
    },
}

impl RopeNode {
    fn len(&self) -> usize {
        match self {
            RopeNode::Leaf(s) => s.len(),
            RopeNode::Branch { len, .. } => *len,
        }
    }

    fn to_string_buf(&self, buf: &mut String) {
        match self {
            RopeNode::Leaf(s) => buf.push_str(s),
            RopeNode::Branch { left, right, .. } => {
                left.to_string_buf(buf);
                right.to_string_buf(buf);
            }
        }
    }

    fn char_at(&self, index: usize) -> Option<char> {
        match self {
            RopeNode::Leaf(s) => s.chars().nth(index),
            RopeNode::Branch { left, right, .. } => {
                let left_len = left.len();
                if index < left_len {
                    left.char_at(index)
                } else {
                    right.char_at(index - left_len)
                }
            }
        }
    }

    fn split(self, at: usize) -> (RopeNode, RopeNode) {
        match self {
            RopeNode::Leaf(s) => {
                let (l, r) = s.split_at(at.min(s.len()));
                (RopeNode::Leaf(l.to_string()), RopeNode::Leaf(r.to_string()))
            }
            RopeNode::Branch { left, right, .. } => {
                let left_len = left.len();
                if at <= left_len {
                    let (ll, lr) = left.split(at);
                    let new_right = RopeNode::concat(lr, *right);
                    (ll, new_right)
                } else {
                    let (rl, rr) = right.split(at - left_len);
                    let new_left = RopeNode::concat(*left, rl);
                    (new_left, rr)
                }
            }
        }
    }

    fn concat(left: RopeNode, right: RopeNode) -> RopeNode {
        let len = left.len() + right.len();
        if len == 0 {
            return RopeNode::Leaf(String::new());
        }
        RopeNode::Branch {
            left: Box::new(left),
            right: Box::new(right),
            len,
        }
    }
}

pub struct Rope {
    root: RopeNode,
}

impl Rope {
    pub fn from(s: &str) -> Self {
        Rope { root: RopeNode::Leaf(s.to_string()) }
    }

    pub fn len(&self) -> usize { self.root.len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    pub fn char_at(&self, index: usize) -> Option<char> {
        self.root.char_at(index)
    }

    pub fn concat(self, other: Rope) -> Rope {
        Rope { root: RopeNode::concat(self.root, other.root) }
    }

    pub fn split(self, at: usize) -> (Rope, Rope) {
        let (l, r) = self.root.split(at);
        (Rope { root: l }, Rope { root: r })
    }
}

impl fmt::Display for Rope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = String::with_capacity(self.len());
        self.root.to_string_buf(&mut buf);
        f.write_str(&buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_string() {
        let r = Rope::from("hello");
        assert_eq!(r.to_string(), "hello");
    }

    #[test]
    fn test_concat() {
        let a = Rope::from("hello");
        let b = Rope::from(" world");
        let c = a.concat(b);
        assert_eq!(c.to_string(), "hello world");
    }

    #[test]
    fn test_split() {
        let r = Rope::from("hello world");
        let (a, b) = r.split(5);
        assert_eq!(a.to_string(), "hello");
        assert_eq!(b.to_string(), " world");
    }

    #[test]
    fn test_char_at() {
        let r = Rope::from("abcdef");
        assert_eq!(r.char_at(0), Some('a'));
        assert_eq!(r.char_at(5), Some('f'));
        assert_eq!(r.char_at(6), None);
    }

    #[test]
    fn test_len() {
        assert_eq!(Rope::from("test").len(), 4);
    }

    #[test]
    fn test_empty() {
        let r = Rope::from("");
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
        assert_eq!(r.char_at(0), None);
    }

    #[test]
    fn test_concat_then_split() {
        let a = Rope::from("abc");
        let b = Rope::from("defgh");
        let c = a.concat(b); // "abcdefgh"
        let (left, right) = c.split(4);
        assert_eq!(left.to_string(), "abcd");
        assert_eq!(right.to_string(), "efgh");
    }
}
