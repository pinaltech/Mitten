use chrono::Datelike;
use desert_photos::gallery::{
    self, GalleryLayout, GridFrame, Group, Tier, extend_groups, month_key,
    zoom::{self, Crossfade, Input as ZoomInput, Ladder, Lattice, Layer, Placement, Zoom},
};
use desert_photos::loading::{BoundedCache, Square, THUMBNAIL_CACHE, THUMBNAIL_JOBS, VIEWER_CACHE};
use desert_photos::map;
use desert_photos::sources::{Connection, PhotoPage, ProviderKind};
use desert_photos::storage::{self, Library, Photo, PhotosDriver, Preferences, Source};
use desert_ui::{self as ui, Accent, Colors, library as kit};
use gpui::{prelude::*, *};
use gpui_component::{
    Root, Theme, ThemeMode,
    input::{Input, InputEvent, InputState},
    scroll::{Scrollbar, ScrollbarShow},
    switch::Switch,
};
use photo_view::PhotoView;
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

actions!(
    photos,
    [
        Import,
        OpenFolder,
        Settings,
        Find,
        SelectAll,
        OpenSelected,
        Back,
        Next,
        Previous,
        Delete,
        Favorite,
        CopyPhotos,
        NewAlbum,
        Quit,
        CloseWindow,
        Minimize,
        Hide,
        HideOthers,
        ZoomIn,
        ZoomOut,
        FitPhoto
    ]
);
#[derive(Clone, PartialEq)]
enum Filter {
    All,
    Videos,
    Favorites,
    Recent,
    Trash,
    Album(String),
    /// People and places (sources with face recognition and geocoding).
    Explore,
    /// Photos of one person: ID and name.
    Person(String, String),
    Place(String),
}
#[derive(Clone, Copy, PartialEq)]
enum Modal {
    Sources,
    Browse,
    Album,
    AddToAlbum,
    Help,
    DeleteConfirm,
}
#[derive(Clone)]
struct DragPhotos {
    ids: Arc<RefCell<Vec<String>>>,
    seed: String,
    photos: Arc<RefCell<Vec<Arc<Photo>>>>,
    driver: Arc<dyn PhotosDriver>,
    source: PathBuf,
    paths: Arc<RefCell<Vec<PathBuf>>>,
    offset: Point<Pixels>,
    preview: Option<PathBuf>,
    colors: Colors,
}
impl Render for DragPhotos {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors;
        div().pl(self.offset.x).pt(self.offset.y).child(
            div()
                .w(px(96.))
                .h(px(80.))
                .rounded(px(8.))
                .overflow_hidden()
                .bg(rgb(c.soft))
                .when_some(self.preview.clone(), |d, path| {
                    d.child(
                        img(Arc::<std::path::Path>::from(path))
                            .w_full()
                            .h(px(55.))
                            .object_fit(ObjectFit::Cover),
                    )
                })
                .child(
                    div()
                        .px_2()
                        .text_size(px(11.))
                        .text_color(rgb(c.accent))
                        .child(format!("{} photos", self.ids.borrow().len())),
                ),
        )
    }
}
struct Marquee {
    start: Point<Pixels>,
    end: Point<Pixels>,
    content_start: Point<Pixels>,
    baseline: BTreeSet<String>,
}
mod assets;
mod explore;
mod image_cache;
mod info;
mod native_drag;
mod photo_view;
mod progress;
mod settings;
mod tiles;
mod video;
fn provider_logo(provider: ProviderKind, c: Colors) -> AnyElement {
    provider_logo_sized(provider, 30., c)
}
fn provider_logo_sized(provider: ProviderKind, edge: f32, c: Colors) -> AnyElement {
    if provider == ProviderKind::Local {
        return div()
            .size(px(edge))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(kit::glyph("folder", edge * 0.86, c.accent))
            .into_any_element();
    }
    img(SharedString::from(format!(
        "providers/{}.svg",
        provider.id()
    )))
    .size(px(edge))
    .flex_shrink_0()
    .into_any_element()
}
/// A library's name in the switcher: the service, or a local folder's name.
fn library_name(source: &Source) -> String {
    match ProviderKind::of(source) {
        ProviderKind::Local => source.label(),
        provider => provider.name().into(),
    }
}
/// "0:42" or "1:02:03".
fn duration_label(seconds: f32) -> String {
    let total = seconds.max(0.).round() as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
#[derive(Clone)]
struct GalleryAnchor {
    id: String,
    fallback: usize,
    within: f32,
    screen: f32,
    end: bool,
}
/// A library zoom in progress: its size, which levels' photos are shown, and
/// each level's grid as it will be once settled (made on first use, then
/// fixed).
struct GalleryZoom {
    zoom: Zoom,
    ladder: Ladder,
    content: Crossfade,
    /// The photo at the centre of the grid when the zoom began, and how far
    /// down its cell the centre was. Every level keeps that spot there.
    anchor: (usize, f32),
    /// The centre of the grid on the gallery surface (title + grid area):
    /// what the zoom scales about.
    center: (f32, f32),
    area: f32,
    /// Bottom-up browsing at the newest row stays there.
    at_end: bool,
    frames: HashMap<usize, GridFrame>,
}
/// The best ready image for a photo drawn `pixels` wide while zooming: a
/// painted square when small in gapless grids (requested if missing), else
/// its thumbnail, else whichever square is ready.
fn zoom_image(
    tiles: &mut tiles::Tiles,
    thumbnails: &BoundedCache<PathBuf>,
    cache: &image_cache::PhotoImageCache,
    photo: &Arc<Photo>,
    pixels: f32,
    dense: bool,
) -> Option<Arc<RenderImage>> {
    let thumbnail = || {
        thumbnails
            .peek(&photo.fingerprint)
            .and_then(|path| cache.ready(path))
    };
    if !dense || pixels > 170. {
        thumbnail().or_else(|| tiles.peek(photo))
    } else {
        tiles
            .get(photo, Square::for_pixels(pixels))
            .or_else(thumbnail)
    }
}
fn aspect_of(
    photo: &Photo,
    thumbnails: &BoundedCache<PathBuf>,
    cache: &image_cache::PhotoImageCache,
) -> f32 {
    if let Some(image) = thumbnails
        .peek(&photo.fingerprint)
        .and_then(|path| cache.ready(path))
    {
        let s = image.size(0);
        return s.width.0.max(1) as f32 / s.height.0.max(1) as f32;
    }
    if photo.width > 0 && photo.height > 0 {
        photo.width as f32 / photo.height as f32
    } else {
        1.
    }
}
/// Paints an image filling `rect`: directly (with rounded corners) when its
/// shape matches, else cropped to cover it; a placeholder while loading.
fn paint_cover(
    rect: Bounds<Pixels>,
    image: Option<Arc<RenderImage>>,
    radius: f32,
    c: Colors,
    w: &mut Window,
) {
    let corners = Corners::all(px(radius));
    let Some(image) = image else {
        w.paint_quad(fill(rect, rgb(c.surface)).corner_radii(corners));
        return;
    };
    let s = image.size(0);
    let (iw, ih) = (s.width.0.max(1) as f32, s.height.0.max(1) as f32);
    let (rw, rh) = (
        f32::from(rect.size.width).max(0.01),
        f32::from(rect.size.height).max(0.01),
    );
    if ((iw / ih) / (rw / rh) - 1.).abs() < 0.02 {
        let _ = w.paint_image(rect, corners, image, 0, false);
        return;
    }
    let scale = (rw / iw).max(rh / ih);
    let cover = Bounds::new(
        rect.center() - point(px(iw * scale / 2.), px(ih * scale / 2.)),
        size(px(iw * scale), px(ih * scale)),
    );
    w.with_content_mask(Some(ContentMask { bounds: rect }), |w| {
        let _ = w.paint_image(cover, Corners::default(), image, 0, false);
    });
}
/// A month ("October 2026") or year ("2026") section header, scaled by `k`.
fn section_label(tier: Tier, key: i32, k: f32, c: Colors) -> Div {
    let (title, detail) = match tier {
        Tier::Years => (key.to_string(), String::new()),
        _ => (
            chrono::Month::try_from((key.rem_euclid(12) + 1) as u8)
                .map(|m| m.name())
                .unwrap_or_default()
                .to_owned(),
            gallery::year_of(key).to_string(),
        ),
    };
    let size = if tier == Tier::Years { 26. } else { 17. } * k;
    div()
        .flex()
        .items_end()
        .gap(px(6. * k))
        .px(px(16. * k))
        .pb(px(8. * k))
        .overflow_hidden()
        .child(
            div()
                .text_size(px(size))
                .line_height(px(size * 1.2))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(c.text))
                .child(title),
        )
        .when(!detail.is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(size))
                    .line_height(px(size * 1.2))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c.muted))
                    .child(detail),
            )
        })
}
/// Height of the title row a layout shows above the grid.
fn gallery_title(_layout: &GalleryLayout) -> f32 {
    0.
}
/// Width of a margined tile's border: its photo sits this far inside its
/// frame, and a selected tile draws its accent ring there.
const TILE_BORDER: f32 = 2.;
/// A photo while zooming: its frame, image and whether it is selected.
type ZoomTile = (gallery::Rect, Option<Arc<RenderImage>>, bool);
/// One level's photos while zooming, painted as its tiles are drawn at rest:
/// margined photos inside their border, square-cornered, and date grids edge
/// to edge; selected ones ringed. Each layer is a single draw order, so
/// thousands of photos cost one pass, and its opacity (set on the parent)
/// fades every part of a photo together.
fn zoom_layer(tiles: Vec<ZoomTile>, margined: bool, radius: f32, c: Colors) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, w, _| {
            let at = |r: &gallery::Rect| {
                Bounds::new(
                    bounds.origin + point(px(r.x), px(r.y)),
                    size(px(r.w), px(r.h)),
                )
            };
            let (inset, image_radius) = if margined {
                (TILE_BORDER, 0.)
            } else {
                (0., radius)
            };
            w.paint_layer(bounds, |w| {
                for (r, image, _) in &tiles {
                    let rect = at(r);
                    let rect = Bounds::new(
                        rect.origin + point(px(inset), px(inset)),
                        size(
                            (rect.size.width - px(2. * inset)).max(px(0.)),
                            (rect.size.height - px(2. * inset)).max(px(0.)),
                        ),
                    );
                    paint_cover(rect, image.clone(), image_radius, c, w);
                }
            });
            w.paint_layer(bounds, |w| {
                for (r, _, selected) in &tiles {
                    if *selected {
                        w.paint_quad(quad(
                            at(r),
                            Corners::all(px(radius)),
                            transparent_black(),
                            px(TILE_BORDER),
                            rgb(c.accent),
                            BorderStyle::Solid,
                        ));
                    }
                }
            });
        },
    )
    .absolute()
    .inset_0()
}
/// Month runs of the visible photos. Built in library order so appended
/// catalog pages extend it; years and reversed browsing derive from it.
struct GroupCache {
    epoch: u64,
    filter: Filter,
    query: String,
    indices: Option<Arc<Vec<usize>>>,
    count: usize,
    source: Vec<Group>,
    reversed: bool,
    months: Arc<Vec<Group>>,
    years: Arc<Vec<Group>>,
}
/// Trackpad ⌘-scroll steps whole levels, each animated: the first once the
/// fingers have moved this far, so the zoom starts at once...
const SCROLL_FIRST_STEP: f32 = 12.;
/// ...and each further one after this much more travel the same way.
const SCROLL_STEP: f32 = 200.;
/// A pause this long between ⌘-scroll events starts a new gesture.
const SCROLL_GESTURE_GAP: Duration = Duration::from_millis(300);
/// Margined photos whose thumbnails start decoding per frame while zooming.
const ZOOM_DECODES: usize = 48;
/// Decoded-thumbnail budget for the grid, on top of the visible working set.
const GRID_IMAGE_BUDGET: usize = 768 << 20;
/// Animated tiles appear only where few are on screen at once.
const MOTION_TILE_COLUMNS: usize = 3;
/// Viewer previews kept: the photo on screen and both neighbours, so a swipe
/// lands on a decoded image.
const VIEWER_PREVIEWS: usize = 5;
/// How long a photo takes to grow out of, or settle back into, its tile.
const OPEN_ANIMATION: Duration = Duration::from_millis(260);
/// A swipe (or arrow press) sliding the full width; shorter slides are quicker.
const SWIPE_ANIMATION: Duration = Duration::from_millis(220);
/// How long a status message stays up when nothing is selected.
const NOTICE_TIME: Duration = Duration::from_millis(2400);
struct VisibleCache {
    revision: u64,
    count: usize,
    filter: Filter,
    query: String,
    photos: PhotoView,
}
pub struct PhotosApp {
    config_path: PathBuf,
    preferences: Preferences,
    driver: Option<Arc<dyn PhotosDriver>>,
    library: Library,
    filter: Filter,
    search: Entity<InputState>,
    name: Entity<InputState>,
    caption: Entity<InputState>,
    focus: FocusHandle,
    selected: BTreeSet<String>,
    anchor: Option<usize>,
    viewer: Option<String>,
    previews: BoundedCache<PathBuf>,
    thumbnails: BoundedCache<PathBuf>,
    failed_order: std::collections::VecDeque<String>,
    revision: u64,
    visible_cache: RefCell<Option<VisibleCache>>,
    counts: (usize, usize),
    favorite_count: usize,
    video_count: usize,
    photo_index: std::collections::HashMap<String, usize>,
    viewer_render_cache: Entity<image_cache::PhotoImageCache>,
    next_page: Option<String>,
    has_more: bool,
    page_failed: bool,
    total: Option<usize>,
    source_label: String,
    album_counts: std::collections::BTreeMap<String, usize>,
    provider_step: Option<ProviderKind>,
    server: Entity<InputState>,
    username: Entity<InputState>,
    secret: Entity<InputState>,
    remote_folder: Entity<InputState>,
    render_cache: Entity<image_cache::PhotoImageCache>,
    pending: BTreeSet<String>,
    failed: BTreeSet<String>,
    sidebar: bool,
    inspector: bool,
    gallery_level: usize,
    gallery_width: f32,
    gallery_height: f32,
    group_cache: RefCell<Option<GroupCache>>,
    library_epoch: u64,
    tiles: tiles::Tiles,
    scale_factor: f32,
    motions: BoundedCache<PathBuf>,
    motion_cache: Entity<image_cache::PhotoImageCache>,
    gallery_reset: bool,
    gallery_restore: Option<GalleryAnchor>,
    gallery_zoom: Option<GalleryZoom>,
    gallery_ticking: bool,
    // Trackpad momentum after a ⌘-scroll ends must not restart the zoom.
    gallery_momentum: Option<std::time::Instant>,
    // The ⌘-scroll in progress: travel toward its next step, the direction
    // of its last step (0 before the first), and when it last moved.
    zoom_scroll: Option<(f32, f32, std::time::Instant)>,
    swipe_offset: f32,
    swipe_active: bool,
    swipe_epoch: u64,
    swipe_settling: bool,
    swipe_blocked: bool,
    swipe_last: Option<std::time::Instant>,
    swipe_idle: bool,
    // The step a running slide commits when it lands.
    swipe_amount: isize,
    decoded_preview: Option<String>,
    // The photo shown last, and whether its preview was decoded when it
    // appeared (then it needs no fade from the thumbnail).
    viewer_shown: Option<(String, bool)>,
    opening: Option<OpeningPhoto>,
    // Horizontal margin of a video's frame in the viewer, and where the
    // native player goes this frame.
    video_inset: f32,
    video_frame: Option<(storage::VideoSource, Bounds<Pixels>)>,
    catalog_epoch: u64,
    viewport: ui::image_viewport::ImageViewport,
    viewer_bounds: Bounds<Pixels>,
    // A pinch may close the viewer only if it began at 100%; the close
    // commits on release, never mid-gesture.
    pinch_armed: bool,
    dismiss_pinch: Option<f32>,
    viewer_base: Option<Arc<RenderImage>>,
    closing: Option<ClosingPhoto>,
    closing_epoch: u64,
    dismiss_guard: Option<std::time::Instant>,
    pan_start: Option<Point<Pixels>>,
    zoom_input: Entity<InputState>,
    menu: Option<Point<Pixels>>,
    library_menu: bool,
    switcher_bounds: Bounds<Pixels>,
    notice_seen: String,
    notice_until: Option<std::time::Instant>,
    settings_page: u8,
    marquee: Option<Marquee>,
    pending_delete: Vec<String>,
    modal: Option<Modal>,
    error: Option<String>,
    notice: String,
    busy: bool,
    loading: bool,
    generation: u64,
    scan_epoch: u64,
    scroll: UniformListScrollHandle,
    map_url: Entity<InputState>,
    custom_map: bool,
    measuring: bool,
    sizes_dirty: bool,
    // Info panel: the shown photo's details, its map and people's faces.
    details: Option<(String, Arc<storage::PhotoDetails>)>,
    details_pending: Option<String>,
    map: Option<(String, Option<PathBuf>)>,
    map_pending: Option<String>,
    faces: HashMap<String, Option<PathBuf>>,
    faces_pending: BTreeSet<String>,
    caption_for: Option<String>,
    // Explore (people and places) and the photos of the one chosen.
    explore: Option<Explore>,
    explore_pending: bool,
    all_people: bool,
    query_ids: Option<(Filter, Arc<std::collections::HashSet<String>>)>,
    query_pending: Option<Filter>,
    // The source's own search results for the current query.
    remote_search: Option<(String, Arc<std::collections::HashSet<String>>)>,
    search_epoch: u64,
    _subscriptions: Vec<Subscription>,
}
#[derive(Clone, Default)]
struct Explore {
    people: Vec<storage::Person>,
    places: Vec<storage::Place>,
}
impl PhotosApp {
    fn new(config_path: PathBuf, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let (preferences, error) = match Preferences::load(&config_path) {
            Ok(p) => (p, None),
            Err(e) => (Preferences::default(), Some(e.to_string())),
        };
        let search = cx.new(|cx| InputState::new(w, cx).placeholder("Search photos"));
        let name = cx.new(|cx| InputState::new(w, cx).placeholder("Album name"));
        let zoom_input = cx.new(|cx| {
            let mut e = InputState::new(w, cx);
            e.set_value("100%", w, cx);
            e
        });
        let caption = cx.new(|cx| InputState::new(w, cx).placeholder("Add a description"));
        let map_url = cx.new(|cx| {
            let mut e =
                InputState::new(w, cx).placeholder("https://tiles.example.com/{z}/{x}/{y}.png");
            e.set_value(preferences.map_tiles.clone(), w, cx);
            e
        });
        let mut subscriptions = vec![
            cx.subscribe_in(&zoom_input, w, |this, _, event, w, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let value = this
                        .zoom_input
                        .read(cx)
                        .value()
                        .trim()
                        .trim_end_matches('%')
                        .parse::<f32>();
                    if let Ok(value) = value {
                        this.set_viewer_zoom(value / 100., (0., 0.), false, w, cx);
                    }
                    this.sync_zoom(w, cx);
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.focus.focus(w);
                    }
                    cx.notify();
                }
            }),
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.selected.clear();
                    this.anchor = None;
                    this.viewer = None;
                    if this.filter == Filter::Explore {
                        this.filter = Filter::All;
                    }
                    this.reset_gallery();
                    this.search_source(cx);
                    cx.notify();
                }
            }),
            cx.subscribe_in(&caption, w, |this, _, event, w, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.save_caption(cx);
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.focus.focus(w);
                    }
                }
            }),
            cx.subscribe_in(&map_url, w, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let url = this.map_url.read(cx).value().trim().to_string();
                    if url == this.preferences.map_tiles {
                        return;
                    }
                    if map::valid_template(&url) {
                        this.preferences.map_tiles = url;
                        this.error = None;
                        this.map = None;
                        this.save_preferences();
                    } else {
                        this.error =
                            Some("Enter an http(s) tile address with {z}, {x} and {y}.".into());
                    }
                    cx.notify();
                }
            }),
            cx.subscribe_in(&name, w, |this, _, event, w, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.create_album(w, cx);
                }
            }),
            cx.observe_window_appearance(w, |this, w, cx| this.theme(w, cx)),
        ];
        let server = cx.new(|cx| InputState::new(w, cx).placeholder("https://photos.example.com"));
        let username = cx.new(|cx| InputState::new(w, cx).placeholder("Username"));
        let secret = cx.new(|cx| {
            InputState::new(w, cx)
                .placeholder("API key or app password")
                .masked(true)
        });
        let remote_folder =
            cx.new(|cx| InputState::new(w, cx).placeholder("Photos (leave empty for all files)"));
        for field in [&server, &username, &secret, &remote_folder] {
            subscriptions.push(cx.subscribe_in(field, w, |this, _, event, w, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) && this.provider_step.is_some() {
                    this.connect_remote(w, cx);
                }
            }));
        }
        let render_cache = image_cache::PhotoImageCache::with_budget(96, GRID_IMAGE_BUDGET, cx);
        let viewer_render_cache = image_cache::PhotoImageCache::new(VIEWER_PREVIEWS - 1, cx);
        let motion_cache = image_cache::PhotoImageCache::new(16, cx);
        let mut this = Self {
            config_path,
            preferences,
            driver: None,
            library: Library::default(),
            filter: Filter::All,
            search,
            name,
            caption,
            focus: cx.focus_handle(),
            selected: BTreeSet::new(),
            anchor: None,
            viewer: None,
            previews: BoundedCache::new(VIEWER_CACHE.max(VIEWER_PREVIEWS)),
            thumbnails: BoundedCache::new(THUMBNAIL_CACHE),
            failed_order: Default::default(),
            revision: 0,
            visible_cache: RefCell::new(None),
            counts: (0, 0),
            favorite_count: 0,
            video_count: 0,
            photo_index: Default::default(),
            viewer_render_cache,
            next_page: None,
            has_more: false,
            page_failed: false,
            total: None,
            source_label: "Photos".into(),
            album_counts: Default::default(),
            provider_step: None,
            server,
            username,
            secret,
            remote_folder,
            render_cache,
            pending: BTreeSet::new(),
            failed: BTreeSet::new(),
            sidebar: true,
            inspector: false,
            gallery_level: GalleryLayout::DEFAULT_LEVEL,
            gallery_width: 844.,
            gallery_height: 0.,
            group_cache: RefCell::new(None),
            library_epoch: 0,
            tiles: tiles::Tiles::default(),
            scale_factor: 2.,
            motions: BoundedCache::new(256),
            motion_cache,
            gallery_reset: true,
            gallery_restore: None,
            gallery_zoom: None,
            gallery_ticking: false,
            gallery_momentum: None,
            zoom_scroll: None,
            swipe_offset: 0.,
            swipe_active: false,
            swipe_epoch: 0,
            swipe_settling: false,
            swipe_blocked: false,
            swipe_last: None,
            swipe_idle: false,
            swipe_amount: 0,
            decoded_preview: None,
            viewer_shown: None,
            opening: None,
            video_inset: 0.,
            video_frame: None,
            catalog_epoch: 0,
            viewport: Default::default(),
            viewer_bounds: Bounds::default(),
            pinch_armed: false,
            dismiss_pinch: None,
            viewer_base: None,
            closing: None,
            closing_epoch: 0,
            dismiss_guard: None,
            pan_start: None,
            zoom_input,
            menu: None,
            library_menu: false,
            switcher_bounds: Bounds::default(),
            notice_seen: String::new(),
            notice_until: None,
            settings_page: 0,
            marquee: None,
            pending_delete: vec![],
            modal: None,
            error,
            notice: "".into(),
            busy: false,
            loading: false,
            generation: 0,
            scan_epoch: 0,
            scroll: UniformListScrollHandle::new(),
            map_url,
            custom_map: false,
            measuring: false,
            sizes_dirty: false,
            details: None,
            details_pending: None,
            map: None,
            map_pending: None,
            faces: HashMap::new(),
            faces_pending: BTreeSet::new(),
            caption_for: None,
            explore: None,
            explore_pending: false,
            all_people: false,
            query_ids: None,
            query_pending: None,
            remote_search: None,
            search_epoch: 0,
            _subscriptions: subscriptions,
        };
        this.theme(w, cx);
        this.focus.focus(w);
        #[cfg(not(test))]
        {
            let weak = cx.weak_entity();
            w.on_window_should_close(cx, move |_, cx| {
                weak.update(cx, |this, cx| {
                    if this.busy {
                        this.error = Some("Operation in progress. Wait before closing.".into());
                        cx.notify();
                        false
                    } else {
                        true
                    }
                })
                .unwrap_or(true)
            });
        }
        if let Some(path) = this.preferences.active.clone() {
            let source = this
                .preferences
                .sources
                .iter()
                .find(|s| s.path == path)
                .cloned()
                .unwrap_or(Source {
                    driver: "local-folder".into(),
                    path,
                });
            this.connect(source, w, cx);
        }
        cx.spawn_in(w, async move |this, cx| {
            loop {
                Timer::after(Duration::from_secs(5)).await;
                if this
                    .update_in(cx, |this, w, cx| this.refresh(w, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        #[cfg(not(test))]
        cx.spawn_in(w, async move |this, cx| {
            let mut tick = 0u32;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                tick = tick.wrapping_add(1);
                if this
                    .update_in(cx, |this, w, cx| {
                        // A few times a second: recover the window if it is
                        // frontmost but stuck not key (no hover, typing beeps).
                        if tick.is_multiple_of(12) && video::restore_key_window(w) {
                            cx.notify();
                        }
                        let gesture = ui::native_transfer::take_magnify_gesture(w);
                        if gesture.began && this.viewer.is_some() {
                            this.viewer_pinch_began();
                        }
                        let delta = gesture.delta;
                        if delta != 0. && this.modal.is_none() {
                            if this.block_dismissed_gesture(false) {
                                return;
                            }
                            if this.viewer.is_some() {
                                let at = w.mouse_position() - this.viewer_bounds.center();
                                this.viewer_pinch(delta, (at.x.into(), at.y.into()), w, cx);
                            } else {
                                this.zoom_library(ZoomInput::Magnify(1. + delta), true, cx);
                            }
                            cx.notify();
                        }
                        if gesture.ended {
                            this.viewer_pinch_ended(w, cx);
                            // The gallery holds its zoom until the fingers lift.
                            if this.viewer.is_none() {
                                this.zoom_library(ZoomInput::Release, true, cx);
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }
    fn colors(&self, w: &Window) -> Colors {
        Colors::new(
            match self.preferences.appearance.as_str() {
                "dark" => true,
                "light" => false,
                _ => ui::system_dark(w),
            },
            self.accent(),
        )
    }
    fn accent(&self) -> Accent {
        Accent::ALL
            .into_iter()
            .find(|a| a.key() == self.preferences.accent)
            .unwrap_or(Accent::Orange)
    }
    fn theme(&self, w: &mut Window, cx: &mut Context<Self>) {
        let dark = match self.preferences.appearance.as_str() {
            "dark" => true,
            "light" => false,
            _ => ui::system_dark(w),
        };
        Theme::change(
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            Some(w),
            cx,
        );
        let c = self.colors(w);
        let theme = Theme::global_mut(cx);
        theme.colors.primary = rgb(c.accent).into();
        theme.colors.ring = rgb(c.accent).into();
        theme.colors.accent = rgb(c.soft).into();
        theme.colors.background = rgb(c.bg).into();
        theme.colors.foreground = rgb(c.text).into();
        theme.colors.input = rgb(c.border).into();
        theme.colors.muted_foreground = rgb(c.muted).into();
        theme.colors.border = rgb(c.border).into();
        cx.notify();
    }
    fn set_accent(&mut self, accent: Accent, w: &mut Window, cx: &mut Context<Self>) {
        self.preferences.accent = accent.key().into();
        if let Err(e) = self.preferences.save(&self.config_path) {
            self.error = Some(e.to_string());
        }
        self.theme(w, cx);
    }
    fn loading_status(&self) -> Option<(String, Option<f32>)> {
        if self.busy {
            return Some((
                if self.notice.is_empty() {
                    "Working…".into()
                } else {
                    self.notice.clone()
                },
                None,
            ));
        }
        if self.loading || self.has_more && !self.page_failed {
            let count = self.library.photos.len();
            return Some(match self.total {
                Some(total) if total > 0 => (
                    format!("Cataloging {count} / {total}"),
                    Some(count as f32 / total as f32),
                ),
                _ => (format!("Cataloging · {count} photos"), None),
            });
        }
        None
    }
    fn image_loading(&self) -> bool {
        self.current()
            .filter(|_| self.viewer.is_some())
            .is_some_and(|p| {
                self.decoded_preview.as_deref() != Some(&p.fingerprint)
                    && !self.failed.contains(&format!("{}-preview", p.fingerprint))
            })
    }
    fn remove_library(&mut self, source: Source, w: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = "Removing library…".into();
        cx.spawn_in(w, async move |this, cx| {
            let credential_source = source.clone();
            let result = cx
                .background_executor()
                .spawn(
                    async move { desert_photos::sources::forget_credentials(&credential_source) },
                )
                .await;
            let _ = this.update_in(cx, |this, w, cx| {
                this.busy = false;
                if let Err(error) = result {
                    this.error = Some(format!("Cannot remove saved credentials: {error}"));
                    this.load_more(cx);
                    cx.notify();
                    return;
                }
                let was_active = this.preferences.active.as_ref() == Some(&source.path);
                let mut preferences = this.preferences.clone();
                preferences.remove(&source);
                if let Err(error) = preferences.save(&this.config_path) {
                    this.error = Some(error.to_string());
                    this.load_more(cx);
                    cx.notify();
                    return;
                }
                this.preferences = preferences;
                this.notice = "Library removed".into();
                if was_active {
                    this.generation += 1;
                    this.catalog_epoch += 1;
                    this.scan_epoch += 1;
                    this.driver = None;
                    this.library = Library::default();
                    this.library_changed();
                    this.selected.clear();
                    this.anchor = None;
                    this.viewer = None;
                    this.decoded_preview = None;
                    this.has_more = false;
                    this.next_page = None;
                    this.loading = false;
                    this.total = None;
                    this.page_failed = false;
                    this.pending.clear();
                    this.failed.clear();
                    this.failed_order.clear();
                    this.previews.clear();
                    this.thumbnails.clear();
                    this.tiles.clear();
                    this.motions.clear();
                    this.render_cache.update(cx, |cache, cx| cache.clear(w, cx));
                    this.viewer_render_cache
                        .update(cx, |cache, cx| cache.clear(w, cx));
                    this.source_label = "Photos".into();
                    this.modal = None;
                    this.provider_step = None;
                    this.search.update(cx, |e, cx| e.set_value("", w, cx));
                    if let Some(next) = this.preferences.sources.first().cloned() {
                        this.connect(next, w, cx);
                    }
                } else {
                    this.load_more(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn connect(&mut self, source: Source, w: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            self.error = Some("Operation in progress. Wait before switching libraries.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        self.notice = "Connecting…".into();
        cx.spawn_in(w, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let driver: Arc<dyn PhotosDriver> = Arc::from(storage::open_source(&source)?);
                    let page = driver.page(None)?;
                    Ok::<_, anyhow::Error>((driver, page))
                })
                .await;
            let _ = this.update_in(cx, |this, w, cx| this.finish_connection(result, w, cx));
        })
        .detach();
        cx.notify();
    }
    fn finish_connection(
        &mut self,
        result: anyhow::Result<(Arc<dyn PhotosDriver>, PhotoPage)>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        match result {
            Ok((driver, page)) => {
                let mut preferences = self.preferences.clone();
                preferences.connect(driver.source());
                if let Err(e) = preferences.save(&self.config_path) {
                    self.error = Some(e.to_string());
                    self.load_more(cx);
                    cx.notify();
                    return;
                }
                self.preferences = preferences;
                desert_photos::loading::set_image_sizes(
                    self.preferences.sizes_for(Some(&driver.source().path)),
                );
                self.driver = Some(driver);
                self.details = None;
                self.details_pending = None;
                self.faces.clear();
                self.faces_pending.clear();
                self.caption_for = None;
                self.explore = None;
                self.explore_pending = false;
                self.query_ids = None;
                self.query_pending = None;
                self.remote_search = None;
                self.search_epoch += 1;
                self.decoded_preview = None;
                self.library = page.library;
                self.next_page = page.next;
                self.has_more = self.next_page.is_some();
                self.total = page
                    .total
                    .or_else(|| (!self.has_more).then_some(self.library.photos.len()));
                self.page_failed = false;
                self.source_label = self.driver.as_ref().unwrap().source().label();
                self.library_changed();
                self.selected.clear();
                self.viewer = None;
                self.anchor = None;
                self.filter = Filter::All;
                self.thumbnails.clear();
                self.previews.clear();
                self.tiles.clear();
                self.motions.clear();
                self.pending.clear();
                self.failed.clear();
                self.failed_order.clear();
                self.render_cache.update(cx, |cache, cx| cache.clear(w, cx));
                self.viewer_render_cache
                    .update(cx, |cache, cx| cache.clear(w, cx));
                self.modal = None;
                self.provider_step = None;
                self.generation += 1;
                self.loading = false;
                self.notice.clear();
                self.notice_seen.clear();
                self.library_menu = false;
                self.secret.update(cx, |e, cx| e.set_value("", w, cx));
                self.search.update(cx, |e, cx| {
                    e.set_value("", w, cx);
                    e.set_placeholder("Search photos", w, cx);
                });
                self.reset_gallery();
                self.focus.focus(w);
                self.save_catalog_count();
                self.load_more(cx);
                self.measure_sizes(false, cx);
            }
            Err(e) => {
                self.error = Some(format!("Cannot connect: {e:#}"));
                self.notice.clear();
            }
        }
        self.load_more(cx);
        cx.notify();
    }
    fn library_changed(&mut self) {
        self.revision += 1;
        self.library_epoch += 1;
        self.counts = (
            self.library.photos.iter().filter(|p| !p.deleted).count(),
            self.library.photos.iter().filter(|p| p.deleted).count(),
        );
        self.favorite_count = self
            .library
            .photos
            .iter()
            .filter(|p| !p.deleted && p.favorite)
            .count();
        self.video_count = self
            .library
            .photos
            .iter()
            .filter(|p| !p.deleted && p.duration.is_some())
            .count();
        self.photo_index = self
            .library
            .photos
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id.clone(), i))
            .collect();
        self.update_album_counts();
        *self.visible_cache.borrow_mut() = None;
    }
    fn update_album_counts(&mut self) {
        self.album_counts = self
            .library
            .albums
            .iter()
            .map(|(name, ids)| {
                let count = ids
                    .iter()
                    .filter(|id| {
                        self.photo_index
                            .get(*id)
                            .is_some_and(|&i| !self.library.photos[i].deleted)
                    })
                    .count();
                (name.clone(), count)
            })
            .collect();
    }
    fn choose_provider(&mut self, provider: ProviderKind, w: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !provider.available() {
            return;
        }
        self.error = None;
        match provider {
            ProviderKind::Local => self.choose_folder(w, cx),
            ProviderKind::Apple => {
                let data = self
                    .config_path
                    .parent()
                    .unwrap()
                    .join("sources/apple-photos");
                self.connect(
                    Source {
                        driver: "apple-photos".into(),
                        path: data,
                    },
                    w,
                    cx,
                );
            }
            _ => {
                self.provider_step = Some(provider);
                self.secret.update(cx, |e, cx| e.set_value("", w, cx));
                self.server.update(cx, |e, cx| e.focus(w, cx));
                cx.notify();
            }
        }
    }
    fn connect_remote(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self.provider_step else {
            return;
        };
        if self.busy {
            return;
        }
        let config = Connection {
            provider: provider.id().into(),
            server: self.server.read(cx).value().trim().into(),
            username: if provider == ProviderKind::Nextcloud {
                self.username.read(cx).value().trim().into()
            } else {
                String::new()
            },
            folder: if provider == ProviderKind::Nextcloud {
                self.remote_folder.read(cx).value().trim().into()
            } else {
                String::new()
            },
        };
        if let Err(e) = config.validate() {
            self.error = Some(e.to_string());
            cx.notify();
            return;
        }
        #[cfg(feature = "remote")]
        {
            let secret = self.secret.read(cx).value().to_string();
            let data = self.config_path.parent().unwrap().to_owned();
            self.busy = true;
            self.error = None;
            self.notice = "Connecting…".into();
            cx.spawn_in(w, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let source = config.source(&data)?;
                        let remote =
                            desert_photos::sources::remote::Remote::new(config.clone(), &secret)?;
                        let driver: Arc<dyn PhotosDriver> = Arc::new(
                            desert_photos::sources::ConnectedPhotos::new(source, Box::new(remote))?,
                        );
                        let page = driver.page(None)?;
                        desert_photos::sources::remote::save_connection(&config, &secret, &data)?;
                        Ok::<_, anyhow::Error>((driver, page))
                    })
                    .await;
                let _ = this.update_in(cx, |this, w, cx| this.finish_connection(result, w, cx));
            })
            .detach();
        }
        #[cfg(not(feature = "remote"))]
        {
            let _ = w;
            self.error = Some("Remote providers are not included in this build".into());
        }
        cx.notify();
    }
    /// Only one metadata page may be in flight. Failures keep the cursor for retry.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        if !self.has_more || self.loading || self.busy || self.page_failed {
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        let cursor = self.next_page.clone();
        let generation = self.generation;
        let epoch = self.catalog_epoch;
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                let started=std::time::Instant::now();
                let mut combined=PhotoPage::default(); let mut cursor=cursor; let mut visited=BTreeSet::new();
                for _ in 0..16 {
                    if let Some(token)=&cursor { anyhow::ensure!(visited.insert(token.clone()),"Provider returned a repeated catalog cursor"); }
                    let page=driver.page(cursor.as_deref())?;
                    combined.library.photos.extend(page.library.photos);
                    combined.library.albums.extend(page.library.albums);
                    combined.total=page.total.or(combined.total);
                    combined.next=page.next;
                    if combined.next.is_none() || started.elapsed()>=Duration::from_millis(100) {break;}
                    cursor=combined.next.clone();
                }
                Ok::<_,anyhow::Error>(combined)
            }).await;
            while this.update(cx,|this,_| generation==this.generation && epoch==this.catalog_epoch && this.gallery_zoom.is_some()).unwrap_or(false) {
                cx.background_executor().timer(Duration::from_millis(16)).await;
            }
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation || epoch != this.catalog_epoch { return; }
                this.loading = false;
                match result {
                    Ok(page) => {
                        this.preserve_gallery(cx);
                        let before = this.library.photos.len();
                        for photo in page.library.photos {
                            if this.photo_index.contains_key(&photo.id) { continue; }
                            this.photo_index.insert(photo.id.clone(), this.library.photos.len());
                            this.counts.0 += usize::from(!photo.deleted);
                            this.counts.1 += usize::from(photo.deleted);
                            this.favorite_count += usize::from(!photo.deleted && photo.favorite);
                            this.video_count += usize::from(!photo.deleted && photo.duration.is_some());
                            this.library.photos.push(photo);
                        }
                        this.library.albums.extend(page.library.albums);
                        this.library.synced.extend(page.library.synced);
                        this.next_page = page.next;
                        this.has_more = this.next_page.is_some();
                        if before == this.library.photos.len() && this.has_more {
                            this.page_failed = true;
                            this.error = Some("The server returned no new photos. Retry or reconnect to refresh the library.".into());
                        }
                        this.total = if this.has_more { page.total.or(this.total) } else { Some(this.library.photos.len()) };
                        if !this.has_more { this.save_catalog_count(); }
                        this.revision += 1;
                        this.update_album_counts();
                        *this.visible_cache.borrow_mut() = None;
                    }
                    Err(e) => { this.page_failed = true; this.error = Some(format!("Could not load the next page: {e:#}")); }
                }
                this.load_more(cx);
                cx.notify();
            });
        }).detach();
    }
    fn save_catalog_count(&mut self) {
        if let Some(total) = self.total
            && let Some(driver) = &self.driver
        {
            self.preferences
                .catalog_counts
                .insert(driver.source().path, total);
            if let Err(e) = self.preferences.save(&self.config_path) {
                self.error = Some(format!("Cannot save photo count: {e}"));
            }
        }
    }
    fn choose_folder(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Photo Library Folder".into()),
        });
        cx.spawn_in(w, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await
                && let Some(path) = paths.first()
            {
                let source = Source {
                    driver: "local-folder".into(),
                    path: path.clone(),
                };
                let _ = this.update_in(cx, |this, w, cx| this.connect(source, w, cx));
            }
        })
        .detach();
    }
    fn refresh(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.loading || self.driver.as_ref().is_some_and(|d| d.paged()) {
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        self.loading = true;
        self.scan_epoch += 1;
        let epoch = self.scan_epoch;
        let generation = self.generation;
        cx.spawn_in(w, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { driver.scan() })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.generation != generation || this.scan_epoch != epoch {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(library) => {
                        this.preserve_gallery(cx);
                        this.total = Some(library.photos.len());
                        this.library = library;
                        this.library_changed();
                        this.save_catalog_count();
                        let visible = this
                            .visible(cx)
                            .iter(&this.library)
                            .map(|p| p.id.clone())
                            .collect::<BTreeSet<_>>();
                        this.selected.retain(|id| visible.contains(id));
                        if this.viewer.as_ref().is_some_and(|id| !visible.contains(id)) {
                            this.viewer = None;
                        }
                    }
                    Err(e) => this.error = Some(format!("Cannot refresh this library: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn job<T: Send + 'static>(
        &mut self,
        w: &mut Window,
        cx: &mut Context<Self>,
        work: impl FnOnce(Arc<dyn PhotosDriver>) -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T) + 'static,
    ) {
        if self.busy {
            self.error = Some("Please wait for the current operation to finish.".into());
            cx.notify();
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        self.scan_epoch += 1;
        self.catalog_epoch += 1;
        self.loading = false;
        self.busy = true;
        self.notice = "Working…".into();
        let library = self.library.clone();
        cx.spawn_in(w, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let result = work(driver.clone());
                    let refreshed = driver.refresh_library(library);
                    (result, refreshed)
                })
                .await;
            let _ = this.update_in(cx, |this, w, cx| {
                this.busy = false;
                let (result, refreshed) = result;
                match result {
                    Ok(value) => {
                        this.error = None;
                        done(this, value);
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                // Refresh after failures too: a batch may have completed some files.
                match refreshed {
                    Ok(library) => {
                        this.preserve_gallery(cx);
                        this.library = library;
                        this.library_changed();
                    }
                    Err(e) => this.error = Some(format!("Cannot refresh library: {e}")),
                }
                let _ = w;
                this.load_more(cx);
                cx.notify();
            });
        })
        .detach();
    }
    fn visible(&self, cx: &App) -> PhotoView {
        let query = self.search.read(cx).value().to_lowercase();
        if self.filter == Filter::All && self.counts.1 == 0 && query.is_empty() {
            return PhotoView {
                indices: None,
                count: self.library.photos.len(),
                reversed: self.preferences.bottom_to_top,
            };
        }
        if let Some(cache) = self.visible_cache.borrow().as_ref()
            && cache.revision == self.revision
            && cache.count == self.library.photos.len()
            && cache.filter == self.filter
            && cache.query == query
            && cache.photos.reversed == self.preferences.bottom_to_top
        {
            return cache.photos.clone();
        }
        let recent_cutoff = chrono::Utc::now() - chrono::Duration::days(30);
        // The source's own search results for this query, if they arrived.
        let remote = self
            .remote_search
            .as_ref()
            .filter(|(q, _)| *q == query)
            .map(|(_, ids)| ids.clone());
        let remote = remote.as_deref();
        let indices: Vec<usize> = self
            .library
            .photos
            .iter()
            .enumerate()
            .filter(|(_, p)| match &self.filter {
                Filter::All => !p.deleted,
                Filter::Videos => !p.deleted && p.duration.is_some(),
                Filter::Favorites => !p.deleted && p.favorite,
                Filter::Recent => !p.deleted && p.modified > recent_cutoff,
                Filter::Trash => p.deleted,
                Filter::Album(a) => {
                    !p.deleted
                        && self
                            .library
                            .albums
                            .get(a)
                            .is_some_and(|ids| ids.contains(&p.id))
                }
                Filter::Explore => false,
                Filter::Person(..) | Filter::Place(_) => {
                    !p.deleted
                        && self
                            .query_ids
                            .as_ref()
                            .is_some_and(|(f, ids)| *f == self.filter && ids.contains(&p.id))
                }
            })
            .filter(|(_, p)| {
                query.is_empty()
                    || contains_folded(&p.name, &query)
                    || contains_folded(&p.caption, &query)
                    || contains_folded(&p.id, &query)
                    || remote.is_some_and(|ids| ids.contains(&p.id))
            })
            .map(|(i, _)| i)
            .collect();
        let photos = PhotoView {
            count: indices.len(),
            reversed: self.preferences.bottom_to_top,
            indices: Some(Arc::new(indices)),
        };
        *self.visible_cache.borrow_mut() = Some(VisibleCache {
            revision: self.revision,
            count: self.library.photos.len(),
            filter: self.filter.clone(),
            query,
            photos: photos.clone(),
        });
        photos
    }
    fn set_filter(&mut self, filter: Filter, w: &mut Window, cx: &mut Context<Self>) {
        self.load_query(&filter, cx);
        if matches!(
            filter,
            Filter::Explore | Filter::Person(..) | Filter::Place(_)
        ) && !self.search.read(cx).value().is_empty()
        {
            self.search.update(cx, |e, cx| e.set_value("", w, cx));
        }
        self.filter = filter;
        self.selected.clear();
        self.viewer = None;
        self.anchor = None;
        self.modal = None;
        self.reset_gallery();
        self.focus.focus(w);
        cx.notify();
    }
    fn current(&self) -> Option<Arc<Photo>> {
        self.viewer
            .as_ref()
            .or_else(|| self.selected.iter().next())
            .and_then(|id| {
                self.photo_index
                    .get(id)
                    .and_then(|&i| self.library.photos.get(i))
                    .or_else(|| self.library.photos.iter().find(|p| p.id == *id))
            })
            .cloned()
    }
    fn select(
        &mut self,
        index: usize,
        double: bool,
        modifiers: Modifiers,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if double {
            self.finish_zoom(cx);
        }
        let visible = self.visible(cx);
        let Some(photo) = visible.get(&self.library, index).cloned() else {
            return;
        };
        if modifiers.shift
            && let Some(anchor) = self.anchor
        {
            let from = anchor.min(index);
            let to = anchor.max(index).min(visible.len() - 1);
            if !modifiers.platform {
                self.selected.clear();
            }
            self.selected.extend(
                (from..=to)
                    .filter_map(|i| visible.get(&self.library, i))
                    .map(|p| p.id.clone()),
            );
        } else if modifiers.platform || modifiers.control {
            if !self.selected.remove(&photo.id) {
                self.selected.insert(photo.id.clone());
            }
            self.anchor = Some(index);
        } else {
            self.selected = BTreeSet::from([photo.id.clone()]);
            self.anchor = Some(index);
        }
        // The info panel loads the description itself (saving any edit first).
        if double {
            self.begin_opening(&photo.id, cx);
            self.selected = BTreeSet::from([photo.id.clone()]);
            self.viewer = Some(photo.id.clone());
            self.dismiss_guard = None;
            self.decoded_preview = None;
            self.viewport.reset();
            self.pan_start = None;
            self.sync_zoom(w, cx);
            self.request_image(&photo, true, cx);
        }
        self.focus.focus(w);
        cx.notify();
    }
    fn open_selected(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        self.finish_zoom(cx);
        if let Some(photo) = self.current() {
            self.begin_opening(&photo.id, cx);
            self.selected = BTreeSet::from([photo.id.clone()]);
            self.viewer = Some(photo.id.clone());
            self.dismiss_guard = None;
            self.decoded_preview = None;
            self.viewport.reset();
            self.pan_start = None;
            self.sync_zoom(w, cx);
            self.request_image(&photo, true, cx);
            cx.notify();
        }
    }
    fn step(&mut self, amount: isize, w: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        let photos = self.visible(cx);
        if photos.is_empty() {
            return;
        }
        let current = self.current();
        let index = current
            .and_then(|p| {
                self.photo_index
                    .get(&p.id)
                    .and_then(|&i| photos.position(i))
            })
            .unwrap_or(0);
        let next = (index as isize + amount).clamp(0, photos.len() as isize - 1) as usize;
        let viewer = self.viewer.is_some();
        self.select(next, viewer, Modifiers::none(), w, cx);
    }
    /// A photo's frame on screen in the grid as laid out at `offset`, if it
    /// is visible there.
    fn tile_bounds(
        &self,
        id: &str,
        layout: &GalleryLayout,
        offset: f32,
        cx: &App,
    ) -> Option<Bounds<Pixels>> {
        let &i = self.photo_index.get(id)?;
        let index = self.visible(cx).position(i)?;
        let photo = self.library.photos.get(i)?;
        let bounds = self.scroll_handle().bounds();
        if bounds.size.width <= px(1.) {
            return None;
        }
        let cell = layout.cell(index);
        let frame = gallery::Rect {
            x: cell.x,
            y: cell.y - offset,
            w: cell.edge,
            h: cell.edge,
        };
        let frame = if layout.full_bleed() {
            frame
        } else {
            frame.fit(self.photo_aspect(photo, cx))
        };
        let rect = Bounds::new(
            bounds.origin + point(px(frame.x), px(frame.y)),
            size(px(frame.w), px(frame.h)),
        );
        bounds.intersects(&rect).then_some(rect)
    }
    /// Grows the photo out of its tile into the viewer, when the tile is on
    /// screen. Called just before the viewer opens.
    fn begin_opening(&mut self, id: &str, cx: &mut Context<Self>) {
        self.opening = None;
        if self.viewer.is_some() {
            return;
        }
        let layout = self.gallery_layout(cx);
        let Some(from) = self.tile_bounds(id, &layout, self.gallery_offset(), cx) else {
            return;
        };
        self.closing = None;
        self.closing_epoch += 1;
        let epoch = self.closing_epoch;
        self.opening = Some(OpeningPhoto { from, epoch });
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(OPEN_ANIMATION + Duration::from_millis(30))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.opening.as_ref().is_some_and(|o| o.epoch == epoch) {
                    this.opening = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn close_viewer(&mut self, gesture: bool, w: &mut Window, cx: &mut Context<Self>) {
        self.opening = None;
        // The info panel belongs to the photo: it leaves with it.
        self.inspector = false;
        if let Some(id) = self.viewer.take() {
            let photos = self.visible(cx);
            if let Some(index) = self.photo_index.get(&id).and_then(|&i| photos.position(i)) {
                let cell = self.gallery_layout(cx).cell(index);
                self.gallery_reset = false;
                self.gallery_restore = None;
                let height = f32::from(self.scroll_handle().bounds().size.height);
                self.set_gallery_offset(cell.y + cell.edge / 2. - height / 2.);
            }
        }
        self.viewport.reset();
        self.pinch_armed = false;
        self.dismiss_pinch = None;
        self.sync_zoom(w, cx);
        self.dismiss_guard = gesture.then(std::time::Instant::now);
        self.focus.focus(w);
        cx.notify();
    }
    fn block_dismissed_gesture(&mut self, started: bool) -> bool {
        let Some(last) = self.dismiss_guard else {
            return false;
        };
        let now = std::time::Instant::now();
        if started || now.duration_since(last) > Duration::from_millis(180) {
            self.dismiss_guard = None;
            false
        } else {
            self.dismiss_guard = Some(now);
            true
        }
    }
    fn set_viewer_zoom(
        &mut self,
        target: f32,
        anchor: (f32, f32),
        dismiss: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.viewer.is_none() || !target.is_finite() {
            return;
        }
        if dismiss && self.viewport.zoom <= 1.0001 && target < 1. {
            self.close_viewer_animated(true, w, cx);
            return;
        }
        self.viewport.set_zoom(target.clamp(1., 8.), anchor);
        self.sync_zoom(w, cx);
        cx.notify();
    }
    fn viewer_pinch_began(&mut self) {
        self.pinch_armed = self.viewport.zoom <= 1.0001;
        self.dismiss_pinch = None;
    }
    fn viewer_pinch(
        &mut self,
        delta: f32,
        anchor: (f32, f32),
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.viewer.is_none() {
            return;
        }
        let factor = (1. + delta).max(0.1);
        if self.pinch_armed && self.viewport.zoom <= 1.0001 {
            // Pinching in from fit closes at once, rather than shrinking the
            // photo under the fingers; tiny jitter alone never closes it.
            let scale = self.dismiss_pinch.unwrap_or(1.) * factor;
            if scale < DISMISS_TRIGGER {
                self.pinch_armed = false;
                self.dismiss_pinch = None;
                self.close_viewer_animated(true, w, cx);
                return;
            }
            if scale < 1. {
                self.dismiss_pinch = Some(scale);
                return;
            }
        }
        self.dismiss_pinch = None;
        self.set_viewer_zoom(self.viewport.zoom * factor, anchor, false, w, cx);
    }
    fn viewer_pinch_ended(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.pinch_armed = false;
        self.dismiss_pinch = None;
    }
    /// Close the viewer, the photo flying back into its tile in the gallery
    /// (or, when that tile cannot be shown, shrinking and fading).
    fn close_viewer_animated(&mut self, gesture: bool, w: &mut Window, cx: &mut Context<Self>) {
        if let (Some(image), Some(id)) = (self.viewer_base.clone(), self.viewer.clone()) {
            let fit = self.viewport.fit_size();
            let zoom = self.viewport.zoom;
            let size = size(px(fit.0 * zoom), px(fit.1 * zoom));
            let center = self.viewer_bounds.center()
                + point(
                    px(self.viewport.pan.0 + self.video_inset),
                    px(self.viewport.pan.1),
                );
            self.closing_epoch += 1;
            self.closing = Some(ClosingPhoto {
                image,
                id,
                bounds: Bounds::centered_at(center, size),
                target: None,
                epoch: self.closing_epoch,
            });
            let epoch = self.closing_epoch;
            cx.spawn_in(w, async move |this, cx| {
                cx.background_executor()
                    .timer(OPEN_ANIMATION + Duration::from_millis(30))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.closing.as_ref().is_some_and(|c| c.epoch == epoch) {
                        this.closing = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        self.close_viewer(gesture, w, cx);
    }
    fn can_pan(&self) -> bool {
        let fit = self.viewport.fit_size();
        self.viewport.zoom > 1.
            && (fit.0 * self.viewport.zoom > self.viewport.viewport.0 + 0.5
                || fit.1 * self.viewport.zoom > self.viewport.viewport.1 + 0.5)
    }
    fn viewer_swipe(
        &mut self,
        dx: f32,
        dy: f32,
        phase: TouchPhase,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = std::time::Instant::now();
        let idle = self
            .swipe_last
            .is_none_or(|last| now.duration_since(last) > Duration::from_millis(220));
        self.swipe_last = Some(now);
        if self.swipe_settling {
            return;
        }
        if matches!(phase, TouchPhase::Started)
            || (idle && !self.swipe_active && !matches!(phase, TouchPhase::Ended))
        {
            self.swipe_blocked = false;
            self.swipe_active = false;
            self.swipe_offset = 0.;
        }
        if self.swipe_blocked {
            return;
        }
        if !self.swipe_active && dx.abs() > dy.abs() && dx.abs() > 0. {
            self.swipe_active = true;
        }
        if self.swipe_active {
            let photos = self.visible(cx);
            let index = self
                .viewer
                .as_ref()
                .and_then(|id| self.photo_index.get(id))
                .and_then(|&i| photos.position(i))
                .unwrap_or(0);
            let proposed = self.swipe_offset + dx;
            let edge =
                (proposed > 0. && index == 0) || (proposed < 0. && index + 1 >= photos.len());
            self.swipe_offset = if edge {
                proposed.clamp(-60., 60.)
            } else {
                proposed.clamp(-self.viewport.viewport.0, self.viewport.viewport.0)
            };
            cx.notify();
        }
        if matches!(phase, TouchPhase::Ended) {
            self.finish_viewer_swipe(w, cx);
        } else if self.swipe_active && !self.swipe_idle {
            // Non-phased wheels and interrupted gestures also settle after
            // idle. One watcher serves the whole gesture rather than a timer
            // per scroll event.
            self.swipe_idle = true;
            cx.spawn_in(w, async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                    let done = this
                        .update_in(cx, |this, w, cx| {
                            if !this.swipe_active || this.swipe_settling {
                                this.swipe_idle = false;
                                return true;
                            }
                            let quiet = this
                                .swipe_last
                                .is_none_or(|last| last.elapsed() >= Duration::from_millis(100));
                            if quiet {
                                this.swipe_idle = false;
                                this.finish_viewer_swipe(w, cx);
                            }
                            quiet
                        })
                        .unwrap_or(true);
                    if done {
                        break;
                    }
                }
            })
            .detach();
        }
    }
    fn finish_viewer_swipe(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if !self.swipe_active || self.swipe_settling {
            return;
        }
        self.swipe_blocked = true;
        let width = self.viewport.viewport.0.max(1.);
        let start = self.swipe_offset;
        let photos = self.visible(cx);
        let index = self
            .viewer
            .as_ref()
            .and_then(|id| self.photo_index.get(id))
            .and_then(|&i| photos.position(i))
            .unwrap_or(0);
        let amount = if start < -width.min(600.) * 0.14 && index + 1 < photos.len() {
            1
        } else if start > width.min(600.) * 0.14 && index > 0 {
            -1
        } else {
            0
        };
        self.settle_swipe(amount, w, cx);
    }
    /// Slides from the current swipe offset onto the neighbour `amount` away
    /// (or back to rest for 0), then steps to it. Timed by the clock, so a
    /// slow frame never stretches the slide.
    fn settle_swipe(&mut self, amount: isize, w: &mut Window, cx: &mut Context<Self>) {
        self.swipe_active = false;
        self.swipe_settling = true;
        self.swipe_amount = amount;
        self.swipe_epoch += 1;
        let epoch = self.swipe_epoch;
        let start = self.swipe_offset;
        let width = self.viewport.viewport.0.max(1.);
        let end = -(amount as f32) * width;
        let duration = SWIPE_ANIMATION.mul_f32(((end - start).abs() / width).clamp(0.45, 1.));
        let began = cx.background_executor().now();
        let viewer = self.viewer.clone();
        cx.spawn_in(w, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let keep = this
                    .update_in(cx, |this, w, cx| {
                        if this.swipe_epoch != epoch || this.viewer != viewer {
                            return false;
                        }
                        let now = cx.background_executor().now();
                        let t = (now.duration_since(began).as_secs_f32()
                            / duration.as_secs_f32().max(0.001))
                        .min(1.);
                        this.swipe_offset = start + (end - start) * (1. - (1. - t).powi(3));
                        if t >= 1. {
                            this.complete_swipe(w, cx);
                        }
                        cx.notify();
                        t < 1.
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
    /// Lands a slide: commits its step at once.
    fn complete_swipe(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let amount = std::mem::take(&mut self.swipe_amount);
        self.swipe_offset = 0.;
        self.swipe_settling = false;
        if amount != 0 {
            self.step(amount, w, cx);
        }
    }
    /// Previous/next from the arrows and keys: in the viewer the photo slides
    /// across like a swipe. A press during a slide lands it first.
    fn animate_step(&mut self, amount: isize, w: &mut Window, cx: &mut Context<Self>) {
        if self.viewer.is_none() || self.modal.is_some() || self.viewport.zoom > 1.0001 {
            self.step(amount, w, cx);
            return;
        }
        if self.swipe_settling {
            self.complete_swipe(w, cx);
        }
        let photos = self.visible(cx);
        let index = self
            .viewer
            .as_ref()
            .and_then(|id| self.photo_index.get(id))
            .and_then(|&i| photos.position(i))
            .unwrap_or(0) as isize;
        if !(0..photos.len() as isize).contains(&(index + amount)) {
            return;
        }
        self.opening = None;
        self.swipe_blocked = true;
        self.swipe_offset = 0.;
        self.settle_swipe(amount, w, cx);
    }
    fn sync_zoom(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.swipe_epoch += 1;
        self.swipe_offset = 0.;
        self.swipe_active = false;
        self.swipe_settling = false;
        self.swipe_amount = 0;
        self.pan_start = None;
        self.zoom_input.update(cx, |e, cx| {
            e.set_value(format!("{:.0}%", self.viewport.zoom * 100.), w, cx)
        });
    }
    fn zoom_by(&mut self, factor: f32, w: &mut Window, cx: &mut Context<Self>) {
        if self.viewer.is_none() || self.modal.is_some() {
            return;
        }
        self.set_viewer_zoom(self.viewport.zoom * factor, (0., 0.), true, w, cx);
    }
    fn update_marquee(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let handle = self.scroll_handle();
        let bounds = handle.bounds();
        let offset = handle.offset();
        let end = point(
            position.x.clamp(bounds.left(), bounds.right()),
            position.y.clamp(bounds.top(), bounds.bottom()),
        );
        let Some(m) = &mut self.marquee else {
            return;
        };
        m.end = end;
        let content_end = end - bounds.origin - offset;
        let left = f32::from(m.content_start.x.min(content_end.x));
        let right = f32::from(m.content_start.x.max(content_end.x));
        let top = f32::from(m.content_start.y.min(content_end.y));
        let bottom = f32::from(m.content_start.y.max(content_end.y));
        let base = m.baseline.clone();
        let photos = self.visible(cx);
        let layout = self.gallery_layout(cx);
        self.selected = base;
        for i in layout.visible(top, bottom).into_iter().flatten() {
            let cell = layout.cell(i);
            if cell.x < right
                && cell.x + cell.edge > left
                && cell.y < bottom
                && cell.y + cell.edge > top
                && let Some(photo) = photos.get(&self.library, i)
            {
                self.selected.insert(photo.id.clone());
            }
        }
        cx.notify();
    }
    /// Called for every visible tile on every frame, so the common cases
    /// (already loaded, or no free job) return before allocating anything.
    fn request_image(&mut self, photo: &Arc<Photo>, preview: bool, cx: &mut Context<Self>) {
        if (if preview {
            &self.previews
        } else {
            &self.thumbnails
        })
        .contains_key(&photo.fingerprint)
        {
            return;
        }
        if (!preview && self.pending.len() >= THUMBNAIL_JOBS)
            || (preview
                && self
                    .pending
                    .iter()
                    .filter(|key| key.ends_with("-preview"))
                    .count()
                    >= 2)
        {
            return;
        }
        let key = format!(
            "{}-{}",
            photo.fingerprint,
            if preview { "preview" } else { "thumb" }
        );
        if self.failed.contains(&key) || self.pending.contains(&key) {
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        self.pending.insert(key.clone());
        let generation = self.generation;
        let fingerprint = photo.fingerprint.clone();
        let photo = photo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if preview {
                        driver.preview(&photo)
                    } else {
                        driver.thumbnail(&photo)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.pending.remove(&key);
                match result {
                    Ok(path) => {
                        if preview {
                            this.previews.insert(fingerprint, path);
                        } else {
                            this.thumbnails.insert(fingerprint, path);
                        }
                    }
                    Err(_) => this.remember_failure(key),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn import_paths(&mut self, paths: Vec<PathBuf>, w: &mut Window, cx: &mut Context<Self>) {
        if self.driver.as_ref().is_some_and(|d| !d.can_import()) {
            self.error = Some("Import through the provider app, or choose a local folder.".into());
            cx.notify();
            return;
        }
        if self.modal.is_some() {
            return;
        }
        if self.driver.is_none() {
            if paths.len() == 1 && paths[0].is_dir() {
                self.connect(
                    Source {
                        driver: "local-folder".into(),
                        path: paths[0].clone(),
                    },
                    w,
                    cx,
                );
            } else {
                self.error = Some("Select a photo library first.".into());
                cx.notify();
            }
            return;
        }
        self.job(
            w,
            cx,
            move |driver| driver.import(&paths),
            |this, report| {
                this.notice = format!(
                    "Imported {} · {} duplicates · {} unsupported",
                    report.imported, report.duplicates, report.unsupported
                );
            },
        );
    }
    fn import_picker(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.driver.as_ref().is_some_and(|d| !d.can_import()) {
            self.error = Some("Import through the provider app, or choose a local folder.".into());
            cx.notify();
            return;
        }
        if self.driver.is_none() {
            self.choose_folder(w, cx);
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Import Photos".into()),
        });
        cx.spawn_in(w, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                let _ = this.update_in(cx, |this, w, cx| this.import_paths(paths, w, cx));
            }
        })
        .detach();
    }
    fn favorite(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() || self.selected.is_empty() {
            return;
        }
        let ids = self.selected.iter().cloned().collect::<Vec<_>>();
        let value = !self
            .library
            .photos
            .iter()
            .filter(|p| self.selected.contains(&p.id))
            .all(|p| p.favorite);
        self.job(
            w,
            cx,
            move |driver| driver.favorite(&ids, value),
            |this, _| this.notice = "Favorites updated".into(),
        );
    }
    fn copy_selected(&mut self, cx: &mut Context<Self>) {
        if self.modal.is_some() || self.filter == Filter::Trash {
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        let photos: Vec<_> = self
            .selected
            .iter()
            .filter_map(|id| {
                self.photo_index
                    .get(id)
                    .and_then(|&i| self.library.photos.get(i))
                    .cloned()
            })
            .collect();
        let count = photos.len();
        if count == 0 {
            return;
        }
        self.notice = if native_drag::copy(driver, photos) {
            if count == 1 {
                "Copied photo".into()
            } else {
                format!("Copied {count} photos")
            }
        } else {
            "Could not copy photos".into()
        };
        cx.notify();
    }
    fn favorite_one(&mut self, id: String, w: &mut Window, cx: &mut Context<Self>) {
        let Some(photo) = self.library.photos.iter().find(|p| p.id == id) else {
            return;
        };
        if photo.deleted {
            return;
        }
        let value = !photo.favorite;
        self.job(
            w,
            cx,
            move |driver| driver.favorite(&[id], value),
            |this, _| this.notice = "Favorites updated".into(),
        );
    }
    fn trash(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() || self.selected.is_empty() {
            return;
        }
        if self.filter != Filter::Trash {
            self.pending_delete = self.selected.iter().cloned().collect();
            self.open_modal(Modal::DeleteConfirm, w, cx);
            return;
        }
        self.perform_trash(self.selected.iter().cloned().collect(), false, w, cx);
    }
    fn confirm_delete(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.modal != Some(Modal::DeleteConfirm) {
            return;
        }
        self.modal = None;
        let ids = std::mem::take(&mut self.pending_delete);
        self.perform_trash(ids, true, w, cx);
    }
    fn perform_trash(
        &mut self,
        ids: Vec<String>,
        deleted: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.job(
            w,
            cx,
            move |driver| driver.trash(&ids, deleted),
            move |this, _| {
                this.viewer = None;
                this.selected.clear();
                this.notice = if deleted {
                    "Moved to Recently Deleted".into()
                } else {
                    "Photos restored".into()
                };
            },
        );
    }
    fn open_modal(&mut self, modal: Modal, w: &mut Window, cx: &mut Context<Self>) {
        self.modal = Some(modal);
        self.menu = None;
        if modal == Modal::Album {
            self.name.update(cx, |e, cx| {
                e.set_value("", w, cx);
                e.focus(w, cx);
            });
        } else {
            self.focus.focus(w);
        }
        cx.notify();
    }
    fn create_album(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.modal != Some(Modal::Album) {
            return;
        }
        let name = self.name.read(cx).value().trim().to_owned();
        let ids = self.selected.iter().cloned().collect::<Vec<_>>();
        self.job(
            w,
            cx,
            move |driver| {
                driver.create_album(&name)?;
                driver.add_to_album(&name, &ids)?;
                Ok(name)
            },
            |this, name| {
                this.modal = None;
                this.filter = Filter::Album(name);
                this.viewer = None;
                this.notice = "Album created".into();
            },
        );
    }
    fn add_to_album(
        &mut self,
        name: String,
        ids: Vec<String>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.job(
            w,
            cx,
            move |driver| driver.add_to_album(&name, &ids),
            |this, _| {
                this.modal = None;
                this.notice = "Added to album".into();
            },
        );
    }
    fn export_picker(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            return;
        }
        let ids = self.selected.iter().cloned().collect::<Vec<_>>();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Export Unmodified Originals".into()),
        });
        cx.spawn_in(w, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await
                && let Some(path) = paths.first().cloned()
            {
                let _ = this.update_in(cx, |this, w, cx| {
                    this.job(
                        w,
                        cx,
                        move |driver| driver.export(&ids, &path),
                        |this, count| this.notice = format!("Exported {count} originals"),
                    )
                });
            }
        })
        .detach();
    }
    /// The active library: its service, account, and a menu to switch.
    fn library_switcher(&self, c: Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let source = self.driver.as_ref().map(|d| d.source());
        let provider = source
            .as_ref()
            .map_or(ProviderKind::Local, ProviderKind::of);
        let detail = source
            .as_ref()
            .map(desert_photos::sources::account_detail)
            .unwrap_or_default();
        let entity = cx.entity();
        div()
            .id("library-switcher")
            .debug_selector(|| "library-switcher".into())
            .relative()
            .mb_1()
            .px_2()
            .py(px(6.))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(10.))
            .cursor_pointer()
            .when(self.library_menu, |d| d.bg(rgb(c.soft)))
            .hover(move |s| s.bg(rgb(c.soft)))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        entity.update(cx, |this, _| this.switcher_bounds = bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(provider_logo_sized(provider, 28., c))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(
                                source
                                    .as_ref()
                                    .map_or(self.source_label.clone(), library_name),
                            ),
                    )
                    .when(!detail.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(c.muted))
                                .truncate()
                                .child(detail),
                        )
                    }),
            )
            .child(kit::glyph("chevrons", 14., c.muted))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                this.library_menu = !this.library_menu;
                this.menu = None;
                cx.notify();
            }))
    }
    /// Lists connected libraries for switching, and adds new ones.
    fn library_menu_view(&self, w: &Window, c: Colors, cx: &mut Context<Self>) -> Div {
        let anchor = self.switcher_bounds;
        let width = f32::from(anchor.size.width).max(260.);
        let left = f32::from(anchor.left())
            .min(f32::from(w.viewport_size().width) - width - 8.)
            .max(8.);
        let active = self.driver.as_ref().map(|d| d.source());
        let row = |id: SharedString| {
            div()
                .id(id)
                .w_full()
                .px_2()
                .py(px(6.))
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(c.soft)))
        };
        let mut menu = div()
            .id("library-menu")
            .debug_selector(|| "library-menu".into())
            .absolute()
            .left(px(left))
            .top(anchor.bottom() + px(4.))
            .w(px(width))
            .max_h(px(420.))
            .overflow_y_scroll()
            .p_1()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(c.border))
            .bg(rgb(c.raised))
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c.muted))
                    .child("Photo Libraries"),
            );
        for (i, source) in self.preferences.sources.iter().enumerate() {
            let current = active.as_ref() == Some(source);
            let count = self.preferences.catalog_counts.get(&source.path).copied();
            let detail = desert_photos::sources::account_detail(source);
            let detail = match count {
                Some(n) if detail.is_empty() => format!("{n} items"),
                Some(n) => format!("{detail} · {n} items"),
                None => detail,
            };
            let target = source.clone();
            menu = menu.child(
                row(SharedString::from(format!("switch-library-{i}")))
                    .debug_selector(move || format!("switch-library-{i}"))
                    .child(provider_logo_sized(ProviderKind::of(source), 24., c))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .truncate()
                                    .child(library_name(source)),
                            )
                            .when(!detail.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(c.muted))
                                        .truncate()
                                        .child(detail),
                                )
                            }),
                    )
                    .when(current, |d| d.child(kit::glyph("check", 14., c.accent)))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.library_menu = false;
                        if !current {
                            this.connect(target.clone(), w, cx);
                        }
                        cx.notify();
                    })),
            );
        }
        if !self.preferences.sources.is_empty() {
            menu = menu.child(ui::menu_separator(c));
        }
        menu = menu.child(
            row("add-photo-library".into())
                .debug_selector(|| "add-photo-library".into())
                .child(
                    div()
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(kit::glyph("plus-circle", 20., c.accent)),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .child("Add New Photo Library…"),
                )
                .on_click(cx.listener(|this, _, w, cx| {
                    this.library_menu = false;
                    this.settings_page = 0;
                    this.provider_step = None;
                    this.open_modal(Modal::Sources, w, cx);
                })),
        );
        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.library_menu = false;
                    cx.notify();
                }),
            )
            .child(menu.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()))
    }
    fn sidebar_view(&self, c: Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let mut side = ui::sidebar(c)
            .id("photos-sidebar")
            .debug_selector(|| "photos-sidebar".into())
            .w(px(216.))
            .p_2()
            .bg(rgb(c.bg))
            .child(self.library_switcher(c, cx))
            .child(
                kit::nav_item(
                    "library",
                    "photos",
                    "Library",
                    Some(self.counts.0),
                    self.filter == Filter::All,
                    c,
                )
                .on_click(cx.listener(|this, _, w, cx| this.set_filter(Filter::All, w, cx))),
            )
            .child(
                kit::nav_item(
                    "videos",
                    "video",
                    "Videos",
                    Some(self.video_count),
                    self.filter == Filter::Videos,
                    c,
                )
                .on_click(cx.listener(|this, _, w, cx| this.set_filter(Filter::Videos, w, cx))),
            )
            .child(
                kit::nav_item(
                    "favorites",
                    "heart",
                    "Favorites",
                    Some(self.favorite_count),
                    self.filter == Filter::Favorites,
                    c,
                )
                .on_click(cx.listener(|this, _, w, cx| this.set_filter(Filter::Favorites, w, cx))),
            )
            .child(
                kit::nav_item(
                    "recent",
                    "restore",
                    "Last 30 Days",
                    None,
                    self.filter == Filter::Recent,
                    c,
                )
                .on_click(cx.listener(|this, _, w, cx| this.set_filter(Filter::Recent, w, cx))),
            )
            .when(self.explore_supported(), |d| {
                d.child(
                    kit::nav_item(
                        "explore",
                        "explore",
                        "Explore",
                        None,
                        matches!(
                            self.filter,
                            Filter::Explore | Filter::Person(..) | Filter::Place(_)
                        ),
                        c,
                    )
                    .on_click(
                        cx.listener(|this, _, w, cx| this.set_filter(Filter::Explore, w, cx)),
                    ),
                )
            })
            .child(kit::section("Albums", c));
        let mut albums = div()
            .id("albums-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for name in self.library.albums.keys() {
            let album = name.clone();
            let target = name.clone();
            let native_target = name.clone();
            let count = self.album_counts.get(name).copied().unwrap_or(0);
            let item = kit::nav_item(
                SharedString::from(format!("album-{name}")),
                "folder",
                name.clone(),
                Some(count),
                self.filter == Filter::Album(name.clone()),
                c,
            )
            .on_click(cx.listener(move |this, _, w, cx| {
                this.set_filter(Filter::Album(album.clone()), w, cx)
            }));
            // Synced albums change in their own library, never by dropping here.
            if self.library.synced.contains(name) {
                albums = albums.child(item);
                continue;
            }
            albums = albums.child(
                item.on_drop(cx.listener(move |this, drag: &DragPhotos, w, cx| {
                    if this
                        .driver
                        .as_ref()
                        .is_some_and(|d| d.source().path == drag.source)
                    {
                        this.add_to_album(target.clone(), drag.ids.borrow().clone(), w, cx);
                    }
                }))
                .drag_over::<DragPhotos>(move |s, _, _, _| s.bg(rgb(c.soft)))
                .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(rgb(c.soft)))
                .on_drop(cx.listener(
                    move |this, paths: &ExternalPaths, w, cx| {
                        let ids = this
                            .library
                            .photos
                            .iter()
                            .filter(|p| !p.deleted && paths.paths().contains(&p.path))
                            .map(|p| p.id.clone())
                            .collect::<Vec<_>>();
                        if !ids.is_empty() {
                            this.add_to_album(native_target.clone(), ids, w, cx);
                        } else {
                            this.import_paths(paths.paths().to_vec(), w, cx);
                        }
                    },
                )),
            );
        }
        albums = albums.child(
            kit::nav_item("new-album", "plus", "New album", None, false, c)
                .on_click(cx.listener(|this, _, w, cx| this.open_modal(Modal::Album, w, cx))),
        );
        side =
            side.child(albums)
                .child(
                    kit::nav_item(
                        "deleted-photos",
                        "trash",
                        "Recently Deleted",
                        Some(self.counts.1),
                        self.filter == Filter::Trash,
                        c,
                    )
                    .on_click(cx.listener(|this, _, w, cx| this.set_filter(Filter::Trash, w, cx))),
                )
                .child(
                    div()
                        .mt_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgb(c.border))
                        .child(
                            kit::nav_item("sources", "drive", "Photo sources", None, false, c)
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.open_modal(Modal::Sources, w, cx)
                                })),
                        ),
                );
        side
    }
    fn prepare_photo_drag(&self, drag: &DragPhotos) {
        let ids: Vec<String> = if self.selected.contains(&drag.seed) {
            self.selected.iter().cloned().collect()
        } else {
            vec![drag.seed.clone()]
        };
        let photos: Vec<_> = ids
            .iter()
            .filter_map(|id| {
                self.photo_index
                    .get(id)
                    .and_then(|&i| self.library.photos.get(i))
                    .cloned()
            })
            .collect();
        *drag.ids.borrow_mut() = ids;
        *drag.paths.borrow_mut() = photos
            .iter()
            .filter(|p| !p.path.as_os_str().is_empty())
            .map(|p| p.path.clone())
            .collect();
        if std::env::var_os("DESERT_PHOTOS_TRACE_DRAGS").is_some() {
            eprintln!("Photos drag: GPUI prepared {} photos", photos.len());
        }
        *drag.photos.borrow_mut() = photos;
    }
    fn handoff_photo_drag(
        &mut self,
        event: &DragMoveEvent<DragPhotos>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx).clone();
        if self.start_external_drag(&drag, event.event.position, w) {
            Self::finish_external_handoff(w, cx);
            cx.stop_propagation();
        }
    }
    fn finish_external_handoff(w: &mut Window, cx: &mut App) {
        // Defer because GPUI sets active_drag after the on_drag constructor.
        w.defer(cx, |w, cx| {
            cx.stop_active_drag(w);
        });
    }

    fn start_external_drag(&self, drag: &DragPhotos, at: Point<Pixels>, w: &Window) -> bool {
        if !native_drag::should_handoff(w, at) {
            return false;
        }
        let photos = drag.photos.borrow().clone();
        let previews = photos
            .iter()
            .map(|p| self.thumbnails.peek(&p.fingerprint).cloned())
            .collect();
        native_drag::start(w, drag.driver.clone(), photos, previews)
    }
    fn tile(
        &mut self,
        photo: &Arc<Photo>,
        index: usize,
        layout: &GalleryLayout,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        let edge = layout.edge;
        let motion = if layout.columns <= MOTION_TILE_COLUMNS {
            self.motion_path(photo, false, cx)
        } else {
            None
        };
        let request_photo = photo.clone();
        let request_entity = cx.entity();
        let selected = self.selected.contains(&photo.id);
        let thumb = self.thumbnails.get(&photo.fingerprint).cloned();
        let failed = thumb.is_none()
            && self
                .failed
                .contains(&format!("{}-thumb", photo.fingerprint));
        let drag = DragPhotos {
            ids: Default::default(),
            seed: photo.id.clone(),
            photos: Default::default(),
            driver: self.driver.as_ref().unwrap().clone(),
            paths: Default::default(),
            source: self.driver.as_ref().unwrap().source().path,
            offset: point(px(0.), px(0.)),
            preview: thumb.clone(),
            colors: c,
        };
        let heart_id = photo.id.clone();
        let entity = cx.weak_entity();
        // The closing photo lands here; its tile appears as it arrives.
        let landing = self
            .closing
            .as_ref()
            .is_some_and(|c| c.id == photo.id && matches!(c.target, Some(Some(_))));
        // Gutters show each photo whole at its own aspect ratio, centred in
        // its square cell; gapless grids crop to squares.
        let frame = gallery::Rect {
            x: 0.,
            y: 0.,
            w: edge,
            h: edge,
        };
        let frame = if layout.full_bleed() {
            frame
        } else {
            frame.fit(self.photo_aspect(photo, cx))
        };
        div()
            .w(px(edge))
            .h(px(edge))
            .flex_shrink_0()
            .relative()
            .when(landing, |d| d.opacity(0.))
            .child(
                div()
                    .id(SharedString::from(format!("photo-{}", photo.id)))
                    .debug_selector(move || format!("photo-tile-{index}"))
                    .absolute()
                    .left(px(frame.x))
                    .top(px(frame.y))
                    .group("photo-tile")
                    .w(px(frame.w))
                    .h(px(frame.h))
                    .rounded(px(layout.radius))
                    .overflow_hidden()
                    .when(!layout.full_bleed() || selected, |d| d.border_2())
                    .border_color(if selected {
                        rgb(c.accent).into()
                    } else {
                        transparent_black()
                    })
                    .bg(rgb(c.bg))
                    .hover(|s| s.opacity(0.87))
                    .child(
                        canvas(
                            move |_, _, cx| {
                                request_entity.update(cx, |this, cx| {
                                    this.request_image(&request_photo, false, cx)
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(if let Some(path) = thumb {
                        img(Arc::<std::path::Path>::from(path))
                            .image_cache(&self.render_cache)
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .into_any_element()
                    } else {
                        div()
                            .size_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .child(kit::glyph("photos", 28., c.muted))
                            .when(failed, |d| {
                                d.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(c.muted))
                                        .child("Preview unavailable"),
                                )
                            })
                            .into_any_element()
                    })
                    .when_some(motion, |d, path| {
                        // Moving photos play in large tiles.
                        d.child(
                            img(Arc::<std::path::Path>::from(path))
                                .image_cache(&self.motion_cache)
                                .absolute()
                                .inset_0()
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .id(SharedString::from(format!("motion-{}", photo.id))),
                        )
                    })
                    .when_some(photo.duration.filter(|s| *s > 0.), |d, seconds| {
                        let text = (frame.w * 0.075).clamp(9., 13.);
                        d.child(
                            div()
                                .absolute()
                                .bottom(relative(0.05))
                                .right(relative(0.05))
                                .px(px(text * 0.45))
                                .py(px(text * 0.15))
                                .flex()
                                .items_center()
                                .gap(px(text * 0.25))
                                .rounded(px(text * 0.5))
                                .bg(rgba(0x0000008c))
                                .text_size(px(text))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(0xffffff))
                                .child(kit::glyph("play", text * 0.8, 0xffffff))
                                .child(duration_label(seconds)),
                        )
                    })
                    .when(!photo.deleted, |d| {
                        d.child(
                            div()
                                .id("hover-favorite")
                                .debug_selector(move || format!("photo-heart-{index}"))
                                .absolute()
                                .bottom(relative(0.055))
                                .left(relative(0.055))
                                .size(relative(0.08))
                                .cursor_pointer()
                                .child(
                                    img(ImageSource::Resource(Resource::Embedded(
                                        SharedString::from(if photo.favorite {
                                            "photo-heart-filled.svg"
                                        } else {
                                            "photo-heart.svg"
                                        }),
                                    )))
                                    .size_full(),
                                )
                                .when(!photo.favorite, |d| {
                                    d.invisible().group_hover("photo-tile", |s| s.visible())
                                })
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    cx.stop_propagation();
                                    this.favorite_one(heart_id.clone(), w, cx);
                                })),
                        )
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, w, cx| {
                            cx.stop_propagation();
                            let visible = this.visible(cx);
                            if let Some(photo) = visible.get(&this.library, index)
                                && !this.selected.contains(&photo.id)
                            {
                                this.select(index, false, Modifiers::none(), w, cx);
                            }
                            this.menu = Some(e.position);
                            this.focus.focus(w);
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |this, e: &ClickEvent, w, cx| {
                        cx.stop_propagation();
                        this.select(index, e.click_count() == 2, e.modifiers(), w, cx)
                    }))
                    .on_drag(drag, move |drag, offset, w, cx| {
                        let external = entity
                            .update(cx, |this, _| {
                                this.prepare_photo_drag(drag);
                                this.start_external_drag(drag, w.mouse_position(), w)
                            })
                            .unwrap_or(false);
                        if external {
                            Self::finish_external_handoff(w, cx);
                        }
                        let mut preview = drag.clone();
                        preview.offset = offset; // keep compact preview beside the pointer, at every thumbnail size
                        cx.new(|_| preview)
                    }),
            )
    }
    fn thumbnail_budget(&mut self, layout: &GalleryLayout, height: f32, cx: &mut Context<Self>) {
        // The visible working set (both layers during a transition) and the
        // prefetched screens around it are never evicted. Past that, the
        // decoded-byte budget keeps recently seen thumbnails, so scrolling
        // back shows them immediately instead of decoding them again.
        let rows = (height.max(1.) / layout.pitch).ceil() as usize + 2;
        let layers = if self.gallery_zoom.is_some() { 2 } else { 1 };
        let capacity = (rows * layout.columns * layers * 3).max(96);
        self.render_cache
            .update(cx, |cache, _| cache.set_capacity(capacity));
    }
    fn reset_gallery(&mut self) {
        self.gallery_reset = true;
        self.gallery_restore = None;
        self.gallery_zoom = None;
    }
    fn scroll_handle(&self) -> ScrollHandle {
        self.scroll.0.borrow().base_handle.clone()
    }
    fn gallery_offset(&self) -> f32 {
        f32::from(-self.scroll_handle().offset().y).max(0.)
    }
    fn set_gallery_offset(&self, offset: f32) {
        self.scroll_handle()
            .set_offset(point(px(0.), px(-offset.max(0.))));
    }
    fn gallery_layout(&self, cx: &App) -> GalleryLayout {
        self.layout_for(self.gallery_width, self.gallery_level, cx)
    }
    fn layout_for(&self, width: f32, level: usize, cx: &App) -> GalleryLayout {
        let count = self.visible(cx).len();
        let base = GalleryLayout::new(width, level);
        match base.tier {
            Tier::All => base.with_count(count, self.preferences.bottom_to_top),
            tier => {
                let groups = self.gallery_groups(tier, cx);
                base.with_groups(&groups, count, self.preferences.bottom_to_top)
            }
        }
    }
    /// Month (or year) runs of the visible photos, in display order.
    fn gallery_groups(&self, tier: Tier, cx: &App) -> Arc<Vec<Group>> {
        let photos = self.visible(cx);
        let query = self.search.read(cx).value().to_lowercase();
        let mut cache = self.group_cache.borrow_mut();
        let reusable = cache.as_ref().is_some_and(|g| {
            g.epoch == self.library_epoch
                && g.filter == self.filter
                && g.query == query
                && match (&g.indices, &photos.indices) {
                    // Catalog pages only ever append to an unfiltered library.
                    (None, None) => g.count <= photos.count,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                }
        });
        if !reusable {
            *cache = Some(GroupCache {
                epoch: self.library_epoch,
                filter: self.filter.clone(),
                query,
                indices: photos.indices.clone(),
                count: 0,
                source: vec![],
                reversed: false,
                months: Arc::default(),
                years: Arc::default(),
            });
        }
        let g = cache.as_mut().unwrap();
        if g.count != photos.count || g.reversed != photos.reversed || !reusable {
            let library = &self.library;
            let keys = (g.count..photos.count).map(|i| {
                let photo = &library.photos[photos.indices.as_ref().map_or(i, |ix| ix[i])];
                let local = photo.modified.with_timezone(&chrono::Local);
                month_key(local.year(), local.month0())
            });
            extend_groups(&mut g.source, g.count, keys);
            g.count = photos.count;
            g.reversed = photos.reversed;
            let months = if photos.reversed {
                gallery::reverse_groups(&g.source, g.count)
            } else {
                g.source.clone()
            };
            g.years = Arc::new(gallery::years_from_months(&months));
            g.months = Arc::new(months);
        }
        match tier {
            Tier::Years => g.years.clone(),
            _ => g.months.clone(),
        }
    }
    fn capture_gallery_anchor(&self, cx: &App) -> Option<GalleryAnchor> {
        let photos = self.visible(cx);
        if photos.is_empty() {
            return None;
        }
        let layout = self.gallery_layout(cx);
        let scroll = self.scroll_handle();
        let height = f32::from(scroll.bounds().size.height);
        let offset = f32::from(-scroll.offset().y).max(0.);
        let max = (layout.height - height).max(0.);
        let screen = height / 2.;
        let index = layout.nearest(self.gallery_width / 2., offset + screen)?;
        let cell = layout.cell(index);
        Some(GalleryAnchor {
            id: photos.get(&self.library, index)?.id.clone(),
            fallback: index,
            within: ((offset + screen - cell.y) / cell.edge).clamp(0., 1.),
            screen,
            end: if self.preferences.bottom_to_top {
                max - offset < 2.
            } else {
                offset < 2.
            },
        })
    }
    fn preserve_gallery(&mut self, cx: &App) {
        self.finish_zoom(cx);
        if !self.gallery_reset && self.gallery_restore.is_none() {
            self.gallery_restore = self.capture_gallery_anchor(cx);
        }
    }
    fn set_scroll_direction(&mut self, bottom_to_top: bool, cx: &mut Context<Self>) {
        if self.preferences.bottom_to_top == bottom_to_top {
            return;
        }
        self.preserve_gallery(cx);
        // A direction change keeps the same photo visible, even at either end.
        if let Some(anchor) = &mut self.gallery_restore {
            anchor.end = false;
        }
        self.preferences.bottom_to_top = bottom_to_top;
        if let Err(e) = self.preferences.save(&self.config_path) {
            self.error = Some(e.to_string());
        }
        *self.visible_cache.borrow_mut() = None;
        cx.notify();
    }
    /// Starts a library zoom from the grid on screen, centred on it.
    fn begin_zoom(&mut self, cx: &mut Context<Self>) -> bool {
        if self.gallery_zoom.is_some() {
            return true;
        }
        if self.visible(cx).is_empty() {
            return false;
        }
        let layout = self.gallery_layout(cx);
        let scroll = self.scroll_handle();
        let (bounds, offset) = (scroll.bounds(), f32::from(-scroll.offset().y).max(0.));
        let height = f32::from(bounds.size.height).max(1.);
        let title = gallery_title(&layout);
        let body = self.gallery_height.max(height);
        let header = title
            + if self.preferences.bottom_to_top {
                (body - height).max(0.)
            } else {
                0.
            };
        // Every level keeps the spot at the centre of the grid there.
        let content_y = offset + height / 2.;
        let Some(index) = layout.nearest(self.gallery_width / 2., content_y) else {
            return false;
        };
        let cell = layout.cell(index);
        let within = ((content_y - cell.y) / cell.edge).clamp(0., 1.);
        let at_end = (layout.height - height).max(0.) - offset < 2.;
        let level = layout.level;
        let ladder = Ladder::for_width(self.gallery_width, level);
        let slot = ladder.index_of(level);
        let now = cx.background_executor().now();
        self.gallery_zoom = Some(GalleryZoom {
            zoom: Zoom::new(slot, now),
            content: Crossfade::new(slot, now),
            ladder,
            anchor: (index, within),
            center: (self.gallery_width / 2., header + height / 2.),
            area: title + body,
            at_end,
            frames: [(
                level,
                GridFrame {
                    layout,
                    offset,
                    header,
                    height,
                },
            )]
            .into(),
        });
        self.gallery_restore = None;
        self.marquee = None;
        true
    }
    /// `level`'s grid on the zoom surface, scrolled so the spot that was at
    /// the centre of the grid stays there.
    fn zoom_frame(&mut self, level: usize, cx: &App) -> GridFrame {
        let z = self.gallery_zoom.as_ref().unwrap();
        if let Some(frame) = z.frames.get(&level) {
            return frame.clone();
        }
        let layout = self.layout_for(self.gallery_width, level, cx);
        let title = gallery_title(&layout);
        let frame = GridFrame::anchored(
            layout,
            Some(z.anchor),
            z.center.1,
            z.area,
            title,
            self.preferences.bottom_to_top,
            z.at_end,
        );
        let z = self.gallery_zoom.as_mut().unwrap();
        z.frames.insert(level, frame.clone());
        frame
    }
    /// The one way into a library zoom: trackpad pinch, ⌘-scroll, wheel
    /// notches, ⌘+/− and the toolbar. `phased` input reports when the
    /// fingers lift, so it holds until [`ZoomInput::Release`].
    fn zoom_library(&mut self, input: ZoomInput, phased: bool, cx: &mut Context<Self>) {
        if self.viewer.is_some() || self.modal.is_some() || self.driver.is_none() {
            return;
        }
        if input == ZoomInput::Release && self.gallery_zoom.is_none() {
            return;
        }
        if !self.begin_zoom(cx) {
            return;
        }
        let now = cx.background_executor().now();
        let z = self.gallery_zoom.as_mut().unwrap();
        z.zoom.apply(&z.ladder, input, phased, now);
        // The photos start changing as soon as the zoom heads somewhere.
        let goal = z.zoom.goal(z.content.to, &z.ladder);
        z.content.aim(goal, now);
        self.tick_zoom(cx);
        cx.notify();
    }
    /// Ends the zoom on the nearest level, which becomes the real grid.
    /// Scrolling, resizing and opening a photo end it this way too.
    fn finish_zoom(&mut self, cx: &App) {
        let Some(z) = &self.gallery_zoom else {
            return;
        };
        let slot = z.zoom.position.round().clamp(0., z.ladder.max()) as usize;
        let level = z.ladder.levels[slot];
        let frame = self.zoom_frame(level, cx);
        self.gallery_zoom = None;
        self.gallery_level = level;
        self.set_gallery_offset(frame.offset);
        self.gallery_reset = false;
        self.gallery_restore = None;
    }
    /// Runs the zoom's clock while it moves or its photos fade.
    fn tick_zoom(&mut self, cx: &mut Context<Self>) {
        if self.gallery_ticking {
            return;
        }
        let Some(z) = &mut self.gallery_zoom else {
            return;
        };
        z.zoom.resume(cx.background_executor().now());
        self.gallery_ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        let keep = this.step_zoom(cx);
                        if !keep {
                            this.gallery_ticking = false;
                        }
                        cx.notify();
                        keep
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
    fn step_zoom(&mut self, cx: &mut Context<Self>) -> bool {
        let now = cx.background_executor().now();
        let Some(z) = &mut self.gallery_zoom else {
            return false;
        };
        let moving = z.zoom.tick(&z.ladder, now);
        let goal = z.zoom.goal(z.content.to, &z.ladder);
        z.content.aim(goal, now);
        // Done once the size has landed and that level's photos are in.
        if z.zoom
            .settled()
            .is_some_and(|slot| z.content.shown(now) == Some(slot))
        {
            self.finish_zoom(cx);
            return false;
        }
        moving || z.content.fading(now)
    }
    /// The grid while zooming. Each level whose photos are shown (one, or two
    /// crossfading) is laid out at the current size: margined levels in the
    /// slots of one lattice, date grids scaled whole, all about the centre of
    /// the grid. The outgoing level is drawn first, and each level fades as
    /// one layer, so every part of a photo fades together.
    fn transition_view(&mut self, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        let now = cx.background_executor().now();
        let (ladder, position, center, anchor, area, layers) = {
            let z = self.gallery_zoom.as_ref().unwrap();
            let layers = z.content.layers(now);
            (
                z.ladder.clone(),
                z.zoom.position,
                z.center,
                z.anchor.0,
                z.area,
                layers,
            )
        };
        let lattice = match ladder.lattice_pair(position) {
            Some((a, b, _)) => {
                let la = Lattice::of(&self.zoom_frame(ladder.levels[a], cx), anchor);
                let lb = Lattice::of(&self.zoom_frame(ladder.levels[b], cx), anchor);
                zoom::lattice_at(
                    &ladder,
                    position,
                    center,
                    |slot| {
                        if slot == a { la } else { lb }
                    },
                )
            }
            None => None,
        };
        let width = self.gallery_width;
        let on_screen =
            |r: &gallery::Rect| r.x < width && r.y < area && r.x + r.w > 0. && r.y + r.h > 0.;
        let photos = self.visible(cx);
        let any_selected = !self.selected.is_empty();
        let mut missing = vec![];
        let mut decode = vec![];
        let mut painted = false;
        let mut children = Vec::with_capacity(layers.len());
        for (slot, alpha) in layers {
            if alpha <= 0.002 {
                continue;
            }
            let level = ladder.levels[slot];
            let frame = self.zoom_frame(level, cx);
            let margined = GalleryLayout::margined(level);
            let placement = match lattice {
                Some(lattice) if margined => Placement::Slots(lattice),
                _ => Placement::Scaled(ladder.edge(position) / frame.layout.edge),
            };
            let layer = Layer::new(&frame, placement, center, anchor);
            let dense = frame.layout.dense();
            painted |= dense;
            let mut tiles: Vec<ZoomTile> = vec![];
            {
                let cache = self.render_cache.read(cx);
                for i in layer.visible(area).into_iter().flatten() {
                    let Some(photo) = photos.get(&self.library, i) else {
                        continue;
                    };
                    let aspect = if margined {
                        aspect_of(photo, &self.thumbnails, cache)
                    } else {
                        1.
                    };
                    let r = layer.frame(i, aspect);
                    if !on_screen(&r) {
                        continue;
                    }
                    let pixels = r.w.max(r.h) * self.scale_factor;
                    let image = zoom_image(
                        &mut self.tiles,
                        &self.thumbnails,
                        cache,
                        photo,
                        pixels,
                        dense,
                    );
                    // Margined photos need their thumbnails, decoded, as they
                    // come into the zoom.
                    if margined {
                        match self.thumbnails.peek(&photo.fingerprint) {
                            Some(path) if cache.ready(path).is_none() => {
                                if decode.len() < ZOOM_DECODES {
                                    decode.push(path.clone());
                                }
                            }
                            None if missing.len() < THUMBNAIL_JOBS => missing.push(photo.clone()),
                            _ => {}
                        }
                    }
                    let selected = any_selected && self.selected.contains(&photo.id);
                    tiles.push((r, image, selected));
                }
            }
            let labels = layer.headers(area).into_iter().map(|h| {
                section_label(h.tier, h.key, h.scale, c)
                    .absolute()
                    .left(px(h.rect.x))
                    .top(px(h.rect.y))
                    .w(px(h.rect.w))
                    .h(px(h.rect.h))
                    .into_any_element()
            });
            children.push(
                div()
                    .absolute()
                    .inset_0()
                    .when(alpha < 1., |d| d.opacity(alpha))
                    .child(zoom_layer(tiles, margined, layer.radius(), c))
                    .children(labels)
                    .into_any_element(),
            );
        }
        for photo in missing {
            self.request_image(&photo, false, cx);
        }
        if painted {
            self.pump_tiles(cx);
        }
        // Before painting: free evicted squares, and start decoding the
        // thumbnails that came into view.
        let entity = cx.entity();
        let prepare = canvas(
            move |_, w, cx| {
                entity.update(cx, |this, cx| {
                    this.tiles.release(w);
                    if !decode.is_empty() {
                        this.render_cache.update(cx, |cache, cx| {
                            for path in decode {
                                let _ = cache.load(&Resource::from(path), w, cx);
                            }
                        });
                    }
                })
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0();
        div()
            .id("gallery-motion")
            .debug_selector(|| "gallery-motion".into())
            .absolute()
            .inset_0()
            .overflow_hidden()
            .bg(rgb(c.bg))
            .child(prepare)
            .children(children)
            .into_any_element()
    }
    /// A photo's width / height: from its decoded thumbnail (which is already
    /// orientation-corrected) when available, else from catalog metadata.
    fn photo_aspect(&self, photo: &Photo, cx: &App) -> f32 {
        aspect_of(photo, &self.thumbnails, self.render_cache.read(cx))
    }
    /// Paints a gapless grid in one pass from decoded squares: no per-tile
    /// elements or layout, so many thousands of photos stay cheap. The
    /// screens either side are prefetched once the visible tiles are queued.
    #[allow(clippy::too_many_arguments)]
    fn dense_canvas(
        &mut self,
        photos: PhotoView,
        layout: GalleryLayout,
        top: f32,
        bottom: f32,
        viewport: f32,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let radius = layout.radius;
        canvas(
            move |bounds, w, cx| {
                let class = Square::for_pixels(layout.edge * w.scale_factor());
                entity.update(cx, |this, cx| {
                    this.tiles.release(w);
                    this.scale_factor = w.scale_factor();
                    let any_selected = !this.selected.is_empty();
                    let landing = this
                        .closing
                        .as_ref()
                        .filter(|c| matches!(c.target, Some(Some(_))))
                        .map(|c| c.id.clone());
                    let mut tiles = Vec::with_capacity(layout.columns * 64);
                    for i in layout.visible(top, bottom).into_iter().flatten() {
                        let Some(photo) = photos.get(&this.library, i) else {
                            continue;
                        };
                        let cell = layout.cell(i);
                        let image = this
                            .tiles
                            .get(photo, class)
                            .filter(|_| landing.as_ref() != Some(&photo.id));
                        let selected = any_selected && this.selected.contains(&photo.id);
                        // Video lengths, where the square has room for them.
                        let length = photo
                            .duration
                            .filter(|s| *s > 0. && cell.edge >= 44.)
                            .map(duration_label);
                        tiles.push((
                            Bounds::new(
                                bounds.origin + point(px(cell.x), px(cell.y - top)),
                                size(px(cell.edge), px(cell.edge)),
                            ),
                            image,
                            selected,
                            length,
                        ));
                    }
                    let ahead = viewport * 1.5;
                    let mut prefetch = layout.visible(bottom, bottom + ahead);
                    prefetch.extend(layout.visible((top - ahead).max(0.), top));
                    for i in prefetch.into_iter().flatten() {
                        if let Some(photo) = photos.get(&this.library, i) {
                            this.tiles.want(photo, class);
                        }
                    }
                    this.pump_tiles(cx);
                    tiles
                })
            },
            move |bounds, tiles, w, cx| {
                let corners = Corners::all(px(radius));
                // One layer shares one draw order: tiles never overlap, and this
                // skips GPUI's per-primitive overlap search (the dominant cost
                // with thousands of tiles) and lets every sprite batch together.
                w.paint_layer(bounds, |w| {
                    for (bounds, image, _, _) in &tiles {
                        match image {
                            Some(image) => {
                                let _ = w.paint_image(*bounds, corners, image.clone(), 0, false);
                            }
                            None => {
                                w.paint_quad(fill(*bounds, rgb(c.surface)).corner_radii(corners))
                            }
                        }
                    }
                });
                // Outlines go in a later layer, above the images.
                w.paint_layer(bounds, |w| {
                    for (bounds, _, selected, _) in &tiles {
                        if *selected {
                            w.paint_quad(quad(
                                *bounds,
                                corners,
                                transparent_black(),
                                px(2.),
                                rgb(c.accent),
                                BorderStyle::Solid,
                            ));
                        }
                    }
                });
                // Video lengths sit in the bottom-right corner, as on the
                // margined tiles.
                let font = w.text_style().font();
                for (tile, _, _, length) in &tiles {
                    let Some(length) = length else {
                        continue;
                    };
                    let edge = f32::from(tile.size.width);
                    let text = (edge * 0.11).clamp(9., 12.);
                    let run = TextRun {
                        len: length.len(),
                        font: Font {
                            weight: FontWeight::MEDIUM,
                            ..font.clone()
                        },
                        color: rgb(0xffffff).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let line = w.text_system().shape_line(
                        SharedString::from(length.clone()),
                        px(text),
                        &[run],
                        None,
                    );
                    let (pad_x, height) = (text * 0.45, text * 1.4);
                    let inset = (edge * 0.05).max(3.);
                    let width = f32::from(line.width) + pad_x * 2.;
                    let badge = Bounds::new(
                        tile.origin + point(px(edge - inset - width), px(edge - inset - height)),
                        size(px(width), px(height)),
                    );
                    w.paint_quad(fill(badge, rgba(0x0000008c)).corner_radii(px(height / 2.)));
                    let _ = line.paint(badge.origin + point(px(pad_x), px(0.)), px(height), w, cx);
                }
            },
        )
        .absolute()
        .inset_0()
    }
    /// Starts decoding queued squares on every free worker.
    fn pump_tiles(&mut self, cx: &mut Context<Self>) {
        let Some(driver) = self.driver.clone() else {
            return;
        };
        while self.tiles.in_flight < tiles::workers() {
            let Some((class, batch)) = self.tiles.take_batch() else {
                break;
            };
            self.tiles.in_flight += 1;
            let generation = self.tiles.generation;
            let driver = driver.clone();
            cx.spawn(async move |this, cx| {
                let results = cx
                    .background_executor()
                    .spawn(async move { tiles::decode(driver.as_ref(), class, &batch) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.tiles.generation != generation {
                        return;
                    }
                    this.tiles.in_flight -= 1;
                    this.tiles.finish(class, results);
                    cx.notify();
                });
            })
            .detach();
        }
    }
    fn remember_failure(&mut self, key: String) {
        if self.failed_order.len() >= THUMBNAIL_CACHE
            && let Some(old) = self.failed_order.pop_front()
        {
            self.failed.remove(&old);
        }
        self.failed_order.push_back(key.clone());
        self.failed.insert(key);
    }
    /// The animated rendition of a moving photo, once rendered; requests it
    /// otherwise. Stills are probed once and remembered.
    fn motion_path(
        &mut self,
        photo: &Photo,
        large: bool,
        cx: &mut Context<Self>,
    ) -> Option<PathBuf> {
        if !storage::may_move(&photo.name) {
            return None;
        }
        let key = format!("{}-motion-{large}", photo.fingerprint);
        if let Some(path) = self.motions.get(&key) {
            return Some(path.clone());
        }
        if self.pending.contains(&key)
            || self.failed.contains(&key)
            || self.pending.len() >= THUMBNAIL_JOBS + usize::from(large)
        {
            return None;
        }
        let driver = self.driver.clone()?;
        self.pending.insert(key.clone());
        let generation = self.generation;
        let photo = photo.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { driver.motion(&photo, large) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.pending.remove(&key);
                match result {
                    Ok(Some(path)) => {
                        this.motions.insert(key, path);
                        cx.notify();
                    }
                    _ => this.remember_failure(key),
                }
            });
        })
        .detach();
        None
    }
    /// ⌘-scroll zooms a level at a time, each step animated: a mouse wheel
    /// one per notch; a trackpad as soon as the fingers move, then after each
    /// further stretch of travel the same way. Turning around steps back as
    /// promptly as the first step.
    fn scroll_zoom(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) -> bool {
        if self.viewer.is_some() || self.modal.is_some() {
            return false;
        }
        let now = std::time::Instant::now();
        match event.delta {
            ScrollDelta::Lines(lines) => {
                if lines.y != 0. {
                    let step = ZoomInput::Step(lines.y.signum() as i32);
                    self.zoom_library(step, false, cx);
                }
            }
            ScrollDelta::Pixels(pixels) => {
                match event.touch_phase {
                    TouchPhase::Started => {
                        self.gallery_momentum = None;
                        self.zoom_scroll = None;
                    }
                    // Momentum keeps arriving after the fingers lift; swallow
                    // the stream so the settled level stays put.
                    TouchPhase::Moved
                        if self
                            .gallery_momentum
                            .is_some_and(|t| now - t < Duration::from_millis(150)) =>
                    {
                        self.gallery_momentum = Some(now);
                        return true;
                    }
                    _ => self.gallery_momentum = None,
                }
                let (mut travel, last_step) = match self.zoom_scroll {
                    Some((travel, last_step, at)) if now - at < SCROLL_GESTURE_GAP => {
                        (travel, last_step)
                    }
                    _ => (0., 0.),
                };
                let delta = f32::from(pixels.y);
                if travel * delta < 0. {
                    travel = 0.;
                }
                travel += delta;
                let needed = if travel.signum() == last_step {
                    SCROLL_STEP
                } else {
                    SCROLL_FIRST_STEP
                };
                let mut stepped = last_step;
                if travel.abs() >= needed {
                    stepped = travel.signum();
                    self.zoom_library(ZoomInput::Step(stepped as i32), false, cx);
                    travel = 0.;
                }
                self.zoom_scroll = Some((travel, stepped, now));
                if matches!(event.touch_phase, TouchPhase::Ended) {
                    self.zoom_scroll = None;
                    self.gallery_momentum = Some(now);
                }
            }
        }
        cx.notify();
        true
    }
    fn gallery_step(&mut self, larger: bool, _: &Window, cx: &mut Context<Self>) {
        let step = ZoomInput::Step(if larger { 1 } else { -1 });
        self.zoom_library(step, false, cx);
    }
    fn grid_view(&mut self, available: f32, c: Colors, cx: &mut Context<Self>) -> impl IntoElement {
        if (self.gallery_width - available).abs() > 0.5 {
            self.preserve_gallery(cx);
        }
        self.gallery_width = available;
        let photos = self.visible(cx);
        let layout = self.gallery_layout(cx);
        let bottom_up = self.preferences.bottom_to_top;
        self.tiles.begin_frame();
        let mut content = div()
            .id("photo-grid")
            .debug_selector(|| "photo-grid".into())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden();
        if photos.is_empty() {
            content = content.child(kit::empty(
                "photos",
                if self.loading || self.has_more && !self.page_failed {
                    "Cataloging photos…"
                } else if !self.search.read(cx).value().is_empty() {
                    "No Results"
                } else {
                    match &self.filter {
                        Filter::Favorites => "No Favorites",
                        Filter::Videos => "No Videos",
                        Filter::Recent => "No Recent Photos",
                        Filter::Trash => "No Deleted Photos",
                        _ => "No Photos Yet",
                    }
                },
                "",
                c,
            ));
        } else {
            let handle = self.scroll_handle();
            let viewport = if self.gallery_height > 0. {
                self.gallery_height.min(layout.height)
            } else {
                f32::from(handle.bounds().size.height)
            }
            .max(1.);
            let max = (layout.height - viewport).max(0.);
            let end = if bottom_up { max } else { 0. };
            if self.gallery_reset {
                self.set_gallery_offset(end);
                self.gallery_reset = false;
                self.gallery_restore = None;
            } else if let Some(anchor) = self.gallery_restore.take() {
                if anchor.end {
                    self.set_gallery_offset(end);
                } else {
                    let index = self
                        .photo_index
                        .get(&anchor.id)
                        .and_then(|&i| photos.position(i))
                        .unwrap_or(anchor.fallback.min(photos.len() - 1));
                    let offset = (layout.cell(index).y + anchor.within * layout.edge
                        - anchor.screen)
                        .clamp(0., max);
                    self.set_gallery_offset(offset);
                }
            }
            let offset = self.gallery_offset().min(max);
            // A photo closing from the viewer flies to its tile as laid out now.
            if let Some(id) = self
                .closing
                .as_ref()
                .filter(|c| c.target.is_none())
                .map(|c| c.id.clone())
            {
                let target = self
                    .tile_bounds(&id, &layout, offset, cx)
                    .map(|b| (b, layout.radius));
                if let Some(closing) = &mut self.closing {
                    closing.target = Some(target);
                }
            }
            // Element tiles are built slightly beyond the viewport, so momentum
            // scrolling never reveals an unbuilt edge. Painted grids redraw
            // exactly what is visible every frame and need no margin.
            let margin = if layout.dense() {
                layout.pitch
            } else {
                viewport * 0.25
            };
            let top = (offset - margin).max(0.);
            let bottom = offset + viewport + margin;
            let mut rows = div().relative().w_full().h(px(layout.height));
            let sticky = layout.sticky(offset);
            // While zooming, the zoom covers the grid: it keeps its size and
            // scroll position, but builds and paints no photos.
            let zooming = self.gallery_zoom.is_some();
            if zooming {
                // Nothing under the zoom.
            } else if layout.dense() {
                rows = rows.child(
                    div()
                        .id("dense-grid")
                        .debug_selector(|| "dense-grid".into())
                        .absolute()
                        .left_0()
                        .top(px(top))
                        .w_full()
                        .h(px(bottom - top))
                        .child(self.dense_canvas(
                            photos.clone(),
                            layout.clone(),
                            top,
                            bottom,
                            viewport,
                            c,
                            cx,
                        )),
                );
                for s in layout.sections_in(top, bottom) {
                    if sticky.is_some_and(|(i, _)| i == s) {
                        continue;
                    }
                    let section = layout.sections[s];
                    rows = rows.child(
                        section_label(layout.tier, section.key, 1., c)
                            .id(("gallery-section", s))
                            .debug_selector(move || format!("gallery-section-{s}"))
                            .absolute()
                            .left_0()
                            .top(px(section.top))
                            .w_full()
                            .h(px(layout.header)),
                    );
                }
            } else {
                for range in layout.visible(top, bottom) {
                    for index in range {
                        if let Some(photo) = photos.get(&self.library, index).cloned() {
                            let cell = layout.cell(index);
                            rows = rows.child(
                                self.tile(&photo, index, &layout, c, cx)
                                    .absolute()
                                    .left(px(cell.x))
                                    .top(px(cell.y)),
                            );
                        }
                    }
                }
            }
            // Prefetch the next screens in both directions after the visible
            // tiles have claimed decode slots, so scrolling meets ready images.
            // (Painted grids prefetch inside their own canvas.)
            if !layout.dense() && !zooming {
                let ahead = viewport * 1.5;
                let mut ranges = layout.visible(bottom, bottom + ahead);
                ranges.extend(layout.visible((top - ahead).max(0.), top));
                let photos: Vec<_> = ranges
                    .into_iter()
                    .flatten()
                    .filter_map(|i| photos.get(&self.library, i).cloned())
                    .collect();
                let entity = cx.entity();
                rows = rows.child(
                    canvas(
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                for photo in photos {
                                    if this.pending.len() >= THUMBNAIL_JOBS {
                                        break;
                                    }
                                    this.request_image(&photo, false, cx);
                                }
                            })
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_0(),
                );
            }

            let entity = cx.entity();
            let measured = layout.clone();
            let list_top = if bottom_up {
                (self.gallery_height - layout.height).max(0.)
            } else {
                0.
            };
            content = content.child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .when(bottom_up, |d| d.justify_end())
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                entity.update(cx, |this, cx| {
                                    let full_height = f32::from(bounds.size.height);
                                    this.thumbnail_budget(&measured, full_height, cx);
                                    if (this.gallery_height - full_height).abs() > 0.5 {
                                        this.gallery_height = full_height;
                                        cx.notify();
                                    }
                                    let expected = px(measured.height.min(full_height));
                                    let old = this.scroll_handle().bounds().size.height;
                                    if this.gallery_zoom.is_none()
                                        && old > px(0.)
                                        && (old - expected).abs() > px(0.5)
                                    {
                                        this.preserve_gallery(cx);
                                        cx.notify();
                                    }
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .id("photo-rows")
                            .debug_selector(|| "photo-rows".into())
                            .track_scroll(&handle)
                            .overflow_y_scroll()
                            .w_full()
                            .h(px(layout.height))
                            .max_h_full()
                            .flex_shrink_0()
                            .child(rows),
                    )
                    .child(
                        div()
                            .id("gallery-scrollbar-layer")
                            .debug_selector(|| "gallery-scrollbar-layer".into())
                            .absolute()
                            .inset_0()
                            .child(
                                Scrollbar::vertical(&self.scroll)
                                    .id("gallery-scrollbar")
                                    .scrollbar_show(ScrollbarShow::Always),
                            ),
                    )
                    .when(layout.dense() && !layout.sectioned(), |d| {
                        // Unsorted gapless grid: a floating month/year label.
                        let period = layout
                            .nearest(0., offset + 1.)
                            .and_then(|i| photos.get(&self.library, i))
                            .map(|p| p.modified.with_timezone(&chrono::Local))
                            .map(|d| (d.format("%Y").to_string(), d.format("%B").to_string()));
                        d.when_some(period, |d, (year, month)| {
                            d.child(
                                div()
                                    .id("gallery-date-label")
                                    .debug_selector(|| "gallery-date-label".into())
                                    .absolute()
                                    .top(px(list_top + 12.))
                                    .left_3()
                                    .px_3()
                                    .py_2()
                                    .rounded(px(12.))
                                    .bg(rgba(0x15181dcc))
                                    .text_color(rgb(0xffffff))
                                    .child(div().text_size(px(11.)).child(year))
                                    .child(
                                        div()
                                            .text_size(px(20.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(month),
                                    )
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    }),
                            )
                        })
                    })
                    .when_some(sticky, |d, (s, y)| {
                        // The topmost section's header stays pinned over its
                        // photos until the next header pushes it up.
                        let pinned = y > layout.sections[s].top + 0.5;
                        d.child(
                            section_label(layout.tier, layout.sections[s].key, 1., c)
                                .id("gallery-date-label")
                                .debug_selector(|| "gallery-date-label".into())
                                .absolute()
                                .left_0()
                                .top(px(list_top + y - offset))
                                .w_full()
                                .h(px(layout.header))
                                .when(pinned, |d| d.bg(rgba(c.bg << 8 | 0xe6)))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                        )
                    }),
            );
        }
        if self.gallery_zoom.is_some() {
            content = content.child(self.transition_view(c, cx));
        }
        // ⌘/Ctrl-scroll zooms. It is captured before the photo list sees it, so
        // the grid never scrolls underneath a zoom.
        let zoom_entity = cx.entity();
        content = content.child(
            canvas(
                |_, _, _| (),
                move |bounds, _, w, _| {
                    let entity = zoom_entity.clone();
                    w.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, cx| {
                        if phase != DispatchPhase::Capture
                            || !(event.modifiers.platform || event.modifiers.control)
                            || !bounds.contains(&event.position)
                        {
                            return;
                        }
                        let handled = entity.update(cx, |this, cx| this.scroll_zoom(event, cx));
                        if handled {
                            cx.stop_propagation();
                        }
                    });
                },
            )
            .absolute()
            .inset_0(),
        );
        content
            .on_scroll_wheel(cx.listener(|this, _: &ScrollWheelEvent, _, cx| {
                if this.gallery_zoom.is_some() {
                    this.finish_zoom(cx);
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, w, cx| {
                    if this.modal.is_some() {
                        return;
                    }
                    this.focus.focus(w);
                    // Dense grids are painted, so they are hit-tested here.
                    if let Some(index) = this.dense_hit(e.position, cx) {
                        this.select(index, e.click_count == 2, e.modifiers, w, cx);
                        cx.notify();
                        return;
                    }
                    let baseline =
                        if e.modifiers.platform || e.modifiers.control || e.modifiers.shift {
                            this.selected.clone()
                        } else {
                            BTreeSet::new()
                        };
                    this.selected = baseline.clone();
                    this.anchor = None;
                    let handle = this.scroll_handle();
                    if handle.bounds().contains(&e.position) {
                        this.marquee = Some(Marquee {
                            start: e.position,
                            end: e.position,
                            content_start: e.position - handle.bounds().origin - handle.offset(),
                            baseline,
                        });
                    }
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &MouseDownEvent, w, cx| {
                    let Some(index) = this.dense_hit(e.position, cx) else {
                        return;
                    };
                    let visible = this.visible(cx);
                    if let Some(photo) = visible.get(&this.library, index)
                        && !this.selected.contains(&photo.id)
                    {
                        this.select(index, false, Modifiers::none(), w, cx);
                    }
                    this.menu = Some(e.position);
                    this.focus.focus(w);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if e.dragging() && this.marquee.is_some() {
                    this.update_marquee(e.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.marquee = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.marquee = None;
                    cx.notify();
                }),
            )
    }
    /// The photo under a point of a painted (dense) grid.
    fn dense_hit(&self, position: Point<Pixels>, cx: &App) -> Option<usize> {
        if self.gallery_zoom.is_some() {
            return None;
        }
        let layout = self.gallery_layout(cx);
        let handle = self.scroll_handle();
        let bounds = handle.bounds();
        if !layout.dense() || !bounds.contains(&position) {
            return None;
        }
        let local = position - bounds.origin - handle.offset();
        layout.hit(local.x.into(), local.y.into())
    }
    fn viewer_view(
        &mut self,
        w: &mut Window,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let photo = self.current();
        let mut view = div()
            .id("photo-viewer")
            .debug_selector(|| "photo-viewer".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(rgb(c.bg));
        if let Some(photo) = photo {
            self.request_image(&photo, true, cx);
            let motion = self.motion_path(&photo, true, cx);
            let thumbnail_path = self.thumbnails.get(&photo.fingerprint).cloned();
            let preview_path = self.previews.get(&photo.fingerprint).cloned();
            let thumbnail = thumbnail_path
                .and_then(|path| {
                    self.render_cache
                        .update(cx, |cache, cx| cache.load(&Resource::from(path), w, cx))
                })
                .and_then(|result| result.ok());
            let preview = preview_path.and_then(|path| {
                self.viewer_render_cache
                    .update(cx, |cache, cx| cache.load(&Resource::from(path), w, cx))
            });
            let preview = match preview {
                Some(Ok(image)) => {
                    if self.decoded_preview.as_deref() != Some(&photo.fingerprint) {
                        self.decoded_preview = Some(photo.fingerprint.clone());
                        cx.notify();
                    }
                    Some(image)
                }
                Some(Err(_)) => {
                    self.failed.insert(format!("{}-preview", photo.fingerprint));
                    None
                }
                None => None,
            };
            // A photo whose preview was already decoded when it appeared (a
            // prefetched neighbour) shows it directly, without the fade.
            if self
                .viewer_shown
                .as_ref()
                .is_none_or(|(fingerprint, _)| *fingerprint != photo.fingerprint)
            {
                self.viewer_shown = Some((photo.fingerprint.clone(), preview.is_some()));
            }
            let instant = self.viewer_shown.as_ref().is_some_and(|(_, ready)| *ready);
            let thumbnail = thumbnail.filter(|_| !instant || preview.is_none());
            let base = thumbnail.clone().or_else(|| preview.clone());
            self.viewer_base = preview.clone().or_else(|| thumbnail.clone());
            let viewer_index = self
                .photo_index
                .get(&photo.id)
                .and_then(|&i| self.visible(cx).position(i))
                .unwrap_or(0);
            self.anchor = Some(viewer_index);
            let can_previous = viewer_index > 0;
            let can_next = self.anchor.is_some_and(|i| i + 1 < self.visible(cx).len());
            // Once this photo is sharp, fetch and decode its neighbours'
            // previews so swiping lands on a full image.
            if self.decoded_preview.as_deref() == Some(&photo.fingerprint) {
                let photos = self.visible(cx);
                let neighbours: Vec<_> = [viewer_index + 1, viewer_index.wrapping_sub(1)]
                    .into_iter()
                    .filter_map(|i| photos.get(&self.library, i).cloned())
                    .collect();
                for neighbour in neighbours {
                    match self.previews.get(&neighbour.fingerprint).cloned() {
                        Some(path) => {
                            self.viewer_render_cache
                                .update(cx, |cache, cx| cache.load(&Resource::from(path), w, cx));
                        }
                        None if !self.pending.iter().any(|k| k.ends_with("-preview")) => {
                            self.request_image(&neighbour, true, cx)
                        }
                        None => (),
                    }
                }
            }
            let video = photo
                .duration
                .and_then(|_| self.driver.as_ref()?.video(&photo));
            view = view.child(
                div()
                    .id("viewer-toolbar")
                    .debug_selector(|| "viewer-toolbar".into())
                    .relative()
                    .h(px(if f32::from(self.viewer_bounds.size.width) < 600. {
                        78.
                    } else {
                        48.
                    }))
                    .when(f32::from(self.viewer_bounds.size.width) < 600., |d| {
                        d.pt(px(30.))
                    })
                    .px_3()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .bg(rgb(c.surface))
                    .child(
                        kit::tool("close-viewer", "back", "Back to library · Esc", c).on_click(
                            cx.listener(|this, _, w, cx| this.close_viewer_animated(false, w, cx)),
                        ),
                    )
                    .child(div().flex_1().min_w_0().when(self.image_loading(), |d| {
                        d.child(progress::indicator(
                            "Loading full image…".into(),
                            None,
                            f32::from(self.viewer_bounds.size.width) > 900.,
                            c,
                        ))
                    }))
                    .child(
                        div()
                            .id("viewer-date")
                            .debug_selector(|| "viewer-date".into())
                            .absolute()
                            .top(px(if f32::from(self.viewer_bounds.size.width) < 600. {
                                8.
                            } else {
                                16.
                            }))
                            .left(relative(
                                if f32::from(self.viewer_bounds.size.width) < 600. {
                                    0.05
                                } else {
                                    0.25
                                },
                            ))
                            .w(relative(
                                if f32::from(self.viewer_bounds.size.width) < 600. {
                                    0.9
                                } else {
                                    0.5
                                },
                            ))
                            .text_center()
                            .text_size(px(12.))
                            .truncate()
                            .child(
                                photo
                                    .modified
                                    .with_timezone(&chrono::Local)
                                    .format("%b %-d, %Y · %-I:%M:%S %p")
                                    .to_string(),
                            ),
                    )
                    .child(div().flex_1().min_w_0())
                    .child(
                        kit::tool("zoom-out", "minus", "Zoom out · ⌘−", c)
                            .on_click(cx.listener(|this, _, w, cx| this.zoom_by(1. / 1.2, w, cx))),
                    )
                    .child(
                        div().w(px(64.)).child(
                            Input::new(&self.zoom_input)
                                .appearance(false)
                                .bordered(false)
                                .text_size(px(12.)),
                        ),
                    )
                    .child(
                        kit::tool("zoom-in", "plus", "Zoom in · ⌘+", c)
                            .on_click(cx.listener(|this, _, w, cx| this.zoom_by(1.2, w, cx))),
                    )
                    .child(
                        kit::tool("viewer-info", "info", "Photo details", c).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.inspector = !this.inspector;
                                cx.notify();
                            },
                        )),
                    ),
            );
            // Videos fit between the arrows, which the native player would
            // otherwise cover.
            let inset = if photo.duration.is_some() { 64. } else { 0. };
            self.video_inset = inset;
            let viewport = &mut self.viewport;
            viewport.image = if photo.width > 0 && photo.height > 0 {
                (photo.width as f32, photo.height as f32)
            } else if let Some(image) = &base {
                let size = image.size(0);
                (size.width.0 as f32, size.height.0 as f32)
            } else {
                (1., 1.)
            };
            let fit = viewport.fit_size();
            let shown = viewport.zoom;
            let size = (fit.0 * shown, fit.1 * shown);
            let position = (
                inset + (viewport.viewport.0 - size.0) / 2. + viewport.pan.0,
                (viewport.viewport.1 - size.1) / 2. + viewport.pan.1,
            );
            let canvas_origin = self.viewer_bounds.origin;
            self.video_frame = video.clone().map(|source| {
                (
                    source,
                    Bounds::new(
                        canvas_origin + point(px(position.0), px(position.1)),
                        gpui::size(px(size.0), px(size.1)),
                    ),
                )
            });
            // Growing out of the tile needs the canvas measured; the first
            // viewer ever opened just appears.
            let opening = self
                .opening
                .as_ref()
                .filter(|_| self.viewer_bounds.size.width > px(1.) && fit.0 > 1.)
                .map(|o| {
                    (
                        o.epoch,
                        Bounds::new(o.from.origin - canvas_origin, o.from.size),
                    )
                });
            let entity = cx.entity();
            view = view.child(
                div()
                    .id("viewer-canvas")
                    .debug_selector(|| "viewer-canvas".into())
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .relative()
                    .overflow_hidden()
                    .cursor(if !self.can_pan() {
                        CursorStyle::Arrow
                    } else if self.pan_start.is_some() {
                        CursorStyle::ClosedHand
                    } else {
                        CursorStyle::OpenHand
                    })
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                entity.update(cx, |this, cx| {
                                    let old = this.viewport.viewport;
                                    this.viewer_bounds = bounds;
                                    this.viewport.layout(
                                        (
                                            f32::from(bounds.size.width) - 2. * inset,
                                            bounds.size.height.into(),
                                        ),
                                        this.viewport.image,
                                    );
                                    if old != this.viewport.viewport {
                                        cx.notify();
                                    }
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(if let Some(base) = base {
                        let layers = div()
                            .id("viewer-image-layers")
                            .debug_selector(|| "viewer-image-layers".into())
                            .absolute()
                            .left(px(position.0 + self.swipe_offset))
                            .top(px(position.1))
                            .w(px(size.0))
                            .h(px(size.1))
                            .child(img(base).size_full().object_fit(ObjectFit::Contain))
                            .when_some(preview.filter(|_| thumbnail.is_some()), |d, preview| {
                                d.child(
                                    img(preview)
                                        .absolute()
                                        .inset_0()
                                        .size_full()
                                        .object_fit(ObjectFit::Contain)
                                        .with_animation(
                                            SharedString::from(format!(
                                                "upgrade-{}",
                                                photo.fingerprint
                                            )),
                                            Animation::new(Duration::from_millis(140)),
                                            |image, t| image.opacity(t),
                                        ),
                                )
                            })
                            .when_some(motion, |d, path| {
                                // GIFs and animated WebPs play over their still.
                                d.child(
                                    img(Arc::<std::path::Path>::from(path))
                                        .image_cache(&self.motion_cache)
                                        .absolute()
                                        .inset_0()
                                        .size_full()
                                        .object_fit(ObjectFit::Contain)
                                        .id(SharedString::from(format!(
                                            "viewer-motion-{}",
                                            photo.fingerprint
                                        ))),
                                )
                            })
                            // Where the native player cannot play it (other
                            // platforms), a video opens in the system's player.
                            .when(
                                photo.duration.is_some()
                                    && (video.is_none() || !cfg!(target_os = "macos")),
                                |d| {
                                    let external = photo.clone();
                                    d.child(
                                        div()
                                            .absolute()
                                            .inset_0()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .justify_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .id("play-external")
                                                    .debug_selector(|| "play-external".into())
                                                    .size(px(56.))
                                                    .rounded_full()
                                                    .bg(rgba(0x15181dbb))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgba(0x15181dee)))
                                                    .child(kit::glyph("play", 24., 0xffffff))
                                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                        cx.stop_propagation()
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, w, cx| {
                                                            this.play_externally(
                                                                external.as_ref().clone(),
                                                                w,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py_1()
                                                    .rounded(px(6.))
                                                    .bg(rgba(0x15181dbb))
                                                    .text_size(px(11.))
                                                    .text_color(rgb(0xffffff))
                                                    .child("Play in your video player"),
                                            ),
                                    )
                                },
                            );
                        match opening {
                            Some((epoch, from)) => {
                                let to = Bounds::new(
                                    point(px(position.0), px(position.1)),
                                    gpui::size(px(size.0), px(size.1)),
                                );
                                layers
                                    .with_animation(
                                        ElementId::NamedInteger("opening-photo".into(), epoch),
                                        Animation::new(OPEN_ANIMATION)
                                            .with_easing(ease_out_quint()),
                                        move |d, t| {
                                            let r = lerp_bounds(from, to, t);
                                            d.left(r.origin.x)
                                                .top(r.origin.y)
                                                .w(r.size.width)
                                                .h(r.size.height)
                                        },
                                    )
                                    .into_any_element()
                            }
                            None => layers.into_any_element(),
                        }
                    } else {
                        kit::empty(
                            "photos",
                            if self
                                .failed
                                .contains(&format!("{}-preview", photo.fingerprint))
                            {
                                "Preview unavailable"
                            } else {
                                "Loading preview…"
                            },
                            "",
                            c,
                        )
                        .into_any_element()
                    })
                    .when(self.swipe_offset.abs() > 0.1, |d| {
                        let photos = self.visible(cx);
                        let neighbor = if self.swipe_offset < 0. {
                            viewer_index.checked_add(1)
                        } else {
                            viewer_index.checked_sub(1)
                        };
                        if let Some(photo) =
                            neighbor.and_then(|i| photos.get(&self.library, i)).cloned()
                        {
                            let left = px(self.swipe_offset
                                + if self.swipe_offset < 0. {
                                    self.viewport.viewport.0
                                } else {
                                    -self.viewport.viewport.0
                                });
                            // The prefetched preview when decoded, else the
                            // grid thumbnail.
                            let decoded = self
                                .previews
                                .peek(&photo.fingerprint)
                                .and_then(|path| self.viewer_render_cache.read(cx).ready(path));
                            if let Some(image) = decoded {
                                return d.child(
                                    img(image)
                                        .absolute()
                                        .left(left)
                                        .top_0()
                                        .size_full()
                                        .object_fit(ObjectFit::Contain),
                                );
                            }
                            self.request_image(&photo, false, cx);
                            if let Some(path) = self.thumbnails.get(&photo.fingerprint).cloned() {
                                return d.child(
                                    img(Arc::<std::path::Path>::from(path))
                                        .image_cache(&self.render_cache)
                                        .absolute()
                                        .left(left)
                                        .top_0()
                                        .size_full()
                                        .object_fit(ObjectFit::Contain),
                                );
                            }
                        }
                        d
                    })
                    .child(
                        ui::button("previous-photo", c, false, true)
                            .size(px(42.))
                            .px_0()
                            .absolute()
                            .left_3()
                            .top(relative(0.5))
                            .rounded_full()
                            .bg(rgba(0x15181dbb))
                            .opacity(if can_previous { 1. } else { 0.3 })
                            .child(kit::glyph("back", 22., 0xffffff))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, w, cx| {
                                cx.stop_propagation();
                                if can_previous {
                                    this.animate_step(-1, w, cx);
                                }
                            })),
                    )
                    .child(
                        ui::button("next-photo", c, false, true)
                            .size(px(42.))
                            .px_0()
                            .absolute()
                            .right_3()
                            .top(relative(0.5))
                            .rounded_full()
                            .bg(rgba(0x15181dbb))
                            .opacity(if can_next { 1. } else { 0.3 })
                            .child(kit::glyph("next", 22., 0xffffff))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, w, cx| {
                                cx.stop_propagation();
                                if can_next {
                                    this.animate_step(1, w, cx);
                                }
                            })),
                    )
                    .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, w, cx| {
                        if this.modal.is_some() {
                            return;
                        }
                        let delta = e.delta.pixel_delta(px(28.));
                        if e.modifiers.platform || e.modifiers.control {
                            let at = e.position - this.viewer_bounds.center();
                            this.set_viewer_zoom(
                                this.viewport.zoom * (f32::from(delta.y) * 0.006).exp(),
                                (at.x.into(), at.y.into()),
                                false,
                                w,
                                cx,
                            );
                        } else if !this.can_pan() {
                            let dx = if e.modifiers.shift && delta.x == px(0.) {
                                delta.y
                            } else {
                                delta.x
                            };
                            this.viewer_swipe(
                                dx.into(),
                                if e.modifiers.shift {
                                    0.
                                } else {
                                    delta.y.into()
                                },
                                e.touch_phase,
                                w,
                                cx,
                            );
                        } else if e.modifiers.shift {
                            let horizontal = if delta.x != px(0.) { delta.x } else { delta.y };
                            this.viewport.translate(horizontal.into(), 0.);
                        } else {
                            this.viewport.translate(delta.x.into(), delta.y.into());
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseDownEvent, w, cx| {
                            if this.can_pan() {
                                this.pan_start = Some(e.position);
                            }
                            this.focus.focus(w);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                        if e.dragging()
                            && let Some(previous) = this.pan_start
                        {
                            let delta = e.position - previous;
                            this.viewport.translate(delta.x.into(), delta.y.into());
                            this.pan_start = Some(e.position);
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.pan_start = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.pan_start = None;
                            cx.notify();
                        }),
                    )
                    .when(!self.can_pan(), |canvas| {
                        let drag = DragPhotos {
                            ids: Default::default(),
                            seed: photo.id.clone(),
                            photos: Default::default(),
                            driver: self.driver.as_ref().unwrap().clone(),
                            source: self.driver.as_ref().unwrap().source().path,
                            paths: Default::default(),
                            offset: point(px(0.), px(0.)),
                            preview: self.thumbnails.peek(&photo.fingerprint).cloned(),
                            colors: c,
                        };
                        let entity = cx.weak_entity();
                        canvas.on_drag(drag, move |drag, offset, w, cx| {
                            let external = entity
                                .update(cx, |this, _| {
                                    this.prepare_photo_drag(drag);
                                    this.start_external_drag(drag, w.mouse_position(), w)
                                })
                                .unwrap_or(false);
                            if external {
                                Self::finish_external_handoff(w, cx);
                            }
                            let mut preview = drag.clone();
                            preview.offset = offset;
                            cx.new(|_| preview)
                        })
                    })
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, e: &MouseDownEvent, _, cx| {
                            this.menu = Some(e.position);
                            cx.notify();
                        }),
                    ),
            );
        }
        let _ = w;
        view
    }
    /// Places (or hides) the native player over a video in the viewer. It
    /// shows only while the frame is still: sliding, zooming and the open and
    /// close flights draw the poster instead, and popovers hide it because
    /// native views always cover GPUI's.
    fn sync_video(&mut self, w: &Window) {
        let frame = self.video_frame.take().filter(|_| self.viewer.is_some());
        let still = self.modal.is_none()
            && self.menu.is_none()
            && !self.library_menu
            && self.opening.is_none()
            && self.closing.is_none()
            && !self.swipe_settling
            && self.swipe_offset.abs() < 0.5
            && self.viewport.zoom <= 1.0001;
        video::sync(w, frame.as_ref().map(|(s, b)| (s, *b)), still);
    }
    /// Favorite, album, export and delete for the selection.
    fn selection_actions(&self, docked: bool, c: Colors, cx: &mut Context<Self>) -> Div {
        div()
            .when(docked, |d| d.flex_1())
            .flex()
            .items_center()
            .gap_1()
            .child(
                kit::tool("favorite", "heart", "Favorite · F", c)
                    .when(self.current().is_some_and(|p| p.favorite), |d| {
                        d.bg(rgb(c.soft))
                    })
                    .on_click(cx.listener(|this, _, w, cx| this.favorite(w, cx))),
            )
            .child(
                kit::tool("add-to-album", "folder", "Add to album", c).on_click(
                    cx.listener(|this, _, w, cx| this.open_modal(Modal::AddToAlbum, w, cx)),
                ),
            )
            .when(docked, |d| d.child(div().flex_1()))
            .child(
                kit::tool("export", "export", "Export originals", c)
                    .on_click(cx.listener(|this, _, w, cx| this.export_picker(w, cx))),
            )
            .when(
                matches!(&self.filter, Filter::Album(name) if !self.library.synced.contains(name)),
                |d| {
                    d.child(
                        kit::tool("remove-from-album", "minus", "Remove from album", c).on_click(
                            cx.listener(|this, _, w, cx| {
                                if let Filter::Album(name) = &this.filter {
                                    let name = name.clone();
                                    let ids = this.selected.iter().cloned().collect::<Vec<_>>();
                                    this.job(
                                        w,
                                        cx,
                                        move |driver| driver.remove_from_album(&name, &ids),
                                        |this, _| {
                                            this.selected.clear();
                                            this.notice = "Removed from album".into();
                                        },
                                    );
                                }
                            }),
                        ),
                    )
                },
            )
            .child(
                kit::tool(
                    "delete",
                    if self.filter == Filter::Trash {
                        "restore"
                    } else {
                        "trash"
                    },
                    if self.filter == Filter::Trash {
                        "Restore selection"
                    } else {
                        "Move to Recently Deleted"
                    },
                    c,
                )
                .on_click(cx.listener(|this, _, w, cx| this.trash(w, cx))),
            )
    }
    /// The latest status message, for a few seconds after it changes.
    fn visible_notice(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if self.notice != self.notice_seen {
            self.notice_seen = self.notice.clone();
            self.notice_until = Some(std::time::Instant::now() + NOTICE_TIME);
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(NOTICE_TIME + Duration::from_millis(20))
                    .await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        let shown = !self.busy
            && !self.notice.is_empty()
            && self
                .notice_until
                .is_some_and(|until| std::time::Instant::now() < until);
        shown.then(|| self.notice.clone())
    }
    fn source_choices(&self, w: &Window, c: Colors, cx: &mut Context<Self>) -> Div {
        let mut content = div().flex().flex_col().gap_3();
        if let Some(provider) = self.provider_step {
            content = content
                .child(
                    ui::button("back-to-providers", c, false, !self.busy)
                        .child(kit::glyph("back", 14., c.muted))
                        .child("All services")
                        .on_click(cx.listener(|this, _, w, cx| {
                            if this.busy {
                                return;
                            }
                            this.provider_step = None;
                            this.secret.update(cx, |e, cx| e.set_value("", w, cx));
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(provider_logo(provider, c))
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("Connect {}", provider.name())),
                        ),
                )
                .child(div().text_size(px(12.)).text_color(rgb(c.muted)).child(
                    "Browse your library. Albums, favorites and captions are saved on this device.",
                ))
                .child(div().text_size(px(12.)).child("Server address"))
                .child(Input::new(&self.server));
            if provider == ProviderKind::Nextcloud {
                content = content
                    .child(div().text_size(px(12.)).child("Username"))
                    .child(Input::new(&self.username));
            }
            content = content.child(div().text_size(px(12.)).child(if provider == ProviderKind::Immich { "API key" } else { "App password" }))
                .child(Input::new(&self.secret))
                .child(div().text_size(px(11.)).text_color(rgb(c.muted)).child(if provider == ProviderKind::Immich {
                    "Create an API key in Immich account settings with asset read, view and download permissions."
                } else { "Create an app password in Nextcloud → Personal settings → Security." }));
            if provider == ProviderKind::Nextcloud {
                content = content
                    .child(div().text_size(px(12.)).child("Photo folder (optional)"))
                    .child(Input::new(&self.remote_folder));
            }
            return content.child(
                ui::button("connect-photo-service", c, true, !self.busy)
                    .debug_selector(|| "connect-photo-service".into())
                    .child(if self.busy {
                        "Connecting…"
                    } else {
                        "Connect library"
                    })
                    .on_click(cx.listener(|this, _, w, cx| this.connect_remote(w, cx))),
            );
        }
        if self.busy {
            content = content.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c.muted))
                    .child("Connecting to your library…"),
            );
        }
        let grid = div()
            .id("provider-grid")
            .debug_selector(|| "provider-grid".into())
            .w_full()
            .grid()
            .grid_cols(if w.viewport_size().width < px(560.) {
                1
            } else {
                2
            })
            .gap_2()
            .children(ProviderKind::ALL.into_iter().map(|provider| {
                let id = format!("connect-{}", provider.id());
                let available = provider.available();
                div()
                    .id(SharedString::from(id.clone()))
                    .debug_selector(move || id)
                    .min_w_0()
                    .h(px(72.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(c.border))
                    .bg(rgb(c.raised))
                    .when(available && !self.busy, |d| {
                        d.cursor_pointer()
                            .hover(move |s| s.border_color(rgb(c.accent)))
                            .active(|s| s.opacity(0.75))
                    })
                    .when(!available, |d| d.opacity(0.5))
                    .focusable()
                    .tab_index(0)
                    .focus(move |d| d.border_color(rgb(c.accent)))
                    .child(provider_logo(provider, c))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(provider.name()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(c.muted))
                                    .truncate()
                                    .child(if available {
                                        provider.detail()
                                    } else {
                                        "Unavailable in this build"
                                    }),
                            ),
                    )
                    .child(kit::glyph("next", 14., c.muted))
                    .on_click(
                        cx.listener(move |this, _, w, cx| this.choose_provider(provider, w, cx)),
                    )
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, w, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.choose_provider(provider, w, cx);
                        }
                    }))
            }));
        content = content.child(grid);
        if !self.preferences.sources.is_empty() {
            content = content.child(
                div()
                    .mt_3()
                    .text_size(px(12.))
                    .text_color(rgb(c.muted))
                    .child("Your libraries"),
            );
        }
        for (i, source) in self.preferences.sources.iter().enumerate() {
            let source = source.clone();
            let remove = source.clone();
            let count = self.preferences.catalog_counts.get(&source.path).copied();
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        ui::button(
                            SharedString::from(format!("saved-source-{i}")),
                            c,
                            false,
                            true,
                        )
                        .flex_1()
                        .min_w_0()
                        .h_auto()
                        .py_3()
                        .justify_start()
                        .child(kit::glyph("drive", 20., c.accent))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(div().truncate().child(source.label()))
                                .when_some(count, |d, count| {
                                    d.child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(c.muted))
                                            .child(format!("{count} photos")),
                                    )
                                }),
                        )
                        .on_click(
                            cx.listener(move |this, _, w, cx| this.connect(source.clone(), w, cx)),
                        ),
                    )
                    .child(
                        ui::button(
                            SharedString::from(format!("remove-library-{i}")),
                            c,
                            false,
                            true,
                        )
                        .text_color(rgb(c.danger))
                        .child("Remove")
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.remove_library(remove.clone(), w, cx)
                        })),
                    ),
            );
        }
        content =
            content.child(div().text_size(px(11.)).text_color(rgb(c.muted)).child(
                "Removing a library disconnects it. Your original photos stay where they are.",
            ));
        content
    }
    fn modal_view(
        &self,
        modal: Modal,
        w: &mut Window,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let title = match modal {
            Modal::Sources => "Settings",
            Modal::Browse => "Library & albums",
            Modal::Album => "New album",
            Modal::AddToAlbum => "Add to album",
            Modal::Help => "Tonto Help",
            Modal::DeleteConfirm => "Delete photos",
        };
        let mut content = div()
            .id("modal-content")
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .flex()
            .flex_col()
            .gap_3();
        match modal {
            Modal::Sources => {
                if self.settings_page == 0 {
                    content = content.child(self.source_choices(w, c, cx));
                } else if self.settings_page == 1 {
                    content = content.child(self.appearance_page(w, c, cx));
                } else {
                    content = content.child(self.behavior_page(c, cx));
                }
            }
            Modal::DeleteConfirm => {
                content = content
                    .child(format!(
                        "Move {} {} to Recently Deleted?",
                        self.pending_delete.len(),
                        if self.pending_delete.len() == 1 {
                            "photo"
                        } else {
                            "photos"
                        }
                    ))
                    .child(div().text_size(px(12.)).text_color(rgb(c.muted)).child(
                        "You can restore these photos later. Photos never deletes your originals.",
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                ui::button("cancel-delete", c, false, false)
                                    .child("Cancel")
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.modal = None;
                                        this.pending_delete.clear();
                                        this.focus.focus(w);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                ui::button("confirm-delete", c, true, false)
                                    .bg(rgb(c.danger))
                                    .child("Move to Recently Deleted")
                                    .on_click(
                                        cx.listener(|this, _, w, cx| this.confirm_delete(w, cx)),
                                    ),
                            ),
                    );
            }
            Modal::Browse => {
                content = content.child(div().h(px(340.)).child(self.sidebar_view(c, cx)));
            }
            Modal::Album => {
                content = content
                    .child(Input::new(&self.name))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child(format!("{} photos selected", self.selected.len())),
                    )
                    .child(
                        ui::button("create-album", c, true, false)
                            .child(if self.busy {
                                "Creating…"
                            } else {
                                "Create album"
                            })
                            .on_click(cx.listener(|this, _, w, cx| this.create_album(w, cx))),
                    );
            }
            Modal::AddToAlbum => {
                for name in self
                    .library
                    .albums
                    .keys()
                    .filter(|name| !self.library.synced.contains(*name))
                {
                    let album = name.clone();
                    content = content.child(
                        ui::button(
                            SharedString::from(format!("add-album-{name}")),
                            c,
                            false,
                            true,
                        )
                        .justify_start()
                        .child(kit::glyph("folder", 17., c.accent))
                        .child(name.clone())
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.add_to_album(
                                album.clone(),
                                this.selected.iter().cloned().collect(),
                                w,
                                cx,
                            )
                        })),
                    );
                }
                content = content.child(
                    ui::button("add-new-album", c, true, false)
                        .child("New album…")
                        .on_click(
                            cx.listener(|this, _, w, cx| this.open_modal(Modal::Album, w, cx)),
                        ),
                );
            }
            Modal::Help => {
                content = content.child("Drop photos or folders to import.")
                .child("Double-click or Space  Open viewer\n← / →  Previous / next\n⌘-click  Add to selection\nShift-click  Select range\n⌘A  Select all     F  Favorite\n⌘I  Import     ⌘N  New album\nDelete  Recently Deleted     Esc  Back")
                .child("Drag photos to albums. Restore deleted photos from Recently Deleted.")
                .child("Formats: JPEG, PNG, GIF, WebP, TIFF, BMP.");
            }
        }
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .text_color(rgb(c.danger))
                    .text_size(px(12.))
                    .child(error.clone()),
            );
        }
        let content = if modal == Modal::Sources {
            let narrow = ui::settings_narrow(w);
            let nav = ui::settings_nav(c, narrow)
                .child(
                    kit::nav_item(
                        "settings-libraries",
                        "folder",
                        "Libraries",
                        None,
                        self.settings_page == 0,
                        c,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_page = 0;
                        cx.notify();
                    })),
                )
                .child(
                    kit::nav_item(
                        "settings-appearance",
                        "eye",
                        "Appearance",
                        None,
                        self.settings_page == 1,
                        c,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_page = 1;
                        cx.notify();
                    })),
                )
                .child(
                    kit::nav_item(
                        "settings-behavior",
                        "settings",
                        "Behavior",
                        None,
                        self.settings_page == 2,
                        c,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_page = 2;
                        cx.notify();
                    })),
                );
            ui::settings_shell(narrow)
                .child(nav)
                .child(ui::settings_page(narrow).child(content))
                .into_any_element()
        } else {
            content.into_any_element()
        };
        ui::modal_backdrop().child(
            ui::modal_panel(
                w,
                c,
                if modal == Modal::Sources { 760. } else { 520. },
                if modal == Modal::Sources {
                    Some(520.)
                } else {
                    None
                },
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                if event.keystroke.key == "escape" {
                    this.modal = None;
                    this.focus.focus(w);
                    cx.stop_propagation();
                    cx.notify();
                } else if event.keystroke.key == "tab" && this.provider_step.is_none() {
                    if this.modal == Some(Modal::Album) {
                        this.name.update(cx, |e, cx| e.focus(w, cx));
                    } else {
                        this.focus.focus(w);
                    }
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .p_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(c.border))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        kit::tool("close-modal", "close", "Close", c).on_click(cx.listener(
                            |this, _, w, cx| {
                                this.modal = None;
                                this.focus.focus(w);
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(content),
        )
    }
    fn menu_view(&self, at: Point<Pixels>, w: &Window, c: Colors, cx: &mut Context<Self>) -> Div {
        let mut menu = ui::pointer_menu(w, at, c);
        if self.filter != Filter::Trash {
            menu = menu
                .child(
                    ui::menu_item("menu-open", "Open photo", "Space", c).on_click(cx.listener(
                        |this, _, w, cx| {
                            this.menu = None;
                            this.open_selected(w, cx);
                        },
                    )),
                )
                .child(
                    ui::menu_item(
                        "menu-copy",
                        if self.selected.len() > 1 {
                            "Copy photos"
                        } else {
                            "Copy photo"
                        },
                        "⌘C",
                        c,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        this.copy_selected(cx);
                    })),
                )
                .child(
                    ui::menu_item("menu-favorite", "Favorite / unfavorite", "F", c).on_click(
                        cx.listener(|this, _, w, cx| {
                            this.menu = None;
                            this.favorite(w, cx);
                        }),
                    ),
                )
                .child(
                    ui::menu_item("menu-album", "Add to album…", "", c).on_click(
                        cx.listener(|this, _, w, cx| this.open_modal(Modal::AddToAlbum, w, cx)),
                    ),
                )
                .child(
                    ui::menu_item("menu-export", "Export originals…", "", c).on_click(cx.listener(
                        |this, _, w, cx| {
                            this.menu = None;
                            this.export_picker(w, cx);
                        },
                    )),
                )
                .child(ui::menu_separator(c));
        }
        menu = menu.child(
            ui::menu_item(
                "menu-delete",
                if self.filter == Filter::Trash {
                    "Restore photos"
                } else {
                    "Move to Recently Deleted…"
                },
                "⌫",
                c,
            )
            .text_color(rgb(c.danger))
            .on_click(cx.listener(|this, _, w, cx| {
                this.menu = None;
                this.trash(w, cx);
            })),
        );
        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                }),
            )
            .child(menu)
    }
}
/// Pinching in from fit this far closes the viewer.
const DISMISS_TRIGGER: f32 = 0.97;
struct ClosingPhoto {
    image: Arc<RenderImage>,
    id: String,
    /// Where the photo was in the viewer, in window coordinates.
    bounds: Bounds<Pixels>,
    /// Its tile in the gallery and that tile's corner radius, found when the
    /// gallery first lays out again; `Some(None)` when it is off screen.
    target: Option<Option<(Bounds<Pixels>, f32)>>,
    epoch: u64,
}
struct OpeningPhoto {
    /// The tile the photo grows out of, in window coordinates.
    from: Bounds<Pixels>,
    epoch: u64,
}
fn lerp_bounds(a: Bounds<Pixels>, b: Bounds<Pixels>, t: f32) -> Bounds<Pixels> {
    let mix = |a: Pixels, b: Pixels| a + (b - a) * t;
    Bounds::new(
        point(mix(a.origin.x, b.origin.x), mix(a.origin.y, b.origin.y)),
        size(
            mix(a.size.width, b.size.width),
            mix(a.size.height, b.size.height),
        ),
    )
}
impl Render for PhotosApp {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(not(test))]
        ui::native_titlebar::sync_layout(w);
        let c = self.colors(w);
        let width: f32 = w.viewport_size().width.into();
        let show_sidebar = self.sidebar && width >= 960.;
        let show_info = self.inspector && width >= 900.;
        if std::mem::take(&mut self.sizes_dirty) {
            self.apply_sizes(w, cx);
        }
        let guard_entity = cx.weak_entity();
        let mut root = div()
            .id("photos-app")
            .debug_selector(|| "photos-app".into())
            .key_context("Photos")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(rgb(c.raised))
            .text_color(rgb(c.text))
            .text_size(px(13.))
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, w, _| {
                        let entity = guard_entity.clone();
                        w.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                let blocked = entity
                                    .update(cx, |this, _| {
                                        this.block_dismissed_gesture(matches!(
                                            event.touch_phase,
                                            TouchPhase::Started
                                        ))
                                    })
                                    .unwrap_or(false);
                                if blocked {
                                    cx.stop_propagation();
                                }
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .on_drag_move::<DragPhotos>(
                cx.listener(|this, event, w, cx| this.handoff_photo_drag(event, w, cx)),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                if event.keystroke.key == "escape" && std::mem::take(&mut this.library_menu) {
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if event.keystroke.key == "escape" && this.menu.take().is_some() {
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.modal.is_some() {
                    if event.keystroke.key == "escape" {
                        this.modal = None;
                        this.focus.focus(w);
                        cx.stop_propagation();
                        cx.notify();
                    } else if event.keystroke.key == "tab" && this.modal == Some(Modal::Album) {
                        if this.modal == Some(Modal::Album) {
                            this.name.update(cx, |e, cx| e.focus(w, cx));
                        } else {
                            this.focus.focus(w);
                        }
                        cx.stop_propagation();
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &Import, w, cx| {
                if this.modal.is_none() {
                    this.import_picker(w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenFolder, w, cx| this.choose_folder(w, cx)))
            .on_action(
                cx.listener(|this, _: &Settings, w, cx| this.open_modal(Modal::Sources, w, cx)),
            )
            .on_action(cx.listener(|this, _: &Find, w, cx| {
                if this.modal.is_none() {
                    this.viewer = None;
                    this.search.update(cx, |s, cx| s.focus(w, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                if this.modal.is_none() {
                    this.selected = this
                        .visible(cx)
                        .iter(&this.library)
                        .map(|p| p.id.clone())
                        .collect();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSelected, w, cx| this.open_selected(w, cx)))
            .on_action(cx.listener(|this, _: &Back, w, cx| {
                if this.menu.take().is_none() && this.modal.take().is_none() {
                    if this.viewer.is_some() {
                        this.close_viewer_animated(false, w, cx);
                    } else {
                        this.selected.clear();
                    }
                }
                this.focus.focus(w);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Next, w, cx| this.animate_step(1, w, cx)))
            .on_action(cx.listener(|this, _: &Previous, w, cx| this.animate_step(-1, w, cx)))
            .on_action(cx.listener(|this, _: &Delete, w, cx| this.trash(w, cx)))
            .on_action(cx.listener(|this, _: &Favorite, w, cx| this.favorite(w, cx)))
            .on_action(cx.listener(|this, _: &CopyPhotos, _, cx| this.copy_selected(cx)))
            .on_action(
                cx.listener(|this, _: &NewAlbum, w, cx| this.open_modal(Modal::Album, w, cx)),
            )
            .on_action(cx.listener(|this, _: &CloseWindow, w, cx| {
                if !this.busy {
                    w.remove_window();
                } else {
                    this.error = Some("Operation in progress. Wait before closing.".into());
                    cx.notify();
                }
            }))
            .on_action(|_: &Minimize, w, _| w.minimize_window())
            .on_action(cx.listener(|this, _: &ZoomIn, w, cx| this.zoom_by(1.2, w, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, w, cx| this.zoom_by(1. / 1.2, w, cx)))
            .on_action(cx.listener(|this, _: &FitPhoto, w, cx| {
                this.viewport.reset();
                this.sync_zoom(w, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Quit, _, cx| {
                if this.busy {
                    this.error = Some("Operation in progress. Wait before quitting.".into());
                    cx.notify();
                } else {
                    cx.quit();
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, w, cx| {
                this.import_paths(paths.paths().to_vec(), w, cx)
            }))
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_2().border_color(rgb(c.accent)));
        let mut bar = ui::topbar("topbar", c, true)
            .child(
                kit::tool("toggle-sidebar", "sidebar", "Library & albums", c).on_click(
                    cx.listener(move |this, _, w, cx| {
                        if width < 960. {
                            this.open_modal(Modal::Browse, w, cx);
                        } else {
                            this.sidebar = !this.sidebar;
                            cx.notify();
                        }
                    }),
                ),
            )
            .child(ui::app_brand("brand/tonto.svg", "Tonto", width >= 480.))
            .child(div().flex_1());
        if let Some((label, progress)) = self.loading_status() {
            bar = bar.child(progress::indicator(label, progress, width >= 700., c));
        }
        if self.page_failed {
            bar = bar.child(
                kit::tool("retry-catalog", "refresh", "Retry library catalog", c).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.page_failed = false;
                        this.load_more(cx);
                    }),
                ),
            );
        }
        if self.driver.is_some() && !self.page_failed {
            bar = bar.child(
                kit::tool("refresh-source", "refresh", "Refresh library", c).on_click(cx.listener(
                    |this, _, w, cx| {
                        if let Some(driver) = &this.driver {
                            this.connect(driver.source(), w, cx);
                        }
                    },
                )),
            );
        }
        if self.driver.as_ref().is_some_and(|d| d.can_import()) {
            bar = bar.child(
                kit::tool("import", "import", "Import photos · ⌘I", c)
                    .on_click(cx.listener(|this, _, w, cx| this.import_picker(w, cx))),
            );
        }
        bar =
            bar.child(ui::toolbar_separator(c))
                .when(width >= 520., |d| {
                    d.child(kit::tool("help", "info", "Tonto Help", c).on_click(
                        cx.listener(|this, _, w, cx| this.open_modal(Modal::Help, w, cx)),
                    ))
                })
                .child(
                    kit::tool("settings", "settings", "Libraries & appearance · ⌘,", c).on_click(
                        cx.listener(|this, _, w, cx| this.open_modal(Modal::Sources, w, cx)),
                    ),
                );
        root = root.child(bar);
        if let Some(error) = &self.error {
            root = root.child(
                div()
                    .id("error-banner")
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(rgb(c.soft))
                    .text_size(px(12.))
                    .text_color(rgb(c.danger))
                    .child(div().flex_1().min_w_0().child(error.clone()))
                    .child(
                        kit::tool("dismiss-error", "close", "Dismiss message", c).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.error = None;
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }
        if self.driver.is_none() {
            root = root.child(
                div()
                    .id("onboarding-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .flex()
                    .justify_center()
                    .child(if self.provider_step.is_some() {
                        div()
                            .w_full()
                            .max_w(px(520.))
                            .child(self.source_choices(w, c, cx))
                            .into_any_element()
                    } else {
                        kit::welcome_brand(
                            "brand/tonto.svg",
                            "Welcome to Tonto",
                            "Choose a library to get started.",
                            c,
                        )
                        .child(self.source_choices(w, c, cx))
                        .into_any_element()
                    }),
            );
        } else {
            let available = width
                - if show_sidebar { 216. } else { 0. }
                - if show_info { info::INSPECTOR_WIDTH } else { 0. };
            let mut main = div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .relative()
                .flex()
                .flex_col()
                .when(self.viewer.is_none(), |d| {
                    d.child(
                        div()
                            .h(px(50.))
                            .flex_shrink_0()
                            .px_4()
                            .flex()
                            .items_center()
                            .gap_2()
                            .border_b_1()
                            .border_color(rgb(c.border))
                            .child(
                                div().flex_1().min_w_0().max_w(px(300.)).child(
                                    Input::new(&self.search)
                                        .prefix(kit::glyph("search", 14., c.muted))
                                        .cleanable(true),
                                ),
                            )
                            .child(div().flex_1())
                            .when(available >= 500., |d| {
                                d.child(
                                    kit::tool("smaller-tiles", "minus", "Smaller thumbnails", c)
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            this.gallery_step(false, w, cx);
                                        })),
                                )
                                .child(
                                    kit::tool("larger-tiles", "plus", "Larger thumbnails", c)
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            this.gallery_step(true, w, cx);
                                        })),
                                )
                            })
                            .child(
                                kit::tool("show-info", "info", "Photo details", c)
                                    .when(self.inspector, |d| d.bg(rgb(c.soft)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if width < 900. {
                                            this.notice =
                                                "Photo details require a wider window.".into();
                                        } else {
                                            this.inspector = !this.inspector;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                });
            if self.viewer.is_none() && self.driver.is_some() {
                main = main.child(self.view_header(c, cx));
            }
            main = main.child(if self.viewer.is_some() {
                self.viewer_view(w, c, cx).into_any_element()
            } else if self.filter == Filter::Explore {
                self.explore_view(c, cx)
            } else {
                self.grid_view(available, c, cx).into_any_element()
            });
            if self.viewer.is_some() {
                // The viewer keeps a docked bar: file details and actions.
                let photo = self.current().unwrap();
                main = main.child(
                    div()
                        .id("photo-status")
                        .debug_selector(|| "photo-status".into())
                        .flex_shrink_0()
                        .relative()
                        .bg(rgb(c.surface))
                        .min_h(px(38.))
                        .px_3()
                        .py_1()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .border_t_1()
                        .border_color(rgb(c.border))
                        .text_size(px(11.))
                        .text_color(rgb(c.muted))
                        .child(
                            div()
                                .id("viewer-file-info")
                                .debug_selector(|| "viewer-file-info".into())
                                .absolute()
                                .left(relative(0.25))
                                .w(relative(0.5))
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap_3()
                                .child(div().truncate().min_w_0().child(photo.name.clone()))
                                .child(div().w(px(1.)).h(px(14.)).flex_shrink_0().bg(rgb(c.border)))
                                .child(div().flex_shrink_0().child(
                                    match photo.duration.filter(|s| *s > 0.) {
                                        Some(seconds) => format!(
                                            "{} × {} · {}",
                                            photo.width,
                                            photo.height,
                                            duration_label(seconds)
                                        ),
                                        None => format!("{} × {}", photo.width, photo.height),
                                    },
                                )),
                        )
                        .child(self.selection_actions(true, c, cx)),
                );
            } else if !self.selected.is_empty() {
                // Selecting brings up a floating bar of actions over the grid.
                // It floats so the grid never reflows (or loses its place) as
                // the selection comes and goes; a quick rise draws the eye to
                // it, while clearing the selection removes it at once.
                main = main.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(px(14.))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("selection-bar")
                                .debug_selector(|| "selection-bar".into())
                                .occlude()
                                .h(px(40.))
                                .pl_4()
                                .pr_1()
                                .flex()
                                .items_center()
                                .gap_1()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(rgb(c.border))
                                .bg(rgb(c.raised))
                                .shadow_lg()
                                .text_size(px(12.))
                                .text_color(rgb(c.muted))
                                .child(
                                    div()
                                        .mr_2()
                                        .whitespace_nowrap()
                                        .child(format!("{} selected", self.selected.len())),
                                )
                                .child(self.selection_actions(false, c, cx))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .with_animation(
                                    "selection-bar-in",
                                    Animation::new(Duration::from_millis(160))
                                        .with_easing(ease_out_quint()),
                                    |d, t| d.opacity(t).mb(px(-8. * (1. - t))),
                                ),
                        ),
                );
            } else if let Some(notice) = self.visible_notice(cx) {
                // Results of actions (imports, exports…) show briefly instead.
                main = main.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(px(14.))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("photo-notice")
                                .debug_selector(|| "photo-notice".into())
                                .px_4()
                                .py_2()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(rgb(c.border))
                                .bg(rgb(c.raised))
                                .shadow_lg()
                                .text_size(px(12.))
                                .text_color(rgb(c.muted))
                                .child(notice)
                                .with_animation(
                                    "photo-notice-in",
                                    Animation::new(Duration::from_millis(160))
                                        .with_easing(ease_out_quint()),
                                    |d, t| d.opacity(t),
                                ),
                        ),
                );
            }
            let inspector = show_info.then(|| self.inspector_view(w, c, cx));
            root = root.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .when(show_sidebar, |d| d.child(self.sidebar_view(c, cx)))
                    .child(main)
                    .children(inspector),
            );
        }
        if let Some(m) = &self.marquee {
            root = root.child(
                div()
                    .absolute()
                    .left(m.start.x.min(m.end.x))
                    .top(m.start.y.min(m.end.y))
                    .w((m.end.x - m.start.x).abs())
                    .h((m.end.y - m.start.y).abs())
                    .border_1()
                    .border_color(rgb(c.accent))
                    .bg(rgba((c.accent << 8) | 0x25)),
            );
        }
        if let Some(closing) = &self.closing {
            let from = closing.bounds;
            let target = closing.target.flatten();
            root = root.child(
                div()
                    .id("closing-photo")
                    .debug_selector(|| "closing-photo".into())
                    .absolute()
                    .overflow_hidden()
                    .child(
                        img(closing.image.clone())
                            .size_full()
                            // Lands on square tiles in gapless grids.
                            .object_fit(ObjectFit::Cover),
                    )
                    .with_animation(
                        ElementId::NamedInteger("closing-photo".into(), closing.epoch),
                        Animation::new(OPEN_ANIMATION).with_easing(ease_out_quint()),
                        move |d, t| match target {
                            // Back into its tile, which appears as it lands.
                            Some((to, radius)) => {
                                let r = lerp_bounds(from, to, t);
                                d.left(r.origin.x)
                                    .top(r.origin.y)
                                    .w(r.size.width)
                                    .h(r.size.height)
                                    .rounded(px(radius * t))
                            }
                            // The tile is out of view: shrink and fade.
                            None => {
                                let k = 1. - 0.35 * t;
                                let size = size(from.size.width * k, from.size.height * k);
                                let origin =
                                    from.center() - point(size.width / 2., size.height / 2.);
                                d.left(origin.x)
                                    .top(origin.y)
                                    .w(size.width)
                                    .h(size.height)
                                    .opacity(1. - t)
                            }
                        },
                    ),
            );
        }
        if let Some(at) = self.menu {
            root = root.child(self.menu_view(at, w, c, cx));
        }
        if let Some(modal) = self.modal {
            root = root.child(self.modal_view(modal, w, c, cx));
        }
        if self.library_menu {
            root = root.child(self.library_menu_view(w, c, cx));
        }
        self.sync_video(w);
        root
    }
}
fn register_shortcuts(cx: &mut App) {
    let key = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{key}-i"), Import, Some("Photos")),
        KeyBinding::new(&format!("{key}-o"), OpenFolder, Some("Photos")),
        KeyBinding::new(&format!("{key}-,"), Settings, Some("Photos")),
        KeyBinding::new(&format!("{key}-f"), Find, Some("Photos")),
        KeyBinding::new(&format!("{key}-a"), SelectAll, Some("Photos && !Input")),
        KeyBinding::new(&format!("{key}-n"), NewAlbum, Some("Photos")),
        KeyBinding::new(&format!("{key}-q"), Quit, Some("Photos")),
        KeyBinding::new(&format!("{key}-w"), CloseWindow, Some("Photos")),
        KeyBinding::new(&format!("{key}-m"), Minimize, Some("Photos")),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new(&format!("{key}-="), ZoomIn, Some("Photos && !Input")),
        KeyBinding::new(&format!("{key}-+"), ZoomIn, Some("Photos && !Input")),
        KeyBinding::new(&format!("{key}--"), ZoomOut, Some("Photos && !Input")),
        KeyBinding::new(&format!("{key}-0"), FitPhoto, Some("Photos && !Input")),
        KeyBinding::new("space", OpenSelected, Some("Photos && !Input")),
        KeyBinding::new("enter", OpenSelected, Some("Photos && !Input")),
        KeyBinding::new("escape", Back, Some("Photos")),
        KeyBinding::new("right", Next, Some("Photos && !Input")),
        KeyBinding::new("left", Previous, Some("Photos && !Input")),
        KeyBinding::new("backspace", Delete, Some("Photos && !Input")),
        KeyBinding::new("delete", Delete, Some("Photos && !Input")),
        KeyBinding::new("f", Favorite, Some("Photos && !Input")),
        KeyBinding::new(&format!("{key}-c"), CopyPhotos, Some("Photos && !Input")),
    ]);
}

pub fn run() -> anyhow::Result<()> {
    let project = directories::ProjectDirs::from("com", "pinal", "DesertPhotos")
        .ok_or_else(|| anyhow::anyhow!("Cannot locate application data"))?;
    let data = std::env::var_os("DESERT_PHOTOS_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| project.data_dir().into());
    std::fs::create_dir_all(&data)?;
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx| {
            ui::init(cx);
            cx.on_action(|_: &Hide, cx| cx.hide());
            cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
            register_shortcuts(cx);
            cx.set_menus(vec![
                Menu {
                    name: "Tonto".into(),
                    items: vec![
                        MenuItem::action("Settings…", Settings),
                        MenuItem::separator(),
                        MenuItem::action("Hide Tonto", Hide),
                        MenuItem::action("Hide Others", HideOthers),
                        MenuItem::separator(),
                        MenuItem::action("Quit Tonto", Quit),
                    ],
                },
                Menu {
                    name: "File".into(),
                    items: vec![
                        MenuItem::action("Import…", Import),
                        MenuItem::action("Open Library Folder…", OpenFolder),
                        MenuItem::action("New Album…", NewAlbum),
                        MenuItem::action("Close Window", CloseWindow),
                    ],
                },
                Menu {
                    name: "View".into(),
                    items: vec![
                        MenuItem::action("Search", Find),
                        MenuItem::action("Minimize", Minimize),
                        MenuItem::action("Zoom In", ZoomIn),
                        MenuItem::action("Zoom Out", ZoomOut),
                        MenuItem::action("Fit Photo", FitPhoto),
                        MenuItem::action("Open Selected Photo", OpenSelected),
                    ],
                },
            ]);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1280.), px(820.)),
                        cx,
                    ))),
                    window_min_size: Some(size(px(360.), px(360.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Tonto".into()),
                        appears_transparent: cfg!(target_os = "macos"),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |w, cx| {
                    ui::native_titlebar::install(w);
                    ui::native_transfer::install_magnify(w);
                    let view = cx.new(|cx| PhotosApp::new(data.join("settings.json"), w, cx));
                    cx.new(|cx| Root::new(view, w, cx))
                },
            )
            .expect("Cannot open the Tonto window");
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    Ok(())
}

#[cfg(test)]
mod tests;

/// `haystack.to_lowercase().contains(needle)` for an already-lowercased
/// needle, without allocating for ASCII text (file names, ids, most captions).
/// Search runs this over every photo on each keystroke.
fn contains_folded(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.is_ascii() {
        haystack.len() >= needle.len()
            && haystack
                .as_bytes()
                .windows(needle.len())
                .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
    } else {
        haystack.to_lowercase().contains(needle)
    }
}
