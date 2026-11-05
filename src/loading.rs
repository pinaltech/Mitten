//! UI-independent limits shared by the grid and stress benchmarks.
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

/// Pixel edges of the images each zoom level draws: the Years and Months
/// squares, the margined grid's thumbnails and the viewer's preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImageSizes {
    pub micro: u32,
    pub mini: u32,
    pub grid: u32,
    pub viewer: u32,
}
impl Default for ImageSizes {
    fn default() -> Self {
        Self::DEFAULT
    }
}
impl ImageSizes {
    pub const DEFAULT: Self = Self {
        micro: 64,
        mini: 160,
        grid: 512,
        viewer: 2560,
    };
    /// The choices Settings offers for each level.
    pub const MICRO: [u32; 3] = [48, 64, 96];
    pub const MINI: [u32; 3] = [128, 160, 256];
    pub const GRID: [u32; 5] = [256, 384, 512, 768, 1024];
    pub const VIEWER: [u32; 4] = [1280, 1920, 2560, 3840];
    /// Keeps every edge within the range its level supports.
    pub fn clamped(self) -> Self {
        Self {
            micro: self.micro.clamp(32, 128),
            mini: self.mini.clamp(96, 320),
            grid: self.grid.clamp(192, 1536),
            viewer: self.viewer.clamp(1024, 4096),
        }
    }
}
static MICRO_EDGE: AtomicU32 = AtomicU32::new(ImageSizes::DEFAULT.micro);
static MINI_EDGE: AtomicU32 = AtomicU32::new(ImageSizes::DEFAULT.mini);
static GRID_EDGE: AtomicU32 = AtomicU32::new(ImageSizes::DEFAULT.grid);
static VIEWER_EDGE: AtomicU32 = AtomicU32::new(ImageSizes::DEFAULT.viewer);
/// The sizes every source renders at from now on (process-wide, because the
/// cache files of each size live side by side).
pub fn set_image_sizes(sizes: ImageSizes) {
    let sizes = sizes.clamped();
    MICRO_EDGE.store(sizes.micro, Ordering::Relaxed);
    MINI_EDGE.store(sizes.mini, Ordering::Relaxed);
    GRID_EDGE.store(sizes.grid, Ordering::Relaxed);
    VIEWER_EDGE.store(sizes.viewer, Ordering::Relaxed);
}
pub fn image_sizes() -> ImageSizes {
    ImageSizes {
        micro: MICRO_EDGE.load(Ordering::Relaxed),
        mini: MINI_EDGE.load(Ordering::Relaxed),
        grid: GRID_EDGE.load(Ordering::Relaxed),
        viewer: VIEWER_EDGE.load(Ordering::Relaxed),
    }
}
/// Cache file stem for an image `kind` rendered at `edge`: the historical
/// name at the default size, so existing caches stay valid.
pub fn sized_kind(kind: &str, edge: u32) -> String {
    let default = if kind == "preview" {
        ImageSizes::DEFAULT.viewer
    } else {
        ImageSizes::DEFAULT.grid
    };
    if edge == default {
        kind.to_owned()
    } else {
        format!("{kind}{edge}")
    }
}
/// A grid thumbnail should arrive within this (median, per photo) for the
/// largest zoom level to fill quickly; the viewer can afford longer.
pub const GRID_BUDGET: Duration = Duration::from_millis(140);
pub const VIEWER_BUDGET: Duration = Duration::from_millis(700);
/// The largest candidate whose median time stays within `budget`, measuring
/// ascending sizes and stopping at the first that is too slow. The smallest
/// candidate is the floor. `measure` times one photo (`sample`) at an edge.
pub fn fastest_reasonable(
    candidates: &[u32],
    samples: usize,
    budget: Duration,
    mut measure: impl FnMut(u32, usize) -> Option<Duration>,
) -> u32 {
    let mut chosen = candidates[0];
    for &edge in candidates {
        let mut times: Vec<Duration> = (0..samples).filter_map(|i| measure(edge, i)).collect();
        if times.is_empty() {
            break;
        }
        times.sort();
        if times[times.len() / 2] > budget {
            break;
        }
        chosen = edge;
    }
    chosen
}

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
        if pixels <= MICRO_EDGE.load(Ordering::Relaxed) as f32 {
            Self::Micro
        } else {
            Self::Mini
        }
    }
    pub fn edge(self) -> u32 {
        match self {
            Self::Micro => MICRO_EDGE.load(Ordering::Relaxed),
            Self::Mini => MINI_EDGE.load(Ordering::Relaxed),
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

/// Decodes standard (padded or unpadded) base64, as sources send ThumbHashes.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Decodes a [ThumbHash](https://evanw.github.io/thumbhash/) into a small
/// (at most 32 px) BGRA image, GPUI's upload order: a blurred stand-in at the
/// photo's own aspect ratio. Returns `(width, height, pixels)`.
#[allow(clippy::needless_range_loop)] // mirrors the reference DCT
pub fn thumbhash_bgra(hash: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    use std::f32::consts::PI;
    if hash.len() < 5 {
        return None;
    }
    let header24 = u32::from(hash[0]) | u32::from(hash[1]) << 8 | u32::from(hash[2]) << 16;
    let header16 = u32::from(hash[3]) | u32::from(hash[4]) << 8;
    let l_dc = (header24 & 63) as f32 / 63.;
    let p_dc = ((header24 >> 6) & 63) as f32 / 31.5 - 1.;
    let q_dc = ((header24 >> 12) & 63) as f32 / 31.5 - 1.;
    let l_scale = ((header24 >> 18) & 31) as f32 / 31.;
    let has_alpha = header24 >> 23 != 0;
    let p_scale = ((header16 >> 3) & 63) as f32 / 63.;
    let q_scale = ((header16 >> 9) & 63) as f32 / 63.;
    let landscape = header16 >> 15 != 0;
    let side = (header16 & 7) as usize;
    let long = if has_alpha { 5 } else { 7 };
    let (lx, ly) = if landscape {
        (long, side)
    } else {
        (side, long)
    };
    // The aspect ratio is approximated from the stored (unclamped) counts.
    let ratio = lx.max(1) as f32 / ly.max(1) as f32;
    let (lx, ly) = (lx.max(3), ly.max(3));
    if has_alpha && hash.len() < 6 {
        return None;
    }
    let a_dc = if has_alpha {
        f32::from(hash[5] & 15) / 15.
    } else {
        1.
    };
    let a_scale = if has_alpha {
        f32::from(hash[5] >> 4) / 15.
    } else {
        0.
    };
    let mut index = 0usize;
    let start = if has_alpha { 6 } else { 5 };
    let mut channel = |nx: usize, ny: usize, scale: f32| -> Option<Vec<f32>> {
        let mut ac = vec![];
        for cy in 0..ny {
            let mut cx = usize::from(cy == 0);
            while cx * ny < nx * (ny - cy) {
                let byte = *hash.get(start + (index >> 1))?;
                let nibble = (byte >> ((index & 1) << 2)) & 15;
                ac.push((f32::from(nibble) / 7.5 - 1.) * scale);
                index += 1;
                cx += 1;
            }
        }
        Some(ac)
    };
    let l_ac = channel(lx, ly, l_scale)?;
    let p_ac = channel(3, 3, p_scale * 1.25)?;
    let q_ac = channel(3, 3, q_scale * 1.25)?;
    let a_ac = if has_alpha {
        channel(5, 5, a_scale)?
    } else {
        vec![]
    };
    let (w, h) = if ratio > 1. {
        (32, (32. / ratio).round().max(1.) as usize)
    } else {
        ((32. * ratio).round().max(1.) as usize, 32)
    };
    let n = lx.max(if has_alpha { 5 } else { 3 });
    let m = ly.max(if has_alpha { 5 } else { 3 });
    let mut fx = vec![0f32; n];
    let mut fy = vec![0f32; m];
    let mut out = Vec::with_capacity(w * h * 4);
    let byte = |v: f32| (255. * v.clamp(0., 1.)).round() as u8;
    for y in 0..h {
        for (cy, f) in fy.iter_mut().enumerate() {
            *f = (PI / h as f32 * (y as f32 + 0.5) * cy as f32).cos();
        }
        for x in 0..w {
            for (cx, f) in fx.iter_mut().enumerate() {
                *f = (PI / w as f32 * (x as f32 + 0.5) * cx as f32).cos();
            }
            let (mut l, mut p, mut q, mut a) = (l_dc, p_dc, q_dc, a_dc);
            let mut j = 0;
            for cy in 0..ly {
                let fy2 = fy[cy] * 2.;
                let mut cx = usize::from(cy == 0);
                while cx * ly < lx * (ly - cy) {
                    l += l_ac[j] * fx[cx] * fy2;
                    j += 1;
                    cx += 1;
                }
            }
            let mut j = 0;
            for cy in 0..3 {
                let fy2 = fy[cy] * 2.;
                for cx in usize::from(cy == 0)..3 - cy {
                    let f = fx[cx] * fy2;
                    p += p_ac[j] * f;
                    q += q_ac[j] * f;
                    j += 1;
                }
            }
            if has_alpha {
                let mut j = 0;
                for cy in 0..5 {
                    let fy2 = fy[cy] * 2.;
                    for cx in usize::from(cy == 0)..5 - cy {
                        a += a_ac[j] * fx[cx] * fy2;
                        j += 1;
                    }
                }
            }
            let b = l - 2. / 3. * p;
            let r = (3. * l - b + q) / 2.;
            let g = r - q;
            out.extend_from_slice(&[byte(b), byte(g), byte(r), byte(a)]);
        }
    }
    Some((w as u32, h as u32, out))
}

/// Bounds how many image requests run against one source at once, so a slow
/// server is never flooded and the viewer's photo (`urgent`) goes first.
pub struct Gate {
    limit: usize,
    state: std::sync::Mutex<(usize, usize)>,
    ready: std::sync::Condvar,
}
pub struct Pass<'a>(&'a Gate);
impl Drop for Pass<'_> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().0 -= 1;
        self.0.ready.notify_all();
    }
}
impl Gate {
    pub const fn new(limit: usize) -> Self {
        Self {
            limit,
            state: std::sync::Mutex::new((0, 0)),
            ready: std::sync::Condvar::new(),
        }
    }
    /// Waits for a slot. Urgent requests take any free slot ahead of waiting
    /// grid requests and may run two beyond the limit.
    pub fn enter(&self, urgent: bool) -> Pass<'_> {
        let mut state = self.state.lock().unwrap();
        if urgent {
            state.1 += 1;
            while state.0 >= self.limit + 2 {
                state = self.ready.wait(state).unwrap();
            }
            state.1 -= 1;
        } else {
            while state.0 >= self.limit || state.1 > 0 {
                state = self.ready.wait(state).unwrap();
            }
        }
        state.0 += 1;
        Pass(self)
    }
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
    fn base64_and_thumbhash_decode() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello");
        assert!(base64_decode("a$").is_none());
        // A landscape hash of a flat colour: L = 63/63, no AC energy.
        let mut hash = vec![0u8; 25];
        let header24: u32 = 63 | 32 << 6 | 32 << 12;
        let header16: u32 = 3 | 1 << 15;
        hash[..3].copy_from_slice(&header24.to_le_bytes()[..3]);
        hash[3..5].copy_from_slice(&header16.to_le_bytes()[..2]);
        hash[5..].fill(0x77);
        let (w, h, pixels) = thumbhash_bgra(&hash).unwrap();
        assert_eq!((w, h), (32, 14));
        assert_eq!(pixels.len(), (w * h * 4) as usize);
        assert!(pixels.chunks(4).all(|p| p[3] == 255 && p[1] > 240));
        assert!(thumbhash_bgra(&hash[..4]).is_none());
        assert!(thumbhash_bgra(&hash[..8]).is_none());
    }
    #[test]
    fn gate_bounds_concurrency() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let gate = Gate::new(3);
        let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        std::thread::scope(|s| {
            for _ in 0..12 {
                s.spawn(|| {
                    let _pass = gate.enter(false);
                    peak.fetch_max(now.fetch_add(1, SeqCst) + 1, SeqCst);
                    std::thread::sleep(Duration::from_millis(5));
                    now.fetch_sub(1, SeqCst);
                });
            }
        });
        assert!(peak.load(SeqCst) <= 3);
    }
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
    #[test]
    fn automatic_sizes_pick_the_largest_size_within_budget() {
        let budget = Duration::from_millis(100);
        // Time grows with size; 768 px is the first too slow.
        let chosen = fastest_reasonable(&ImageSizes::GRID, 3, budget, |edge, _| {
            Some(Duration::from_millis(edge as u64 / 6))
        });
        assert_eq!(chosen, 512);
        // Even the smallest too slow: still the smallest, never nothing.
        let floor = fastest_reasonable(&ImageSizes::GRID, 3, budget, |_, _| {
            Some(Duration::from_secs(1))
        });
        assert_eq!(floor, 256);
        // One slow outlier does not decide the median.
        let outlier = fastest_reasonable(&ImageSizes::GRID, 3, budget, |_, i| {
            Some(Duration::from_millis(if i == 0 { 900 } else { 20 }))
        });
        assert_eq!(outlier, 1024);
        assert_eq!(sized_kind("thumb", 512), "thumb");
        assert_eq!(sized_kind("thumb", 768), "thumb768");
        assert_eq!(sized_kind("preview", 2560), "preview");
    }
}
