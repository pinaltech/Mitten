//! Pinal Photos in the browser: the signed-in account's library at
//! photos.pinal.tech, read through the page's session (no tokens here).
//!
//! Thumbnails and previews are handed to the UI as URLs, which GPUI fetches
//! and decodes itself; grid squares are fetched and cropped here, in GPUI's
//! worker threads. Favorites, albums, captions and Recently Deleted are kept
//! for this visit only: Pinal Photos has no API for them yet.
use super::*;
use crate::web;

pub const DRIVER: &str = "pinal-photos";
const METADATA_LIMIT: usize = 16 << 20;
const IMAGE_LIMIT: usize = 8 << 20;

pub fn source() -> Source {
    Source {
        driver: DRIVER.into(),
        path: PathBuf::from("/pinal-photos"),
    }
}

#[derive(Default)]
struct Annotations {
    favorites: BTreeSet<String>,
    deleted: BTreeSet<String>,
    captions: BTreeMap<String, String>,
    albums: BTreeMap<String, BTreeSet<String>>,
}

/// Kept for the whole visit, across reloads of the library.
static ANNOTATIONS: Mutex<Option<Annotations>> = Mutex::new(None);

#[derive(Default)]
pub struct WebPinalPhotos;

fn annotations<T>(f: impl FnOnce(&mut Annotations) -> T) -> T {
    f(ANNOTATIONS.lock().unwrap().get_or_insert_with(Annotations::default))
}

impl WebPinalPhotos {
    fn url(path: &str) -> PathBuf {
        PathBuf::from(format!("{}{path}", web::origin()))
    }
    fn annotate(&self, photos: &mut [Arc<Photo>]) {
        annotations(|a| for photo in photos.iter_mut() {
            let favorite = a.favorites.contains(&photo.id);
            let deleted = a.deleted.contains(&photo.id);
            let caption = a.captions.get(&photo.id).cloned().unwrap_or_default();
            if photo.favorite != favorite || photo.deleted != deleted || photo.caption != caption {
                let p = Arc::make_mut(photo);
                p.favorite = favorite;
                p.deleted = deleted;
                p.caption = caption;
            }
        })
    }
}

impl PhotosDriver for WebPinalPhotos {
    fn source(&self) -> Source {
        source()
    }
    fn paged(&self) -> bool {
        true
    }
    fn download_url(&self, photo: &Photo) -> Option<String> {
        let id = pinal_api::valid(&photo.id).ok()?;
        Some(format!("{}/api/photos/{id}?download=1", web::origin()))
    }
    fn can_import(&self) -> bool {
        // Through the page's own file picker and drops (see the UI's Import).
        true
    }
    fn page(&self, cursor: Option<&str>) -> Result<PhotoPage> {
        let mut path = format!("/api/photos?limit={}", crate::loading::PAGE_SIZE);
        if let Some(c) = cursor {
            path.push_str("&cursor=");
            path.extend(url::form_urlencoded::byte_serialize(c.as_bytes()));
        }
        let mut page = pinal_api::parse_page(&web::get(&path, METADATA_LIMIT)?)?;
        self.annotate(&mut page.library.photos);
        page.library.albums = annotations(|a| a.albums.clone());
        Ok(page)
    }
    fn refresh_library(&self, mut library: Library) -> Result<Library> {
        self.annotate(&mut library.photos);
        library.albums = annotations(|a| a.albums.clone());
        Ok(library)
    }
    fn scan(&self) -> Result<Library> {
        Ok(Library::default())
    }
    fn thumbnail(&self, photo: &Photo) -> Result<PathBuf> {
        Ok(Self::url(&format!("/api/photos/{}/thumb", pinal_api::valid(&photo.id)?)))
    }
    fn preview(&self, photo: &Photo) -> Result<PathBuf> {
        let id = pinal_api::valid(&photo.id)?;
        // Videos are shown by their thumbnail (a frame the uploader took).
        Ok(Self::url(&if photo.duration.is_some() {
            format!("/api/photos/{id}/thumb")
        } else {
            format!("/api/photos/{id}")
        }))
    }
    fn squares(&self, photos: &[Arc<Photo>], edge: u32) -> Vec<Result<Vec<u8>>> {
        photos
            .iter()
            .map(|photo| {
                let id = pinal_api::valid(&photo.id)?;
                let bytes = web::get(&format!("/api/photos/{id}/thumb"), IMAGE_LIMIT)?;
                let image = image::load_from_memory(&bytes).context("Unreadable thumbnail")?;
                Ok(crate::loading::square_bgra(&image, edge))
            })
            .collect()
    }
    fn rendition(&self, edge: u32) -> u32 {
        if edge <= 512 { 512 } else { 2560 }
    }
    fn details(&self, photo: &Photo) -> Result<PhotoDetails> {
        Ok(PhotoDetails {
            bytes: (photo.bytes > 0).then_some(photo.bytes),
            ..Default::default()
        })
    }
    fn import(&self, _: &[PathBuf]) -> Result<ImportReport> {
        bail!("Upload photos with Import, or drop them on the page")
    }
    fn export(&self, _: &[String], _: &Path) -> Result<usize> {
        bail!("Download photos from the viewer")
    }
    fn favorite(&self, ids: &[String], value: bool) -> Result<()> {
        annotations(|a| for id in ids {
            if value {
                a.favorites.insert(id.clone());
            } else {
                a.favorites.remove(id);
            }
        });
        Ok(())
    }
    fn trash(&self, ids: &[String], deleted: bool) -> Result<()> {
        annotations(|a| for id in ids {
            if deleted {
                a.deleted.insert(id.clone());
            } else {
                a.deleted.remove(id);
            }
        });
        Ok(())
    }
    fn create_album(&self, name: &str) -> Result<()> {
        annotations(|a| {
            a.albums.entry(name.to_owned()).or_default();
        });
        Ok(())
    }
    fn add_to_album(&self, name: &str, ids: &[String]) -> Result<()> {
        annotations(|a| a.albums.entry(name.to_owned()).or_default().extend(ids.iter().cloned()));
        Ok(())
    }
    fn remove_from_album(&self, name: &str, ids: &[String]) -> Result<()> {
        annotations(|a| {
            if let Some(album) = a.albums.get_mut(name) {
                for id in ids {
                    album.remove(id);
                }
            }
        });
        Ok(())
    }
    fn caption(&self, id: &str, caption: &str) -> Result<()> {
        annotations(|a| a.captions.insert(id.to_owned(), caption.to_owned()));
        Ok(())
    }
}
