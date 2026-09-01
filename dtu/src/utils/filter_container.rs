/// An index that goes between [0, max] moving back to 0 or max on overflow. Note that this is
/// _inclusive_ in that range. To track a vec or slice, use the helepr methods to not accidentally
/// make an off by one error!
#[derive(Clone, Copy, Debug)]
pub struct CircularIndex {
    at: usize,
    max: usize,
}

impl CircularIndex {
    pub fn new(max: usize) -> Self {
        Self { at: 0, max }
    }

    pub fn reset(&mut self) {
        self.at = 0;
    }

    #[inline]
    pub fn new_for_slice<T>(slice: &[T]) -> Self {
        Self::new(slice.len() - 1)
    }

    #[inline]
    pub fn max_to_slice<T>(&mut self, slice: &[T]) {
        if slice.is_empty() {
            self.set_max(0);
        } else {
            self.set_max(slice.len() - 1);
        }
    }

    pub fn set_max(&mut self, max: usize) {
        self.max = max;
        if self.at >= self.max {
            self.at = 0;
        }
    }

    pub fn index(self) -> usize {
        self.at
    }

    pub fn pos(self) -> usize {
        self.at
    }

    pub fn inc(&mut self) {
        if self.at < self.max {
            self.at += 1;
        } else {
            self.at = 0
        }
    }

    pub fn dec(&mut self) {
        if self.at == 0 {
            self.at = self.max;
        } else {
            self.at = self.at - 1;
        }
    }

    pub fn inc_get(&mut self) -> usize {
        self.inc();
        self.at
    }

    pub fn dec_get(&mut self) -> usize {
        self.dec();
        self.at
    }
}

/// A trait supporting [FilterContainer]s
pub trait Container {
    type Item<'a>
    where
        Self: 'a;

    fn get_item<'a>(&'a self, index: usize) -> &'a Self::Item<'a>;
    fn len(&self) -> usize;
}

impl<T> Container for Vec<T> {
    type Item<'a>
        = T
    where
        Self: 'a;

    fn get_item<'a>(&'a self, index: usize) -> &'a Self::Item<'a> {
        &self[index]
    }

    fn len(&self) -> usize {
        self.len()
    }
}

/// A container wrapper that can be filtered and "scrolled"
pub struct FilterContainer<C: Container> {
    container: C,
    filtered: Vec<usize>,
    idx: CircularIndex,
}

impl<A> FromIterator<A> for FilterContainer<Vec<A>> {
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self {
        let vec = Vec::from_iter(iter);
        Self::new_vec(vec)
    }
}

pub struct FilterContainerIter<'a, C>
where
    C: Container,
{
    container: &'a FilterContainer<C>,
    at: usize,
}

impl<'a, C> Iterator for FilterContainerIter<'a, C>
where
    C: Container + 'a,
{
    type Item = &'a C::Item<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let at = self.at;
        if at >= self.container.idx.max {
            return None;
        }
        self.at += 1;
        self.container.get(at)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.container.idx.max - self.at;
        (size, Some(size))
    }
}

impl<'a, C> ExactSizeIterator for FilterContainerIter<'a, C> where C: Container + 'a {}

impl<T> FilterContainer<Vec<T>> {
    pub fn new_vec(vec: Vec<T>) -> Self {
        let container_len = vec.len();
        let mut filtered = Vec::with_capacity(container_len);
        filtered.extend(0..container_len);
        let idx = CircularIndex::new_for_slice(&filtered);
        Self {
            container: vec,
            idx,
            filtered,
        }
    }
}

pub type FilterVec<T> = FilterContainer<Vec<T>>;

impl<C> FilterContainer<C>
where
    C: Container,
{
    /// Retrieve the underlying container
    pub fn container(&self) -> &C {
        &self.container
    }

    pub fn first(&self) -> Option<&C::Item<'_>> {
        let idx = self.filtered.first()?;
        Some(self.container.get_item(*idx))
    }

    pub fn last(&self) -> Option<&C::Item<'_>> {
        let idx = self.filtered.last()?;
        Some(self.container.get_item(*idx))
    }

    /// Reports the size of the underlying data set, to get the filtered length, use
    /// [Self::len].
    pub fn total_len(&self) -> usize {
        self.container.len()
    }

    pub fn iter(&self) -> FilterContainerIter<'_, C> {
        FilterContainerIter {
            at: 0,
            container: self,
        }
    }

    pub fn new(container: C) -> Self {
        let len = container.len();
        let mut filtered = Vec::with_capacity(len);
        filtered.extend(0..len);
        let idx = CircularIndex::new_for_slice(&filtered);
        Self {
            container,
            idx,
            filtered,
        }
    }

    /// Return whether a filter is currently affecting the view
    ///
    /// A false return here doesn't mean no filter is applied. It may mean either no filter is
    /// applied or the applied filter is not filtering out anything.
    pub fn is_filtered(&self) -> bool {
        self.filtered.len() < self.container.len()
    }

    /// Get the length of the filtered vector
    pub fn len(&self) -> usize {
        self.filtered.len()
    }

    /// Remove any filtering applied
    pub fn unfilter(&mut self) {
        if self.filtered.len() == self.container.len() {
            return;
        }
        self.filtered.clear();
        self.filtered.extend(0..self.container.len());
        self.idx.max_to_slice(&self.filtered);
    }

    /// Add another filter to the container
    ///
    /// This will keep any filters that were already applied in place. To filter without maintaining
    /// previous filters use [Self::filter]
    pub fn and_filter<F>(&mut self, filter: F)
    where
        F: Fn(&C::Item<'_>) -> bool,
    {
        let mut new_filters = Vec::new();
        for idx in self.filtered.iter().copied() {
            let elem = self.container.get_item(idx);
            if filter(elem) {
                new_filters.push(idx);
            }
        }
        self.filtered = new_filters;
        self.idx.max_to_slice(&self.filtered);
    }

    /// Filter the container with the provided function
    ///
    /// Elements are included if the function returns true. This clears all previous filters. If
    /// instead you want to keep previous filters, use [Self::and_filter]
    pub fn filter<F>(&mut self, filter: F)
    where
        F: Fn(&C::Item<'_>) -> bool,
    {
        self.filtered.clear();
        for idx in 0..self.container.len() {
            let elem = self.container.get_item(idx);
            if filter(elem) {
                self.filtered.push(idx);
            }
        }
        self.idx.max_to_slice(&self.filtered);
    }

    /// Get an element via the filtered vector
    pub fn get(&self, idx: usize) -> Option<&C::Item<'_>> {
        let idx = self.filtered.get(idx)?;
        Some(self.container.get_item(*idx))
    }

    /// Get the selected element
    pub fn get_selected(&self) -> Option<&C::Item<'_>> {
        if self.filtered.len() == 0 {
            return None;
        }
        let idx = self.filtered[self.idx.index()];
        Some(self.container.get_item(idx))
    }

    pub fn sel_index(&self) -> usize {
        self.idx.index()
    }

    pub fn inc_sel(&mut self) {
        self.idx.inc()
    }

    pub fn dec_sel(&mut self) {
        self.idx.dec()
    }

    pub fn inc_sel_get(&mut self) -> usize {
        self.idx.inc_get()
    }

    pub fn dec_sel_get(&mut self) -> usize {
        self.idx.dec_get()
    }
}
