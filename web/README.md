# Mitten in the browser

Mitten itself, built for the browser: the same app as on the desktop (the
`mitten` binary, compiled to wasm), drawn by GPUI's web platform on WebGPU
(WebGL 2 where WebGPU is missing). A site serves it with a photo API at its
own origin; [Pinal Photos](https://photos.pinal.tech) is one such site.

```
web/
  build.sh   builds Mitten for wasm32 into OUT/mitten (default OUT: web/dist)
  shell/     the page around the canvas
    index.html, host.js   Mitten's own page and default site settings
    boot.js    loading screen: progress, start-up errors, supported browsers
    glue.js    what Mitten does in the page: copy, drag-out, Safari pinch, theme
    shell.css  the loading screen's look
    icon.png, browsers/   the app icon and the supported browsers' marks
```

In the browser Mitten keeps its whole interface and swaps out what needs a
computer of its own:

- **Library:** `src/sources/web.rs`, the site's photo API read through the
  page's session. Mitten's blocking source work runs on GPUI's Web Workers
  (shared memory, so the page is cross-origin isolated), where synchronous
  requests are allowed (`src/web.rs`). Thumbnails and previews are URLs that
  GPUI fetches and decodes itself.
- **Settings:** the browser's `localStorage` instead of a file.
- **Import / Export:** the site's file picker and downloads (`host.upload`,
  `host.download`).
- **Copy:** one picture, as PNG (all a web page can put on the clipboard).
- **Drag out:** a browser drag can't start from a canvas, so while the
  pointer is over a photo Mitten reports where it is (`mittenDragTarget`)
  and `glue.js` lays a transparent, draggable stand-in over it. Pointer and
  wheel events on the stand-in are passed to the canvas, so the gallery works
  as before; a drag carries the photo's link (and, in Chromium, a download of
  the original). Dragging photos onto an album in Mitten's sidebar is not
  available in the browser.
- **Gestures:** browsers report wheels without gesture phases. A trackpad
  pinch is a Ctrl-wheel run (Safari's gesture events are turned into the same
  wheels), and pinching out at fit closes the viewer; a swipe lands on the
  next photo once it is decisive, and its momentum moves no further.
- **Not in the browser:** local folders, Apple Photos, Immich and Nextcloud.
- `patches/wasm_thread`: GPUI's worker threads without `eval`, which a
  strict content security policy refuses.

## Build

```sh
rustup toolchain install nightly --component rust-src --target wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked   # must match ../Cargo.lock
web/build.sh            # or: web/build.sh path/to/site/public
```

The threaded build needs nightly Rust to rebuild the standard library with
atomics. The page must be served with
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp`.

## Embedding Mitten in a site

`build.sh OUT` writes `OUT/mitten/` and, only when `OUT/index.html` doesn't
exist yet, Mitten's own page and `host.js`. A site keeps its own page: it
loads `host.js`, then `mitten/glue.js`, then `mitten/boot.js` as a module,
with the loading screen's markup from `shell/index.html`.

`host.js` sets `window.mittenHost`:

| Field | Default | |
| --- | --- | --- |
| `name` | `Mitten` | product name in the loading screen's messages |
| `brand` | `{ name: 'Mitten', icon: 'brand/mitten.svg' }` | topbar logo (a Mitten asset) and name |
| `libraries` | `true` | library switcher and library settings; off for a site's single library |
| `topbarHeight` | `38` | topbar height in pixels |
| `help` | `{}` | `{ browser, start }`: help links for unsupported browsers and other start-up failures |
| `signIn`, `signOut` | `null` | where to go when the API answers 401, and to sign out |
| `upload()`, `download(ids)` | none | Import and Export |

The site can draw its own controls over the topbar's trailing end and keep
room for them with `mittenSetTopbarInset(px)`; `mitten-theme` events (and
`data-theme` on `<html>`) follow Mitten's appearance.

### Photo API

`src/sources/web.rs` reads the page's origin:

| | |
|---|---|
| `GET /api/photos?cursor=&limit=` | `{ photos: [{ id, name, type, taken, width, height, size }], cursor }`, newest first |
| `GET /api/photos/:id` | the original (`?download=1`: as an attachment) |
| `GET /api/photos/:id/thumb` | a JPEG of at most 512 px |
