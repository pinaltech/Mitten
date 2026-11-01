use super::*;
use crate::loading::PAGE_SIZE;
use serde_json::Value;
use std::ffi::{CStr, CString, c_char};
use std::sync::Arc;

unsafe extern "C" {
    fn dp_photos_validate_usage() -> *mut c_char;
    fn dp_photos_page(offset: usize, limit: usize) -> *mut c_char;
    fn dp_photos_image(id: *const c_char, destination: *const c_char, edge: u32) -> *mut c_char;
    fn dp_photos_original(id: *const c_char, destination: *const c_char) -> *mut c_char;
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
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(PhotoPage {
            library: Library {
                photos,
                albums: Default::default(),
            },
            next: (end < total).then(|| end.to_string()),
            total: Some(total),
        })
    }
    fn image(&self, photo: &Photo, preview: bool, destination: &Path) -> Result<()> {
        let id = CString::new(photo.id.as_str())?;
        let path = CString::new(destination.as_os_str().as_encoded_bytes())?;
        unsafe {
            result(dp_photos_image(
                id.as_ptr(),
                path.as_ptr(),
                if preview { 2560 } else { 512 },
            ))
        }?;
        Ok(())
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
