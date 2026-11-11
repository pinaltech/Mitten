# Gallery zoom, stable rows and thumbnail residency

## Reference and design

Apple documents [interactive collection-view layout transitions](https://developer.apple.com/documentation/uikit/uicollectionviewtransitionlayout): a temporary layout interpolates between two endpoint layouts, driven by gesture progress. Its [transition delegate documentation](https://developer.apple.com/documentation/uikit/uicollectionviewdelegate/collectionview(_:transitionlayoutforoldlayout:newlayout:)) allows custom paths and timing. The [motion guidelines](https://developer.apple.com/design/human-interface-guidelines/motion) emphasize continuity and helping people track content through a transition. [Photos' Mac guide](https://support.apple.com/guide/photos/pht8b9ba4aa9/mac) documents trackpad pinch as a zoom input.

These sources describe public behavior and building blocks; Apple does not publish Photos' exact grid algorithm. The implementation here is our interpretation of the requested effect: **the grid's size follows the input; the photos crossfade, level by level, on a fixed clock**. It does not claim to reproduce Apple's private code or exact timing.

An earlier version moved every photo from its frame in one level's layout to its frame in the other. Because a photo's column depends on the column count, that sent photos sliding diagonally across the grid in every direction: distracting, and hard to follow. Photos now never travel.

- **Size.** A trackpad pinch moves the zoom continuously with the fingers. ⌘-scroll (trackpad or mouse wheel), ⌘+/− and the toolbar step a whole level, animated by a spring; quick steps chain into one zoom. A trackpad ⌘-scroll steps as soon as the fingers move, then after every further 200 pt of travel; turning around steps back at once.
- **Slots.** The margined levels have 9, 7, 5 and 3 photos per row (three is the closest zoom). They share one centred lattice of slots: going from 3 to 5, a slot appears at each end of every row while the rows close up about the centre of the grid; between levels the lattice is the blend of the two. The spot at the centre of the grid stays there, and so does its photo's row.
- **Photos.** Each level's photos are laid into the lattice at the current size, so the outgoing and incoming photos always share one geometry. As soon as a zoom heads toward the next level (4% of a level's travel), that level's photos crossfade in over 220 ms, whatever the fingers do meanwhile. Each level fades as one layer, so a photo's image, aspect-ratio frame and selection ring fade together. A zoom past several levels fades through each in turn; turning back mid-fade reverses it from where it is.
- **Date grids.** Beyond 9 per row, the whole layout crossfades in one large step (at least half again as many columns) into the edge-to-edge **Months** grid with its section headers, and on to **Years**. Date grids are scaled whole about the same centre, headers included.
- **Release.** A held pinch stays put. Letting go lands on the next level once the gesture has gone 15% of the way there (its photos are already showing), with momentum able to carry one level further; otherwise it springs back and the photos fade back. Pinching past either end only stretches a little.

Ordinary scrolling, resizing, filtering or opening a photo ends the zoom on the nearest level and leaves the real grid visible. While zooming, the real grid keeps its size and scroll position underneath but builds and paints no photos; the zoom draws at most two levels' on-screen photos, independently of total catalog size. Margined thumbnails coming into view are decoded as they appear, so they are ready when the zoom lands.

Metadata fetching continues during a zoom, but presentation of a completed batch waits until it settles; at most one normal metadata batch is waiting.

## Catalog completion shift

A bottom-up library previously reversed the loaded records and packed them from slot zero. A final page with a different remainder modulo the column count changed the latest photo's column and created a partial newest row. Preserving only the scroll offset could not preserve those frames.

Bottom-up packing now pads the **oldest/top** row with empty slots. The newest row is full, and every existing photo keeps the same column and distance from the bottom as older metadata arrives. Short libraries align with the bottom of their container too, so crossing from one short row to two does not move the newest row downward. Top-down browsing still packs from the top. The metadata count can finish without changing the layout mode or reserved header height.

## Disappearing thumbnails: reproduced cause

The old decoded cache retained 96 entries regardless of viewport density. A 640 × 1200 test window contains 240 fully visible thumbnails at the smallest level. Repainting those tiles continually evicted asynchronous decode results needed by other visible tiles. The reproduction measured **0 of 240 visible thumbnails ready** before the fix.

The same reproduction passes with all 240 ready after the fix. Thumbnail paths and decoded images have viewport-sized budgets: at least 256 paths / 96 decoded images, expanded for the visible rows, a small row allowance, and the source/destination transition working set. They shrink again as the viewport working set decreases; there is no allocation proportional to the entire catalog. Viewer images retain their separate three-entry limit. Original downloads are never used to fill grid tiles.

The virtual list's range is now the sole authority for row visibility. Extra culling based on a separately sampled scroll offset was removed, avoiding disagreement during deferred scrolls and density changes. Thumbnail requests still begin in visible-tile prepaint, rather than the list's measurement pass.

## Regression coverage

- Every density level and catalog-size remainder keeps latest frames fixed as older pages arrive.
- Real GPUI final-page tests check unchanged newest-photo bounds for both short and scrolling libraries, including completion during a pinch.
- Slot interpolation preserves the anchor and changes contents within moving frames; transition work stays bounded with a million-record catalog.
- Dense rendering is checked after scrolling, zooming, interruption and reversal, then scrolling into another cold range. All fully visible thumbnails remain decoded; no viewer preview or original is requested.
- Resize cancels obsolete transition geometry. Existing selection, viewer, source, storage and million-photo stress tests remain active.

Native review uses generated JPEGs and an isolated profile. Physical trackpad feel and exact visual parity with Apple's Photos require device review; synthetic gesture tests establish state and geometry correctness, not that equivalence.

## Date tiers, painted grids and the zoom transition

Levels 0–2 (Years) and 3–5 (Months) group photos into date sections laid out
by `GalleryLayout::with_groups`; levels 6–9 have gutters, 9, 7, 5 and 3
photos per row, and show photos at their aspect ratio
(`GalleryLayout::frame`). Sections are built from month runs in library
order, extended incrementally as catalog pages append, and mirrored for
bottom-to-top browsing, so paging never rescans the library. The grid is
virtualized by hand in a scroll container, which lets headers and section
gaps have their own heights.

Gapless levels are painted from one canvas instead of one element per tile,
inside a single `paint_layer`. That matters: GPUI otherwise orders every
primitive with an overlap search, which cost ~7 ms for 6,400 tiles; in one
layer the same frame takes ~1 ms of CPU (release build,
`five_thousand_painted_tiles_scroll_and_zoom_without_per_tile_cost`).

Their 64 px / 160 px squares live in `ui::tiles::Tiles`: one hash probe per
tile per frame, decoded in parallel batches of 32 straight to GPU-ready BGRA
pixels, kept within a 640 MB budget by least-recently-drawn eviction, with
atlas textures released before the next paint. Sources produce squares in
bulk (`PhotosDriver::squares`): Apple Photos reads the thumbnails Photos
already keeps (`PHImageRequestOptionsDeliveryModeFastFormat`, no iCloud
download, one asset fetch per batch); folders use ImageIO, which reads a
camera file's embedded preview or decodes at reduced scale. Squares are
cached as small JPEGs for later sessions. Cache files are renamed into place
without a per-file `F_FULLFSYNC`, which had dominated thumbnail time.

The zoom model lives in `gallery::zoom`, is pure, and is unit-tested
without a window:

- **`Ladder`**: the levels reachable at the current width, with their photo
  edge. A continuous `position` on the ladder maps to a photo edge linearly
  between neighbouring levels, so a pinch factor scales the photos by that
  factor.
- **`Zoom`**: the size's input state machine. Pinch (`Magnify`), steps
  (`Step`) and finger lift (`Release`) go in through `apply`; time goes in
  through `tick`. It tracks the gesture's heading (with a little hysteresis,
  so a trembling finger never flips the photos), and `goal` says which level's
  photos should show. Input without gesture phases settles after 240 ms of
  quiet. Release springs (critically damped, about a third of a second).
- **`Crossfade`**: which levels' photos are shown, one at rest or two mid-fade,
  stepping one level at a time on a fixed 220 ms clock.
- **`Lattice`** and **`Layer`**: what is drawn. Each level's grid is a
  `GridFrame` scrolled so the spot at the centre of the grid stays there
  (`GridFrame::anchored`); bottom-up browsing that started at the newest row
  stays there instead. `lattice_at` blends the margined levels' lattices
  around the position (or scales the nearest past them), and a `Layer` places
  one level's photos into it, or scales a date grid whole.

In the app, `MittenApp::zoom_library` is the single entry for every input,
`transition_view` paints the shown levels' layers over the real grid (each in
a single draw order, faded as a whole), and `finish_zoom` commits a level as
the real grid once the size has landed and its photos are in.

Decoded 512 px thumbnails (gutter levels) are kept by a 768 MB budget beyond
the visible working set, and up to 50,000 thumbnail paths are remembered, so
scrolling back shows images immediately. One and a half screens in each
direction are prefetched.
