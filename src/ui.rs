use desert_photos::loading::{BoundedCache, THUMBNAIL_CACHE, THUMBNAIL_JOBS, VIEWER_CACHE};
use desert_photos::sources::{Connection, PhotoPage, ProviderKind};
use desert_photos::storage::{self, Library, Photo, PhotosDriver, Preferences, Source};
use desert_ui::{self as ui, Accent, Colors, library as kit};
use gpui::{prelude::*, *};
use gpui_component::{
    Root, Theme, ThemeMode,
    input::{Input, InputEvent, InputState},
};
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
    source: PathBuf,
    paths: Arc<RefCell<Vec<PathBuf>>>,
    offset: Point<Pixels>,
    preview: Option<PathBuf>,
}
impl Render for DragPhotos {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let c = Colors::new(false, Accent::Orange);
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
struct VisibleCache {
    revision: u64,
    count: usize,
    filter: Filter,
    query: String,
    photos: Arc<Vec<Arc<Photo>>>,
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
    tile_size: f32,
    viewport: ui::image_viewport::ImageViewport,
    viewer_bounds: Bounds<Pixels>,
    pan_start: Option<Point<Pixels>>,
    zoom_input: Entity<InputState>,
    menu: Option<Point<Pixels>>,
    settings_appearance: bool,
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
                        this.viewport.set_zoom(value / 100., (0., 0.));
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
                    this.scroll.scroll_to_item(0, ScrollStrategy::Top);
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
            tile_size: 172.,
            viewport: Default::default(),
            viewer_bounds: Bounds::default(),
            pan_start: None,
            zoom_input,
            menu: None,
            settings_appearance: false,
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
                Timer::after(Duration::from_millis(16)).await;
                if this
                    .update_in(cx, |this, w, cx| {
                        let delta = ui::native_transfer::take_magnification(w);
                        if delta != 0. && this.viewer.is_some() && this.modal.is_none() {
                            let at = w.mouse_position() - this.viewer_bounds.center();
                            this.viewport.set_zoom(
                                this.viewport.zoom * (1. + delta).max(0.1),
                                (at.x.into(), at.y.into()),
                            );
                            this.sync_zoom(w, cx);
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
            Accent::Orange,
        )
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
                    cx.notify();
                    return;
                }
                self.preferences = preferences;
                self.driver = Some(driver);
                self.library = page.library;
                self.next_page = page.next;
                self.has_more = self.next_page.is_some();
                self.total = page.total;
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
                    e.set_placeholder(
                        if self.driver.as_ref().is_some_and(|d| d.paged()) {
                            "Search loaded photos"
                        } else {
                            "Search photos"
                        },
                        w,
                        cx,
                    );
                });
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
                self.focus.focus(w);
            }
            Err(e) => {
                self.error = Some(format!("Cannot connect: {e:#}"));
                self.notice.clear();
            }
        }
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
        self.loading = true;
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { driver.page(cursor.as_deref()) }).await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation { return; }
                this.loading = false;
                match result {
                    Ok(page) => {
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
                        this.total = page.total.or(this.total);
                        this.revision += 1;
                        this.update_album_counts();
                        *this.visible_cache.borrow_mut() = None;
                    }
                    Err(e) => { this.page_failed = true; this.error = Some(format!("Could not load the next page: {e:#}")); }
                }
                cx.notify();
            });
        }).detach();
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
                        this.library = library;
                        this.library_changed();
                        let visible = this
                            .visible(cx)
                            .iter()
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
        if self.busy || self.loading {
            self.error = Some("Please wait for the current operation to finish.".into());
            cx.notify();
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        self.scan_epoch += 1;
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
                        this.library = library;
                        this.library_changed();
                    }
                    Err(e) => this.error = Some(format!("Cannot refresh library: {e}")),
                }
                let _ = w;
                cx.notify();
            });
        })
        .detach();
    }
    fn visible(&self, cx: &App) -> Arc<Vec<Arc<Photo>>> {
        let query = self.search.read(cx).value().to_lowercase();
        if let Some(cache) = self.visible_cache.borrow().as_ref()
            && cache.revision == self.revision
            && cache.count == self.library.photos.len()
            && cache.filter == self.filter
            && cache.query == query
        {
            return cache.photos.clone();
        }
        let recent_cutoff = chrono::Utc::now() - chrono::Duration::days(30);
        let photos = Arc::new(
            self.library
                .photos
                .iter()
                .filter(|p| match &self.filter {
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
                .filter(|p| {
                    query.is_empty()
                        || p.name.to_lowercase().contains(&query)
                        || p.caption.to_lowercase().contains(&query)
                        || p.id.to_lowercase().contains(&query)
                })
                .cloned()
                .collect(),
        );
        *self.visible_cache.borrow_mut() = Some(VisibleCache {
            revision: self.revision,
            count: self.library.photos.len(),
            filter: self.filter.clone(),
            query,
            photos: Arc::clone(&photos),
        });
        photos
    }
    fn set_filter(&mut self, filter: Filter, w: &mut Window, cx: &mut Context<Self>) {
        self.filter = filter;
        self.selected.clear();
        self.viewer = None;
        self.anchor = None;
        self.modal = None;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
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
        let visible = self.visible(cx);
        let Some(photo) = visible.get(index) else {
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
            self.selected
                .extend(visible[from..=to].iter().map(|p| p.id.clone()));
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
        if let Some(photo) = self.current() {
            self.selected = BTreeSet::from([photo.id.clone()]);
            self.viewer = Some(photo.id.clone());
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
            .and_then(|p| photos.iter().position(|f| f.id == p.id))
            .unwrap_or(0);
        let next = (index as isize + amount).clamp(0, photos.len() as isize - 1) as usize;
        let viewer = self.viewer.is_some();
        self.select(next, viewer, Modifiers::none(), w, cx);
    }
    fn sync_zoom(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.zoom_input.update(cx, |e, cx| {
            e.set_value(format!("{:.0}%", self.viewport.zoom * 100.), w, cx)
        });
    }
    fn zoom_by(&mut self, factor: f32, w: &mut Window, cx: &mut Context<Self>) {
        if self.viewer.is_none() || self.modal.is_some() {
            return;
        }
        self.viewport
            .set_zoom(self.viewport.zoom * factor, (0., 0.));
        self.sync_zoom(w, cx);
        cx.notify();
    }
    fn update_marquee(
        &mut self,
        position: Point<Pixels>,
        columns: usize,
        edge: f32,
        cx: &mut Context<Self>,
    ) {
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
        for (i, photo) in photos.iter().enumerate() {
            let x = px(20. + (i % columns) as f32 * (edge + 10.));
            let y = px((i / columns) as f32 * (edge + 36.));
            if x < right && x + px(edge) > left && y < bottom && y + px(edge + 26.) > top {
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
    fn tile(
        &mut self,
        photo: &Photo,
        index: usize,
        edge: f32,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> Div {
        self.request_image(photo.clone(), false, cx);
        let selected = self.selected.contains(&photo.id);
        let thumb = self.thumbnails.get(&photo.fingerprint).cloned();
        let failed = self
            .failed
            .contains(&format!("{}-thumb", photo.fingerprint));
        let drag = DragPhotos {
            ids: Default::default(),
            seed: photo.id.clone(),
            paths: Default::default(),
            source: self.driver.as_ref().unwrap().source().path,
            offset: point(px(0.), px(0.)),
            preview: thumb.clone(),
        };
        let heart_id = photo.id.clone();
        let entity = cx.weak_entity();
        div()
            .w(px(edge))
            .h(px(edge + 26.))
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
                    .rounded(px(9.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .border_2()
                    .border_color(if selected {
                        rgb(c.accent).into()
                    } else {
                        transparent_black()
                    })
                    .bg(rgb(c.bg))
                    .hover(|s| s.opacity(0.87))
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
                            kit::tool("hover-favorite", "heart", "Favorite photo", c)
                                .absolute()
                                .bottom_2()
                                .left_2()
                                .rounded(px(7.))
                                .bg(rgb(c.surface))
                                .when(!photo.favorite, |d| {
                                    d.invisible().group_hover("photo-tile", |s| s.visible())
                                })
                                .when(photo.favorite, |d| d.bg(rgb(c.soft)))
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
                            if let Some(photo) = visible.get(index)
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
                        let _ = entity.update(cx, |this, _| {
                            let ids: Vec<String> = if this.selected.contains(&drag.seed) {
                                this.selected.iter().cloned().collect()
                            } else {
                                vec![drag.seed.clone()]
                            };
                            let paths = ids
                                .iter()
                                .filter_map(|id| {
                                    this.photo_index
                                        .get(id)
                                        .and_then(|&i| this.library.photos.get(i))
                                        .or_else(|| {
                                            this.library.photos.iter().find(|p| &p.id == id)
                                        })
                                        .filter(|p| !p.path.as_os_str().is_empty())
                                        .map(|p| p.path.clone())
                                })
                                .collect();
                            *drag.ids.borrow_mut() = ids;
                            *drag.paths.borrow_mut() = paths;
                        });
                        #[cfg(not(test))]
                        if ui::native_transfer::start_file_drag(w, &drag.paths.borrow()) {
                            w.defer(cx, |w, cx| {
                                cx.stop_active_drag(w);
                            });
                        }
                        #[cfg(test)]
                        let _ = (w, &drag.paths);
                        let mut preview = drag.clone();
                        preview.offset = offset; // keep compact preview beside the pointer, at every thumbnail size
                        cx.new(|_| preview)
                    }),
            )
            .child(
                div()
                    .px_1()
                    .text_size(px(10.))
                    .text_color(rgb(c.muted))
                    .truncate()
                    .child(photo.name.clone()),
            )
    }
    fn grid_view(&mut self, available: f32, c: Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let photos = self.visible(cx);
        let columns = ((available - 40.) / (self.tile_size + 10.)).floor().max(1.) as usize;
        let edge = ((available - 40. - (columns.saturating_sub(1)) as f32 * 10.) / columns as f32)
            .max(40.);
        let rows = photos.len().div_ceil(columns);
        let mut content = div()
            .id("photo-grid")
            .debug_selector(|| "photo-grid".into())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .pt_5()
                    .pb_4()
                    .flex_shrink_0()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(28.))
                                    .font_weight(FontWeight::BOLD)
                                    .truncate()
                                    .child(self.title()),
                            )
                            .when(!photos.is_empty(), |d| {
                                d.child(
                                    div()
                                        .mt_1()
                                        .text_size(px(12.))
                                        .text_color(rgb(c.muted))
                                        .child(format!(
                                            "{} {} · {}",
                                            photos.len(),
                                            if self.has_more {
                                                "photos loaded"
                                            } else {
                                                "photos"
                                            },
                                            photos[0].modified.format("%B %Y")
                                        )),
                                )
                            }),
                    ),
            );
        if photos.is_empty() {
            content = content.child(kit::empty(
                "photos",
                if self.loading {
                    "Loading photos…"
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
            let entity = cx.entity();
            content = content.child(
                uniform_list("photo-rows", rows, move |range, _, cx| {
                    entity.update(cx, |this, cx| {
                        if range.end.saturating_add(3) >= rows {
                            this.load_more(cx);
                        }
                        range
                            .map(|row| {
                                let visible = desert_photos::loading::visible_range(
                                    row..row + 1,
                                    columns,
                                    photos.len(),
                                );
                                let from = visible.start;
                                let to = visible.end;
                                let mut line = div()
                                    .px_5()
                                    .pb(px(10.))
                                    .h(px(edge + 36.))
                                    .flex()
                                    .gap(px(10.));
                                for (offset, photo) in photos[from..to].iter().enumerate() {
                                    line = line.child(this.tile(photo, from + offset, edge, c, cx));
                                }
                                line
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .track_scroll(self.scroll.clone())
                .flex_1()
                .min_h_0(),
            );
        }
        content = content.when(self.has_more, |d| {
            d.child(
                ui::button("load-more-photos", c, false, false)
                    .debug_selector(|| "load-more-photos".into())
                    .child(if self.loading {
                        "Loading photos…"
                    } else if self.page_failed {
                        "Retry loading photos"
                    } else {
                        "Load more photos"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page_failed = false;
                        this.load_more(cx);
                    })),
            )
        });
        content
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, w, cx| {
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
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                if e.dragging() && this.marquee.is_some() {
                    this.update_marquee(e.position, columns, edge, cx);
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
            let path = self
                .previews
                .get(&photo.fingerprint)
                .or_else(|| self.thumbnails.get(&photo.fingerprint))
                .cloned();
            view = view.child(
                div()
                    .h(px(48.))
                    .px_3()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .bg(rgb(c.surface))
                    .child(
                        kit::tool("close-viewer", "back", "Back to library · Esc", c).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.viewer = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .child(photo.name.clone()),
                    )
                    .child(
                        kit::tool("previous-photo", "back", "Previous photo · ←", c)
                            .on_click(cx.listener(|this, _, w, cx| this.step(-1, w, cx))),
                    )
                    .child(
                        kit::tool("next-photo", "next", "Next photo · →", c)
                            .on_click(cx.listener(|this, _, w, cx| this.step(1, w, cx))),
                    )
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
            viewport.image = (photo.width as f32, photo.height as f32);
            let fit = viewport.fit_size();
            let size = (fit.0 * viewport.zoom, fit.1 * viewport.zoom);
            let position = (
                (viewport.viewport.0 - size.0) / 2. + viewport.pan.0,
                (viewport.viewport.1 - size.1) / 2. + viewport.pan.1,
            );
            let entity = cx.entity();
            view = view
                .child(
                    div()
                        .id("viewer-canvas")
                        .debug_selector(|| "viewer-canvas".into())
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .relative()
                        .overflow_hidden()
                        .cursor(if self.pan_start.is_some() {
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
                        .child(if let Some(path) = path {
                            img(Arc::<std::path::Path>::from(path))
                                .image_cache(&self.viewer_render_cache)
                                .absolute()
                                .left(px(position.0))
                                .top(px(position.1))
                                .w(px(size.0))
                                .h(px(size.1))
                                .object_fit(ObjectFit::Contain)
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
                        .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, w, cx| {
                            if this.modal.is_some() {
                                return;
                            }
                            let delta = e.delta.pixel_delta(px(28.));
                            if e.modifiers.platform || e.modifiers.control {
                                let at = e.position - this.viewer_bounds.center();
                                this.viewport.set_zoom(
                                    this.viewport.zoom * (f32::from(delta.y) * 0.006).exp(),
                                    (at.x.into(), at.y.into()),
                                );
                                this.sync_zoom(w, cx);
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
                                this.pan_start = Some(e.position);
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
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, e: &MouseDownEvent, _, cx| {
                                this.menu = Some(e.position);
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    div()
                        .h(px(30.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.))
                        .text_color(rgb(c.muted))
                        .child(format!(
                            "{} × {}  ·  {}",
                            photo.width,
                            photo.height,
                            photo.modified.format("%B %-d, %Y")
                        )),
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
            content = content.child(
                ui::button(
                    SharedString::from(format!("saved-source-{i}")),
                    c,
                    false,
                    !self.busy,
                )
                .w_full()
                .h_auto()
                .py_3()
                .justify_start()
                .child(kit::glyph("drive", 20., c.accent))
                .child(div().flex_1().min_w_0().truncate().child(source.label()))
                .on_click(cx.listener(move |this, _, w, cx| this.connect(source.clone(), w, cx))),
            );
        }
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
            Modal::Sources => "Libraries & appearance",
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
                if !self.settings_appearance {
                    content = content.child(self.source_choices(w, c, cx));
                } else {
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
                        !self.settings_appearance,
                        c,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_appearance = false;
                        cx.notify();
                    })),
                )
                .child(
                    kit::nav_item(
                        "settings-appearance",
                        "eye",
                        "Appearance",
                        None,
                        self.settings_appearance,
                        c,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_appearance = true;
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
                    } else if event.keystroke.key == "tab" && this.provider_step.is_none() {
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
                    this.selected = this.visible(cx).iter().map(|p| p.id.clone()).collect();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSelected, w, cx| this.open_selected(w, cx)))
            .on_action(cx.listener(|this, _: &Back, w, cx| {
                if this.menu.take().is_none()
                    && this.modal.take().is_none()
                    && this.viewer.take().is_none()
                {
                    this.selected.clear();
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
                    .child("Photos"),
            )
            .child(div().flex_1());
        if self.driver.is_some() {
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
        bar = bar
            .child(ui::toolbar_separator(c))
            .child(
                kit::tool("help", "info", "Photos Help", c)
                    .on_click(cx.listener(|this, _, w, cx| this.open_modal(Modal::Help, w, cx))),
            )
            .child(
                kit::tool("settings", "settings", "Libraries & appearance · ⌘,", c)
                    .on_click(cx.listener(|this, _, w, cx| this.open_modal(Modal::Sources, w, cx))),
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
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.tile_size = (this.tile_size - 32.).max(96.);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    kit::tool("larger-tiles", "plus", "Larger thumbnails", c)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.tile_size = (this.tile_size + 32.).min(320.);
                                            cx.notify();
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
                    .child(div().flex_1().min_w(px(70.)).truncate().child(
                        if self.selected.is_empty() {
                            self.notice.clone()
                        } else {
                            format!("{} selected", self.selected.len())
                        },
                    ))
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
