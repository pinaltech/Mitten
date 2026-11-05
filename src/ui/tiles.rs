//! Decoded squares for painted (gapless) grids. Thousands of tiles are on
//! screen at once, so lookups are one hash probe with no allocation, images
//! are decoded in parallel batches straight to GPU-ready pixels, and memory is
//! bounded by bytes with least-recently-drawn eviction.
use desert_photos::{
    loading::Square,
    storage::{Photo, PhotosDriver},
};
use gpui::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// Photos per background request: one PhotoKit fetch or one worker's run.
pub const BATCH: usize = 32;
/// Decoded squares kept beyond what is on screen.
const BUDGET: usize = 640 << 20;
/// Most queued requests remembered per frame (the rest are asked again later).
const QUEUE: usize = 6000;
/// Decoded ThumbHash placeholders kept (each at most 32×32).
const PLACEHOLDERS: usize = 12_000;
/// ThumbHashes decoded per frame; the rest follow on the next frames.
const PLACEHOLDERS_PER_FRAME: usize = 600;

#[derive(Default)]
struct Entry {
    images: [Option<Arc<RenderImage>>; 2],
    pending: [bool; 2],
    failed: [bool; 2],
    drawn: u64,
}
#[derive(Default)]
pub struct Tiles {
    entries: HashMap<String, Entry>,
    frame: u64,
    bytes: usize,
    pub in_flight: usize,
    pub generation: u64,
    queue: Vec<(Arc<Photo>, Square)>,
    queued: HashSet<(usize, Square)>,
    /// Photos per decode request; 0 means [`BATCH`].
    pub batch: usize,
    /// Blurred stand-ins decoded from sources' ThumbHashes, with the frame
    /// each was last drawn.
    placeholders: HashMap<String, (Arc<RenderImage>, u64)>,
    placeholder_budget: usize,
    /// A placeholder was skipped this frame for lack of budget.
    pub placeholders_pending: bool,
    // Evicted images whose atlas textures are released on the next paint.
    graveyard: Vec<Arc<RenderImage>>,
}
pub fn workers() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get().clamp(4, 12))
}
impl Tiles {
    /// Starts a frame: requests are re-prioritised from what is drawn now.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.queue.clear();
        self.queued.clear();
        self.placeholder_budget = PLACEHOLDERS_PER_FRAME;
        self.placeholders_pending = false;
    }
    /// The photo's blurred ThumbHash stand-in, if the source sent one.
    pub fn placeholder(&mut self, photo: &Photo) -> Option<Arc<RenderImage>> {
        let hash = photo.placeholder.as_ref()?;
        let frame = self.frame;
        if let Some((image, drawn)) = self.placeholders.get_mut(photo.fingerprint.as_str()) {
            *drawn = frame;
            return Some(image.clone());
        }
        if self.placeholder_budget == 0 {
            self.placeholders_pending = true;
            return None;
        }
        self.placeholder_budget -= 1;
        let (w, h, pixels) = desert_photos::loading::thumbhash_bgra(hash)?;
        let buffer = image::RgbaImage::from_raw(w, h, pixels)?;
        let image = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
        if self.placeholders.len() >= PLACEHOLDERS {
            let mut old: Vec<_> = self
                .placeholders
                .iter()
                .filter(|(_, (_, drawn))| drawn + 1 < frame)
                .map(|(k, (_, drawn))| (*drawn, k.clone()))
                .collect();
            old.sort_unstable();
            for (_, key) in old.into_iter().take(PLACEHOLDERS / 4) {
                if let Some((image, _)) = self.placeholders.remove(&key) {
                    self.graveyard.push(image);
                }
            }
        }
        self.placeholders
            .insert(photo.fingerprint.clone(), (image.clone(), frame));
        Some(image)
    }
    /// Frees textures evicted since the last paint. Call before painting.
    pub fn release(&mut self, window: &mut Window) {
        for image in self.graveyard.drain(..) {
            let _ = window.drop_image(image);
        }
    }
    pub fn clear(&mut self) {
        for (_, entry) in self.entries.drain() {
            self.graveyard.extend(entry.images.into_iter().flatten());
        }
        self.graveyard
            .extend(self.placeholders.drain().map(|(_, (image, _))| image));
        self.bytes = 0;
        self.generation += 1;
        self.in_flight = 0;
        self.queue.clear();
        self.queued.clear();
    }
    /// The best square ready for `photo` (`class`, else the other size), and
    /// queues `class` if it is not ready yet.
    pub fn get(&mut self, photo: &Arc<Photo>, class: Square) -> Option<Arc<RenderImage>> {
        let frame = self.frame;
        if let Some(entry) = self.entries.get_mut(photo.fingerprint.as_str()) {
            entry.drawn = frame;
            if let Some(image) = &entry.images[class.index()] {
                return Some(image.clone());
            }
            let other = entry.images[1 - class.index()].clone();
            if !entry.pending[class.index()] && !entry.failed[class.index()] {
                self.enqueue(photo, class);
            }
            return other;
        }
        self.enqueue(photo, class);
        None
    }
    pub fn peek(&self, photo: &Photo) -> Option<Arc<RenderImage>> {
        let entry = self.entries.get(photo.fingerprint.as_str())?;
        entry.images[1].clone().or_else(|| entry.images[0].clone())
    }
    /// Prefetch without counting as drawn.
    pub fn want(&mut self, photo: &Arc<Photo>, class: Square) {
        if self.queue.len() >= QUEUE {
            return;
        }
        match self.entries.get(photo.fingerprint.as_str()) {
            Some(e)
                if e.images[class.index()].is_some()
                    || e.pending[class.index()]
                    || e.failed[class.index()] => {}
            _ => self.enqueue(photo, class),
        }
    }
    fn enqueue(&mut self, photo: &Arc<Photo>, class: Square) {
        if self.queue.len() < QUEUE && self.queued.insert((Arc::as_ptr(photo) as usize, class)) {
            self.queue.push((photo.clone(), class));
        }
    }
    /// The next batch to decode, in draw order, of one size class.
    pub fn take_batch(&mut self) -> Option<(Square, Vec<Arc<Photo>>)> {
        let class = self.queue.first()?.1;
        let mut batch = vec![];
        let mut rest = vec![];
        for (photo, c) in std::mem::take(&mut self.queue) {
            let size = if self.batch == 0 { BATCH } else { self.batch };
            if c == class && batch.len() < size {
                let entry = self.entries.entry(photo.fingerprint.clone()).or_default();
                if entry.images[c.index()].is_none() && !entry.pending[c.index()] {
                    entry.pending[c.index()] = true;
                    batch.push(photo);
                }
            } else {
                rest.push((photo, c));
            }
        }
        self.queue = rest;
        (!batch.is_empty()).then_some((class, batch))
    }
    pub fn finish(&mut self, class: Square, results: Vec<(String, Option<Arc<RenderImage>>)>) {
        for (fingerprint, image) in results {
            let entry = self.entries.entry(fingerprint).or_default();
            entry.pending[class.index()] = false;
            match image {
                Some(image) => {
                    if let Some(old) = entry.images[class.index()].replace(image) {
                        self.graveyard.push(old);
                    } else {
                        self.bytes += class.bytes();
                    }
                }
                None => entry.failed[class.index()] = true,
            }
        }
        self.evict();
    }
    fn evict(&mut self) {
        if self.bytes <= BUDGET {
            return;
        }
        // Never evict what this or the previous frame drew.
        let mut old: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| e.drawn + 1 < self.frame && e.images.iter().any(Option::is_some))
            .map(|(k, e)| (e.drawn, k.clone()))
            .collect();
        old.sort_unstable();
        for (_, key) in old {
            if self.bytes <= BUDGET * 9 / 10 {
                break;
            }
            if let Some(entry) = self.entries.remove(&key) {
                for (class, image) in [Square::Micro, Square::Mini].into_iter().zip(entry.images) {
                    if let Some(image) = image {
                        self.bytes -= class.bytes();
                        self.graveyard.push(image);
                    }
                }
            }
        }
    }
    #[cfg(test)]
    pub fn has_placeholder(&self, photo: &Photo) -> bool {
        self.placeholders.contains_key(photo.fingerprint.as_str())
    }
    #[cfg(test)]
    pub fn ready(&self, photo: &Photo) -> bool {
        self.peek(photo).is_some()
    }
}

/// Decodes one batch on a background thread into GPU-ready images.
pub fn decode(
    driver: &dyn PhotosDriver,
    class: Square,
    photos: &[Arc<Photo>],
) -> Vec<(String, Option<Arc<RenderImage>>)> {
    let edge = class.edge();
    driver
        .squares(photos, edge)
        .into_iter()
        .zip(photos)
        .map(|(pixels, photo)| {
            let image = pixels
                .ok()
                .and_then(|pixels| image::RgbaImage::from_raw(edge, edge, pixels))
                .map(|buffer| Arc::new(RenderImage::new([image::Frame::new(buffer)])));
            (photo.fingerprint.clone(), image)
        })
        .collect()
}
