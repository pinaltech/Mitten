# Gallery zoom, stable rows and thumbnail residency

## Reference and design

Apple documents [interactive collection-view layout transitions](https://developer.apple.com/documentation/uikit/uicollectionviewtransitionlayout): a temporary layout interpolates between two endpoint layouts, driven by gesture progress. Its [transition delegate documentation](https://developer.apple.com/documentation/uikit/uicollectionviewdelegate/collectionview(_:transitionlayoutforoldlayout:newlayout:)) allows custom paths and timing. The [motion guidelines](https://developer.apple.com/design/human-interface-guidelines/motion) emphasize continuity and helping people track content through a transition. [Photos' Mac guide](https://support.apple.com/guide/photos/pht8b9ba4aa9/mac) documents trackpad pinch as a zoom input.

These sources describe public behavior and building blocks; Apple does not publish Photos' exact grid algorithm. The implementation here is our interpretation of the requested effect: **move the frames, dissolve the photographs inside them**. It does not claim to reproduce Apple's private code or exact timing.

A transition matches grid slots by row/column displacement from the photo under the gesture. Each slot has a source frame/content and a target frame/content. Frame position, edge size, spacing and clipping interpolate with pinch progress. Contents crossfade within that moving frame; the anchor slot retains the same photo. This avoids sending every individual photograph diagonally across several rows when five columns become three. Headers and floating dates follow the transition too.

The gesture controls progress immediately. Release or input idle settles to a density level over 224 ms with cubic easing. A new pinch can take control of a settling transition at its current progress. Ordinary scrolling, resizing, filtering or opening a photo clears the transition and leaves the real grid visible. The animated surface never hides the underlying grid using persistent visibility state. Its working set includes only source/destination viewport slots, independently of total catalog size.

The real destination grid renders behind the transition so its scroll metrics and thumbnail decodes can prepare before handoff. Ready thumbnails are retained for the transition. Metadata fetching continues, but presentation of a completed batch waits until the active pinch settles; at most one normal metadata batch is waiting.

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
