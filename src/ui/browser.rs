//! Mitten in a web page: what the page does for it outside the canvas
//! (web/shell/glue.js), and what the page tells it. Main thread only.
//!
//! - The site's settings (`host.js`): see [`super::host`].
//! - Import and Export: the site's file picker and downloads.
//! - Copy: one picture on the clipboard, as PNG.
//! - Dragging photos out: the photo under the pointer is reported, and the
//!   page lays a draggable stand-in over it (a browser drag can't start
//!   from a canvas).
use super::host::Host;
use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_name = mittenHostConfig)]
    fn host_config() -> Result<JsValue, JsValue>;
    /// The window has drawn: the page takes its loading screen away.
    #[wasm_bindgen(catch, js_name = mittenReady)]
    fn page_ready() -> Result<(), JsValue>;
    #[wasm_bindgen(catch, js_name = mittenTheme)]
    fn page_theme(dark: bool, accent: u32) -> Result<(), JsValue>;
    /// Opens the site's file picker; the site uploads the choice.
    #[wasm_bindgen(catch, js_name = mittenUpload)]
    fn page_upload() -> Result<(), JsValue>;
    /// Downloads these photos' originals.
    #[wasm_bindgen(catch, js_name = mittenDownload)]
    fn page_download(ids: js_sys::Array) -> Result<(), JsValue>;
    #[wasm_bindgen(catch, js_name = mittenNavigate)]
    fn page_navigate(to: &str) -> Result<(), JsValue>;
    /// Starts copying the picture at this URL; [`copy_finished`] follows.
    #[wasm_bindgen(catch, js_name = mittenCopy)]
    fn page_copy(url: &str) -> Result<bool, JsValue>;
    #[wasm_bindgen(catch, js_name = mittenDragTarget)]
    fn page_drag_target(
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        photos: &str,
        preview: &str,
    ) -> Result<(), JsValue>;
    #[wasm_bindgen(catch, js_name = mittenDragTarget)]
    fn page_no_drag_target() -> Result<(), JsValue>;
    /// What Mitten panicked with, just before the module traps.
    #[wasm_bindgen(catch, js_name = mittenPanic)]
    fn page_panic(message: &str) -> Result<(), JsValue>;
}

/// Panics abort (the module traps with "unreachable"), so say what went
/// wrong first: in the console, and to the loading screen, which explains it.
/// (GPUI's own hook, which logs the panic to the console, runs after.)
pub fn report_panics() {
    let console = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = page_panic(&info.to_string());
        console(info);
    }));
}

fn field(object: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(object, &JsValue::from_str(name)).unwrap_or(JsValue::UNDEFINED)
}

pub(super) fn host() -> Host {
    let mut host = Host::default();
    let Ok(config) = host_config() else {
        return host;
    };
    if let Some(name) = field(&config, "brandName").as_string() {
        host.brand_name = name.into();
    }
    if let Some(icon) = field(&config, "brandIcon").as_string() {
        host.brand_icon = icon.into();
    }
    if let Some(libraries) = field(&config, "libraries").as_bool() {
        host.libraries = libraries;
    }
    if let Some(height) = field(&config, "topbarHeight").as_f64().filter(|h| (28. ..=80.).contains(h)) {
        host.topbar_height = height as f32;
    }
    if let Some(inset) = field(&config, "topbarInset").as_f64() {
        INSET.store(inset.clamp(0., 600.) as u32, Ordering::Relaxed);
    }
    host.sign_in = field(&config, "signIn").as_string().filter(|s| !s.is_empty());
    host.sign_out = field(&config, "signOut").as_string().filter(|s| !s.is_empty());
    host
}

/// The page runs on a Mac (or iPad): shortcuts use ⌘ rather than Ctrl.
pub fn is_mac() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let navigator = field(&window.into(), "navigator");
    let platform = field(&navigator, "platform").as_string().unwrap_or_default();
    let agent = field(&navigator, "userAgent").as_string().unwrap_or_default();
    platform.starts_with("Mac") || platform.starts_with("iP") || agent.contains("Mac OS X")
}

pub fn ready() {
    let _ = page_ready();
}
pub fn theme(dark: bool, accent: u32) {
    let _ = page_theme(dark, accent);
}
pub fn upload() {
    let _ = page_upload();
}
pub fn download(ids: js_sys::Array) {
    let _ = page_download(ids);
}
pub fn navigate(to: &str) {
    let _ = page_navigate(to);
}
pub fn copy(url: &str) -> bool {
    page_copy(url).unwrap_or(false)
}

// ── What the page tells Mitten ──────────────────────────────────────────────
// Read by Mitten's page loop (a few times a second), which redraws.

/// The latest upload progress (done, total, failed), until Mitten reads it.
pub static UPLOADS: Mutex<Option<(u32, u32, u32)>> = Mutex::new(None);
/// How the latest copy went, until Mitten reads it.
pub static COPIED: Mutex<Option<bool>> = Mutex::new(None);
static INSET: AtomicU32 = AtomicU32::new(0);
/// Something the page told Mitten needs a redraw.
pub static CHANGED: AtomicBool = AtomicBool::new(false);

/// Called by the site's upload script as each file finishes; done == total ends a batch.
#[wasm_bindgen]
pub fn upload_progress(done: u32, total: u32, failed: u32) {
    *UPLOADS.lock().unwrap() = Some((done, total, failed));
}

/// Room for the site's own controls at the topbar's trailing edge, in CSS pixels.
#[wasm_bindgen]
pub fn set_topbar_inset(px: u32) {
    if INSET.swap(px.min(600), Ordering::Relaxed) != px.min(600) {
        CHANGED.store(true, Ordering::Relaxed);
    }
}
pub(super) fn topbar_inset() -> f32 {
    INSET.load(Ordering::Relaxed) as f32
}

#[wasm_bindgen]
pub fn copy_finished(ok: bool) {
    *COPIED.lock().unwrap() = Some(ok);
}

// ── Drag-out target ────────────────────────────────────────────────────────

#[derive(serde::Serialize, PartialEq, Clone)]
pub struct DragPhoto {
    /// What a drop receives: the picture shown here.
    pub url: String,
    /// The original, downloaded where browsers support dropping files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(PartialEq, Clone)]
pub struct DragTarget {
    pub bounds: (f32, f32, f32, f32),
    pub photos: Vec<DragPhoto>,
    pub preview: String,
}

thread_local! {
    static LAST: RefCell<Option<DragTarget>> = const { RefCell::new(None) };
}

/// Tells the page what can be dragged out from under the pointer, when that changed.
pub fn drag_target(target: Option<DragTarget>) {
    LAST.with(|last| {
        let mut last = last.borrow_mut();
        if *last == target {
            return;
        }
        match &target {
            Some(t) => {
                let photos = serde_json::to_string(&t.photos).unwrap_or_default();
                let (x, y, w, h) = t.bounds;
                let _ = page_drag_target(x, y, w, h, &photos, &t.preview);
            }
            None => {
                let _ = page_no_drag_target();
            }
        }
        *last = target;
    });
}
