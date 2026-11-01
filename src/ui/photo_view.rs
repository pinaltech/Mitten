use desert_photos::storage::{Library, Photo};
use std::sync::Arc;

/// Unfiltered libraries need only a length; filtered views share indices, never
/// duplicate photo records. Appending catalog pages stays O(page size).
#[derive(Clone)]
pub struct PhotoView {
    pub indices: Option<Arc<Vec<usize>>>,
    pub count: usize,
    pub reversed: bool,
}
impl PhotoView {
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn get<'a>(&self, library: &'a Library, index: usize) -> Option<&'a Arc<Photo>> {
        if index >= self.count {
            return None;
        }
        let index = if self.reversed {
            self.count - 1 - index
        } else {
            index
        };
        library.photos.get(
            self.indices
                .as_ref()
                .map_or(index, |indices| indices[index]),
        )
    }
    pub fn position(&self, library_index: usize) -> Option<usize> {
        let i = if let Some(indices) = &self.indices {
            indices.iter().position(|&i| i == library_index)?
        } else {
            library_index
        };
        (i < self.count).then(|| if self.reversed { self.count - 1 - i } else { i })
    }
    pub fn iter<'a>(&'a self, library: &'a Library) -> impl Iterator<Item = &'a Arc<Photo>> + 'a {
        (0..self.count).filter_map(move |i| self.get(library, i))
    }
    #[cfg(test)]
    pub fn same_mapping(&self, other: &Self) -> bool {
        if self.reversed != other.reversed {
            return false;
        }
        match (&self.indices, &other.indices) {
            (None, None) => self.count == other.count,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
