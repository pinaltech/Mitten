use std::{
    io,
    num::NonZeroUsize,
    sync::{LockResult, Mutex, MutexGuard, TryLockError},
};

use wasm_bindgen::prelude::*;
use web_sys::{Blob, Url, WorkerGlobalScope};

pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    if let Some(window) = web_sys::window() {
        return Ok(NonZeroUsize::new(window.navigator().hardware_concurrency() as usize).unwrap());
    }

    if let Ok(worker) = js_sys::global().dyn_into::<WorkerGlobalScope>() {
        return Ok(NonZeroUsize::new(worker.navigator().hardware_concurrency() as usize).unwrap());
    }

    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "hardware_concurrency unsupported",
    ))
}

pub fn is_web_worker_thread() -> bool {
    js_sys::global().dyn_into::<WorkerGlobalScope>().is_ok()
}

/// Extracts path of the `wasm_bindgen` generated .js shim script.
///
/// Internally, this intentionally generates a javascript exception to obtain a stacktrace containing the current script
/// URL.
pub fn get_wasm_bindgen_shim_script_path() -> String {
    // Patched for Mitten: no `eval` (photos.pinal.tech's content security
    // policy refuses it). The JS glue that builds this error is the shim, so
    // its stack's first script URL is the shim's, as script_path.js found it.
    let error = js_sys::Error::new("");
    let stack = js_sys::Reflect::get(&error, &"stack".into())
        .ok()
        .and_then(|s| s.as_string())
        .unwrap_or_default();
    for part in stack.split(|c: char| c == '(' || c == '@' || c.is_whitespace()) {
        let part = part.trim_end_matches(')');
        if !(part.starts_with("http://") || part.starts_with("https://")) {
            continue;
        }
        // Drop ":line:col".
        let mut url = part;
        for _ in 0..2 {
            if let Some((rest, tail)) = url.rsplit_once(':') {
                if tail.bytes().all(|b| b.is_ascii_digit()) {
                    url = rest;
                }
            }
        }
        if url.ends_with(".js") {
            return url.to_owned();
        }
    }
    panic!("wasm_thread: cannot find the wasm-bindgen shim's URL")
}

/// Generates worker entry script as URL encoded blob
pub fn get_worker_script(wasm_bindgen_shim_url: Option<String>) -> String {
    // Cache URL so that subsequent calls are less expensive
    static CACHED_URL: Mutex<Option<String>> = Mutex::new(None);

    if let Some(url) = CACHED_URL.lock_spin().unwrap().clone() {
        return url;
    }

    // If wasm bindgen shim url is not provided, try to obtain one automatically
    let wasm_bindgen_shim_url = wasm_bindgen_shim_url.unwrap_or_else(get_wasm_bindgen_shim_script_path);

    // Generate script from template
    #[cfg(feature = "es_modules")]
    let template = include_str!("js/web_worker_module.js");
    #[cfg(not(feature = "es_modules"))]
    let template = include_str!("js/web_worker.js");

    let script = template.replace("WASM_BINDGEN_SHIM_URL", &wasm_bindgen_shim_url);

    // Create url encoded blob
    let arr = js_sys::Array::new();
    arr.set(0, JsValue::from_str(&script));
    let blob = Blob::new_with_str_sequence(&arr).unwrap();
    let url = Url::create_object_url_with_blob(
        &blob
            .slice_with_f64_and_f64_and_content_type(0.0, blob.size(), "text/javascript")
            .unwrap(),
    )
    .unwrap();

    *CACHED_URL.lock_spin().unwrap() = Some(url.clone());

    url
}

/// A spin lock mutex extension.
///
/// Atomic wait panics in wasm main thread so we can't use `Mutex::lock()`.
/// This is a helper, which implement spinlock by calling `Mutex::try_lock()` in a loop.
/// Care must be taken not to introduce deadlocks when using this trait.
pub trait SpinLockMutex {
    type Inner;

    fn lock_spin<'a>(&'a self) -> LockResult<MutexGuard<'a, Self::Inner>>;
}

impl<T> SpinLockMutex for Mutex<T> {
    type Inner = T;

    fn lock_spin<'a>(&'a self) -> LockResult<MutexGuard<'a, Self::Inner>> {
        loop {
            match self.try_lock() {
                Ok(guard) => break Ok(guard),
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Poisoned(e)) => break Err(e),
            }
        }
    }
}
