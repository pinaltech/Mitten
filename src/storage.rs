use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub driver: String,
    pub path: PathBuf,
}
impl Source {
    pub fn label(&self) -> String {
        if self.driver != "local-folder" {
            return crate::sources::connection_label(self);
        }
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Preferences {
    pub sources: Vec<Source>,
    pub active: Option<PathBuf>,
    #[serde(default)]
    pub appearance: String,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default = "default_bottom_to_top")]
    pub bottom_to_top: bool,
    #[serde(default)]
    pub catalog_counts: BTreeMap<PathBuf, usize>,
    /// Measure each library's loading speed to pick image sizes.
    #[serde(default = "default_true")]
    pub automatic_sizes: bool,
    /// Sizes chosen by hand (used when `automatic_sizes` is off).
    #[serde(default)]
    pub image_sizes: crate::loading::ImageSizes,
    /// Sizes measured for each library.
    #[serde(default)]
    pub measured_sizes: BTreeMap<PathBuf, crate::loading::ImageSizes>,
    /// Show a map of where a photo was taken in its info panel.
    #[serde(default = "default_true")]
    pub show_map: bool,
    /// Raster tile URL template with {z}, {x} and {y}.
    #[serde(default = "default_map_tiles")]
    pub map_tiles: String,
}
fn default_true() -> bool {
    true
}
pub fn default_map_tiles() -> String {
    "https://tile.openstreetmap.org/{z}/{x}/{y}.png".into()
}
fn default_bottom_to_top() -> bool {
    true
}
fn default_accent() -> String {
    "orange".into()
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            sources: vec![],
            active: None,
            appearance: "system".into(),
            accent: default_accent(),
            catalog_counts: BTreeMap::new(),
            bottom_to_top: true,
            automatic_sizes: true,
            image_sizes: Default::default(),
            measured_sizes: BTreeMap::new(),
            show_map: true,
            map_tiles: default_map_tiles(),
        }
    }
}
impl Preferences {
    /// The image sizes to use for the library at `path`.
    pub fn sizes_for(&self, path: Option<&Path>) -> crate::loading::ImageSizes {
        if !self.automatic_sizes {
            return self.image_sizes.clamped();
        }
        path.and_then(|p| self.measured_sizes.get(p))
            .copied()
            .unwrap_or_default()
            .clamped()
    }
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&fs::read(path)?).context("Cannot read Photos settings.")
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Ok(bytes) = fs::read(path)
            && serde_json::from_slice::<Self>(&bytes).is_err()
        {
            let backup =
                path.with_file_name(format!("settings-damaged-{}.json", uuid::Uuid::new_v4()));
            fs::copy(path, backup)?;
        }
        atomic_json(path, self)
    }
    pub fn connect(&mut self, source: Source) {
        self.active = Some(source.path.clone());
        if !self.sources.contains(&source) {
            self.sources.push(source);
        }
    }
    pub fn remove(&mut self, source: &Source) {
        self.sources.retain(|s| s != source);
        self.catalog_counts.remove(&source.path);
        self.measured_sizes.remove(&source.path);
        if self.active.as_ref() == Some(&source.path) {
            self.active = self.sources.first().map(|s| s.path.clone());
        }
    }
}

#[derive(Clone, Debug)]
pub struct Photo {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub modified: DateTime<Utc>,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub favorite: bool,
    pub deleted: bool,
    pub caption: String,
    pub fingerprint: String,
    /// Length of a video in seconds; `None` for still photos.
    pub duration: Option<f32>,
}
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub photos: Vec<Arc<Photo>>,
    pub albums: BTreeMap<String, BTreeSet<String>>,
    /// Albums mirrored from the provider. Their membership is read-only here.
    pub synced: BTreeSet<String>,
}
/// Where a video can be played from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoSource {
    File(PathBuf),
    /// A PhotoKit local identifier.
    PhotoKit(String),
    /// A server stream, with the headers that authorize it.
    Remote {
        url: String,
        headers: Vec<(String, String)>,
    },
}
/// Someone a source recognised in photos (Immich face recognition).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Person {
    pub id: String,
    #[serde(default)]
    pub name: String,
}
/// A place photos were taken, with one photo to represent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub photo: String,
}
/// Photos a source can find for the Explore view.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Query {
    Person(String),
    Place(String),
}
/// Everything the info panel shows beyond the catalog: capture settings,
/// camera, location and people. Each source fills what its metadata has.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PhotoDetails {
    pub description: Option<String>,
    /// Local capture time as the camera recorded it.
    pub taken: Option<chrono::NaiveDateTime>,
    /// Its UTC offset, such as "+02:00", or a time zone name.
    pub zone: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub f_number: Option<f32>,
    /// Shutter speed in seconds.
    pub exposure: Option<f32>,
    pub iso: Option<u32>,
    pub focal_length: Option<f32>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Place names from most to least specific (city, region, country).
    pub place: Vec<String>,
    pub bytes: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub people: Vec<Person>,
}
impl PhotoDetails {
    /// "Apple iPhone 15 Pro": the make is left out when the model repeats it.
    pub fn camera(&self) -> Option<String> {
        let model = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let make = self
            .make
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        match (make, model) {
            (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase()) => {
                Some(model.to_owned())
            }
            (Some(make), Some(model)) => Some(format!("{make} {model}")),
            (make, model) => make.or(model).map(str::to_owned),
        }
    }
    /// "ƒ/1.8 · 1/120 s · 6.9 mm · ISO 80"
    pub fn exposure_line(&self) -> Option<String> {
        let mut parts = vec![];
        if let Some(f) = self.f_number.filter(|f| *f > 0.) {
            parts.push(format!("ƒ/{}", trim_float(f, 1)));
        }
        if let Some(t) = self.exposure.filter(|t| *t > 0.) {
            parts.push(if t < 1. {
                format!("1/{} s", (1. / t).round())
            } else {
                format!("{} s", trim_float(t, 1))
            });
        }
        if let Some(mm) = self.focal_length.filter(|f| *f > 0.) {
            parts.push(format!("{} mm", trim_float(mm, 1)));
        }
        if let Some(iso) = self.iso.filter(|i| *i > 0) {
            parts.push(format!("ISO {iso}"));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
    pub fn coordinates(&self) -> Option<(f64, f64)> {
        let (lat, lon) = (self.latitude?, self.longitude?);
        ((-90. ..=90.).contains(&lat)
            && (-180. ..=180.).contains(&lon)
            && !(lat == 0. && lon == 0.))
            .then_some((lat, lon))
    }
}
fn trim_float(value: f32, places: usize) -> String {
    let s = format!("{value:.places$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}
/// Capture details from a file's EXIF block (JPEG, TIFF, HEIF, PNG, WebP).
pub fn exif_details(path: &Path) -> PhotoDetails {
    let mut details = PhotoDetails::default();
    let Ok(file) = fs::File::open(path) else {
        return details;
    };
    let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file))
    else {
        return details;
    };
    use exif::{In, Tag, Value};
    let field = |tag| exif.get_field(tag, In::PRIMARY);
    let text = |tag| {
        field(tag).and_then(|f| match &f.value {
            Value::Ascii(v) => v
                .first()
                .map(|s| {
                    String::from_utf8_lossy(s)
                        .trim()
                        .trim_matches('\0')
                        .to_owned()
                })
                .filter(|s| !s.is_empty()),
            _ => None,
        })
    };
    let number = |tag| {
        field(tag).and_then(|f| match &f.value {
            Value::Rational(v) => v.first().map(|r| r.to_f64()),
            Value::SRational(v) => v.first().map(|r| r.to_f64()),
            _ => f.value.get_uint(0).map(f64::from),
        })
    };
    details.make = text(Tag::Make);
    details.model = text(Tag::Model);
    details.lens = text(Tag::LensModel);
    details.f_number = number(Tag::FNumber).map(|v| v as f32);
    details.exposure = number(Tag::ExposureTime).map(|v| v as f32);
    details.focal_length = number(Tag::FocalLength).map(|v| v as f32);
    details.iso = field(Tag::PhotographicSensitivity).and_then(|f| f.value.get_uint(0));
    details.taken = text(Tag::DateTimeOriginal)
        .or_else(|| text(Tag::DateTime))
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(&s, "%Y:%m:%d %H:%M:%S").ok());
    details.zone = text(Tag::OffsetTimeOriginal).or_else(|| text(Tag::OffsetTime));
    details.description = text(Tag::ImageDescription);
    let degrees = |tag, reference, negative: &str| {
        let value = match &field(tag)?.value {
            Value::Rational(v) if v.len() >= 3 => {
                v[0].to_f64() + v[1].to_f64() / 60. + v[2].to_f64() / 3600.
            }
            _ => return None,
        };
        let sign = if text(reference).is_some_and(|r| r.eq_ignore_ascii_case(negative)) {
            -1.
        } else {
            1.
        };
        value.is_finite().then_some(sign * value)
    };
    details.latitude = degrees(Tag::GPSLatitude, Tag::GPSLatitudeRef, "S");
    details.longitude = degrees(Tag::GPSLongitude, Tag::GPSLongitudeRef, "W");
    details
}
#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    pub imported: usize,
    pub duplicates: usize,
    pub unsupported: usize,
}

/// Platform-independent source access. Metadata pages, thumbnails, viewer images
/// and explicit original export stay separate from the UI.
pub trait PhotosDriver: Send + Sync {
    fn source(&self) -> Source;
    fn scan(&self) -> Result<Library>;
    fn thumbnail(&self, photo: &Photo) -> Result<PathBuf>;
    fn preview(&self, photo: &Photo) -> Result<PathBuf>;
    fn import(&self, paths: &[PathBuf]) -> Result<ImportReport>;
    fn favorite(&self, ids: &[String], value: bool) -> Result<()>;
    fn trash(&self, ids: &[String], deleted: bool) -> Result<()>;
    fn create_album(&self, name: &str) -> Result<()>;
    fn add_to_album(&self, name: &str, ids: &[String]) -> Result<()>;
    fn remove_from_album(&self, name: &str, ids: &[String]) -> Result<()>;
    fn caption(&self, id: &str, caption: &str) -> Result<()>;
    fn export(&self, ids: &[String], directory: &Path) -> Result<usize>;
    /// Connected libraries fetch metadata in bounded pages; image data is separate.
    fn paged(&self) -> bool {
        false
    }
    fn page(&self, _cursor: Option<&str>) -> Result<crate::sources::PhotoPage> {
        let library = self.scan()?;
        Ok(crate::sources::PhotoPage {
            library,
            next: None,
            total: None,
        })
    }
    fn can_import(&self) -> bool {
        true
    }
    fn refresh_library(&self, _library: Library) -> Result<Library> {
        self.scan()
    }
    /// An animated rendition (GIF) of a moving photo, sized for grid tiles
    /// (`large == false`) or the viewer. `None` for still images.
    fn motion(&self, _photo: &Photo, _large: bool) -> Result<Option<PathBuf>> {
        Ok(None)
    }
    /// Where the viewer plays a video from; `None` for stills, or when this
    /// source cannot stream videos.
    fn video(&self, _photo: &Photo) -> Option<VideoSource> {
        None
    }
    /// Centre-cropped `edge`² BGRA squares for painted grids, one result per
    /// photo. Batched so sources can resolve many photos in one request.
    fn squares(&self, photos: &[Arc<Photo>], edge: u32) -> Vec<Result<Vec<u8>>> {
        squares_from_thumbnails(self, photos, edge)
    }
    /// Renders `photo` at `edge` without caching it, for timing how fast
    /// this source delivers each size.
    fn sample(&self, _photo: &Photo, _edge: u32) -> Result<()> {
        bail!("This source cannot be measured")
    }
    /// Capture details for the info panel.
    fn details(&self, _photo: &Photo) -> Result<PhotoDetails> {
        Ok(PhotoDetails::default())
    }
    /// A local file of a video, for playing it in the system's player.
    fn video_file(&self, photo: &Photo) -> Result<PathBuf> {
        match self.video(photo) {
            Some(VideoSource::File(path)) => Ok(path),
            _ => bail!("This video cannot be opened here"),
        }
    }
    /// Whether this source has people and places to explore.
    fn explore(&self) -> bool {
        false
    }
    fn people(&self) -> Result<Vec<Person>> {
        Ok(vec![])
    }
    fn person_thumbnail(&self, _id: &str) -> Result<PathBuf> {
        bail!("This source has no people")
    }
    fn places(&self) -> Result<Vec<Place>> {
        Ok(vec![])
    }
    /// IDs of the photos matching an Explore query.
    fn matching(&self, _query: &Query) -> Result<Vec<String>> {
        Ok(vec![])
    }
    /// IDs of the photos the source's own search finds for `text` (beyond
    /// names and captions), or `None` when it has no search.
    fn search(&self, _text: &str) -> Result<Option<Vec<String>>> {
        Ok(None)
    }
}
/// Times `driver` rendering a few `photos` at each candidate size and keeps
/// the largest that stays quick (see [`crate::loading::fastest_reasonable`]).
/// Squares stay at their defaults: they come from these thumbnails.
pub fn measure_sizes(
    driver: &dyn PhotosDriver,
    photos: &[Arc<Photo>],
) -> crate::loading::ImageSizes {
    use crate::loading::{GRID_BUDGET, ImageSizes, VIEWER_BUDGET, fastest_reasonable};
    let stills: Vec<_> = photos
        .iter()
        .filter(|p| p.duration.is_none() && !p.deleted)
        .collect();
    // Spread samples across the library rather than its newest few.
    let samples: Vec<&Arc<Photo>> = (0..3)
        .filter_map(|i| stills.get(i * stills.len() / 3))
        .copied()
        .collect();
    let mut sizes = ImageSizes::DEFAULT;
    if samples.is_empty() {
        return sizes;
    }
    let mut time = |edge: u32, i: usize| {
        let started = std::time::Instant::now();
        driver.sample(samples.get(i)?, edge).ok()?;
        Some(started.elapsed())
    };
    sizes.grid = fastest_reasonable(&ImageSizes::GRID, samples.len(), GRID_BUDGET, &mut time);
    sizes.viewer = fastest_reasonable(&ImageSizes::VIEWER, samples.len(), VIEWER_BUDGET, &mut time);
    sizes
}

/// Squares cropped from each photo's grid thumbnail, cached beside it.
pub fn squares_from_thumbnails<D: PhotosDriver + ?Sized>(
    driver: &D,
    photos: &[Arc<Photo>],
    edge: u32,
) -> Vec<Result<Vec<u8>>> {
    photos
        .iter()
        .map(|photo| {
            let thumbnail = driver.thumbnail(photo)?;
            let stem = thumbnail
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let cache = thumbnail.with_file_name(format!("{stem}-sq{edge}.jpg"));
            crate::loading::cached_square(&cache, edge, || {
                crate::loading::square_from_file(&thumbnail, edge)
            })
        })
        .collect()
}

/// Whether a photo may be animated; only these formats are ever probed.
pub fn may_move(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".gif") || name.ends_with(".webp")
}
pub(crate) const MOTION_SMALL: (u32, usize) = (360, 90);
pub(crate) const MOTION_LARGE: (u32, usize) = (1600, 400);

/// Writes `target` as a looping GIF of `original`'s animation, bounded to
/// `edge` px and `max_frames` frames (longer clips keep their duration by
/// dropping frames evenly). Returns false for still images. A GIF that already
/// fits is copied byte for byte.
pub(crate) fn write_motion(
    original: &Path,
    target: &Path,
    (edge, max_frames): (u32, usize),
) -> Result<bool> {
    use image::{AnimationDecoder, Frame, codecs::gif};
    let bytes = fs::read(original)?;
    let format = image::guess_format(&bytes)?;
    let frames = match format {
        image::ImageFormat::Gif => gif::GifDecoder::new(std::io::Cursor::new(&bytes))?
            .into_frames()
            .take(4096)
            .collect::<image::ImageResult<Vec<_>>>()?,
        image::ImageFormat::WebP => {
            let decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(&bytes))?;
            if !decoder.has_animation() {
                return Ok(false);
            }
            decoder
                .into_frames()
                .take(4096)
                .collect::<image::ImageResult<Vec<_>>>()?
        }
        _ => return Ok(false),
    };
    if frames.len() < 2 {
        return Ok(false);
    }
    let (w, h) = frames[0].buffer().dimensions();
    let parent = target.parent().context("motion cache has no directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    if format == image::ImageFormat::Gif && w <= edge && h <= edge && frames.len() <= max_frames {
        temp.write_all(&bytes)?;
    } else {
        let step = frames.len().div_ceil(max_frames);
        let scale = (edge as f32 / w.max(h) as f32).min(1.);
        let (tw, th) = (
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
        );
        let mut encoder = gif::GifEncoder::new_with_speed(temp.as_file_mut(), 10);
        encoder.set_repeat(gif::Repeat::Infinite)?;
        for chunk in frames.chunks(step) {
            let (numer, denom) = chunk
                .iter()
                .map(|f| f.delay().numer_denom_ms())
                .fold((0u32, 1u32), |(n, _), (fnum, fden)| {
                    (n + fnum / fden.max(1), 1)
                });
            let buffer = image::imageops::thumbnail(chunk[0].buffer(), tw, th);
            encoder.encode_frame(Frame::from_parts(
                buffer,
                0,
                0,
                image::Delay::from_numer_denom_ms(numer.max(20), denom),
            ))?;
        }
    }
    // A cache file: atomic rename is enough; a full flush per file is slow.
    match temp.persist_noclobber(target) {
        Ok(_) => Ok(true),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(true),
        Err(e) => Err(e.error.into()),
    }
}
/// Cached motion lookup shared by the drivers: `<cache>/<key>-motion<edge>.gif`,
/// or a `-still` marker so stills are only probed once.
pub(crate) fn cached_motion(
    cache: &Path,
    key: &str,
    original: impl FnOnce() -> Result<PathBuf>,
    large: bool,
) -> Result<Option<PathBuf>> {
    let spec = if large { MOTION_LARGE } else { MOTION_SMALL };
    let target = cache.join(format!("{key}-motion{}.gif", spec.0));
    let still = cache.join(format!("{key}-still"));
    if target.is_file() {
        return Ok(Some(target));
    }
    if still.is_file() {
        return Ok(None);
    }
    if write_motion(&original()?, &target, spec)? {
        Ok(Some(target))
    } else {
        fs::write(still, b"")?;
        Ok(None)
    }
}
pub fn open_source(source: &Source) -> Result<Box<dyn PhotosDriver>> {
    match source.driver.as_str() {
        "local-folder" => Ok(Box::new(LocalPhotos::open(&source.path)?)),
        _ => crate::sources::open(source),
    }
}
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Metadata {
    #[serde(default)]
    pub(crate) favorites: BTreeSet<String>,
    #[serde(default)]
    pub(crate) unfavorites: BTreeSet<String>,
    #[serde(default)]
    pub(crate) deleted: BTreeSet<String>,
    #[serde(default)]
    pub(crate) albums: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub(crate) captions: BTreeMap<String, String>,
}
pub struct LocalPhotos {
    root: PathBuf,
}
impl LocalPhotos {
    pub fn open(path: &Path) -> Result<Self> {
        ensure!(
            path.is_dir(),
            "Choose an existing folder for your photo library"
        );
        let this = Self {
            root: path.canonicalize()?,
        };
        let cache = this.safe(".desert-photos/cache")?;
        fs::create_dir_all(cache)?;
        this.metadata()?;
        Ok(this)
    }
    fn safe(&self, id: &str) -> Result<PathBuf> {
        ensure!(!id.is_empty(), "Invalid photo path");
        let mut path = self.root.clone();
        for component in Path::new(id).components() {
            match component {
                Component::Normal(p) => path.push(p),
                _ => bail!("Invalid photo path"),
            }
            if let Ok(meta) = path.symlink_metadata() {
                ensure!(
                    !meta.file_type().is_symlink(),
                    "Symbolic links are not followed"
                );
            }
        }
        Ok(path)
    }
    fn original(&self, id: &str) -> Result<PathBuf> {
        ensure!(
            !id.split('/').any(|p| p.starts_with('.')),
            "Reserved library folder"
        );
        self.safe(id)
    }
    pub(crate) fn metadata(&self) -> Result<Metadata> {
        let path = self.safe(".desert-photos/library.json")?;
        if !path.exists() {
            return Ok(Metadata::default());
        }
        serde_json::from_slice(&fs::read(path)?).context("Cannot read photo metadata.")
    }
    fn write_lock(&self) -> Result<fs::File> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.safe(".desert-photos/write.lock")?)?;
        file.lock()?;
        Ok(file)
    }
    pub(crate) fn mutate(&self, f: impl FnOnce(&mut Metadata) -> Result<()>) -> Result<()> {
        let _lock = self.write_lock()?;
        let mut meta = self.metadata()?;
        f(&mut meta)?;
        atomic_json(&self.safe(".desert-photos/library.json")?, &meta)
    }
    fn walk(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_symlink() || entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if ty.is_dir() {
                Self::walk(&entry.path(), files)?;
            } else if supported(&entry.path()) {
                files.push(entry.path());
            }
        }
        Ok(())
    }
    fn decode(&self, photo: &Photo, edge: u32, kind: &str) -> Result<PathBuf> {
        let target = self.safe(&format!(
            ".desert-photos/cache/{}-{kind}.jpg",
            photo.fingerprint
        ))?;
        if target.is_file() {
            return Ok(target);
        }
        let path = self.original(&photo.id)?;
        // ImageIO decodes at reduced scale (and reads HEIC); far faster than a
        // full decode followed by a resize.
        {
            let temp = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
            if crate::native_image::thumbnail(&path, temp.path(), edge) {
                return match temp.persist_noclobber(&target) {
                    Ok(_) => Ok(target),
                    Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(target),
                    Err(e) => Err(e.error.into()),
                };
            }
        }
        ensure!(
            photo.duration.is_none(),
            "Video previews are unavailable on this platform"
        );
        let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(512 * 1024 * 1024);
        reader.limits(limits);
        let mut decoder = reader.into_decoder()?;
        let orientation = image::ImageDecoder::orientation(&mut decoder)?;
        let mut image = image::DynamicImage::from_decoder(decoder)?;
        image.apply_orientation(orientation);
        let image = if image.width() <= edge && image.height() <= edge {
            image.to_rgb8()
        } else {
            image.thumbnail(edge, edge).to_rgb8()
        };
        let mut temp = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
        image::codecs::jpeg::JpegEncoder::new_with_quality(temp.as_file_mut(), 88)
            .encode_image(&image)?;
        // Cache files are renamed into place atomically; flushing each one to
        // disk (F_FULLFSYNC on macOS) would dominate thumbnail time.
        // Multiple visible views may request the same thumbnail concurrently.
        match temp.persist_noclobber(&target) {
            Ok(_) => Ok(target),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(target),
            Err(error) => Err(error.error.into()),
        }
    }
}
impl PhotosDriver for LocalPhotos {
    fn source(&self) -> Source {
        Source {
            driver: "local-folder".into(),
            path: self.root.clone(),
        }
    }
    fn scan(&self) -> Result<Library> {
        let meta = self.metadata()?;
        let mut files = vec![];
        Self::walk(&self.root, &mut files)?;
        let mut photos = vec![];
        for path in files {
            let stat = path.metadata()?;
            let modified: DateTime<Utc> = stat.modified()?.into();
            let id = path
                .strip_prefix(&self.root)?
                .to_string_lossy()
                .replace('\\', "/");
            let (width, height, duration) = if is_video(&path) {
                crate::native_image::video_info(&path)
                    .map_or((0, 0, Some(0.)), |(w, h, d)| (w, h, Some(d)))
            } else {
                let (w, h) = image::image_dimensions(&path).unwrap_or((0, 0));
                (w, h, None)
            };
            let fingerprint = format!(
                "{:x}",
                Sha256::digest(
                    format!(
                        "{id}:{}:{}",
                        stat.len(),
                        modified.timestamp_nanos_opt().unwrap_or_default()
                    )
                    .as_bytes()
                )
            );
            photos.push(Arc::new(Photo {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path,
                modified,
                bytes: stat.len(),
                width,
                height,
                favorite: meta.favorites.contains(&id),
                deleted: meta.deleted.contains(&id),
                caption: meta.captions.get(&id).cloned().unwrap_or_default(),
                fingerprint,
                id,
                duration,
            }));
        }
        photos.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id)));
        Ok(Library {
            photos,
            albums: meta.albums,
            synced: BTreeSet::new(),
        })
    }
    fn thumbnail(&self, photo: &Photo) -> Result<PathBuf> {
        let edge = crate::loading::image_sizes().grid;
        self.decode(photo, edge, &crate::loading::sized_kind("thumb", edge))
    }
    fn preview(&self, photo: &Photo) -> Result<PathBuf> {
        let edge = crate::loading::image_sizes().viewer;
        self.decode(photo, edge, &crate::loading::sized_kind("preview", edge))
    }
    fn sample(&self, photo: &Photo, edge: u32) -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = self.original(&photo.id)?;
        let target = directory.path().join("sample.jpg");
        if crate::native_image::thumbnail(&path, &target, edge) {
            return Ok(());
        }
        let image = image::ImageReader::open(path)?
            .with_guessed_format()?
            .decode()?;
        image
            .thumbnail(edge, edge)
            .to_rgb8()
            .save_with_format(&target, image::ImageFormat::Jpeg)?;
        Ok(())
    }
    fn details(&self, photo: &Photo) -> Result<PhotoDetails> {
        let path = self.original(&photo.id)?;
        let mut details = if photo.duration.is_none() {
            exif_details(&path)
        } else {
            PhotoDetails::default()
        };
        details.bytes = Some(photo.bytes);
        Ok(details)
    }
    fn squares(&self, photos: &[Arc<Photo>], edge: u32) -> Vec<Result<Vec<u8>>> {
        photos
            .iter()
            .map(|photo| {
                let cache = self.safe(&format!(
                    ".desert-photos/cache/{}",
                    crate::loading::Square::file_name_for(&photo.fingerprint, edge)
                ))?;
                crate::loading::cached_square(&cache, edge, || {
                    // Embedded previews make this nearly free for camera files.
                    let original = self.original(&photo.id)?;
                    if let Some(pixels) = crate::native_image::square(&original, edge) {
                        return Ok(pixels);
                    }
                    let thumbnail = self.thumbnail(photo)?;
                    crate::loading::square_from_file(&thumbnail, edge)
                })
            })
            .collect()
    }
    fn motion(&self, photo: &Photo, large: bool) -> Result<Option<PathBuf>> {
        if !may_move(&photo.name) {
            return Ok(None);
        }
        let cache = self.safe(".desert-photos/cache/probe")?;
        cached_motion(
            cache.parent().unwrap(),
            &photo.fingerprint,
            || self.original(&photo.id),
            large,
        )
    }
    fn video(&self, photo: &Photo) -> Option<VideoSource> {
        photo
            .duration
            .and_then(|_| self.original(&photo.id).ok())
            .map(VideoSource::File)
    }
    fn import(&self, paths: &[PathBuf]) -> Result<ImportReport> {
        let _lock = self.write_lock()?;
        let mut files = vec![];
        let mut report = ImportReport::default();
        for path in paths {
            let ty = path.symlink_metadata()?.file_type();
            if ty.is_symlink() {
                report.unsupported += 1;
            } else if ty.is_dir() {
                Self::walk(path, &mut files)?;
            } else if supported(path) {
                files.push(path.clone());
            } else {
                report.unsupported += 1;
            }
        }
        let mut existing = vec![];
        Self::walk(&self.root, &mut existing)?;
        let mut hashes = existing
            .iter()
            .map(|path| hash_file(path))
            .collect::<Result<BTreeSet<_>>>()?;
        let directory = self.safe("Imports")?;
        fs::create_dir_all(&directory)?;
        for path in files {
            // Header validation rejects mislabeled and damaged files before copying.
            if image::image_dimensions(&path).is_err() {
                report.unsupported += 1;
                continue;
            }
            let hash = hash_file(&path)?;
            if !hashes.insert(hash) {
                report.duplicates += 1;
                continue;
            }
            let mut temp = tempfile::NamedTempFile::new_in(&directory)?;
            std::io::copy(&mut fs::File::open(&path)?, &mut temp)?;
            temp.as_file().sync_all()?;
            let target = directory.join(format!(
                "{}-{}",
                uuid::Uuid::new_v4(),
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            temp.persist_noclobber(target)?;
            report.imported += 1;
        }
        Ok(report)
    }
    fn favorite(&self, ids: &[String], value: bool) -> Result<()> {
        for id in ids {
            self.original(id)?;
        }
        self.mutate(|m| {
            for id in ids {
                if value {
                    m.unfavorites.remove(id);
                    m.favorites.insert(id.clone());
                } else {
                    m.unfavorites.insert(id.clone());
                    m.favorites.remove(id);
                }
            }
            Ok(())
        })
    }
    fn trash(&self, ids: &[String], deleted: bool) -> Result<()> {
        for id in ids {
            self.original(id)?;
        }
        self.mutate(|m| {
            for id in ids {
                if deleted {
                    m.deleted.insert(id.clone());
                } else {
                    m.deleted.remove(id);
                }
            }
            Ok(())
        })
    }
    fn create_album(&self, name: &str) -> Result<()> {
        let name = album_name(name)?;
        self.mutate(|m| {
            ensure!(!m.albums.contains_key(&name), "Album name already exists.");
            m.albums.insert(name, BTreeSet::new());
            Ok(())
        })
    }
    fn add_to_album(&self, name: &str, ids: &[String]) -> Result<()> {
        for id in ids {
            self.original(id)?;
        }
        self.mutate(|m| {
            m.albums
                .get_mut(name)
                .context("Album not found.")?
                .extend(ids.iter().cloned());
            Ok(())
        })
    }
    fn remove_from_album(&self, name: &str, ids: &[String]) -> Result<()> {
        self.mutate(|m| {
            let album = m.albums.get_mut(name).context("Album not found.")?;
            for id in ids {
                album.remove(id);
            }
            Ok(())
        })
    }
    fn caption(&self, id: &str, caption: &str) -> Result<()> {
        self.original(id)?;
        self.mutate(|m| {
            m.captions.insert(id.to_owned(), caption.to_owned());
            Ok(())
        })
    }
    fn export(&self, ids: &[String], directory: &Path) -> Result<usize> {
        ensure!(directory.is_dir(), "Choose an existing export folder");
        let mut count = 0;
        for id in ids {
            let original = self.original(id)?;
            let name = original.file_name().context("Invalid photo name")?;
            let target = directory.join(name);
            let mut temp = tempfile::NamedTempFile::new_in(directory)?;
            std::io::copy(&mut fs::File::open(original)?, &mut temp)?;
            temp.as_file().sync_all()?;
            temp.persist_noclobber(&target).with_context(|| {
                format!(
                    "Export failed: {}. Check for an existing file.",
                    target.display()
                )
            })?;
            count += 1;
        }
        Ok(count)
    }
}
pub fn supported(path: &Path) -> bool {
    is_video(path)
        || path.extension().is_some_and(|e| {
            ["jpg", "jpeg", "png", "gif", "webp", "tif", "tiff", "bmp"]
                .iter()
                .any(|s| e.eq_ignore_ascii_case(s))
        })
}
/// Videos are catalogued only where the platform can make their previews.
pub fn is_video(path: &Path) -> bool {
    crate::native_image::VIDEO
        && path.extension().is_some_and(|e| {
            ["mov", "mp4", "m4v"]
                .iter()
                .any(|s| e.eq_ignore_ascii_case(s))
        })
}
pub fn album_name(name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(
        !name.is_empty() && name.len() <= 100 && !name.chars().any(char::is_control),
        "Album names must be 1–100 characters."
    );
    Ok(name.to_owned())
}
fn hash_file(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let directory = path.parent().context("Invalid storage path")?;
    fs::create_dir_all(directory)?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    temp.write_all(&serde_json::to_vec_pretty(value)?)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
