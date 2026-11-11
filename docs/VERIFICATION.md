# Verification

## Linux overlays, stuck cataloging and slow-server loading — November 5, 2026

- **71 tests pass** (five new: ThumbHash/base64 decoding, the request gate,
  a catalog that keeps paging when image sizes change mid-page, marquee and
  closing photo in an inset window, placeholders standing in for failed
  images). The two UI regressions were confirmed to fail on the previous
  behaviour (the marquee was off by exactly the 13 px Linux inset; the
  catalog stayed `loading` forever). Formatting passes; strict Clippy passes
  except three lints already present in untouched code.
- Release build succeeds and launched cleanly on Wayland with an isolated
  profile. Real Immich browsing speed, trackpad feel and the visual result on
  a fractional-scale display were **not** measured here.
- Root causes: overlays in window coordinates were drawn inside a root inset
  by gpui-component's 12 px client shadow plus a 1 px border; finishing
  automatic sizing bumped the catalog generation, orphaning the page in
  flight; painted grids and the viewer had no fallback while squares or
  previews loaded; remote I/O blocked GPUI's executor threads, fetched
  squares serially in batches of 32 and re-measured sizes that map to the
  same Immich rendition (then dropped every loaded image).

## Gallery zoom reflow and cursor — November 1, 2026

- Zoom transitions no longer draw a layout below its natural size, so the grid
  always fills the width (no empty side strips, no edge fade-ins). Layouts
  switch near the next size; short moves glide, far wraps crossfade. Each
  layout scales about the zoomed photo, which holds still between reflows and
  ends in the pointer's row. Tiles use the arrow cursor at every level.
- Covered by the gallery switching/detent test and a GPUI test zooming in and
  out across gutter, gapless and month levels (the spot under the pointer stays
  put, every layer is drawn at ≥ natural size and spans the width, the photo
  ends in the pointer's row). Visual smoothness on a trackpad still needs a
  person at the machine.

## Viewer transitions, videos, album sync and library switcher — November 1, 2026

- **61 tests pass** (four new: Apple-style album sync, zoom anchoring, tile
  open/close flights, Videos tab and switcher). Formatting and strict
  all-target Clippy pass. Portable core, remote-only and PhotoKit-free native
  UI feature sets check. The macOS debug bundle builds, links AVKit,
  AVFoundation and CoreMedia, and launched cleanly with an empty isolated profile.
- A generated 3-second 640×360 MP4 in a local folder catalogued with its size
  and duration, produced a 512 px frame thumbnail and a grid square, and
  resolved to a playable file. Native playback, Apple Photos videos and album
  sync against a real Photos library, and trackpad feel were **not** exercised
  here: they need a person at the machine.
- Swipe performance changes (neighbour preview prefetch/decode, one idle
  watcher per gesture, clock-timed slides, two previews in flight) are covered
  for state, not measured as frame times on a device.

## Original drag export and minimum viewer zoom — October 31, 2026

- **46 tests pass:** six geometry/cache tests, fourteen GPUI interaction tests,
  two native drag tests, three portable export tests, one privacy-packaging test,
  nine source/protocol tests and eleven storage tests. Formatting and strict
  all-target Clippy pass. The macOS debug bundle builds, and both embedded and
  bundled privacy-description checks pass.
- Linux/Windows x86-64 core and remote-adapter checks pass. Intel macOS core,
  remote adapters and PhotoKit cross-check successfully; the Intel Objective-C
  drag bridge compiles with warnings treated as errors. The macOS native UI also
  checks with PhotoKit and remote features disabled. These are not native
  Linux/Windows GUI builds or Intel execution tests.
- Fixed native export from both gallery and fitted viewer. Connected photos
  supply deferred original-file promises instead of empty local paths. Local
  files additionally advertise the standard file URL. In-app album drags remain
  covered by the existing interaction test.
- Reproduced a native fulfillment stall after Finder accepted the drop: the
  process sample showed AppKit holding a coordinated write claim while our
  delegate waited for another claim. The delegate now writes directly. A native
  regression runs the delegate under an outer coordinated write and verifies
  exact original bytes. It validates a private pasteboard, not a synthesized
  end-to-end receiver operation. The user then quit/reopened the rebuilt bundle
  and confirmed that a real Finder drop creates the file. Automated cross-app
  release remains unreliable; live PhotoKit/Immich/Nextcloud drag drops were not
  exercised. See [DRAG_EXPORT.md](DRAG_EXPORT.md).
- Zoom is constrained to 100–800%. Zooming/pinching out from fit closes the viewer;
  downward scrolling from fit also closes it. Typed percentages below 100 clamp
  to fit. Tests cover stopping at fit when zooming down from above it, closing
  from fit, selected-tile visibility and suppression of trailing gallery zoom.
  Viewer dragging exports at fit and pans when zoomed. Native review confirmed
  downward-scroll dismissal and revealing the selected tile using only generated
  JPEGs and an isolated profile. Physical pinch feel remains unverified.
- Existing million-photo and dense-grid loading regressions pass. The synthetic
  million-record catalog plus initial thumbnails completed in about 3.0 seconds;
  requested originals remain zero during cataloging and scrolling. These tests
  do not measure a live remote server or production frame rates.

## Catalog stability, interactive zoom and disappearing thumbnails — October 31, 2026

- **38 tests pass:** six geometry/cache tests, twelve GPUI tests, one PhotoKit
  packaging regression, eight source/protocol tests and eleven storage tests.
  Formatting, strict Clippy, macOS compilation/bundling and both privacy-description
  checks pass. Linux/Windows x86-64 core/adapters and Intel macOS core/adapters/
  PhotoKit compile checks pass too; these are not native Linux/Windows GUI builds.
- Reproduced the disappearing-grid bug with a 640 × 1200 dense viewport:
  **0 of 240 fully visible thumbnails remained ready** with the fixed 96-entry
  decoded cache. The same regression now retains all 240. It checks cold scroll
  ranges, zooming, scrolling during a pinch, reversing a settle, returning to the
  dense level, and scrolling again. No preview or original is requested.
  Thumbnail path/decode budgets now follow visible rows and active transition
  slots, with the earlier 256/96 limits as base budgets. Viewer caches stay at
  three. Cache size depends on viewport work, never total catalog size.
- Bottom-up rows are packed from the newest end. A partial oldest row absorbs
  changes in metadata count, keeping the latest photos in the same columns and
  at the same distance from the bottom. Short libraries also align to the bottom.
  Real GPUI tests compare newest-photo bounds before/after final-page completion
  for 3→6, 5→8 and 200→203 photos, including catalog completion during a pinch.
  There is no layout-mode change when the known count becomes final.
- Replaced the static crossfade with moving grid-slot frames and independently
  dissolving contents. The anchor slot keeps its photo, gesture progress drives
  geometry immediately, and a 224 ms settle completes/cancels the level change.
  Reversing a settle resumes its current geometry. Scroll, resize and view changes
  cancel obsolete geometry safely; the underlying grid is always live.
- The virtual list now supplies the only visible row range; independently sampled
  offset-based tile culling was removed. Measurements still make no image requests.
  Transition work is bounded by source/destination viewport slots even for a
  million-record catalog. Existing million-photo catalog and scroll tests pass.
- Native review used the generated-JPEG library and isolated profile. Confirmed
  the full latest row, loaded thumbnails after density changes, bottom-aligned
  short grids, floating dates and the scrollbar. The isolated review process was
  stopped after verification. Physical pinch feel and exact
  equivalence to Apple's Photos remain unverified. Research, design reasoning
  and the distinction from Apple's private implementation are documented in
  [GALLERY_TRANSITIONS.md](GALLERY_TRANSITIONS.md).
- One full run exposed an intermittent truncated localhost HTTP response in the
  test fixture. Accepted fixture connections now explicitly use blocking I/O,
  and write failures are asserted. All eight source/protocol tests then passed
  **ten consecutive runs**. Production networking behavior was not changed.

Earlier fixed-cache limits and static-fade descriptions below are historical and
superseded by this revision. Live provider accounts, real TCC approval and native
Linux/Windows GUI execution retain their earlier verification limits.

## Gallery direction and viewer revision — October 31, 2026

- **34 tests pass:** four geometry/cache tests, ten GPUI interaction tests,
  one PhotoKit packaging regression, eight source tests and eleven storage tests.
  Strict all-target Clippy, formatting, the native macOS build/bundle and both
  usage-description checks pass. Core/remote compilation also passes for Linux
  and Windows x86-64; Intel macOS core/remote/PhotoKit compilation passes.
- Gallery levels now snap instead of interpolating photo positions. Existing
  decoded thumbnails briefly crossfade over the new layout. The two smallest
  levels are gapless squares with floating month/year; no level has filename
  captions. Favorite hearts use proportional sizing, a contrasting outline,
  matching selected fill and no background. Embedded image resources are
  explicit, avoiding URI inference for single-component asset names.
- Newest-at-bottom is the default and migrates automatically for old settings.
  Behavior includes a persisted direction switch. GPUI tests cover initial
  bottom placement, older metadata arriving above the view, staying at the
  bottom, preserving a photo while browsing history, changing direction,
  resizing, density changes, incomplete rows, new local arrivals, empty filters
  and short libraries. Row heights match the virtual list's pixel rounding to
  avoid accumulated scroll drift in long libraries.
- Viewer tests exercise horizontal gesture movement, animated settle, momentum
  suppression, cancellation and end boundaries. They verify pan eligibility,
  centered header/footer metadata, and that the image canvas occupies all space
  between the toolbars. Existing decode-ready thumbnail crossfades remain tested.
- The million-record catalog test completes 5,000 synthetic metadata pages in
  about **2.8 seconds**, fetching fewer than 100 thumbnails and zero viewer
  previews/originals. Thumbnail requests now start during visible-tile prepaint,
  so virtual-list measurement does not fetch off-screen rows. The million-record
  scroll test still fills and evicts the 256-path/96-decoded-thumbnail caches;
  cached test-frame checks were about **4–7 ms**, including test-executor work.
- The optimized bookkeeping benchmark completed 100,000 visits in **71 ms**,
  1,000,000 in **591 ms**, and 10,000,000 in **7.22 s**, retaining 256 paths and
  requesting zero previews/originals. These are synthetic tests, not live-server
  throughput or production FPS measurements.
- Native review used the isolated generated-JPEG library/profile. Confirmed
  margin direction, floating date labels, caption-free thumbnails, bottom startup,
  the right-hand scrollbar, proportional outline/filled favorite hearts and direct
  favorite toggling, viewer loading placement, centered precise timestamp,
  filename/resolution divider, full-height image canvas and Behavior switch.
  No personal library or provider account was opened. The review process was
  stopped after verification.
- Rebuilding an already launched macOS bundle exposed stale kernel signature
  caching when overwriting its executable. The bundler now replaces the inode
  atomically; rebuilding, both privacy checks and launching the review copy pass.

Physical trackpad pinch/swipe feel remains unverified; synthetic GPUI gesture
phases are covered. CUA's horizontal scroll action did not establish a physical
trackpad gesture. Live providers, real TCC approval and native Linux/Windows GUI
execution retain the limits recorded below. Earlier continuous-zoom descriptions
are historical and superseded by this revision.

## Appearance, catalog and gallery batch — October 31, 2026

Before implementation, the production data directory at
`~/Library/Application Support/tech.pinal.mitten/` was reset. No saved remote
credentials were present to remove. Original libraries were preserved. A Git
repository was initialized in this folder and the baseline committed as
`f993866` before the batch. The production profile was checked again after all
verification and remains empty; all tests and desktop review used other profiles.

- **32 tests pass:** four geometry/cache tests, eight GPUI interaction/layout
  tests, one PhotoKit packaging regression, eight source/protocol tests, and
  eleven storage tests. Formatting, strict all-target Clippy and native macOS
  debug compilation/bundling pass. The bundle and executable still pass the
  Foundation usage-description checks without requesting photo access.
- Core/network-adapter compilation passes for Linux and Windows x86-64; core,
  adapters and the PhotoKit bridge also compile for Intel macOS. Cross-checks
  use the same temporary Zig compiler described in the earlier source batch.
- Native review used 48 generated JPEG fixtures and an isolated bundle/profile.
  Verified the saved count, compact grid, edge-to-edge square view with floating
  year/month, left/right viewer controls, loading spinner during image upgrade,
  ready image display, appearance swatches and immediate purple accent updates.
  Removing the review library returned to onboarding and preserved all 48 JPEGs.
  The isolated review process was stopped after review.

Automatic cataloging fetches metadata pages continuously without a Load more
control. Background batches combine at most 16 pages, yielding after 100 ms
between requests. The synthetic **1,000,000-photo** GPUI catalog test completed
all **5,000 metadata pages in 2.57 seconds** in a standalone instrumented run
(2.67 seconds process elapsed), with **447,594,496 bytes (~427 MiB) maximum RSS**
and zero swaps. It fetched fewer than 100 visible/prefetched thumbnails and
**zero viewer previews or originals**. Counts are persisted, the loading state
finishes, and unfiltered catalog growth does not allocate a full filtered index.
This measures synthetic metadata/fixtures, not live-server throughput.

The separate million-photo scroll test still fills and evicts the 256-path and
96-decoded-thumbnail caches. Cached scroll checks were around 4.8–7.9 ms; new
views took roughly 145–159 ms to drain the unoptimized test executor, including
thumbnail encoding, synchronization and redraws. These are not production FPS
measurements. Tests caught and fixed stale viewport culling on large scroll
jumps; tile positions now use coordinates local to each virtual block.

New coverage checks accent persistence and old-settings compatibility, removing
active libraries without deleting originals, complete automatic cataloging,
failure/retry without a request loop, modifier-wheel continuous zoom, gapless
square geometry, month labels, and actual side-button navigation. Viewer tests
force paint frames through cache eviction and re-decoding, checking that the
thumbnail and loading state remain until the detailed image is decoded. The
ready image fades over its thumbnail over 140 ms. Immich tests also cover a
nullable `nextCursor` alongside a valid legacy `nextPage` token.

Physical trackpad pinch, live provider accounts, real keyring credential removal,
and native Linux/Windows GUI execution remain unverified. The native macOS pinch
hook routes into the same tested continuous geometry as modifier-wheel zoom.
Full metadata remains resident after cataloging; persistent disk thumbnails have
no automatic quota. The earlier verification records below retain their original
scope and timings.

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
  `target/Mitten.app` contains the new binary and Photos permission text.
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
  `target/Mitten.app`.
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
