//! Bound decoded CPU/GPU images too; bounding only file paths is insufficient.
use gpui::*;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Weak},
};
pub struct PhotoImageCache {
    inner: Entity<RetainAllImageCache>,
    order: VecDeque<Resource>,
    capacity: usize,
    ready: HashMap<Resource, Weak<RenderImage>>,
}
impl PhotoImageCache {
    pub fn new(capacity: usize, cx: &mut App) -> Entity<Self> {
        let inner = RetainAllImageCache::new(cx);
        cx.new(|_| Self {
            inner,
            order: VecDeque::new(),
            capacity,
            ready: HashMap::new(),
        })
    }
    pub fn clear(&mut self, window: &mut Window, cx: &mut App) {
        self.inner.update(cx, |cache, cx| cache.clear(window, cx));
        self.order.clear();
        self.ready.clear();
    }
    pub fn ready(&self, path: &std::path::Path) -> Option<Arc<RenderImage>> {
        self.ready
            .get(&Resource::from(path.to_path_buf()))
            .and_then(Weak::upgrade)
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.order.len()
    }
}
impl ImageCache for PhotoImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.order.retain(|r| r != resource);
        self.order.push_back(resource.clone());
        // The grid and viewer use separate limits for decoded CPU/GPU images.
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.ready.remove(&old);
                self.inner
                    .update(cx, |cache, cx| cache.remove(&old, window, cx));
            }
        }
        let result = self
            .inner
            .update(cx, |cache, cx| cache.load(resource, window, cx));
        if let Some(Ok(image)) = &result {
            self.ready.insert(resource.clone(), Arc::downgrade(image));
        }
        result
    }
}
