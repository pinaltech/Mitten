# wasm_thread 0.3.3, patched for Mitten

GPUI's web platform runs background work with `wasm_thread`, which calls
`eval` to reach the global scope, find its JS shim's URL and load a module
worker polyfill. photos.pinal.tech's content security policy refuses `eval`,
so this copy uses `js_sys::global()`, reads the shim's URL from an error's
stack in Rust, and leaves out the polyfill. Used through `[patch.crates-io]`
in Mitten's Cargo.toml.
