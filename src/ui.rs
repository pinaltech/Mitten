use desert_photos::gallery::{FramePair, GalleryLayout, GridFrame};
use desert_photos::loading::{BoundedCache, THUMBNAIL_CACHE, THUMBNAIL_JOBS, VIEWER_CACHE};
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
use std::{cell::RefCell, collections::BTreeSet, path::PathBuf, sync::Arc, time::Duration};

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
    Favorites,
    Recent,
    Trash,
    Album(String),
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
mod image_cache;
mod native_drag;
mod photo_view;
mod progress;
fn provider_logo(provider: ProviderKind, c: Colors) -> AnyElement {
    if provider == ProviderKind::Local {
        return div()
            .size(px(30.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(kit::glyph("folder", 26., c.accent))
            .into_any_element();
    }
    img(SharedString::from(format!(
        "providers/{}.svg",
        provider.id()
    )))
    .size(px(30.))
    .flex_shrink_0()
    .into_any_element()
}
#[derive(Clone)]
struct GalleryAnchor {
    id: String,
    fallback: usize,
    within: f32,
    screen: f32,
    end: bool,
}
struct MotionTile {
    frames: FramePair,
    source: Option<Arc<Photo>>,
    target: Option<Arc<Photo>>,
    source_image: Option<Arc<RenderImage>>,
    target_image: Option<Arc<RenderImage>>,
}
struct GalleryMotion {
    from: GridFrame,
    to: GridFrame,
    tiles: Arc<Vec<MotionTile>>,
    progress: f32,
    epoch: u64,
    settling: bool,
}
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
    gallery_zoom: f32,
    gallery_width: f32,
    gallery_height: f32,
    gallery_period: String,
    gallery_reset: bool,
    gallery_restore: Option<GalleryAnchor>,
    gallery_pinch: Option<f32>,
    gallery_pinch_epoch: u64,
    gallery_motion: Option<GalleryMotion>,
    gallery_motion_epoch: u64,
    swipe_offset: f32,
    swipe_active: bool,
    swipe_epoch: u64,
    swipe_settling: bool,
    swipe_blocked: bool,
    swipe_last: Option<std::time::Instant>,
    decoded_preview: Option<String>,
    catalog_epoch: u64,
    viewport: ui::image_viewport::ImageViewport,
    viewer_bounds: Bounds<Pixels>,
    dismiss_scroll: f32,
    dismiss_scroll_last: Option<std::time::Instant>,
    dismiss_guard: Option<std::time::Instant>,
    pan_start: Option<Point<Pixels>>,
    zoom_input: Entity<InputState>,
    menu: Option<Point<Pixels>>,
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
    _subscriptions: Vec<Subscription>,
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
        let caption = cx.new(|cx| {
            InputState::new(w, cx)
                .multi_line(true)
                .placeholder("Add a caption…")
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
                    this.reset_gallery();
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
        let render_cache = image_cache::PhotoImageCache::new(96, cx);
        let viewer_render_cache = image_cache::PhotoImageCache::new(3, cx);
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
            previews: BoundedCache::new(VIEWER_CACHE),
            thumbnails: BoundedCache::new(THUMBNAIL_CACHE),
            failed_order: Default::default(),
            revision: 0,
            visible_cache: RefCell::new(None),
            counts: (0, 0),
            favorite_count: 0,
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
            gallery_zoom: 2.,
            gallery_width: 844.,
            gallery_height: 0.,
            gallery_period: String::new(),
            gallery_reset: true,
            gallery_restore: None,
            gallery_pinch: None,
            gallery_pinch_epoch: 0,
            gallery_motion: None,
            gallery_motion_epoch: 0,
            swipe_offset: 0.,
            swipe_active: false,
            swipe_epoch: 0,
            swipe_settling: false,
            swipe_blocked: false,
            swipe_last: None,
            decoded_preview: None,
            catalog_epoch: 0,
            viewport: Default::default(),
            viewer_bounds: Bounds::default(),
            dismiss_scroll: 0.,
            dismiss_scroll_last: None,
            dismiss_guard: None,
            pan_start: None,
            zoom_input,
            menu: None,
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
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this
                    .update_in(cx, |this, w, cx| {
                        let delta = ui::native_transfer::take_magnification(w);
                        if delta != 0. && this.modal.is_none() {
                            if this.block_dismissed_gesture(false) {
                                return;
                            }
                            if this.viewer.is_some() {
                                let at = w.mouse_position() - this.viewer_bounds.center();
                                this.set_viewer_zoom(
                                    this.viewport.zoom * (1. + delta).max(0.1),
                                    (at.x.into(), at.y.into()),
                                    true,
                                    w,
                                    cx,
                                );
                            } else {
                                this.pinch_gallery(delta * 2.6, w.mouse_position(), false, cx);
                            }
                            cx.notify();
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
                self.driver = Some(driver);
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
                self.notice = if self.driver.as_ref().is_some_and(|d| d.paged()) {
                    "Connected · edits are saved in Photos on this device".into()
                } else {
                    String::new()
                };
                self.secret.update(cx, |e, cx| e.set_value("", w, cx));
                self.search.update(cx, |e, cx| {
                    e.set_value("", w, cx);
                    e.set_placeholder("Search photos", w, cx);
                });
                self.reset_gallery();
                self.focus.focus(w);
                self.save_catalog_count();
                self.load_more(cx);
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
            while this.update(cx,|this,_| generation==this.generation && epoch==this.catalog_epoch && this.gallery_motion.is_some()).unwrap_or(false) {
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
                            this.library.photos.push(photo);
                        }
                        this.library.albums.extend(page.library.albums);
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
        let indices: Vec<usize> = self
            .library
            .photos
            .iter()
            .enumerate()
            .filter(|(_, p)| match &self.filter {
                Filter::All => !p.deleted,
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
            })
            .filter(|(_, p)| {
                query.is_empty()
                    || p.name.to_lowercase().contains(&query)
                    || p.caption.to_lowercase().contains(&query)
                    || p.id.to_lowercase().contains(&query)
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
        self.filter = filter;
        self.selected.clear();
        self.viewer = None;
        self.anchor = None;
        self.modal = None;
        self.reset_gallery();
        self.focus.focus(w);
        cx.notify();
    }
    fn title(&self) -> String {
        match &self.filter {
            Filter::All => "Library".into(),
            Filter::Favorites => "Favorites".into(),
            Filter::Recent => "Last 30 Days".into(),
            Filter::Trash => "Recently Deleted".into(),
            Filter::Album(a) => a.clone(),
        }
    }
    fn current(&self) -> Option<Photo> {
        self.viewer
            .as_ref()
            .or_else(|| self.selected.iter().next())
            .and_then(|id| {
                self.photo_index
                    .get(id)
                    .and_then(|&i| self.library.photos.get(i))
                    .or_else(|| self.library.photos.iter().find(|p| p.id == *id))
            })
            .map(|p| p.as_ref().clone())
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
            self.cancel_gallery_interaction();
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
        self.caption
            .update(cx, |e, cx| e.set_value(photo.caption.clone(), w, cx));
        if double {
            self.selected = BTreeSet::from([photo.id.clone()]);
            self.viewer = Some(photo.id.clone());
            self.dismiss_guard = None;
            self.decoded_preview = None;
            self.viewport.reset();
            self.pan_start = None;
            self.sync_zoom(w, cx);
            self.request_image(photo.as_ref().clone(), true, cx);
        }
        self.focus.focus(w);
        cx.notify();
    }
    fn open_selected(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        self.cancel_gallery_interaction();
        if let Some(photo) = self.current() {
            self.selected = BTreeSet::from([photo.id.clone()]);
            self.viewer = Some(photo.id.clone());
            self.dismiss_guard = None;
            self.decoded_preview = None;
            self.viewport.reset();
            self.pan_start = None;
            self.sync_zoom(w, cx);
            self.caption
                .update(cx, |e, cx| e.set_value(photo.caption.clone(), w, cx));
            self.request_image(photo, true, cx);
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
    fn close_viewer(&mut self, gesture: bool, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.viewer.take() {
            let photos = self.visible(cx);
            if let Some(index) = self.photo_index.get(&id).and_then(|&i| photos.position(i)) {
                let layout = GalleryLayout::new(self.gallery_width, self.gallery_zoom)
                    .with_count(photos.len(), self.preferences.bottom_to_top);
                self.gallery_reset = false;
                self.gallery_restore = None;
                self.scroll.scroll_to_item(
                    (index + layout.leading) / layout.block_len,
                    ScrollStrategy::Center,
                );
            }
        }
        self.viewport.reset();
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
            self.close_viewer(true, w, cx);
            return;
        }
        self.viewport.set_zoom(target.clamp(1., 8.), anchor);
        self.sync_zoom(w, cx);
        cx.notify();
    }
    fn scroll_to_close(
        &mut self,
        dy: f32,
        phase: TouchPhase,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = std::time::Instant::now();
        if matches!(phase, TouchPhase::Started)
            || self
                .dismiss_scroll_last
                .is_none_or(|last| now.duration_since(last) > Duration::from_millis(220))
        {
            self.dismiss_scroll = 0.;
        }
        self.dismiss_scroll_last = Some(now);
        self.dismiss_scroll = (self.dismiss_scroll - dy).max(0.);
        if self.dismiss_scroll >= 18. {
            self.close_viewer(true, w, cx);
        }
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
        self.swipe_epoch += 1;
        let epoch = self.swipe_epoch;
        if matches!(phase, TouchPhase::Ended) {
            self.finish_viewer_swipe(w, cx);
        } else if self.swipe_active {
            // Non-phased wheels and interrupted gestures also settle after idle.
            cx.spawn_in(w, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let _ = this.update_in(cx, |this, w, cx| {
                    if this.swipe_epoch == epoch {
                        this.finish_viewer_swipe(w, cx);
                    }
                });
            })
            .detach();
        }
    }
    fn finish_viewer_swipe(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if !self.swipe_active || self.swipe_settling {
            return;
        }
        self.swipe_active = false;
        self.swipe_blocked = true;
        self.swipe_settling = true;
        self.swipe_epoch += 1;
        let epoch = self.swipe_epoch;
        let start = self.swipe_offset;
        let width = self.viewport.viewport.0.max(1.);
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
        let end = -(amount as f32) * width;
        let viewer = self.viewer.clone();
        cx.spawn_in(w, async move |this, cx| {
            for frame in 1..=12 {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let keep = this
                    .update_in(cx, |this, w, cx| {
                        if this.swipe_epoch != epoch || this.viewer != viewer {
                            this.swipe_settling = false;
                            this.swipe_offset = 0.;
                            return false;
                        }
                        let t = frame as f32 / 12.;
                        this.swipe_offset = start + (end - start) * (1. - (1. - t).powi(3));
                        if frame == 12 {
                            this.swipe_offset = 0.;
                            this.swipe_settling = false;
                            if amount != 0 {
                                this.step(amount, w, cx);
                            }
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
    fn sync_zoom(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.swipe_epoch += 1;
        self.swipe_offset = 0.;
        self.swipe_active = false;
        self.swipe_settling = false;
        self.pan_start = None;
        self.dismiss_scroll = 0.;
        self.dismiss_scroll_last = None;
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
        let handle = &self.scroll.0.borrow().base_handle;
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
        let left = m.content_start.x.min(content_end.x);
        let right = m.content_start.x.max(content_end.x);
        let top = m.content_start.y.min(content_end.y);
        let bottom = m.content_start.y.max(content_end.y);
        let base = m.baseline.clone();
        let photos = self.visible(cx);
        self.selected = base;
        for (i, photo) in photos.iter(&self.library).enumerate() {
            let layout = GalleryLayout::new(self.gallery_width, self.gallery_zoom)
                .with_count(self.visible(cx).len(), self.preferences.bottom_to_top);
            let cell = layout.cell(i);
            let x = px(cell.x);
            let y = px(cell.y);
            if x < right
                && x + px(cell.edge) > left
                && y < bottom
                && y + px(cell.edge + layout.caption) > top
            {
                self.selected.insert(photo.id.clone());
            }
        }
        cx.notify();
    }
    fn request_image(&mut self, photo: Photo, preview: bool, cx: &mut Context<Self>) {
        let key = format!(
            "{}-{}",
            photo.fingerprint,
            if preview { "preview" } else { "thumb" }
        );
        if self.failed.contains(&key)
            || self.pending.contains(&key)
            || (if preview {
                &self.previews
            } else {
                &self.thumbnails
            })
            .contains_key(&photo.fingerprint)
        {
            return;
        }
        if (!preview && self.pending.len() >= THUMBNAIL_JOBS)
            || (preview && self.pending.iter().any(|key| key.ends_with("-preview")))
        {
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        self.pending.insert(key.clone());
        let generation = self.generation;
        let fingerprint = photo.fingerprint.clone();
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
                    Err(_) => {
                        if this.failed_order.len() >= THUMBNAIL_CACHE
                            && let Some(old) = this.failed_order.pop_front()
                        {
                            this.failed.remove(&old);
                        }
                        this.failed_order.push_back(key.clone());
                        this.failed.insert(key);
                    }
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
    fn sidebar_view(&self, c: Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let mut side = ui::sidebar(c)
            .id("photos-sidebar")
            .debug_selector(|| "photos-sidebar".into())
            .w(px(216.))
            .p_2()
            .bg(rgb(c.bg))
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(self.source_label.clone()),
            )
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
            albums = albums.child(
                kit::nav_item(
                    SharedString::from(format!("album-{name}")),
                    "folder",
                    name.clone(),
                    Some(count),
                    self.filter == Filter::Album(name.clone()),
                    c,
                )
                .on_click(cx.listener(move |this, _, w, cx| {
                    this.set_filter(Filter::Album(album.clone()), w, cx)
                }))
                .on_drop(cx.listener(move |this, drag: &DragPhotos, w, cx| {
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
                .on_drop(cx.listener(move |this, paths: &ExternalPaths, w, cx| {
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
                })),
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
        photo: &Photo,
        index: usize,
        layout: GalleryLayout,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        let edge = layout.edge;
        let request_photo = photo.clone();
        let request_entity = cx.entity();
        let selected = self.selected.contains(&photo.id);
        let thumb = self.thumbnails.get(&photo.fingerprint).cloned();
        let failed = self
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
        div()
            .w(px(edge))
            .h(px(edge))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id(SharedString::from(format!("photo-{}", photo.id)))
                    .debug_selector(move || format!("photo-tile-{index}"))
                    .relative()
                    .group("photo-tile")
                    .w_full()
                    .h(px(edge))
                    .rounded(px(layout.radius))
                    .overflow_hidden()
                    .cursor_pointer()
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
                                    this.request_image(request_photo, false, cx)
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
    fn thumbnail_budget(&mut self, layout: GalleryLayout, height: f32, cx: &mut Context<Self>) {
        let visible =
            ((height.max(1.) / layout.block_height).ceil() as usize + 3) * layout.block_len;
        let capacity = visible
            .max(
                self.gallery_motion
                    .as_ref()
                    .map_or(0, |m| m.tiles.len() * 2 + layout.block_len),
            )
            .max(96);
        self.render_cache
            .update(cx, |cache, _| cache.set_capacity(capacity));
        self.thumbnails.set_capacity(capacity.max(THUMBNAIL_CACHE));
    }
    fn reset_gallery(&mut self) {
        self.gallery_reset = true;
        self.gallery_restore = None;
        self.gallery_motion = None;
        self.gallery_pinch = None;
    }
    fn capture_gallery_anchor(&self, cx: &App) -> Option<GalleryAnchor> {
        let photos = self.visible(cx);
        if photos.is_empty() {
            return None;
        }
        let layout = GalleryLayout::new(self.gallery_width, self.gallery_zoom)
            .with_count(self.visible(cx).len(), self.preferences.bottom_to_top);
        let scroll = &self.scroll.0.borrow().base_handle;
        let height = f32::from(scroll.bounds().size.height);
        let offset = f32::from(-scroll.offset().y).max(0.);
        let max = (layout.blocks(photos.len()) as f32 * layout.block_height - height).max(0.);
        let screen = height / 2.;
        let index = layout.nearest(self.gallery_width / 2., offset + screen, photos.len());
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
        self.cancel_gallery_interaction();
        if !self.gallery_reset
            && self.gallery_restore.is_none()
            && self.scroll.0.borrow().deferred_scroll_to_item.is_none()
        {
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
    fn begin_gallery_motion(
        &mut self,
        target: f32,
        pointer: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let photos = self.visible(cx);
        if photos.is_empty() || target == self.gallery_zoom {
            return;
        }
        let old = GalleryLayout::new(self.gallery_width, self.gallery_zoom)
            .with_count(photos.len(), self.preferences.bottom_to_top);
        let next = GalleryLayout::new(self.gallery_width, target)
            .with_count(photos.len(), self.preferences.bottom_to_top);
        let (bounds, offset) = {
            let h = &self.scroll.0.borrow().base_handle;
            (h.bounds(), f32::from(-h.offset().y).max(0.))
        };
        let height = f32::from(bounds.size.height).max(1.);
        let old_title = if old.full_bleed() { 0. } else { 84. };
        let new_title = if next.full_bleed() { 0. } else { 84. };
        let available = self.gallery_height.max(height);
        let next_available = (available + old_title - new_title).max(1.);
        let next_height =
            (next.blocks(photos.len()) as f32 * next.block_height).min(next_available);
        let from_header = old_title
            + if self.preferences.bottom_to_top {
                (available - height).max(0.)
            } else {
                0.
            };
        let to_header = new_title
            + if self.preferences.bottom_to_top {
                (next_available - next_height).max(0.)
            } else {
                0.
            };
        let local = if bounds.contains(&pointer) {
            pointer - bounds.origin
        } else {
            point(bounds.size.width / 2., bounds.size.height / 2.)
        };
        let anchor = old.nearest(local.x.into(), offset + f32::from(local.y), photos.len());
        let max = (old.blocks(photos.len()) as f32 * old.block_height - height).max(0.);
        let next_max = (next.blocks(photos.len()) as f32 * next.block_height - next_height).max(0.);
        let to_offset = if self.preferences.bottom_to_top && max - offset < 2. {
            next_max
        } else {
            old.anchored_offset(
                &next,
                (local.x.into(), local.y.into()),
                offset,
                photos.len(),
            )
            .min(next_max)
        };
        let from = GridFrame {
            layout: old,
            offset,
            header: from_header,
            height,
        };
        let to = GridFrame {
            layout: next,
            offset: to_offset,
            header: to_header,
            height: next_height,
        };
        let cache = self.render_cache.read(cx);
        let image = |photo: &Option<Arc<Photo>>| {
            photo
                .as_ref()
                .and_then(|p| self.thumbnails.peek(&p.fingerprint))
                .and_then(|path| cache.ready(path))
        };
        let tiles = from
            .slots_to(to, anchor, photos.len())
            .into_iter()
            .map(|frames| {
                let source = frames
                    .source
                    .and_then(|i| photos.get(&self.library, i))
                    .cloned();
                let target = frames
                    .target
                    .and_then(|i| photos.get(&self.library, i))
                    .cloned();
                MotionTile {
                    frames,
                    source_image: image(&source),
                    target_image: image(&target),
                    source,
                    target,
                }
            })
            .collect();
        self.gallery_motion_epoch += 1;
        self.gallery_motion = Some(GalleryMotion {
            from,
            to,
            tiles: Arc::new(tiles),
            progress: 0.,
            epoch: self.gallery_motion_epoch,
            settling: false,
        });
        self.gallery_restore = None;
        self.marquee = None;
    }
    fn finish_gallery_motion(&mut self, commit: bool) {
        if let Some(motion) = self.gallery_motion.take() {
            let frame = if commit { motion.to } else { motion.from };
            self.gallery_zoom = frame.layout.zoom;
            self.scroll.0.borrow_mut().deferred_scroll_to_item = None;
            self.scroll
                .0
                .borrow()
                .base_handle
                .set_offset(point(px(0.), px(-frame.offset)));
            self.gallery_reset = false;
            self.gallery_restore = None;
        }
        self.gallery_motion_epoch += 1;
    }
    fn cancel_gallery_interaction(&mut self) {
        if let Some(motion) = &self.gallery_motion {
            let commit = motion.progress >= 0.5;
            self.finish_gallery_motion(commit);
        }
        self.gallery_pinch = None;
        self.gallery_pinch_epoch += 1;
    }
    fn pinch_gallery(
        &mut self,
        delta: f32,
        pointer: Point<Pixels>,
        ended: bool,
        cx: &mut Context<Self>,
    ) {
        if !delta.is_finite()
            || self.viewer.is_some()
            || self.modal.is_some()
            || self.driver.is_none()
        {
            return;
        }
        // Grabbing a settling transition resumes its visible geometry, rather than
        // restarting an animation from an invisible endpoint.
        if let Some(motion) = &mut self.gallery_motion
            && motion.settling
        {
            motion.settling = false;
            self.gallery_motion_epoch += 1;
            motion.epoch = self.gallery_motion_epoch;
            self.gallery_zoom = motion.from.layout.zoom;
            self.gallery_pinch = Some(
                motion.from.layout.zoom
                    + (motion.to.layout.zoom - motion.from.layout.zoom) * motion.progress,
            );
        }
        let value = (self.gallery_pinch.unwrap_or(self.gallery_zoom) + delta)
            .clamp(0., GalleryLayout::MAX_ZOOM);
        self.gallery_pinch = Some(value);
        if let Some(m) = &self.gallery_motion
            && (value - m.from.layout.zoom) * (m.to.layout.zoom - m.from.layout.zoom) < 0.
        {
            self.finish_gallery_motion(false);
        }
        if self.gallery_motion.is_none() && (value - self.gallery_zoom).abs() > 0.001 {
            self.begin_gallery_motion(
                (self.gallery_zoom + (value - self.gallery_zoom).signum())
                    .clamp(0., GalleryLayout::MAX_ZOOM),
                pointer,
                cx,
            );
        }
        if let Some(m) = &mut self.gallery_motion {
            m.progress = ((value - m.from.layout.zoom) / (m.to.layout.zoom - m.from.layout.zoom))
                .clamp(0., 1.);
        }
        if self
            .gallery_motion
            .as_ref()
            .is_some_and(|m| m.progress >= 1.)
        {
            self.finish_gallery_motion(true);
        }
        self.gallery_pinch_epoch += 1;
        if ended {
            self.finish_gallery_pinch(pointer, cx);
            return;
        }
        let epoch = self.gallery_pinch_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.gallery_pinch_epoch == epoch {
                    this.finish_gallery_pinch(pointer, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn finish_gallery_pinch(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(value) = self.gallery_pinch.take() {
            if let Some(motion) = &self.gallery_motion {
                let commit = (value.round() - motion.to.layout.zoom).abs()
                    < (value.round() - motion.from.layout.zoom).abs();
                self.settle_gallery_motion(commit, cx);
            } else {
                self.zoom_gallery(value.round() - self.gallery_zoom, pointer, cx);
            }
        }
    }
    fn settle_gallery_motion(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(m) = &mut self.gallery_motion else {
            return;
        };
        m.settling = true;
        self.gallery_motion_epoch += 1;
        m.epoch = self.gallery_motion_epoch;
        let epoch = m.epoch;
        let start = m.progress;
        let end = if commit { 1. } else { 0. };
        // Install the final real grid behind the opaque animated surface now, so
        // its scroll metrics and asynchronous thumbnails are ready at handoff.
        let frame = if commit { m.to } else { m.from };
        self.gallery_zoom = frame.layout.zoom;
        self.scroll.0.borrow_mut().deferred_scroll_to_item = None;
        self.scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.), px(-frame.offset)));
        self.gallery_restore = None;
        self.gallery_reset = false;
        cx.spawn(async move |this, cx| {
            for tick in 1..=14 {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        let Some(m) = &mut this.gallery_motion else {
                            return false;
                        };
                        if m.epoch != epoch || !m.settling {
                            return false;
                        }
                        let t = tick as f32 / 14.;
                        m.progress = start + (end - start) * (1. - (1. - t).powi(3));
                        if tick == 14 {
                            this.finish_gallery_motion(commit);
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }
    fn zoom_gallery(&mut self, delta: f32, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        if self.driver.is_none()
            || self.viewer.is_some()
            || self.modal.is_some()
            || !delta.is_finite()
        {
            return;
        }
        if self.gallery_motion.is_some() {
            let commit = self
                .gallery_motion
                .as_ref()
                .is_some_and(|m| m.progress >= 0.5);
            self.finish_gallery_motion(commit);
        }
        let target = (self.gallery_zoom + delta)
            .round()
            .clamp(0., GalleryLayout::MAX_ZOOM);
        self.begin_gallery_motion(target, pointer, cx);
        self.settle_gallery_motion(true, cx);
    }
    fn motion_view(&mut self, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        let m = self.gallery_motion.as_ref().unwrap();
        let tiles = m.tiles.clone();
        let progress = m.progress;
        let from = m.from;
        let to = m.to;
        let blend = ((progress - 0.12) / 0.76).clamp(0., 1.);
        let blend = blend * blend * (3. - 2. * blend);
        let mut surface = div()
            .id("gallery-motion")
            .debug_selector(|| "gallery-motion".into())
            .absolute()
            .inset_0()
            .overflow_hidden()
            .bg(rgb(c.bg));
        let header = from.header + (to.header - from.header) * progress;
        let mut grid = div()
            .absolute()
            .left_0()
            .right_0()
            .top(px(header))
            .bottom_0()
            .overflow_hidden();
        for (i, tile) in tiles.iter().enumerate() {
            for photo in [&tile.source, &tile.target].into_iter().flatten() {
                self.request_image(photo.as_ref().clone(), false, cx);
            }
            let ready = |photo: &Option<Arc<Photo>>| {
                photo
                    .as_ref()
                    .and_then(|p| self.thumbnails.peek(&p.fingerprint))
                    .and_then(|path| self.render_cache.read(cx).ready(path))
            };
            let source = tile.source_image.clone().or_else(|| ready(&tile.source));
            let target = tile.target_image.clone().or_else(|| ready(&tile.target));
            let frame = tile.frames.at(progress);
            let radius = from.layout.radius + (to.layout.radius - from.layout.radius) * progress;
            let same = tile
                .source
                .as_ref()
                .zip(tile.target.as_ref())
                .is_some_and(|(a, b)| a.id == b.id);
            let mut cell = div()
                .id(("motion-frame", i))
                .absolute()
                .left(px(frame.x))
                .top(px(frame.y - header))
                .size(px(frame.edge))
                .rounded(px(radius))
                .overflow_hidden()
                .bg(rgb(c.surface));
            if let Some(image) = source {
                cell = cell.child(img(image).size_full().object_fit(ObjectFit::Cover).opacity(
                    if tile.target.is_none() {
                        1. - blend
                    } else {
                        1.
                    },
                ));
            }
            if let Some(image) = target {
                cell = cell.child(
                    img(image)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .opacity(if same || tile.source.is_none() {
                            1.
                        } else {
                            blend
                        }),
                );
            }
            // These loads run in the paint lifecycle. Retained snapshots cover
            // pending decodes, and the target grid stays alive behind this layer.
            for photo in [&tile.source, &tile.target].into_iter().flatten() {
                if let Some(path) = self.thumbnails.peek(&photo.fingerprint).cloned() {
                    cell = cell.child(
                        img(Arc::<std::path::Path>::from(path))
                            .image_cache(&self.render_cache)
                            .absolute()
                            .inset_0()
                            .size_full()
                            .opacity(0.),
                    );
                }
            }
            grid = grid.child(cell);
        }
        let header_opacity = match (from.layout.full_bleed(), to.layout.full_bleed()) {
            (false, false) => 1.,
            (true, false) => progress,
            (false, true) => 1. - progress,
            _ => 0.,
        };
        surface = surface.child(grid);
        surface
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(header))
                    .overflow_hidden()
                    .px_5()
                    .pt_4()
                    .opacity(header_opacity)
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(FontWeight::BOLD)
                            .child(self.title()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child(format!(
                                "{} photos",
                                self.total.unwrap_or(self.library.photos.len())
                            )),
                    ),
            )
            .into_any_element()
    }
    fn gallery_step(&mut self, larger: bool, w: &Window, cx: &mut Context<Self>) {
        let target = if larger {
            self.gallery_zoom.floor() + 1.
        } else {
            self.gallery_zoom.ceil() - 1.
        };
        self.zoom_gallery(target - self.gallery_zoom, w.mouse_position(), cx);
    }
    fn grid_view(&mut self, available: f32, c: Colors, cx: &mut Context<Self>) -> impl IntoElement {
        if (self.gallery_width - available).abs() > 0.5 {
            self.preserve_gallery(cx);
        }
        self.gallery_width = available;
        let photos = self.visible(cx);
        let photo_count = photos.len();
        let layout = GalleryLayout::new(available, self.gallery_zoom)
            .with_count(photos.len(), self.preferences.bottom_to_top);
        let blocks = layout.blocks(photos.len());
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
        if !layout.full_bleed() {
            let count = if self.filter == Filter::All && self.search.read(cx).value().is_empty() {
                self.total.unwrap_or(photos.len())
            } else {
                photos.len()
            };
            let subtitle = if self.has_more {
                format!("{count} photos · cataloging {}", self.library.photos.len())
            } else {
                format!("{count} photos")
            };
            content = content.child(
                div()
                    .h(px(84.))
                    .w_full()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .child(
                        div()
                            .w(px((available - 40.).max(1.)))
                            .text_size(px(28.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(self.title()),
                    )
                    .child(
                        div()
                            .w(px((available - 40.).max(1.)))
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child(subtitle),
                    ),
            );
        }
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
                        Filter::Recent => "No Recent Photos",
                        Filter::Trash => "No Deleted Photos",
                        _ => "No Photos Yet",
                    }
                },
                "",
                c,
            ));
        } else {
            if self.gallery_reset {
                self.scroll.scroll_to_item_strict(
                    if self.preferences.bottom_to_top {
                        blocks.saturating_sub(1)
                    } else {
                        0
                    },
                    if self.preferences.bottom_to_top {
                        ScrollStrategy::Bottom
                    } else {
                        ScrollStrategy::Top
                    },
                );
                self.gallery_reset = false;
                self.gallery_restore = None;
            } else if let Some(anchor) = self.gallery_restore.take() {
                if anchor.end {
                    self.scroll.scroll_to_item_strict(
                        if self.preferences.bottom_to_top {
                            blocks.saturating_sub(1)
                        } else {
                            0
                        },
                        if self.preferences.bottom_to_top {
                            ScrollStrategy::Bottom
                        } else {
                            ScrollStrategy::Top
                        },
                    );
                } else {
                    let index = self
                        .photo_index
                        .get(&anchor.id)
                        .and_then(|&i| photos.position(i))
                        .unwrap_or(anchor.fallback.min(photos.len() - 1));
                    let height = f32::from(self.scroll.0.borrow().base_handle.bounds().size.height);
                    let max = (blocks as f32 * layout.block_height - height).max(0.);
                    let offset = (layout.cell(index).y + anchor.within * layout.edge
                        - anchor.screen)
                        .clamp(0., max);
                    self.scroll
                        .0
                        .borrow()
                        .base_handle
                        .set_offset(point(px(0.), px(-offset)));
                }
            }
            let entity = cx.entity();
            let resize_entity = entity.clone();
            content = content.child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .when(self.preferences.bottom_to_top, |d| d.justify_end())
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                resize_entity.update(cx, |this, cx| {
                                    this.thumbnail_budget(
                                        layout,
                                        f32::from(bounds.size.height),
                                        cx,
                                    );
                                    let full_height = f32::from(bounds.size.height);
                                    if (this.gallery_height - full_height).abs() > 0.5 {
                                        this.gallery_height = full_height;
                                        cx.notify();
                                    }
                                    let expected =
                                        px((blocks as f32 * layout.block_height).min(full_height));
                                    let old =
                                        this.scroll.0.borrow().base_handle.bounds().size.height;
                                    if this.gallery_motion.is_none()
                                        && old > px(0.)
                                        && (old - expected).abs() > px(0.5)
                                    {
                                        this.preserve_gallery(cx);
                                        if this.gallery_restore.as_ref().is_some_and(|a| a.end) {
                                            this.scroll.scroll_to_item_strict(
                                                if this.preferences.bottom_to_top {
                                                    blocks.saturating_sub(1)
                                                } else {
                                                    0
                                                },
                                                if this.preferences.bottom_to_top {
                                                    ScrollStrategy::Bottom
                                                } else {
                                                    ScrollStrategy::Top
                                                },
                                            );
                                        }
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
                        uniform_list("photo-rows", blocks, move |range, _, cx| {
                            entity.update(cx, |this, cx| {
                                let offset = {
                                    let scroll = &this.scroll.0.borrow().base_handle;
                                    f32::from(-scroll.offset().y)
                                };
                                let mut period = None;
                                let lines = range
                                    .map(|block| {
                                        let indices = layout.indices(block, photos.len());
                                        let mut line =
                                            div().relative().w_full().h(px(layout.block_height));
                                        for index in indices {
                                            let cell = layout.cell(index);
                                            if let Some(photo) =
                                                photos.get(&this.library, index).cloned()
                                            {
                                                if period.is_none() && cell.y + cell.edge >= offset
                                                {
                                                    period = Some(
                                                        photo.modified.format("%Y\n%B").to_string(),
                                                    );
                                                }
                                                line = line.child(
                                                    this.tile(&photo, index, layout, c, cx)
                                                        .absolute()
                                                        .left(px(cell.x))
                                                        .top_0(),
                                                );
                                            }
                                        }
                                        line
                                    })
                                    .collect::<Vec<_>>();
                                if let Some(period) = period
                                    && this.gallery_period != period
                                {
                                    this.gallery_period = period;
                                    cx.notify();
                                }
                                lines
                            })
                        })
                        .track_scroll(self.scroll.clone())
                        .w_full()
                        .h(px(blocks as f32 * layout.block_height))
                        .max_h_full()
                        .flex_shrink_0(),
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
                    ),
            );
        }
        if self.gallery_motion.is_some() {
            content = content.child(self.motion_view(c, cx));
        }
        let date_opacity =
            self.gallery_motion
                .as_ref()
                .map_or(if layout.full_bleed() { 1. } else { 0. }, |m| {
                    match (m.from.layout.full_bleed(), m.to.layout.full_bleed()) {
                        (true, true) => 1.,
                        (true, false) => 1. - m.progress,
                        (false, true) => m.progress,
                        _ => 0.,
                    }
                });
        if date_opacity > 0. && photo_count > 0 {
            let photo_top = self.gallery_motion.as_ref().map_or_else(
                || {
                    if self.preferences.bottom_to_top {
                        (self.gallery_height - blocks as f32 * layout.block_height).max(0.)
                    } else {
                        0.
                    }
                },
                |m| m.from.header + (m.to.header - m.from.header) * m.progress,
            );
            let date_top = (photo_top + 12.).min((self.gallery_height - 76.).max(12.));
            let (year, month) = self
                .gallery_period
                .split_once('\n')
                .unwrap_or(("", "Library"));
            content = content.child(
                div()
                    .id("gallery-date-label")
                    .opacity(date_opacity)
                    .debug_selector(|| "gallery-date-label".into())
                    .absolute()
                    .top(px(date_top))
                    .left_3()
                    .px_3()
                    .py_2()
                    .rounded(px(12.))
                    .bg(rgba(0x15181dcc))
                    .text_color(rgb(0xffffff))
                    .child(div().text_size(px(11.)).child(year.to_owned()))
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(month.to_owned()),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
            );
        }
        content
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, w, cx| {
                if event.modifiers.platform || event.modifiers.control {
                    let delta = event.delta.pixel_delta(px(28.));
                    this.pinch_gallery(
                        f32::from(delta.y) * 0.006,
                        event.position,
                        matches!(event.touch_phase, TouchPhase::Ended),
                        cx,
                    );
                    let _ = w;
                    cx.stop_propagation();
                } else if this.gallery_motion.is_some() {
                    this.cancel_gallery_interaction();
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, w, cx| {
                    if this.modal.is_some() {
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
                    let handle = &this.scroll.0.borrow().base_handle;
                    if handle.bounds().contains(&e.position) {
                        this.marquee = Some(Marquee {
                            start: e.position,
                            end: e.position,
                            content_start: e.position - handle.bounds().origin - handle.offset(),
                            baseline,
                        });
                    }
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
            self.request_image(photo.clone(), true, cx);
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
            let base = thumbnail.clone().or_else(|| preview.clone());
            let viewer_index = self
                .photo_index
                .get(&photo.id)
                .and_then(|&i| self.visible(cx).position(i))
                .unwrap_or(0);
            self.anchor = Some(viewer_index);
            let can_previous = viewer_index > 0;
            let can_next = self.anchor.is_some_and(|i| i + 1 < self.visible(cx).len());
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
                            cx.listener(|this, _, w, cx| this.close_viewer(false, w, cx)),
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
            let size = (fit.0 * viewport.zoom, fit.1 * viewport.zoom);
            let position = (
                (viewport.viewport.0 - size.0) / 2. + viewport.pan.0,
                (viewport.viewport.1 - size.1) / 2. + viewport.pan.1,
            );
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
                                        (bounds.size.width.into(), bounds.size.height.into()),
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
                        div()
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
                            .into_any_element()
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
                            self.request_image(photo.as_ref().clone(), false, cx);
                            if let Some(path) = self.thumbnails.get(&photo.fingerprint).cloned() {
                                return d.child(
                                    img(Arc::<std::path::Path>::from(path))
                                        .image_cache(&self.render_cache)
                                        .absolute()
                                        .left(px(self.swipe_offset
                                            + if self.swipe_offset < 0. {
                                                self.viewport.viewport.0
                                            } else {
                                                -self.viewport.viewport.0
                                            }))
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
                                    this.step(-1, w, cx);
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
                                    this.step(1, w, cx);
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
                                true,
                                w,
                                cx,
                            );
                        } else if !this.can_pan()
                            && !e.modifiers.shift
                            && !this.swipe_active
                            && f32::from(delta.y).abs() > f32::from(delta.x).abs()
                        {
                            this.scroll_to_close(delta.y.into(), e.touch_phase, w, cx);
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
    fn inspector_view(&self, c: Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let mut info = div()
            .id("photo-inspector")
            .debug_selector(|| "photo-inspector".into())
            .w(px(250.))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(rgb(c.border))
            .bg(rgb(c.surface))
            .flex()
            .flex_col()
            .child(kit::panel_heading("Photo info", "", c));
        if let Some(photo) = self.current() {
            info = info.child(
                div()
                    .id("inspector-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(photo.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child(format!(
                                "{} × {} pixels\n{:.2} MB\nModified {}",
                                photo.width,
                                photo.height,
                                photo.bytes as f64 / 1_000_000.,
                                photo.modified.format("%b %-d, %Y, %-I:%M %p")
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c.muted))
                            .child(photo.id),
                    )
                    .child(Input::new(&self.caption).h(px(100.)))
                    .child(
                        ui::button("save-caption", c, false, true)
                            .child("Save caption")
                            .on_click(cx.listener(|this, _, w, cx| {
                                if let Some(photo) = this.current() {
                                    let caption = this.caption.read(cx).value().to_string();
                                    this.job(
                                        w,
                                        cx,
                                        move |driver| driver.caption(&photo.id, &caption),
                                        |this, _| this.notice = "Caption saved".into(),
                                    );
                                }
                            })),
                    )
                    .when(!photo.path.as_os_str().is_empty(), |d| {
                        d.child(
                            ui::button("reveal-photo", c, false, true)
                                .child("Show in folder")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(photo) = this.current()
                                        && !photo.path.as_os_str().is_empty()
                                    {
                                        cx.reveal_path(&photo.path);
                                    }
                                })),
                        )
                    }),
            );
        } else {
            info = info.child(kit::empty("info", "Select a photo", "", c));
        }
        info
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
            Modal::Help => "Photos Help",
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
                    content = content.child(kit::panel_heading(
                        "Appearance",
                        "Choose how Photos looks on this device.",
                        c,
                    ));
                    for (key, label) in [("system", "System"), ("light", "Light"), ("dark", "Dark")]
                    {
                        content = content.child(
                            ui::button(
                                SharedString::from(format!("appearance-{key}")),
                                c,
                                self.preferences.appearance == key,
                                false,
                            )
                            .child(label)
                            .on_click(cx.listener(
                                move |this, _, w, cx| {
                                    this.preferences.appearance = key.into();
                                    if let Err(e) = this.preferences.save(&this.config_path) {
                                        this.error = Some(e.to_string());
                                    }
                                    this.theme(w, cx);
                                },
                            )),
                        );
                    }
                    content = content
                        .child(
                            div()
                                .mt_3()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Accent color"),
                        )
                        .child(div().flex().flex_wrap().gap_3().children(Accent::ALL.map(
                            |accent| {
                                let color = Colors::new(
                                    matches!(self.preferences.appearance.as_str(), "dark")
                                        || self.preferences.appearance != "light"
                                            && ui::system_dark(w),
                                    accent,
                                );
                                let active = self.accent() == accent;
                                let id = format!("accent-{}", accent.key());
                                div()
                                    .id(SharedString::from(id.clone()))
                                    .debug_selector(move || id)
                                    .size(px(32.))
                                    .rounded_full()
                                    .p(px(3.))
                                    .border_2()
                                    .border_color(rgb(if active {
                                        color.accent
                                    } else {
                                        c.surface
                                    }))
                                    .cursor_pointer()
                                    .focusable()
                                    .tab_index(0)
                                    .child(
                                        div()
                                            .size_full()
                                            .rounded_full()
                                            .bg(rgb(color.accent))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(active, |d| {
                                                d.child(kit::glyph("check", 12., color.on_accent))
                                            }),
                                    )
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.set_accent(accent, w, cx)
                                    }))
                                    .on_key_down(cx.listener(
                                        move |this, event: &KeyDownEvent, w, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
                                                cx.stop_propagation();
                                                this.set_accent(accent, w, cx);
                                            }
                                        },
                                    ))
                            },
                        )))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c.muted))
                                .child(format!(
                                    "{} · buttons, selection and highlights",
                                    self.accent().label()
                                )),
                        );
                } else {
                    content = content.child(kit::panel_heading("Behavior", "Choose how you browse your library.",c))
                        .child(div().flex().items_center().gap_4()
                            .child(div().flex_1().child(div().text_size(px(14.)).child("Browse from bottom to top"))
                                .child(div().mt_1().text_size(px(12.)).text_color(rgb(c.muted)).child("Start at the newest photos at the bottom. Scroll up to go back in time.")))
                            .child(div().id("bottom-to-top-setting").debug_selector(|| "bottom-to-top-setting".into()).child(Switch::new("bottom-to-top")
                                .checked(self.preferences.bottom_to_top)
                                .on_click(cx.listener(|this,value: &bool,_,cx| this.set_scroll_direction(*value,cx))))));
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
                for name in self.library.albums.keys() {
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
impl Render for PhotosApp {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(not(test))]
        ui::native_titlebar::sync_layout(w);
        let c = self.colors(w);
        let width: f32 = w.viewport_size().width.into();
        let show_sidebar = self.sidebar && width >= 960.;
        let show_info = self.inspector && width >= 900.;
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
                        this.close_viewer(false, w, cx);
                    } else {
                        this.selected.clear();
                    }
                }
                this.focus.focus(w);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Next, w, cx| this.step(1, w, cx)))
            .on_action(cx.listener(|this, _: &Previous, w, cx| this.step(-1, w, cx)))
            .on_action(cx.listener(|this, _: &Delete, w, cx| this.trash(w, cx)))
            .on_action(cx.listener(|this, _: &Favorite, w, cx| this.favorite(w, cx)))
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
            .child(
                div()
                    .ml_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(width >= 480., |d| d.child("Photos")),
            )
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
                    d.child(kit::tool("help", "info", "Photos Help", c).on_click(
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
                        kit::welcome(
                            "photos",
                            "Your photos, together",
                            "Choose a library to get started.",
                            c,
                        )
                        .child(self.source_choices(w, c, cx))
                        .into_any_element()
                    }),
            );
        } else {
            let available =
                width - if show_sidebar { 216. } else { 0. } - if show_info { 250. } else { 0. };
            let mut main = div().flex_1().min_h_0().min_w_0().flex().flex_col().when(
                self.viewer.is_none(),
                |d| {
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
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        if width < 900. {
                                            this.notice =
                                                "Photo details require a wider window.".into();
                                        } else {
                                            this.inspector = !this.inspector;
                                            if let Some(photo) = this.current() {
                                                this.caption.update(cx, |e, cx| {
                                                    e.set_value(photo.caption, w, cx)
                                                });
                                            }
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            );
            main = main.child(if self.viewer.is_some() {
                self.viewer_view(w, c, cx).into_any_element()
            } else {
                self.grid_view(available, c, cx).into_any_element()
            });
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
                    .when(self.viewer.is_some(), |d| {
                        let photo = self.current().unwrap();
                        d.child(
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
                                .child(div().truncate().min_w_0().child(photo.name))
                                .child(div().w(px(1.)).h(px(14.)).flex_shrink_0().bg(rgb(c.border)))
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .child(format!("{} × {}", photo.width, photo.height)),
                                ),
                        )
                    })
                    .when(self.viewer.is_none(), |d| {
                        d.child(div().flex_1().min_w(px(70.)).truncate().child(
                            if self.selected.is_empty() {
                                self.notice.clone()
                            } else {
                                format!("{} selected", self.selected.len())
                            },
                        ))
                    })
                    .when(!self.selected.is_empty(), |d| {
                        d.child(
                            kit::tool("favorite", "heart", "Favorite · F", c)
                                .when(self.current().is_some_and(|p| p.favorite), |d| {
                                    d.bg(rgb(c.soft))
                                })
                                .on_click(cx.listener(|this, _, w, cx| this.favorite(w, cx))),
                        )
                        .child(
                            kit::tool("add-to-album", "folder", "Add to album", c).on_click(
                                cx.listener(|this, _, w, cx| {
                                    this.open_modal(Modal::AddToAlbum, w, cx)
                                }),
                            ),
                        )
                        .when(self.viewer.is_some(), |d| d.child(div().flex_1()))
                        .child(
                            kit::tool("export", "export", "Export originals", c)
                                .on_click(cx.listener(|this, _, w, cx| this.export_picker(w, cx))),
                        )
                        .when(matches!(self.filter, Filter::Album(_)), |d| {
                            d.child(
                                kit::tool("remove-from-album", "minus", "Remove from album", c)
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        if let Filter::Album(name) = &this.filter {
                                            let name = name.clone();
                                            let ids =
                                                this.selected.iter().cloned().collect::<Vec<_>>();
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
                                    })),
                            )
                        })
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
                    }),
            );
            root = root.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .when(show_sidebar, |d| d.child(self.sidebar_view(c, cx)))
                    .child(main)
                    .when(show_info, |d| d.child(self.inspector_view(c, cx))),
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
        if let Some(at) = self.menu {
            root = root.child(self.menu_view(at, w, c, cx));
        }
        if let Some(modal) = self.modal {
            root = root.child(self.modal_view(modal, w, c, cx));
        }
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
                    name: "Photos".into(),
                    items: vec![
                        MenuItem::action("Settings…", Settings),
                        MenuItem::separator(),
                        MenuItem::action("Hide Photos", Hide),
                        MenuItem::action("Hide Others", HideOthers),
                        MenuItem::separator(),
                        MenuItem::action("Quit Photos", Quit),
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
                        title: Some("Photos".into()),
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
            .expect("Cannot open the Photos window");
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
