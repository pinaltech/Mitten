//! Browser plumbing for Mitten's wasm build.
//!
//! Mitten's sources do blocking I/O off the UI thread. In the browser GPUI
//! runs that work in Web Workers, where a synchronous `XMLHttpRequest` is
//! allowed, so sources keep their blocking shape: [`get`] and [`send`] must
//! not be called on the page's main thread (the browser refuses binary
//! synchronous requests there). Requests carry the page's own sign-in cookie.
use anyhow::{Context, Result, anyhow, bail};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The photo service ended the session (it answered 401). Requests run in
/// workers, which can't move the page; the UI sends it to sign in again.
pub static SIGNED_OUT: AtomicBool = AtomicBool::new(false);

pub fn take_signed_out() -> bool {
    SIGNED_OUT.swap(false, Ordering::Relaxed)
}
use wasm_bindgen::JsCast;

static ORIGIN: OnceLock<String> = OnceLock::new();

/// The page's origin (`https://photos.example`), recorded at start-up on
/// the main thread: workers spawned from blob URLs cannot be relied on for it.
pub fn origin() -> &'static str {
    ORIGIN.get_or_init(|| {
        web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_default()
    })
}

fn js(error: wasm_bindgen::JsValue) -> anyhow::Error {
    anyhow!("{}", error.as_string().unwrap_or_else(|| format!("{error:?}")))
}

/// A same-origin request, answered with its status and body.
pub fn send(method: &str, path: &str) -> Result<(u16, Vec<u8>)> {
    let xhr = web_sys::XmlHttpRequest::new().map_err(js)?;
    xhr.open_with_async(method, &format!("{}{path}", origin()), false)
        .map_err(js)?;
    xhr.set_response_type(web_sys::XmlHttpRequestResponseType::Arraybuffer);
    xhr.send()
        .map_err(js)
        .context("Could not reach the photo library. Check your connection.")?;
    let status = xhr.status().map_err(js)?;
    let body = xhr.response().map_err(js)?;
    let bytes = body
        .dyn_into::<js_sys::ArrayBuffer>()
        .map(|b| js_sys::Uint8Array::new(&b).to_vec())
        .unwrap_or_default();
    if status == 401 {
        // The session ended (or was signed out elsewhere): sign in again.
        SIGNED_OUT.store(true, Ordering::Relaxed);
        bail!("Signed out");
    }
    Ok((status, bytes))
}

/// `GET path`, failing on anything but success.
pub fn get(path: &str, limit: usize) -> Result<Vec<u8>> {
    let (status, body) = send("GET", path)?;
    if !(200..300).contains(&status) {
        bail!("The photo library answered {status}");
    }
    if body.len() > limit {
        bail!("The photo library sent more than expected");
    }
    Ok(body)
}

/// Small text the app keeps in this browser (settings): `localStorage`,
/// main thread only. Missing or blocked storage reads as empty.
pub fn load(key: &str) -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()??
        .get_item(&format!("mitten:{key}"))
        .ok()?
}
pub fn store(key: &str, value: &str) -> Result<()> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .context("This browser is not keeping Mitten's settings")?;
    storage
        .set_item(&format!("mitten:{key}"), value)
        .map_err(js)
        .context("This browser is not keeping Mitten's settings")
}
