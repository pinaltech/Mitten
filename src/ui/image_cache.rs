//! Bound decoded CPU/GPU images too; bounding only file paths is insufficient.
use gpui::*;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::{Arc, Weak},
};
pub struct PhotoImageCache {
    inner: Entity<RetainAllImageCache>,
    // Access order: stamp -> resource, and resource -> (stamp, decoded bytes).
    order: BTreeMap<u64, Resource>,
    entries: HashMap<Resource, (u64, usize)>,
    clock: u64,
    bytes: usize,
    capacity: usize,
    budget: usize,
    ready: HashMap<Resource, Weak<RenderImage>>,
    // Evictions found while painting. Dropping an image frees its atlas
    // texture immediately, but sprites already in this frame's scene still
    // reference it, so the renderer would panic. Release them next frame.
    pending_evictions: Rc<RefCell<Vec<Resource>>>,
}
impl PhotoImageCache {
    /// `capacity` is the minimum number of images always kept (the visible
    /// working set); beyond it, images stay decoded until `budget` bytes.
    pub fn new(capacity: usize, cx: &mut App) -> Entity<Self> {
        Self::with_budget(capacity, 0, cx)
    }
    pub fn with_budget(capacity: usize, budget: usize, cx: &mut App) -> Entity<Self> {
        let inner = RetainAllImageCache::new(cx);
        cx.new(|_| Self {
            inner,
            order: BTreeMap::new(),
            entries: HashMap::new(),
            clock: 0,
            bytes: 0,
            capacity,
            budget,
            ready: HashMap::new(),
            pending_evictions: Rc::default(),
        })
    }
    pub fn clear(&mut self, window: &mut Window, cx: &mut App) {
        self.inner.update(cx, |cache, cx| cache.clear(window, cx));
        self.order.clear();
        self.entries.clear();
        self.bytes = 0;
        self.ready.clear();
        self.pending_evictions.borrow_mut().clear();
        // The last rendered scene may reference the dropped textures; never
        // present it again without redrawing.
        window.refresh();
    }
    // A viewport can contain more than 96 dense tiles. The working-set budget
    // must fit every visible image plus a measurement row, or asynchronous loads
    // repeatedly evict one another before decoding ever completes.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
    }
    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.capacity
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
    fn touch(&mut self, resource: &Resource) {
        self.clock += 1;
        let entry = self.entries.entry(resource.clone()).or_insert((0, 0));
        if entry.0 != 0 {
            self.order.remove(&entry.0);
        }
        entry.0 = self.clock;
        self.order.insert(self.clock, resource.clone());
    }
    fn over(&self) -> bool {
        self.order.len() > self.capacity && (self.budget == 0 || self.bytes > self.budget)
    }
    fn defer_eviction(&mut self, resource: Resource, window: &mut Window) {
        let mut pending = self.pending_evictions.borrow_mut();
        let schedule = pending.is_empty();
        pending.push(resource);
        if schedule {
            let pending = self.pending_evictions.clone();
            let inner = self.inner.clone();
            window.on_next_frame(move |window, cx| {
                let evicted = std::mem::take(&mut *pending.borrow_mut());
                if evicted.is_empty() {
                    return;
                }
                inner.update(cx, |cache, cx| {
                    for resource in &evicted {
                        cache.remove(resource, window, cx);
                    }
                });
                // Redraw before presenting so no stale sprite survives.
                window.refresh();
            });
        }
    }
}
impl ImageCache for PhotoImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.touch(resource);
        // Requested again before its deferred eviction ran: keep it.
        self.pending_evictions
            .borrow_mut()
            .retain(|r| r != resource);
        // The grid and viewer use separate limits for decoded CPU/GPU images.
        while self.over() {
            let Some((_, old)) = self.order.pop_first() else {
                break;
            };
            if let Some((_, bytes)) = self.entries.remove(&old) {
                self.bytes -= bytes;
            }
            self.ready.remove(&old);
            self.defer_eviction(old, window);
        }
        let result = self
            .inner
            .update(cx, |cache, cx| cache.load(resource, window, cx));
        if let Some(Ok(image)) = &result
            && let Some(entry) = self.entries.get_mut(resource)
            && entry.1 == 0
        {
            let size = image.size(0);
            entry.1 = (size.width.0.max(1) * size.height.0.max(1)) as usize
                * 4
                * image.frame_count().max(1);
            self.bytes += entry.1;
            self.ready.insert(resource.clone(), Arc::downgrade(image));
        }
        result
    }
}
