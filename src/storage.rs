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
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Preferences {
    pub sources: Vec<Source>,
    pub active: Option<PathBuf>,
    #[serde(default)]
    pub appearance: String,
}
impl Preferences {
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
}
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub photos: Vec<Arc<Photo>>,
    pub albums: BTreeMap<String, BTreeSet<String>>,
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
        temp.as_file().sync_all()?;
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
            let (width, height) = image::image_dimensions(&path).unwrap_or((0, 0));
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
            }));
        }
        photos.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id)));
        Ok(Library {
            photos,
            albums: meta.albums,
        })
    }
    fn thumbnail(&self, photo: &Photo) -> Result<PathBuf> {
        self.decode(photo, 512, "thumb")
    }
    fn preview(&self, photo: &Photo) -> Result<PathBuf> {
        self.decode(photo, 2560, "preview")
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
    path.extension().is_some_and(|e| {
        ["jpg", "jpeg", "png", "gif", "webp", "tif", "tiff", "bmp"]
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
