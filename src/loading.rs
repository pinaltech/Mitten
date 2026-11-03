//! UI-independent limits shared by the grid and stress benchmarks.
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

pub const PAGE_SIZE: usize = 200;
/// Concurrent thumbnail/derivative jobs. Decoding is CPU-bound, so this
/// tracks typical core counts rather than network concurrency.
pub const THUMBNAIL_JOBS: usize = 8;
/// Remembered thumbnail *paths* (strings, not pixels). Files persist on disk, so
/// a large index lets scrolling back reuse them without re-queuing any work.
pub const THUMBNAIL_CACHE: usize = 50_000;
pub const VIEWER_CACHE: usize = 3;

/// Bounded least-recently-used cache with O(log n) touches. Values are paths,
/// never decoded originals.
pub struct BoundedCache<T> {
    capacity: usize,
    values: HashMap<String, (T, u64)>,
    order: BTreeMap<u64, String>,
    clock: u64,
}
impl<T> BoundedCache<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            values: HashMap::new(),
            order: BTreeMap::new(),
            clock: 0,
        }
    }
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
        self.trim();
    }
    fn trim(&mut self) {
        while self.values.len() > self.capacity {
            let Some((_, old)) = self.order.pop_first() else {
                break;
            };
            self.values.remove(&old);
        }
    }
    fn touch(&mut self, key: &str) {
        if let Some((_, stamp)) = self.values.get_mut(key) {
            self.order.remove(stamp);
            self.clock += 1;
            *stamp = self.clock;
            self.order.insert(self.clock, key.to_owned());
        }
    }
    pub fn peek(&self, key: &str) -> Option<&T> {
        self.values.get(key).map(|(v, _)| v)
    }
    pub fn get(&mut self, key: &str) -> Option<&T> {
        self.touch(key);
        self.peek(key)
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }
    pub fn insert(&mut self, key: String, value: T) {
        if let Some((_, stamp)) = self.values.remove(&key) {
            self.order.remove(&stamp);
        }
        self.clock += 1;
        self.order.insert(self.clock, key.clone());
        self.values.insert(key, (value, self.clock));
        self.trim();
    }
    pub fn clear(&mut self) {
        self.values.clear();
        self.order.clear();
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Square, centre-cropped thumbnails for painted dense grids. Small tiles
/// then decode a few KB instead of a 512 px image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Square {
    /// 64 px: Years tier.
    Micro,
    /// 160 px: Months tier and the gapless grid.
    Mini,
}
impl Square {
    /// The smallest square that stays sharp for a tile `pixels` wide.
    pub fn for_pixels(pixels: f32) -> Self {
        if pixels <= 64. {
            Self::Micro
        } else {
            Self::Mini
        }
    }
    pub fn edge(self) -> u32 {
        match self {
            Self::Micro => 64,
            Self::Mini => 160,
        }
    }
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn bytes(self) -> usize {
        (self.edge() * self.edge() * 4) as usize
    }
    /// Cache file name for a photo key.
    pub fn file_name(self, key: &str) -> String {
        Self::file_name_for(key, self.edge())
    }
    pub fn file_name_for(key: &str, edge: u32) -> String {
        format!("{key}-sq{edge}.jpg")
    }
}
/// Centre-crops and scales `image` to `edge`², as BGRA (GPUI's upload order).
pub fn square_bgra(image: &image::DynamicImage, edge: u32) -> Vec<u8> {
    let side = image.width().min(image.height()).max(1);
    let square = image.crop_imm(
        (image.width() - side) / 2,
        (image.height() - side) / 2,
        side,
        side,
    );
    let mut pixels = square.thumbnail_exact(edge, edge).to_rgba8().into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    pixels
}
/// Reads a square from a cached or derived file (any decodable image).
pub fn square_from_file(path: &Path, edge: u32) -> anyhow::Result<Vec<u8>> {
    let image = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?;
    Ok(square_bgra(&image, edge))
}
/// Persists a BGRA square as a small JPEG, so later sessions read it back
/// instead of decoding the original again.
pub fn write_square(pixels: &[u8], edge: u32, target: &Path) -> anyhow::Result<()> {
    let mut rgb = Vec::with_capacity(pixels.len() / 4 * 3);
    for pixel in pixels.chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let parent = target.parent().unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(temp.as_file_mut(), 85).encode(
        &rgb,
        edge,
        edge,
        image::ExtendedColorType::Rgb8,
    )?;
    match temp.persist_noclobber(target) {
        Ok(_) => Ok(()),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.error.into()),
    }
}
/// Square from `cache` if present, else from `make` (then cached).
pub fn cached_square(
    cache: &Path,
    edge: u32,
    make: impl FnOnce() -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<Vec<u8>> {
    if cache.is_file()
        && let Ok(pixels) = square_from_file(cache, edge)
    {
        return Ok(pixels);
    }
    let pixels = make()?;
    let _ = write_square(&pixels, edge, cache);
    Ok(pixels)
}

/// Convert just the requested virtual rows to a bounded photo range.
pub fn visible_range(
    rows: std::ops::Range<usize>,
    columns: usize,
    count: usize,
) -> std::ops::Range<usize> {
    rows.start.saturating_mul(columns).min(count)..rows.end.saturating_mul(columns).min(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrolling_a_million_photos_keeps_caches_bounded() {
        let mut thumbs = BoundedCache::new(THUMBNAIL_CACHE);
        let mut previews = BoundedCache::new(VIEWER_CACHE);
        for row in (0..200_000).step_by(7) {
            let range = visible_range(row..row + 7, 5, 1_000_000);
            assert!(range.len() <= 35);
            for id in range {
                thumbs.insert(id.to_string(), id);
            }
            assert!(thumbs.len() <= THUMBNAIL_CACHE);
            assert!(
                previews.is_empty(),
                "scrolling must never request a viewer image"
            );
        }
        for id in 0..100 {
            previews.insert(id.to_string(), id);
        }
        assert_eq!(previews.len(), VIEWER_CACHE);
    }
    #[test]
    fn least_recently_used_entries_are_evicted() {
        let mut c = BoundedCache::new(2);
        c.insert("a".into(), 1);
        c.insert("b".into(), 2);
        assert_eq!(c.get("a"), Some(&1));
        c.insert("c".into(), 3);
        assert!(!c.contains_key("b"));
        assert!(c.contains_key("a"));
    }
    #[test]
    fn squares_are_centre_cropped_cached_and_sized_per_class() {
        let dir = tempfile::tempdir().unwrap();
        let thumb = dir.path().join("abc-thumb.png");
        let mut wide = image::RgbImage::from_pixel(512, 300, image::Rgb([0, 0, 255]));
        for y in 0..300 {
            for x in 0..106 {
                wide.put_pixel(x, y, image::Rgb([255, 0, 0]));
            }
        }
        wide.save(&thumb).unwrap();
        for class in [Square::Micro, Square::Mini] {
            let edge = class.edge();
            let pixels = square_from_file(&thumb, edge).unwrap();
            assert_eq!(pixels.len(), class.bytes());
            // The red left strip is cropped away; BGRA puts blue first.
            assert!(pixels[0] > 200 && pixels[2] < 50);
            let cache = dir.path().join(class.file_name("abc"));
            let first = cached_square(&cache, edge, || Ok(pixels.clone())).unwrap();
            assert!(cache.is_file());
            let again =
                cached_square(&cache, edge, || anyhow::bail!("must read the cache")).unwrap();
            assert_eq!(again.len(), first.len());
        }
        assert_eq!(Square::for_pixels(22.), Square::Micro);
        assert_eq!(Square::for_pixels(152.), Square::Mini);
    }
}
