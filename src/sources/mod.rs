//! Paged, read-only provider access. Edits are private Photos annotations.
use crate::storage::{ImportReport, Library, LocalPhotos, Photo, PhotosDriver, Source};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(all(target_os = "macos", feature = "apple-photos"))]
mod apple;
#[cfg(all(target_os = "macos", feature = "apple-photos"))]
pub use apple::verify_configuration as verify_photokit_configuration;
#[cfg(feature = "remote")]
pub mod remote;

#[derive(Default)]
pub struct PhotoPage {
    pub library: Library,
    pub next: Option<String>,
    pub total: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Local,
    Apple,
    Immich,
    Nextcloud,
}
impl ProviderKind {
    pub const ALL: [Self; 4] = [Self::Local, Self::Apple, Self::Immich, Self::Nextcloud];
    pub fn id(self) -> &'static str {
        match self {
            Self::Local => "local-folder",
            Self::Apple => "apple-photos",
            Self::Immich => "immich",
            Self::Nextcloud => "nextcloud",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Local => "Local folder",
            Self::Apple => "Apple Photos",
            Self::Immich => "Immich",
            Self::Nextcloud => "Nextcloud",
        }
    }
    pub fn detail(self) -> &'static str {
        match self {
            Self::Local => "On this device",
            Self::Apple => "macOS photo library",
            Self::Immich => "Your photo server",
            Self::Nextcloud => "Connect with WebDAV",
        }
    }
    pub fn available(self) -> bool {
        match self {
            Self::Local => true,
            Self::Apple => cfg!(all(target_os = "macos", feature = "apple-photos")),
            _ => cfg!(feature = "remote"),
        }
    }
}

/// Settings contain connection details only. Secrets are stored in the OS keyring.
#[derive(Clone, Serialize, Deserialize)]
pub struct Connection {
    pub provider: String,
    pub server: String,
    pub username: String,
    #[serde(default)]
    pub folder: String,
}
impl Connection {
    pub fn validate(&self) -> Result<url::Url> {
        ensure!(
            matches!(self.provider.as_str(), "immich" | "nextcloud"),
            "Unknown provider"
        );
        let mut url = url::Url::parse(self.server.trim()).context("Enter a complete server URL")?;
        ensure!(
            matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
            "Use an HTTP or HTTPS server URL"
        );
        ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Put credentials in the sign-in fields, not the URL"
        );
        if self.provider == "nextcloud" {
            ensure!(
                !self.username.trim().is_empty(),
                "Enter your Nextcloud username"
            );
            ensure!(
                !self.folder.split('/').any(|p| matches!(p, "." | "..")),
                "Use a folder relative to your Nextcloud files"
            );
        }
        let path = format!("{}/", url.path().trim_end_matches('/'));
        url.set_path(&path);
        Ok(url)
    }
    pub fn source(&self, data: &Path) -> Result<Source> {
        let url = self.validate()?;
        let identity = format!(
            "{}:{}:{}:{}",
            self.provider, url, self.username, self.folder
        );
        Ok(Source {
            driver: self.provider.clone(),
            path: data.join("sources").join(digest(&identity)),
        })
    }
}
pub fn connection_label(source: &Source) -> String {
    if source.driver == "apple-photos" {
        return "Apple Photos".into();
    }
    let config = fs::read(source.path.join("connection.json"))
        .ok()
        .and_then(|s| serde_json::from_slice::<Connection>(&s).ok());
    config
        .and_then(|c| url::Url::parse(&c.server).ok())
        .and_then(|u| u.host_str().map(str::to_owned))
        .map(|host| {
            format!(
                "{} · {host}",
                if source.driver == "immich" {
                    "Immich"
                } else {
                    "Nextcloud"
                }
            )
        })
        .unwrap_or_else(|| source.driver.clone())
}
pub fn digest(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
/// Disconnect credentials without deleting originals or library annotations.
pub fn forget_credentials(source: &Source) -> Result<()> {
    #[cfg(feature = "remote")]
    if matches!(source.driver.as_str(), "immich" | "nextcloud") {
        match remote::credential(source)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => (),
            Err(error) => return Err(error.into()),
        }
    }
    let _ = source;
    Ok(())
}

pub fn open(source: &Source) -> Result<Box<dyn PhotosDriver>> {
    match source.driver.as_str() {
        #[cfg(feature = "remote")]
        "immich" | "nextcloud" => {
            let config: Connection =
                serde_json::from_slice(&fs::read(source.path.join("connection.json"))?)?;
            ensure!(
                config.provider == source.driver,
                "Connection provider mismatch"
            );
            let secret = remote::credential(source)?
                .get_password()
                .context("Credentials unavailable. Reconnect this photo source.")?;
            let provider = Box::new(remote::Remote::new(config, &secret)?);
            Ok(Box::new(ConnectedPhotos::new(source.clone(), provider)?))
        }
        #[cfg(all(target_os = "macos", feature = "apple-photos"))]
        "apple-photos" => Ok(Box::new(ConnectedPhotos::new(
            source.clone(),
            Box::new(apple::ApplePhotos),
        )?)),
        _ => bail!("This build does not include {}", source.driver),
    }
}

/// Provider implementations never mutate server libraries. All I/O runs off the UI thread.
pub trait Provider: Send + Sync {
    fn page(&self, cursor: Option<&str>) -> Result<PhotoPage>;
    fn image(&self, photo: &Photo, preview: bool, destination: &Path) -> Result<()>;
    fn original(&self, id: &str, destination: &Path) -> Result<String>;
}
pub struct ConnectedPhotos {
    source: Source,
    provider: Box<dyn Provider>,
    annotations: LocalPhotos,
}
impl ConnectedPhotos {
    pub fn new(source: Source, provider: Box<dyn Provider>) -> Result<Self> {
        fs::create_dir_all(&source.path)?;
        let annotations = LocalPhotos::open(&source.path)?;
        Ok(Self {
            source,
            provider,
            annotations,
        })
    }
    fn image(&self, photo: &Photo, preview: bool) -> Result<PathBuf> {
        let kind = if preview { "preview" } else { "thumb" };
        let directory = self.source.path.join(".desert-photos/cache");
        let path = directory.join(format!("{}-{kind}.jpg", digest(&photo.fingerprint)));
        if path.is_file() {
            return Ok(path);
        }
        let temp = tempfile::NamedTempFile::new_in(&directory)?;
        self.provider.image(photo, preview, temp.path())?;
        // Reject HTML error pages and enforce decode bounds before GPUI sees data.
        let mut reader = image::ImageReader::open(temp.path())?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(128 * 1024 * 1024);
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        reader.limits(limits);
        let mut decoder = reader.into_decoder()?;
        let orientation = image::ImageDecoder::orientation(&mut decoder)?;
        let mut image = image::DynamicImage::from_decoder(decoder)?;
        image.apply_orientation(orientation);
        let edge = if preview { 2560 } else { 512 };
        let image = if image.width() <= edge && image.height() <= edge {
            image.to_rgb8()
        } else {
            image.thumbnail(edge, edge).to_rgb8()
        };
        image::codecs::jpeg::JpegEncoder::new_with_quality(fs::File::create(temp.path())?, 88)
            .encode_image(&image)?;
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(&path) {
            Ok(_) => (),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.error.into()),
        }
        Ok(path)
    }
    fn annotate(&self, library: &mut Library) -> Result<()> {
        let meta = self.annotations.metadata()?;
        library.albums = meta.albums;
        for photo in &mut library.photos {
            let favorite = (photo.favorite || meta.favorites.contains(&photo.id))
                && !meta.unfavorites.contains(&photo.id);
            let deleted = meta.deleted.contains(&photo.id);
            let caption = meta.captions.get(&photo.id);
            if favorite != photo.favorite
                || deleted != photo.deleted
                || caption.is_some_and(|c| c != &photo.caption)
            {
                let p = std::sync::Arc::make_mut(photo);
                p.favorite = favorite;
                p.deleted = deleted;
                if let Some(c) = caption {
                    p.caption.clone_from(c);
                }
            }
        }
        Ok(())
    }
}
impl PhotosDriver for ConnectedPhotos {
    fn source(&self) -> Source {
        self.source.clone()
    }
    fn paged(&self) -> bool {
        true
    }
    fn can_import(&self) -> bool {
        false
    }
    fn page(&self, cursor: Option<&str>) -> Result<PhotoPage> {
        let mut page = self.provider.page(cursor)?;
        ensure!(
            page.next.as_deref().is_none_or(|next| Some(next) != cursor),
            "Provider returned a repeated page cursor"
        );
        self.annotate(&mut page.library)?;
        Ok(page)
    }
    fn refresh_library(&self, mut library: Library) -> Result<Library> {
        self.annotate(&mut library)?;
        Ok(library)
    }
    fn scan(&self) -> Result<Library> {
        Ok(self.page(None)?.library)
    }
    fn thumbnail(&self, photo: &Photo) -> Result<PathBuf> {
        self.image(photo, false)
    }
    fn preview(&self, photo: &Photo) -> Result<PathBuf> {
        self.image(photo, true)
    }
    fn import(&self, _: &[PathBuf]) -> Result<ImportReport> {
        bail!("Import photos through this provider's own app, or choose a local folder")
    }
    fn favorite(&self, ids: &[String], value: bool) -> Result<()> {
        self.annotations.mutate(|m| {
            for id in ids {
                if value {
                    m.unfavorites.remove(id);
                    m.favorites.insert(id.clone());
                } else {
                    m.favorites.remove(id);
                    m.unfavorites.insert(id.clone());
                }
            }
            Ok(())
        })
    }
    fn trash(&self, ids: &[String], deleted: bool) -> Result<()> {
        self.annotations.mutate(|m| {
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
    fn caption(&self, id: &str, caption: &str) -> Result<()> {
        self.annotations.mutate(|m| {
            m.captions.insert(id.to_owned(), caption.to_owned());
            Ok(())
        })
    }
    fn create_album(&self, name: &str) -> Result<()> {
        self.annotations.create_album(name)
    }
    fn add_to_album(&self, name: &str, ids: &[String]) -> Result<()> {
        self.annotations.mutate(|m| {
            m.albums
                .get_mut(name)
                .context("Album not found")?
                .extend(ids.iter().cloned());
            Ok(())
        })
    }
    fn remove_from_album(&self, name: &str, ids: &[String]) -> Result<()> {
        self.annotations.remove_from_album(name, ids)
    }
    fn export(&self, ids: &[String], directory: &Path) -> Result<usize> {
        ensure!(directory.is_dir(), "Choose an existing export folder");
        for id in ids {
            let temp = tempfile::NamedTempFile::new_in(directory)?;
            let name = self.provider.original(id, temp.path())?;
            ensure!(
                !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\']),
                "Invalid original filename"
            );
            temp.as_file().sync_all()?;
            temp.persist_noclobber(directory.join(name)).context(
                "Cannot export original: filename already exists or folder is unavailable",
            )?;
        }
        Ok(ids.len())
    }
}
