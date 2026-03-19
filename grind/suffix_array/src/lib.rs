//! Suffix Array with LCP array.
//!
//! Construction: O(n log n) via sort_by with string slice comparison.
//! LCP: O(n^2) naive — correct and simple.
//! Search/count: O(m log n) binary search where m = pattern length.

pub struct SuffixArray {
    pub sa: Vec<usize>,
    pub lcp: Vec<usize>,
    text: String,
}

impl SuffixArray {
    pub fn new(s: &str) -> Self {
        let n = s.len();
        let text = s.to_string();

        let mut sa: Vec<usize> = (0..n).collect();
        sa.sort_by(|&i, &j| s[i..].cmp(&s[j..]));

        // Naive O(n^2) LCP: lcp[i] = common prefix between sa[i] and sa[i-1]
        let mut lcp = vec![0usize; n];
        for i in 1..n {
            let a = &s[sa[i]..];
            let b = &s[sa[i - 1]..];
            lcp[i] = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
        }

        SuffixArray { sa, lcp, text }
    }

    /// Returns any occurrence position of `pattern` in the text (O(m log n)).
    pub fn search(&self, pattern: &str) -> Option<usize> {
        if pattern.is_empty() || self.sa.is_empty() {
            return None;
        }
        let p = pattern.len();
        let n = self.sa.len();
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let start = self.sa[mid];
            let end = (start + p).min(self.text.len());
            let cmp = pattern.as_bytes().cmp(&self.text.as_bytes()[start..end]);
            match cmp {
                std::cmp::Ordering::Less => hi = mid,
                std::cmp::Ordering::Greater => lo = mid + 1,
                std::cmp::Ordering::Equal => return Some(start),
            }
        }
        None
    }

    /// Count occurrences of `pattern` as a substring (O(m log n)).
    pub fn count(&self, pattern: &str) -> usize {
        if pattern.is_empty() || self.sa.is_empty() {
            return 0;
        }
        let p = pattern.len();
        let n = self.sa.len();
        let pat = pattern.as_bytes();

        // Lower bound: first index where suffix[..p] >= pattern
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let start = self.sa[mid];
            let end = (start + p).min(self.text.len());
            if self.text.as_bytes()[start..end] < *pat {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let left = lo;

        // Upper bound: first index where suffix[..p] > pattern
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let start = self.sa[mid];
            let end = (start + p).min(self.text.len());
            if self.text.as_bytes()[start..end] <= *pat {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo - left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_search() {
        let sa = SuffixArray::new("banana");
        assert!(sa.search("an").is_some());
        assert!(sa.search("xyz").is_none());
        assert!(sa.search("banana").is_some());
    }

    #[test]
    fn test_count() {
        let sa = SuffixArray::new("banana");
        assert_eq!(sa.count("an"), 2);
        assert_eq!(sa.count("a"), 3);
        assert_eq!(sa.count("b"), 1);
        assert_eq!(sa.count("xyz"), 0);
    }

    #[test]
    fn test_empty_text() {
        let sa = SuffixArray::new("");
        assert_eq!(sa.search("a"), None);
        assert_eq!(sa.count("a"), 0);
        assert!(sa.sa.is_empty());
    }

    #[test]
    fn test_empty_pattern() {
        let sa = SuffixArray::new("hello");
        assert_eq!(sa.search(""), None);
        assert_eq!(sa.count(""), 0);
    }

    #[test]
    fn test_single_char() {
        let sa = SuffixArray::new("a");
        assert_eq!(sa.search("a"), Some(0));
        assert_eq!(sa.count("a"), 1);
        assert_eq!(sa.search("b"), None);
    }

    #[test]
    fn test_lcp_length() {
        let sa = SuffixArray::new("banana");
        assert_eq!(sa.sa.len(), 6);
        assert_eq!(sa.lcp.len(), 6);
        assert_eq!(sa.lcp[0], 0); // first entry always 0
    }

    #[test]
    fn test_repeated() {
        let sa = SuffixArray::new("aaaa");
        assert_eq!(sa.count("a"), 4);
        assert_eq!(sa.count("aa"), 3);
        assert_eq!(sa.count("aaa"), 2);
        assert_eq!(sa.count("aaaa"), 1);
        assert_eq!(sa.count("aaaaa"), 0);
    }

    #[test]
    fn test_mississippi() {
        let sa = SuffixArray::new("mississippi");
        assert_eq!(sa.count("ss"), 2);
        assert_eq!(sa.count("issi"), 2);
        assert_eq!(sa.count("ippi"), 1);
        assert_eq!(sa.count("z"), 0);
    }

    #[test]
    fn test_sorted_order() {
        // Suffix array must sort suffixes lexicographically
        let s = "abcabc";
        let sa = SuffixArray::new(s);
        let suffixes: Vec<&str> = sa.sa.iter().map(|&i| &s[i..]).collect();
        for i in 1..suffixes.len() {
            assert!(suffixes[i - 1] <= suffixes[i], "{} > {}", suffixes[i - 1], suffixes[i]);
        }
    }
}
