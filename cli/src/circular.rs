use std::ops::Index;
use std::slice::{Iter, SliceIndex};
use std::vec::IntoIter;

/// Simple wrapper type that just wraps a vec and lets you move through it
/// more easily. Add features as needed.
pub struct CircularVec<T> {
    cur: usize,
    wrapped: Vec<T>,
}

impl<T> CircularVec<T> {
    pub fn new(wrapped: Vec<T>) -> Self {
        Self { cur: 0, wrapped }
    }

    pub fn idx(&self) -> usize {
        self.cur
    }

    #[allow(dead_code)]
    pub fn push(&mut self, item: T) {
        self.wrapped.push(item)
    }

    #[allow(dead_code)]
    pub fn get(&self, idx: usize) -> Option<&T> {
        self.wrapped.get(idx)
    }

    pub fn goto_first<F: Fn(&T) -> bool>(&mut self, find: F) {
        for (i, e) in self.wrapped.iter().enumerate() {
            if find(e) {
                self.cur = i;
                return;
            }
        }
    }

    /// Get the currently selected item. This will only ever be None if the
    /// wrapped vec is empty
    pub fn current(&self) -> Option<&T> {
        self.wrapped.get(self.cur)
    }

    /// Decrement the index, wrapping if needed
    pub fn dec(&mut self) {
        self.cur = self.cur.checked_sub(1).unwrap_or(self.wrapped.len() - 1);
    }

    /// Increment the index, wrapping if needed
    pub fn inc(&mut self) {
        let next = self.cur.checked_add(1).unwrap_or(0);
        self.cur = if next < self.wrapped.len() { next } else { 0 };
    }

    pub fn iter(&self) -> Iter<'_, T> {
        self.wrapped.iter()
    }
}

impl<T> IntoIterator for CircularVec<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.wrapped.into_iter()
    }
}

impl<T> Extend<T> for CircularVec<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.wrapped.extend(iter)
    }
}

impl<T, I: SliceIndex<[T]>> Index<I> for CircularVec<T> {
    type Output = I::Output;

    #[inline]
    fn index(&self, index: I) -> &Self::Output {
        self.wrapped.index(index)
    }
}
