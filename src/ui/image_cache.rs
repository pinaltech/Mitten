//! Bound decoded CPU/GPU images too; bounding only file paths is insufficient.
use gpui::*;
use std::{collections::VecDeque, sync::Arc};
pub struct PhotoImageCache {
    inner: Entity<RetainAllImageCache>,
    order: VecDeque<Resource>,
    capacity: usize,
}
impl PhotoImageCache {
    pub fn new(capacity: usize, cx: &mut App) -> Entity<Self> {
        let inner = RetainAllImageCache::new(cx);
        cx.new(|_| Self {
            inner,
            order: VecDeque::new(),
            capacity,
        })
    }
    pub fn clear(&mut self, window: &mut Window, cx: &mut App) {
        self.inner.update(cx, |cache, cx| cache.clear(window, cx));
        self.order.clear();
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
                self.inner
                    .update(cx, |cache, cx| cache.remove(&old, window, cx));
            }
        }
        self.inner
            .update(cx, |cache, cx| cache.load(resource, window, cx))
    }
}
