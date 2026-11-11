# Using Mitten

Choose a service from the onboarding logo grid, or drop a local folder into the
empty window. Settings remembers connected libraries and offers light/dark/system
appearance, with the same seven accent colors as the mail/password apps. Sources
supported by the build are enabled automatically. Remove a library from Settings
→ Libraries to disconnect it and forget its saved credentials; original photos
and library annotations are preserved.

The top of the sidebar shows the active library: its service logo, name and
account (folder path, *System Photo Library*, or `user@host`). Click it, or its
up/down arrows, to list every connected library with its item count and switch
between them, or choose **Add New Photo Library…** to connect another.

| Source | Connection | Image access |
| --- | --- | --- |
| Local folder | Choose an existing folder | Originals stay in place |
| Apple Photos | macOS PhotoKit permission | Paged asset metadata and size-targeted PhotoKit requests |
| Immich | Server URL and API key | REST metadata search, thumbnail/preview endpoints, original export |
| Nextcloud | Server URL, username, app password, optional relative folder | WebDAV SEARCH through `reqwest_dav`, Nextcloud preview service, WebDAV original export |

For Nextcloud, enter the server base URL (including an installation subpath, if
needed), rather than the `/remote.php/dav/` URL. The optional folder is relative
to your files. The adapter uses the same bounded `nresults` / SearchDAV
`firstresult` query pattern as Nextcloud Photos. Previews must be enabled on the
server; unavailable previews show a placeholder and never trigger an original
download as a fallback. Immich supports legacy page tokens and newer opaque
cursors. Use an API key with asset read/view/download permissions (plus the
write permissions below for two-way sync).

Remote credentials are stored with `keyring` (Keychain on macOS, Windows
Credential Manager, or Linux kernel keyring). Settings contain no API keys or
passwords. If the system keyring cannot save credentials, connection fails
instead of saving them in plaintext. Use HTTPS for remote servers; explicit HTTP
URLs are supported for local servers. Redirects are rejected so custom API keys
cannot be forwarded to another origin.

When a library connects, Mitten automatically catalogs its image metadata and
records the photo count without downloading originals. Apple Photos supplies its
total immediately; other providers are enumerated in background metadata pages
until the count is complete. There is no manual “Load more” step. The top bar
shows a circular progress indicator while connecting or cataloging and offers
Retry if a metadata request fails. Completed counts are remembered in Settings.

**Immich syncs two ways:** favorites, descriptions and albums (creating
albums, adding and removing photos) are written to the server, and its own
albums (owned and shared) appear in the sidebar. Its API key then also needs
the `asset.update`, `album.create`, `albumAsset.create` and `albumAsset.delete`
permissions; edits made in Immich's own apps appear when the library reloads.
Favorites and descriptions made here before two-way sync are sent to the
server once. Other connected libraries are read-only sources: favorites, captions, albums, and
Recently Deleted are **Photos annotations on this device**, not two-way edits
to Apple Photos or the remote server. Use each provider's app to upload photos.
Apple Photos albums are **synced one way** each time the library connects or
refreshes: every user album (including those in folders) appears in the
sidebar with its photos and videos. Their membership is read-only here; change
it in Photos. A local album with the same name merges with it and stays editable.
The refresh toolbar button reloads a connected metadata snapshot. Local folders
continue to discover external changes every five seconds.

- Import images or folders with the toolbar, ⌘I, or external drag and drop.
  Imports **copy** supported images into `Imports/`; source files stay in place.
  SHA-256 checks skip duplicate content, including images already in the library.
- Browse a virtualized grid and change thumbnail sizes with the −/+ controls.
  Resize freely; the grid changes column count and narrow windows move navigation
  into a toolbar-accessible panel. Pinch the trackpad on macOS, or hold ⌘/Ctrl
  and scroll, to move between ten gallery levels, keeping your place. The
  margined levels show 9, 7, 5 or 3 photos per row (never fewer), each whole
  at its own aspect ratio; zooming out adds a photo at each end of every row,
  centred on the middle of the grid, so nothing travels across it. A pinch
  sizes the grid as the fingers move; ⌘-scroll, wheel notches and ⌘+/− step a
  level at a time, animated, and fast steps chain into one zoom. The photos
  change level as soon as a zoom heads somewhere, each change a crossfade of
  fixed length (photo, shape and selection ring together). A held pinch stays
  put; letting go springs to a level with the gesture's momentum. Beyond 9
  per row the whole layout crossfades, in one big step, into **Months**,
  edge-to-edge squares grouped by month under their own headers, then
  **Years**, each step shrinking photos by about a third. The current
  section's header stays pinned until the next one pushes it away. Gapless
  grids are painted in a single pass and stay smooth with thousands of photos
  on screen; their small thumbnails come straight from Photos' own cache for
  Apple Photos, and from embedded previews via ImageIO for folders. Animated
  GIFs and WebPs play in
  large tiles (three columns or fewer) and in the viewer. The grid has no filename
  captions; small favorite hearts scale with each photo and have no background.
  Use the right-hand scrollbar to move through the library.
- Click to select, ⌘-click to toggle individual images, or Shift-click a range.
  Double-click, Enter, or Space opens the viewer: the photo grows out of its
  tile into the full view. Left/right navigate.
  Click blank grid space to clear selection, or drag a rectangle to select photos.
  Favorite hearts appear on hover and in the selection action bar, which floats
  over the bottom of the grid only while photos are selected (it rises in
  quickly, so the grid never reflows, and leaves at once when cleared). Results
  such as imports and exports show briefly in the same place.
- **Videos** has its own sidebar view. Apple Photos videos and local `.mov`,
  `.mp4` and `.m4v` files, and Immich and Nextcloud videos, show their duration
  on larger tiles and play in the viewer through the native macOS player
  (server videos stream with the connection's credentials). On other platforms
  the viewer opens a video in the system's player, downloading server videos
  once into the cache.
- Favorite selections, create albums, add/remove album members, or drag selected
  photos onto an album. Album names preserve spaces. On macOS, drag selected
  photos out of the gallery, or drag the fitted viewer image, to copy originals
  into Finder or another compatible app. Local folders, Apple Photos, Immich and
  Nextcloud use asynchronous file promises; connected originals are fetched only
  when the receiving app fulfills the drop. Albums reference photos without
  copying originals. See [drag-export implementation notes](DRAG_EXPORT.md).
- Search names, relative paths, and descriptions; Immich libraries add the
  server's smart search (falling back to description and filename search when
  its machine learning is off). Search, filters and Select All cover
  the complete catalog after background indexing finishes. During cataloging,
  newly discovered photos appear automatically.
- The info panel (widths of 900 logical pixels or more) leaves with the photo.
  It starts with an underlined **Add a description** field, then the people in
  the photo (Immich), and every detail the metadata has: capture date and time
  zone, file, megapixels and size, camera and exposure (ƒ-number, shutter, focal
  length, ISO), lens, place and coordinates, and albums. Local files are read
  with an EXIF parser; Apple Photos details take PhotoKit's extra step of
  reading the image data's EXIF (never downloading from iCloud for it), plus
  reverse geocoding; Immich supplies its own. A map of the location sits at the
  bottom, composed from raster tiles of an open tile server (OpenStreetMap by
  default; Humanitarian, OpenTopoMap or any custom `{z}/{x}/{y}` server in
  Settings → Behavior → Map). Only tiles around the photo are requested, and
  they are cached under the app data folder (`map-cache/`).
- **Explore** (Immich) lists recognised people and places; choose one to see
  its photos.
- Export selections as byte-for-byte original copies. Existing destination
  filenames are never overwritten.
- Confirm before moving selections to Recently Deleted, and restore them there. This is an
  application-level soft deletion: original files remain on disk, and there is
  no permanent-delete or automatic-purge command.

| Shortcut | Action |
| --- | --- |
| ⌘I | Import |
| ⌘O | Open library folder |
| ⌘N | New album |
| ⌘F | Search |
| ⌘A | Select visible photos |
| Space / Enter | Open selected photo |
| ← / → | Previous / next |
| F | Favorite selection |
| Delete | Move to Recently Deleted, or restore while viewing it |
| Escape | Close dialog/viewer or clear selection |
| ⌘, | Settings |
| ⌘+ / ⌘− / ⌘0 | Zoom in / out / fit the photo |
| ⌘W / ⌘M / ⌘H | Close / minimize / hide |

Use Ctrl in place of ⌘ on other platforms. Text inputs retain their own shortcuts,
including spaces when naming albums. Settings has Libraries, Appearance and Behavior pages.

By default, the gallery starts at the bottom with the newest photos. Scroll up
for older photos. Settings → Behavior → **Browse from bottom to top** switches
between this layout and newest-first browsing from the top. Your visible photo
stays anchored when older catalog pages arrive, when density or window size
changes, and when the direction changes. Browsing older photos does not pull you
back to the bottom; a view already at the bottom stays there as photos arrive.
Partial rows stay at the oldest end, and short libraries align to the bottom, so
catalog completion does not shift the newest row.

In the photo viewer, wheel or trackpad scrolling pans only where the zoomed image
extends beyond the viewport; Shift-scroll pans horizontally. ⌘/Ctrl-scroll and
macOS trackpad pinch zoom around the pointer. Drag with the left mouse button to
pan, use −/+ to zoom, or type an editable fit-relative percentage (100–800%).
Typed values below 100% clamp to fit. Pinching in or zooming out at 100% closes
the photo at once — no need to finish the gesture — and it flies back into its
tile in the library (Back and Escape do the same); leftover momentum is
suppressed. Zooming down from above fit first stops at 100%.
The hand cursor appears only when the image can actually pan (above 100%).
At fit, a horizontal two-finger swipe follows your gesture and settles
onto the previous/next image. Small gestures and swipes beyond either end return
to the current photo; momentum does not skip multiple photos. The previous/next
arrows and ←/→ slide photos across the same way; pressing again mid-slide lands
it first. Library search and thumbnail-size
controls are hidden while viewing a photo. Previous/next controls sit at the left
and right edges of the image. The thumbnail stays visible while the larger image
decodes, then fades into the ready image. Once it is sharp, both neighbours'
previews are fetched and decoded, so a swipe lands on a full image. Its loading circle lives in the viewer
header and stays active through decoding. The precise local date and time are
centered in that header; filename and resolution are centered in the bottom
action bar, separated by a divider. The canvas fills the space between the bars,
preserving the image's aspect ratio.

## Data locations and reset

The default application-data directories are:

| Platform | Application data |
| --- | --- |
| macOS | `~/Library/Application Support/tech.pinal.mitten/` |
| Linux | `$XDG_DATA_HOME/mitten/`, or `~/.local/share/mitten/` |
| Windows | `%APPDATA%\pinal\mitten\data\` |

Set `MITTEN_DATA_DIR` to use a different application-data directory. Inside
it, `settings.json` stores connected library references, photo counts, appearance
accent settings and browsing direction. Connected-library folders live under `sources/`:

- `sources/apple-photos/` stores Apple Photos annotations and cached images.
- `sources/<connection-hash>/connection.json` stores a remote server address,
  username and folder. API keys and passwords live in the OS keyring under the
  service name `tech.pinal.mitten`, never in the settings or connection JSON.
- Each source's `.mitten/library.json` stores private annotations, and
  `.mitten/cache/` stores generated/downloaded thumbnail and viewer files.

**Local-folder libraries also have a `.mitten/` folder inside the selected
photo folder**, holding their annotations and cache. Imported originals are
copied to that library's `Imports/` folder; other originals remain in place.

To reset the app, close Mitten first, then clear its application-data directory.
That resets connections, counts, theme, and connected-source annotations/cache.
It does not delete original photos or the `.mitten/` folders inside local
libraries. To clear a local library's annotations/cache too, remove only its
`.mitten/` folder. Removing libraries through Settings also forgets saved
remote credentials; deleting the data directory alone does not clear the OS
keyring. Apple Photos privacy permission is managed separately by macOS.
