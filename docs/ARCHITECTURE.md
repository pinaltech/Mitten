# Architecture

## Storage and driver boundary

`src/storage.rs` exposes a UI-independent `PhotosDriver`. `src/sources/` provides
the source registry, platform availability, paged metadata, and separate image
and original access. `ConnectedPhotos` stores private annotations independently
of each source. PhotoKit is isolated in a small ARC Objective-C bridge compiled
by `build.rs` only for macOS targets. The bridge fetches a lazy `PHFetchResult`
and enumerates at most 200 assets per page. Both the app bundle and command-line
executable share the canonical `native/Info.plist` with
`NSPhotoLibraryUsageDescription`. The bundler validates the usage description
through Foundation in both executables without requesting photo access.
PhotoKit entry points reject missing descriptions before calling privacy APIs.

Cargo features separate `native-ui`, `remote`, and `apple-photos`; all are enabled
by default, with PhotoKit additionally gated by target OS. For example:

```sh
# Portable storage core without GPUI or platform APIs:
cargo check --locked --no-default-features --lib
# Network adapters without the native window system:
cargo check --locked --no-default-features --features remote --lib
# Host app, or adapters for an installed Rust target:
scripts/check-platform.sh
scripts/check-platform.sh x86_64-unknown-linux-gnu
```

Cross-compiling requires the target standard library and any target C compiler
needed by TLS dependencies. Use native Windows/Linux toolchains for their GUI
builds; a successful core/adapter check does not verify the target window system.
On macOS, Xcode command-line tools provide clang and the Photos SDK.

`.mitten/library.json` holds favorites, captions, album memberships, and
Recently Deleted state. Originals are never rewritten. Metadata writes are
atomic and serialized by an OS file lock. The same lock serializes imports.
Symlinked directories, symlinked files, and hidden paths are excluded from scans.
Externally added and changed images are discovered every five seconds.

Connected sources fetch metadata in pages of 200. The background catalog worker
combines up to 16 pages, yielding progress after at most a 100 ms batch (a single
slow server request can take longer). One catalog request runs at a time. Errors
preserve the last committed cursor and expose Retry without a request loop.
Unfiltered browsing uses a zero-copy range over the catalog; filtered views share
indices. Zoom transitions lay each level's on-screen photos into one shared,
centred slot lattice (or scale a date grid whole) and crossfade between
levels on a fixed clock, so the work per frame is bounded by what is on
screen. See
[the transition design and regression notes](GALLERY_TRANSITIONS.md). Selection drag payloads are prepared only
when a drag starts.

Only visible tiles request thumbnails (up to 512 px by default). Opening a photo
requests a separate viewer preview (up to 2560 px by default). Settings →
Behavior → Thumbnails chooses these sizes, and the Months and Years squares,
automatically by default: once per library, Photos times a few photos at each
size and keeps the sharpest that stays within ~140 ms per thumbnail (~700 ms
per viewer preview), so the largest zoom fills quickly. Choose sizes by hand
there, or Measure Again; originals are streamed only on explicit
export. Up to four thumbnail jobs and one viewer job may run at once. Path caches
retain at least a 256-thumbnail working-set budget and up to three viewer
previews. Decoded thumbnails have a base budget of 96, growing with visible
rows and active transition slots so dense grids cannot evict their own images.
Both thumbnail budgets shrink with the viewport and remain independent of
catalog size; the viewer's decoded cache stays capped at three images. Small images
are never upscaled during cache generation. Cache files persist on disk under
`.mitten/cache/` (source data folders for connected providers). There is
currently no automatic disk-cache quota. The completed metadata catalog remains
resident in memory, so metadata use grows with library size; decoded image
caches remain bounded independently of catalog size.

File fingerprints invalidate changed images. Remote response sizes and decode
allocations are limited; oversized or corrupt data displays a placeholder.
PhotoKit can download an iCloud representation when needed. The viewer remains
a fitted preview rather than a full-resolution editing surface.

Settings live in the platform application-data directory for
`tech.pinal.mitten`; set `MITTEN_DATA_DIR` to isolate another profile.
Corrupt library metadata is preserved and reported. A replaced damaged settings
file is backed up first. External file moves/renames currently change a photo's
identity; album membership and captions do not follow such a rename.

Local formats: **JPEG, PNG, GIF, WebP, TIFF, BMP**, plus **MOV, MP4, M4V** videos
on macOS (frames and durations via AVFoundation). GIFs display their first frame.
PhotoKit and server-generated previews can expose additional image formats such
as HEIC without a local decoder. Local HEIC/RAW decoding,
photo adjustments, album renaming/deletion, and cloud sync are not implemented.
Last 30 Days uses local modification time or the connected asset's capture time
where supplied. Apple Photos is accessed through PhotoKit; the app does not read
or modify the `.photoslibrary` database directly.

## Verify

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
scripts/bench-loading.sh
```

See [VERIFICATION.md](VERIFICATION.md) for checks actually performed.

API references: [Immich OpenAPI](https://github.com/immich-app/immich/blob/main/open-api/immich-openapi-specs.json),
[Nextcloud WebDAV](https://docs.nextcloud.com/server/stable/developer_manual/client_apis/WebDAV/),
[Nextcloud Photos paging](https://github.com/nextcloud/photos/blob/master/src/services/PhotoSearch.ts),
[PhotoKit image requests](https://developer.apple.com/documentation/photos/phimagemanager).
