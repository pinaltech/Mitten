# Verification

## PhotoKit TCC crash fix — October 31, 2026

The first source build used the incorrect `NSPhotosUsageDescription` key. Plist
syntax checks did not catch it; PhotoKit access then caused the user-reported
TCC abort. The required key is `NSPhotoLibraryUsageDescription`.

- The executable and bundle now use the same canonical `native/Info.plist`.
- The bundler runs `--verify-photo-permissions` against both executables, checking
  the nonempty description through `NSBundle.mainBundle` without invoking any
  PhotoKit access or changing permissions. Both checks pass on the rebuilt app.
- All PhotoKit access entry points guard against a missing/blank description
  before authorization APIs. An integration test without an app plist exercises
  metadata, thumbnail, viewer-preview and original-export rejection; configuration
  errors return normally instead of terminating the process.
- All **26 tests** pass, including that regression test. Formatting, strict
  all-target Clippy, the rebuilt macOS bundle and Intel PhotoKit compile check pass.

Live permission approval and photo-library access remain unverified. The earlier
compile and plist syntax checks below did not validate this privacy requirement.

## Photo sources and large-library loading — October 31, 2026

Apple Silicon macOS, Rust 1.96.0. All source and UI tests use isolated temporary
data; no personal photo library or real server account was connected.

- **25 tests pass:** two loading/cache tests, five GPUI interaction/layout tests,
  eight source/protocol tests, and ten existing storage tests. Formatting and
  strict all-target Clippy (`-D warnings`) pass.
- Full native macOS debug compilation and `scripts/bundle-macos.sh` succeed.
  `target/GPUI Photos.app` contains the new binary and Photos permission text.
  Both Info.plists pass `plutil`; the binary links Photos.framework. The small
  PhotoKit bridge compiles with ARC, blocks, and `-Wall -Wextra -Werror`.
- Platform compile checks pass for the **core plus remote adapters** on
  `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-gnu`; the core, adapters and
  PhotoKit bridge also pass on `x86_64-apple-darwin`. Linux/Windows checks used a
  temporary, checksum-verified Zig 0.15.2 C cross-compiler for TLS dependencies.
  Storage-only, remote-only, and local-only native UI feature builds pass too.
- Native visual review of an isolated review bundle checked the four-service
  logo grid, loaded SVG logos, and the Nextcloud form. The review app was closed.
  GPUI tests verify onboarding/forms at 360, 820, and 1440 px widths, with the
  existing grid/viewer layout checks down to 360×360 retained.

### Performance measurements

- A real GPUI virtual-list test holds **1,000,000 synthetic photo records**,
  jumps through 30 scroll positions, fills and evicts both caches, and verifies
  that scrolling makes **zero viewer-preview or original requests**. Opening a
  photo makes exactly one separate preview request and no original request.
  First setup/frame took about **0.66 seconds**. A standalone instrumented run
  completed in **3.88 seconds**, with **400,670,720 bytes (~382 MiB) maximum RSS**
  and zero swaps. These are test-process measurements using synthetic metadata
  and small procedural images, not a production GPU-memory bound.
- Cached scroll/idle checks were about **4–7 ms** in that run. Visits needing new
  thumbnails took about **111–121 ms to drain all test-executor work**, including
  encoding, file synchronization, image loading and redraws. This is an
  unoptimized test harness, not a claim of frame rate or live-server latency.
- The GPUI stress test asserts that thumbnail paths actually reach the **256**
  entry cap and decoded thumbnails reach the **96** entry cap; both remain
  bounded while old entries are evicted. Viewer caches are capped at three.
  A separate test checks four thumbnail jobs plus at most one viewer job,
  automatic page loading near the last row, and retry without a request loop.
- A synthetic paged provider enumerates **1,000,000 metadata entries in 5,000
  bounded pages**, about **0.21 seconds** in the recorded run, with no pixel
  requests until a thumbnail, preview, or original export is explicitly asked
  for. The provider retains only the current generated page in this test.
- `scripts/bench-loading.sh` compiles the production loading module directly
  with `rustc -O`, avoiding native UI build dependencies. Recorded results:

  | Simulated photo/tile visits | Time | Retained thumbnail paths |
  | --- | --- | --- |
  | 100,000 | 65 ms | 256 |
  | 1,000,000 | 584 ms | 256 |
  | 10,000,000 | 6.73 s | 256 |

  This microbenchmark measures virtual-range and cache bookkeeping; it does not
  decode ten million photographs or measure a network service.

### Protocol and behavior coverage

Localhost HTTP fixtures exercise actual `reqwest` and `reqwest_dav` requests:
Immich API-key headers, metadata POST bodies, legacy pages, opaque cursors,
thumbnail versus preview routes, and original export. WebDAV coverage includes
Basic authentication, bounded SEARCH plus SearchDAV offsets, URL-encoded names,
installation subpaths, namespace-aware multistatus parsing, failed propstats,
ETag invalidation, scope validation and the separate Nextcloud preview endpoint.
401/403/404/429/500 and redirect failures never fall back to original downloads.
Tests also cover annotation persistence, recoverable deletion/restoration,
credential-free connection JSON, invalid URLs/folders, duplicate-free settings,
onboarding interactions, pagination failure/retry, and non-overwriting exports.

### Remaining limits

Live PhotoKit permission/TCC, iCloud downloads, OS keyring round trips, and real
Immich/Nextcloud servers were not tested without accounts or permission changes.
Linux/Windows GUI compilation and native execution were not verified; successful
headless adapter checks do not validate their window-system dependencies. Intel
macOS was cross-checked, not run. Code signing/notarization remain outside scope.

Connected sources are read-only, with device-local annotations. Search/filter
counts apply to loaded metadata. Previously loaded metadata remains resident,
and persistent disk thumbnails currently have no automatic quota. Tests cover
million-record rendering and ten-million-visit bookkeeping, not a million-file
filesystem scan or the performance of a particular remote server. Cargo still
reports GPUI's preexisting `block` future-compatibility notice.

The earlier baseline below is retained for context.

## Photos interaction update — October 31, 2026

- Locked offline suite: **13 pass** (10 storage, 3 GPUI interaction/layout).
  Formatting and strict all-target Clippy pass. Native debug compilation and
  macOS app bundling succeed.
- Regression coverage adds blank-area deselection, rectangular selection,
  actual multi-photo drags into albums, typing album spaces with the production
  shortcuts registered, continuous wheel zoom, no panning at fit, and cancellation
  and confirmation before soft deletion. Responsive coverage remains active.
- Native visual review with an isolated fictional library checked selection
  borders without checkmarks, favorite hearts, drag selection, album spaces,
  delete confirmation, the fitted viewer with editable percentage, zoom buttons
  and mouse panning, hidden library controls in the viewer, and settings.
- AppKit file dragging was observed to create a native drag session, and an
  actual native drop into an album persisted membership. Copying files all the
  way into Finder was attempted but not verified. Physical trackpad pinch and
  non-macOS native behavior remain unverified. No browser was used. Desktop
  interaction stopped at the user's request; final validation used the terminal.

The earlier baseline below is retained for context.


October 31, 2026, Apple Silicon macOS, Rust 1.96.0.

- Formatting passes.
- Locked offline tests: **12 pass** (10 storage, 2 real GPUI interaction/layout).
- Strict all-target Clippy passes with `-D warnings`.
- Native debug build and `scripts/bundle-macos.sh` succeed. The launchable app is
  `target/GPUI Photos.app`.
- DesertUI's 3 existing tests pass; Picacho and the Ironwood workspace also
  compile against the updated crate.

Storage tests cover recursive discovery, cache exclusion, dimensions, thumbnail
and bounded-preview generation, cache invalidation, import copies, SHA-256
deduplication, unsupported files, album/favorite/caption persistence, recoverable
deletion without original mutation, non-overwriting export, damaged metadata,
path traversal, symlinks, source settings, and concurrent metadata writers.

GPUI interaction tests exercise real tile clicks, viewer opening and next/back
controls, selection ranges and toggle selection, settings, and onboarding.
Bounds are checked at 1440×900, 1060×700, 960×600, 820×500, 640×480, 480×420, and
360×360. First-launch onboarding is also rendered at 360×540.

Native visual review used a separately identified review bundle and isolated
procedural image fixtures under `/private/tmp/desert-library-review`. Verified
grid rendering, selected-state controls, automatic discovery of changed files,
thumbnail invalidation, and opening the fitted viewer with Space. No personal
photos or Apple Photos database was opened.

Limits: Linux/Windows native runs, native external drag-and-drop, very large
libraries, codec-specific EXIF orientation fixtures, code signing, notarization,
and store distribution were not verified. GPUI external file drops and album
drag targets are implemented; the corresponding storage operations are tested
separately. Unsupported-format and library-identity limits are in README.md.
Cargo reports GPUI's existing `block` future-compatibility notice.
