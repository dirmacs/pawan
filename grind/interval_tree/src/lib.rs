/// Simple interval tree implementation using a sorted Vec approach
/// Intervals are stored in a vector sorted by their low endpoint
/// Querying for overlaps at a point is O(log n) for search + O(k) for result collection

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Interval {
 pub low: i64,
 pub high: i64,
}

impl Interval {
 /// Check if this interval contains a point
 pub fn contains(&self, point: i64) -> bool {
 self.low <= point && point <= self.high
 }
}

#[derive(Debug, Default)]
pub struct IntervalTree {
 intervals: Vec<Interval>,
}

impl IntervalTree {
 /// Create a new empty interval tree
 pub fn new() -> Self {
 Self {
 intervals: Vec::new(),
 }
 }

 /// Insert an interval into the tree
 /// Maintains the intervals sorted by their low endpoint
 pub fn insert(&mut self, interval: Interval) {
 // Use binary search to find the insertion position
 let pos = self.intervals.partition_point(|i| i.low < interval.low);
 self.intervals.insert(pos, interval);
 }

 /// Query all intervals that contain the given point
 /// Returns a vector of references to the overlapping intervals
 pub fn query(&self, point: i64) -> Vec<&Interval> {
 // Find the first interval where low > point
 // All intervals before this could potentially contain the point
 let split_idx = self.intervals.partition_point(|i| i.low <= point);
 
 // Check all intervals up to split_idx for containment
 let mut result = Vec::new();
 for interval in &self.intervals[..split_idx] {
 if interval.contains(point) {
 result.push(interval);
 }
 }
 
 result
 }

 /// Get the number of intervals in the tree
 pub fn len(&self) -> usize {
 self.intervals.len()
 }

 /// Check if the tree is empty
 pub fn is_empty(&self) -> bool {
 self.intervals.is_empty()
 }
}

#[cfg(test)]
mod tests {
 use super::*;

 fn sort_intervals(intervals: Vec<&Interval>) -> Vec<Interval> {
 let mut sorted: Vec<Interval> = intervals.into_iter().copied().collect();
 sorted.sort();
 sorted
 }

 #[test]
 fn insert_query() {
 let mut tree = IntervalTree::new();
 
 // Insert intervals
 tree.insert(Interval { low: 5, high: 10 });
 tree.insert(Interval { low: 1, high: 3 });
 tree.insert(Interval { low: 8, high: 15 });
 tree.insert(Interval { low: 0, high: 2 });
 
 // Query points
 assert_eq!(sort_intervals(tree.query(0)), vec![Interval { low: 0, high: 2 }]);
 assert_eq!(sort_intervals(tree.query(1)), vec![Interval { low: 0, high: 2 }, Interval { low: 1, high: 3 }]);
 assert_eq!(sort_intervals(tree.query(2)), vec![Interval { low: 0, high: 2 }, Interval { low: 1, high: 3 }]);
 assert_eq!(sort_intervals(tree.query(5)), vec![Interval { low: 5, high: 10 }]);
 assert_eq!(sort_intervals(tree.query(8)), vec![
 Interval { low: 5, high: 10 },
 Interval { low: 8, high: 15 }
 ]);
 assert_eq!(sort_intervals(tree.query(10)), vec![
 Interval { low: 5, high: 10 },
 Interval { low: 8, high: 15 }
 ]);
 assert_eq!(sort_intervals(tree.query(12)), vec![Interval { low: 8, high: 15 }]);
 assert_eq!(tree.query(20), Vec::<&Interval>::new());
 }

 #[test]
 fn no_overlap() {
 let mut tree = IntervalTree::new();
 tree.insert(Interval { low: 10, high: 20 });
 tree.insert(Interval { low: 30, high: 40 });
 
 assert_eq!(tree.query(5), Vec::<&Interval>::new());
 assert_eq!(tree.query(25), Vec::<&Interval>::new());
 assert_eq!(tree.query(45), Vec::<&Interval>::new());
 }

 #[test]
 fn multiple_overlaps() {
 let mut tree = IntervalTree::new();
 tree.insert(Interval { low: 1, high: 10 });
 tree.insert(Interval { low: 2, high: 11 });
 tree.insert(Interval { low: 3, high: 12 });
 tree.insert(Interval { low: 4, high: 9 });
 
 // At point 5, all intervals should overlap
 let result = tree.query(5);
 assert_eq!(result.len(), 4);
 
 // At point 1, only first interval should overlap
 let result = tree.query(1);
 assert_eq!(sort_intervals(result), vec![Interval { low: 1, high: 10 }]);
 
 // At point 10, intervals 1, 2, and 3 should overlap
 let result = tree.query(10);
 assert_eq!(sort_intervals(result), vec![
 Interval { low: 1, high: 10 },
 Interval { low: 2, high: 11 },
 Interval { low: 3, high: 12 }
 ]);
 }

 #[test]
 fn edge_cases() {
 let mut tree = IntervalTree::new();
 
 // Single point interval
 tree.insert(Interval { low: 5, high: 5 });
 assert_eq!(sort_intervals(tree.query(5)), vec![Interval { low: 5, high: 5 }]);
 assert_eq!(tree.query(4), Vec::<&Interval>::new());
 assert_eq!(tree.query(6), Vec::<&Interval>::new());
 
 // Large intervals
 tree.insert(Interval { low: i64::MIN, high: i64::MAX });
 assert_eq!(sort_intervals(tree.query(0)), vec![Interval { low: i64::MIN, high: i64::MAX }]);
 assert_eq!(sort_intervals(tree.query(i64::MIN)), vec![Interval { low: i64::MIN, high: i64::MAX }]);
 assert_eq!(sort_intervals(tree.query(i64::MAX)), vec![Interval { low: i64::MIN, high: i64::MAX }]);
 }

 #[test]
 fn empty_tree() {
 let tree = IntervalTree::new();
 assert_eq!(tree.query(0), Vec::<&Interval>::new());
 assert_eq!(tree.len(), 0);
 assert!(tree.is_empty());
 }

 #[test]
 fn len_tracking() {
 let mut tree = IntervalTree::new();
 assert_eq!(tree.len(), 0);
 
 tree.insert(Interval { low: 1, high: 5 });
 assert_eq!(tree.len(), 1);
 
 tree.insert(Interval { low: 2, high: 3 });
 assert_eq!(tree.len(), 2);
 
 tree.insert(Interval { low: 10, high: 20 });
 assert_eq!(tree.len(), 3);
 
 tree.insert(Interval { low: 5, high: 15 });
 assert_eq!(tree.len(), 4);
 }
}
