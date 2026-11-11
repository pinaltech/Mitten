//! OS handoff for an explicit original-file drag or copy. In-app drags stay in GPUI.
//! Each photo is offered as its original file plus JPEG, PNG and TIFF data,
//! converted by the system only when a receiver asks for that format.
use super::*;

#[cfg(target_os = "macos")]
#[derive(serde::Serialize)]
struct Item {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    local: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<PathBuf>,
}
#[cfg(target_os = "macos")]
struct ExportContext {
    driver: Arc<dyn PhotosDriver>,
    photos: Vec<Arc<Photo>>,
}

pub(super) fn should_handoff(w: &Window, at: Point<Pixels>) -> bool {
    let size = w.viewport_size();
    if at.x < px(0.) || at.y < px(0.) || at.x >= size.width || at.y >= size.height {
        return true;
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = HasWindowHandle::window_handle(w)
            && let RawWindowHandle::AppKit(handle) = handle.as_raw()
        {
            return unsafe { dp_drag_is_over_other_window(handle.ns_view.as_ptr()) };
        }
    }
    false
}

#[cfg(target_os = "macos")]
fn records(photos: &[Arc<Photo>], previews: Vec<Option<PathBuf>>) -> Option<std::ffi::CString> {
    let names = mitten::drag_export::filenames(photos.iter().map(|p| p.name.as_str()));
    let items: Vec<_> = photos
        .iter()
        .zip(names)
        .zip(previews)
        .map(|((p, name), preview)| Item {
            name,
            local: (!p.path.as_os_str().is_empty() && p.path.is_file()).then(|| p.path.clone()),
            preview,
        })
        .collect();
    std::ffi::CString::new(serde_json::to_string(&items).ok()?).ok()
}

/// Replace the clipboard with these photos.
pub(super) fn copy(driver: Arc<dyn PhotosDriver>, photos: Vec<Arc<Photo>>) -> bool {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        if photos.is_empty() {
            return false;
        }
        let previews = vec![None; photos.len()];
        let Some(json) = records(&photos, previews) else {
            return false;
        };
        let context = Box::into_raw(Box::new(ExportContext { driver, photos }));
        // The pasteboard owns the context until the clipboard changes.
        unsafe { dp_copy_photos(json.as_ptr(), context.cast(), write, release) }
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = (driver, photos);
        false
    }
}

pub(super) fn start(
    w: &Window,
    driver: Arc<dyn PhotosDriver>,
    photos: Vec<Arc<Photo>>,
    previews: Vec<Option<PathBuf>>,
) -> bool {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = HasWindowHandle::window_handle(w) else {
            return false;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return false;
        };
        if photos.is_empty() {
            return false;
        }
        let Some(json) = records(&photos, previews) else {
            return false;
        };
        let context = Box::into_raw(Box::new(ExportContext { driver, photos }));
        // The ARC context takes ownership on both success and failure. Promise
        // writers retain it beyond the drag session, until fulfillment completes.
        unsafe {
            dp_start_original_drag(
                handle.ns_view.as_ptr(),
                json.as_ptr(),
                context.cast(),
                write,
                release,
            )
        }
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = (w, driver, photos, previews);
        false
    }
}

#[cfg(target_os = "macos")]
type WriteOriginal = extern "C" fn(
    *mut std::ffi::c_void,
    usize,
    *const std::ffi::c_char,
    *mut std::ffi::c_char,
    usize,
) -> bool;
#[cfg(target_os = "macos")]
unsafe extern "C" {
    #[cfg(not(test))]
    fn dp_copy_photos(
        json: *const std::ffi::c_char,
        context: *mut std::ffi::c_void,
        write: WriteOriginal,
        release: extern "C" fn(*mut std::ffi::c_void),
    ) -> bool;
    #[cfg(test)]
    fn dp_verify_photo_flavors(
        json: *const std::ffi::c_char,
        context: *mut std::ffi::c_void,
        write: WriteOriginal,
        release: extern "C" fn(*mut std::ffi::c_void),
        width: usize,
        height: usize,
    ) -> bool;
    #[cfg(not(test))]
    fn dp_drag_is_over_other_window(view: *mut std::ffi::c_void) -> bool;
    #[cfg(test)]
    fn dp_verify_original_promise(
        directory: *const std::ffi::c_char,
        context: *mut std::ffi::c_void,
        write: extern "C" fn(
            *mut std::ffi::c_void,
            usize,
            *const std::ffi::c_char,
            *mut std::ffi::c_char,
            usize,
        ) -> bool,
        release: extern "C" fn(*mut std::ffi::c_void),
    ) -> bool;
    fn dp_start_original_drag(
        view: *mut std::ffi::c_void,
        json: *const std::ffi::c_char,
        context: *mut std::ffi::c_void,
        write: extern "C" fn(
            *mut std::ffi::c_void,
            usize,
            *const std::ffi::c_char,
            *mut std::ffi::c_char,
            usize,
        ) -> bool,
        release: extern "C" fn(*mut std::ffi::c_void),
    ) -> bool;
}
#[cfg(target_os = "macos")]
extern "C" fn release(context: *mut std::ffi::c_void) {
    unsafe {
        drop(Box::from_raw(context.cast::<ExportContext>()));
    }
}
#[cfg(target_os = "macos")]
extern "C" fn write(
    context: *mut std::ffi::c_void,
    index: usize,
    destination: *const std::ffi::c_char,
    error: *mut std::ffi::c_char,
    capacity: usize,
) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let context = unsafe { &*context.cast::<ExportContext>() };
        let path = unsafe { std::ffi::CStr::from_ptr(destination) };
        let destination = std::path::Path::new(std::ffi::OsStr::from_bytes(path.to_bytes()));
        let photo = context
            .photos
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Photo missing from drag"))?;
        mitten::drag_export::write_original(context.driver.as_ref(), &photo.id, destination)
    }));
    let message = match result {
        Ok(Ok(())) => return true,
        Ok(Err(e)) => format!("{e:#}"),
        Err(_) => "Could not deliver the original photo".into(),
    };
    if !error.is_null() && capacity > 0 {
        let bytes = message.as_bytes();
        let len = bytes.len().min(capacity - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), error.cast(), len);
            *error.add(len) = 0;
        }
    }
    false
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    #[test]
    fn native_promise_fulfillment_preserves_original_bytes() {
        let source = tempfile::tempdir().unwrap();
        let receiver = tempfile::tempdir().unwrap();
        let original = source.path().join("Original.png");
        image::RgbImage::from_pixel(40, 30, image::Rgb([70, 90, 110]))
            .save(&original)
            .unwrap();
        let driver: Arc<dyn PhotosDriver> =
            Arc::new(storage::LocalPhotos::open(source.path()).unwrap());
        let photos = driver.scan().unwrap().photos;
        let context = Box::into_raw(Box::new(ExportContext { driver, photos }));
        let destination = std::ffi::CString::new(receiver.path().to_str().unwrap()).unwrap();
        assert!(
            unsafe {
                dp_verify_original_promise(destination.as_ptr(), context.cast(), write, release)
            },
            "the native promise delegate must deliver the original file"
        );
        assert_eq!(
            std::fs::read(receiver.path().join("Promised.png")).unwrap(),
            std::fs::read(original).unwrap()
        );
    }
    #[test]
    fn copied_photos_offer_original_file_and_converted_image_flavors() {
        let source = tempfile::tempdir().unwrap();
        image::RgbaImage::from_pixel(40, 30, image::Rgba([70, 90, 110, 128]))
            .save(source.path().join("Original.png"))
            .unwrap();
        image::RgbImage::from_pixel(40, 30, image::Rgb([20, 40, 60]))
            .save(source.path().join("Second.jpg"))
            .unwrap();
        let driver: Arc<dyn PhotosDriver> =
            Arc::new(storage::LocalPhotos::open(source.path()).unwrap());
        let photos = driver.scan().unwrap().photos;
        assert_eq!(photos.len(), 2);
        let json = records(&photos, vec![None; photos.len()]).unwrap();
        let context = Box::into_raw(Box::new(ExportContext {
            driver: driver.clone(),
            photos,
        }));
        assert!(
            unsafe {
                dp_verify_photo_flavors(json.as_ptr(), context.cast(), write, release, 40, 30)
            },
            "every copied photo needs a file URL and decodable JPEG, PNG and TIFF data"
        );
        assert_eq!(
            Arc::strong_count(&driver),
            1,
            "the pasteboard releases its context"
        );
    }
    #[test]
    fn heic_originals_convert_to_universal_flavors() {
        let folder = tempfile::tempdir().unwrap();
        let png = folder.path().join("Source.png");
        let heic = folder.path().join("Photo.heic");
        image::RgbImage::from_pixel(64, 48, image::Rgb([200, 120, 40]))
            .save(&png)
            .unwrap();
        let converted = std::process::Command::new("sips")
            .args(["-s", "format", "heic"])
            .arg(&png)
            .arg("--out")
            .arg(&heic)
            .output()
            .is_ok_and(|o| o.status.success() && heic.is_file());
        if !converted {
            eprintln!("skipping: this Mac cannot encode HEIC");
            return;
        }
        let driver: Arc<dyn PhotosDriver> =
            Arc::new(storage::LocalPhotos::open(folder.path()).unwrap());
        let json = std::ffi::CString::new(
            serde_json::to_string(&[Item {
                name: "Photo.heic".into(),
                local: Some(heic),
                preview: None,
            }])
            .unwrap(),
        )
        .unwrap();
        let context = Box::into_raw(Box::new(ExportContext {
            driver,
            photos: vec![],
        }));
        assert!(unsafe {
            dp_verify_photo_flavors(json.as_ptr(), context.cast(), write, release, 64, 48)
        });
    }
    #[test]
    fn native_failure_releases_context_and_write_errors_cross_ffi_without_panicking() {
        let folder = tempfile::tempdir().unwrap();
        let driver: Arc<dyn PhotosDriver> =
            Arc::new(storage::LocalPhotos::open(folder.path()).unwrap());
        let context = Box::into_raw(Box::new(ExportContext {
            driver: driver.clone(),
            photos: vec![],
        }));
        let destination =
            std::ffi::CString::new(folder.path().join("test.png").to_str().unwrap()).unwrap();
        let mut error = [0_i8; 128];
        assert!(!write(
            context.cast(),
            0,
            destination.as_ptr(),
            error.as_mut_ptr(),
            error.len()
        ));
        assert!(
            unsafe { std::ffi::CStr::from_ptr(error.as_ptr()) }
                .to_string_lossy()
                .contains("Photo missing")
        );
        let json = std::ffi::CString::new("[]").unwrap();
        assert_eq!(Arc::strong_count(&driver), 2);
        assert!(!unsafe {
            dp_start_original_drag(
                std::ptr::null_mut(),
                json.as_ptr(),
                context.cast(),
                write,
                release,
            )
        });
        assert_eq!(
            Arc::strong_count(&driver),
            1,
            "native failure must release its Rust-owned context"
        );
    }
}
