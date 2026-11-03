//! macOS ImageIO thumbnails (see `native/imageio.m`); elsewhere these report
//! "unavailable" and callers fall back to the `image` crate. Video files get
//! a representative frame through AVFoundation.
use std::path::Path;

#[cfg(all(
    target_os = "macos",
    any(feature = "apple-photos", feature = "native-ui")
))]
mod imp {
    use std::{ffi::CString, os::raw::c_char, path::Path};
    unsafe extern "C" {
        fn dp_image_square(path: *const c_char, edge: u32, pixels: *mut u8) -> i32;
        fn dp_image_thumbnail(path: *const c_char, destination: *const c_char, edge: u32) -> i32;
        fn dp_video_info(
            path: *const c_char,
            width: *mut u32,
            height: *mut u32,
            seconds: *mut f64,
        ) -> i32;
    }
    pub const VIDEO: bool = true;
    fn c(path: &Path) -> Option<CString> {
        CString::new(path.as_os_str().as_encoded_bytes()).ok()
    }
    pub fn square(path: &Path, edge: u32) -> Option<Vec<u8>> {
        let path = c(path)?;
        let mut pixels = vec![0u8; (edge * edge * 4) as usize];
        (unsafe { dp_image_square(path.as_ptr(), edge, pixels.as_mut_ptr()) } == 1)
            .then_some(pixels)
    }
    pub fn thumbnail(path: &Path, destination: &Path, edge: u32) -> bool {
        let (Some(path), Some(destination)) = (c(path), c(destination)) else {
            return false;
        };
        unsafe { dp_image_thumbnail(path.as_ptr(), destination.as_ptr(), edge) == 1 }
    }
    pub fn video_info(path: &Path) -> Option<(u32, u32, f32)> {
        let path = c(path)?;
        let (mut width, mut height, mut seconds) = (0, 0, 0.);
        (unsafe { dp_video_info(path.as_ptr(), &mut width, &mut height, &mut seconds) } == 1)
            .then_some((width, height, seconds as f32))
    }
}
#[cfg(not(all(
    target_os = "macos",
    any(feature = "apple-photos", feature = "native-ui")
)))]
mod imp {
    use std::path::Path;
    pub const VIDEO: bool = false;
    pub fn video_info(_: &Path) -> Option<(u32, u32, f32)> {
        None
    }
    pub fn square(_: &Path, _: u32) -> Option<Vec<u8>> {
        None
    }
    pub fn thumbnail(_: &Path, _: &Path, _: u32) -> bool {
        false
    }
}
/// An edge² BGRA centre-cropped square, preferring the file's embedded preview.
pub fn square(path: &Path, edge: u32) -> Option<Vec<u8>> {
    imp::square(path, edge)
}
/// Writes an orientation-corrected JPEG at most `edge` px on its long side,
/// decoded at reduced scale.
pub fn thumbnail(path: &Path, destination: &Path, edge: u32) -> bool {
    imp::thumbnail(path, destination, edge)
}
/// Whether videos get previews (frames via AVFoundation) on this platform.
pub const VIDEO: bool = imp::VIDEO;
/// Display size (orientation applied) and length in seconds of a video file.
pub fn video_info(path: &Path) -> Option<(u32, u32, f32)> {
    imp::video_info(path)
}
