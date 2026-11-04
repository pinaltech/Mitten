use super::*;
use crate::loading::PAGE_SIZE;
use crate::storage::VideoSource;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString, c_char};
use std::sync::Arc;

unsafe extern "C" {
    fn dp_photos_validate_usage() -> *mut c_char;
    fn dp_photos_page(offset: usize, limit: usize) -> *mut c_char;
    fn dp_photos_image(id: *const c_char, destination: *const c_char, edge: u32) -> *mut c_char;
    fn dp_photos_original(id: *const c_char, destination: *const c_char) -> *mut c_char;
    fn dp_photos_squares(
        ids: *const *const c_char,
        count: usize,
        edge: u32,
        pixels: *mut u8,
        ok: *mut u8,
    ) -> *mut c_char;
    fn dp_photos_albums() -> *mut c_char;
    fn dp_photos_details(id: *const c_char) -> *mut c_char;
    fn dp_photos_free(value: *mut c_char);
}
pub struct ApplePhotos;
pub fn verify_configuration() -> Result<()> {
    unsafe { result(dp_photos_validate_usage()) }?;
    Ok(())
}
/// Each bridge result is owned UTF-8 JSON and is freed exactly once.
unsafe fn result(raw: *mut c_char) -> Result<Value> {
    ensure!(!raw.is_null(), "PhotoKit returned no result");
    let data = unsafe { CStr::from_ptr(raw) }.to_bytes().to_vec();
    unsafe { dp_photos_free(raw) };
    let value: Value = serde_json::from_slice(&data)?;
    if let Some(error) = value["error"].as_str() {
        bail!("{error}");
    }
    Ok(value)
}
impl Provider for ApplePhotos {
    fn page(&self, cursor: Option<&str>) -> Result<PhotoPage> {
        let offset = cursor.unwrap_or("0").parse::<usize>()?;
        let data = unsafe { result(dp_photos_page(offset, PAGE_SIZE)) }?;
        let total = data["total"].as_u64().context("Invalid PhotoKit count")? as usize;
        let end = data["end"].as_u64().context("Invalid PhotoKit page")? as usize;
        let photos = data["items"]
            .as_array()
            .context("Invalid PhotoKit assets")?
            .iter()
            .map(|a| {
                let id = a["id"]
                    .as_str()
                    .context("PhotoKit asset has no ID")?
                    .to_owned();
                Ok(Arc::new(Photo {
                    fingerprint: digest(&format!("{id}:{}", a["modified"])),
                    id,
                    path: PathBuf::new(),
                    name: a["name"].as_str().unwrap_or("Photo").into(),
                    modified: chrono::DateTime::from_timestamp(
                        a["created"].as_f64().unwrap_or_default() as i64,
                        0,
                    )
                    .unwrap_or_default(),
                    width: a["width"].as_u64().unwrap_or(0) as u32,
                    height: a["height"].as_u64().unwrap_or(0) as u32,
                    bytes: 0,
                    favorite: a["favorite"].as_bool().unwrap_or(false),
                    deleted: false,
                    caption: String::new(),
                    duration: a["duration"].as_f64().map(|d| d as f32),
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(PhotoPage {
            library: Library {
                photos,
                ..Default::default()
            },
            next: (end < total).then(|| end.to_string()),
            total: Some(total),
        })
    }
    fn image(&self, photo: &Photo, preview: bool, destination: &Path) -> Result<()> {
        self.image_at(photo, if preview { 2560 } else { 512 }, destination)
    }
    fn image_at(&self, photo: &Photo, edge: u32, destination: &Path) -> Result<()> {
        let id = CString::new(photo.id.as_str())?;
        let path = CString::new(destination.as_os_str().as_encoded_bytes())?;
        unsafe { result(dp_photos_image(id.as_ptr(), path.as_ptr(), edge)) }?;
        Ok(())
    }
    fn details(&self, photo: &Photo) -> Result<crate::storage::PhotoDetails> {
        let id = CString::new(photo.id.as_str())?;
        let value = unsafe { result(dp_photos_details(id.as_ptr())) }?;
        Ok(serde_json::from_value(value)?)
    }
    /// Photos keeps small thumbnails for every asset; reading those (fast
    /// format, no download) is far quicker than rendering 512 px previews.
    fn squares(&self, photos: &[Arc<Photo>], edge: u32) -> Option<Vec<Result<Vec<u8>>>> {
        let ids: Vec<CString> = photos
            .iter()
            .map(|p| CString::new(p.id.as_str()).unwrap_or_default())
            .collect();
        let pointers: Vec<*const c_char> = ids.iter().map(|id| id.as_ptr()).collect();
        let size = (edge * edge * 4) as usize;
        let mut pixels = vec![0u8; size * photos.len()];
        let mut ok = vec![0u8; photos.len()];
        let status = unsafe {
            result(dp_photos_squares(
                pointers.as_ptr(),
                photos.len(),
                edge,
                pixels.as_mut_ptr(),
                ok.as_mut_ptr(),
            ))
        };
        if let Err(e) = status {
            let message = e.to_string();
            return Some(
                photos
                    .iter()
                    .map(|_| Err(anyhow::anyhow!("{message}")))
                    .collect(),
            );
        }
        Some(
            pixels
                .chunks_exact(size)
                .zip(ok)
                .map(|(square, ok)| {
                    if ok == 1 {
                        Ok(square.to_vec())
                    } else {
                        Err(anyhow::anyhow!("Photos has no cached thumbnail"))
                    }
                })
                .collect(),
        )
    }
    /// Albums keep their Apple Photos names; a repeated name gets a number.
    fn albums(&self) -> Result<BTreeMap<String, BTreeSet<String>>> {
        let data = unsafe { result(dp_photos_albums()) }?;
        let mut albums = BTreeMap::new();
        for album in data["albums"]
            .as_array()
            .context("Invalid PhotoKit albums")?
        {
            let base = album["name"].as_str().unwrap_or("Untitled Album").trim();
            let base = if base.is_empty() {
                "Untitled Album"
            } else {
                base
            };
            let mut name = base.to_owned();
            for n in 2.. {
                if !albums.contains_key(&name) {
                    break;
                }
                name = format!("{base} ({n})");
            }
            let ids = album["ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect();
            albums.insert(name, ids);
        }
        Ok(albums)
    }
    fn video(&self, photo: &Photo) -> Option<VideoSource> {
        photo
            .duration
            .map(|_| VideoSource::PhotoKit(photo.id.clone()))
    }
    fn original(&self, id: &str, destination: &Path) -> Result<String> {
        let id = CString::new(id)?;
        let path = CString::new(destination.as_os_str().as_encoded_bytes())?;
        let value = unsafe { result(dp_photos_original(id.as_ptr(), path.as_ptr())) }?;
        Ok(value["name"]
            .as_str()
            .context("PhotoKit filename unavailable")?
            .into())
    }
}
