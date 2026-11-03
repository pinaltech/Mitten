//! The viewer's native video player (see `native/video.m`). GPUI cannot decode
//! video, so an AVPlayerView is laid over the fitted frame on macOS.
use super::*;

#[cfg(all(target_os = "macos", not(test)))]
unsafe extern "C" {
    fn dp_video_sync(
        view: *mut std::ffi::c_void,
        kind: i32,
        identifier: *const std::ffi::c_char,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        visible: i32,
    );
}

/// Shows `target` in the player (paused and hidden unless `visible`), or
/// releases the player when there is nothing to play.
pub(super) fn sync(
    w: &Window,
    target: Option<(&storage::VideoSource, Bounds<Pixels>)>,
    visible: bool,
) {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = HasWindowHandle::window_handle(w) else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        let (kind, identifier) = match target.map(|(s, _)| s) {
            Some(storage::VideoSource::File(path)) => (0, path.as_os_str().as_encoded_bytes()),
            Some(storage::VideoSource::PhotoKit(id)) => (1, id.as_bytes()),
            None => (0, &[][..]),
        };
        let Ok(identifier) = std::ffi::CString::new(identifier) else {
            return;
        };
        let frame = target.map(|(_, b)| b).unwrap_or_default();
        unsafe {
            dp_video_sync(
                handle.ns_view.as_ptr(),
                kind,
                identifier.as_ptr(),
                f32::from(frame.origin.x).into(),
                f32::from(frame.origin.y).into(),
                f32::from(frame.size.width).into(),
                f32::from(frame.size.height).into(),
                i32::from(visible),
            );
        }
    }
    #[cfg(any(not(target_os = "macos"), test))]
    let _ = (w, target, visible);
}
