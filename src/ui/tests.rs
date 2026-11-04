use super::*;
use desert_photos::gallery::zoom::{FADE, Phase};
use std::prelude::v1::test;

/// A trackpad pinch that moves the zoom by exactly `levels` ladder slots,
/// then lifts the fingers if `ended`.
fn pinch(app: &mut PhotosApp, levels: f32, ended: bool, cx: &mut Context<PhotosApp>) {
    let (ladder, position) = match &app.gallery_zoom {
        Some(z) => (z.ladder.clone(), z.zoom.position),
        None => {
            let ladder = Ladder::for_width(app.gallery_width, app.gallery_level);
            let position = ladder.index_of(app.gallery_level) as f32;
            (ladder, position)
        }
    };
    let factor = ladder.edge(position + levels) / ladder.edge(position);
    app.zoom_library(ZoomInput::Magnify(factor), true, cx);
    if ended {
        app.zoom_library(ZoomInput::Release, true, cx);
    }
}
fn zoom_state(app: &PhotosApp) -> &Zoom {
    &app.gallery_zoom.as_ref().unwrap().zoom
}

fn frame(cx: &mut VisualTestContext) {
    cx.simulate_event(MouseMoveEvent {
        position: point(px(1.), px(1.)),
        pressed_button: None,
        modifiers: Modifiers::none(),
    });
    cx.run_until_parked();
}
fn bounds(cx: &mut VisualTestContext, name: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(name)
        .unwrap_or_else(|| panic!("Missing {name}"))
}
/// Runs clock-driven animations (photo slides, open/close flights) to the end.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..24 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    frame(cx);
}
fn click(cx: &mut VisualTestContext, name: &'static str) {
    let target = bounds(cx, name);
    cx.simulate_click(target.center(), Modifiers::none());
}
fn inside(cx: &mut VisualTestContext, name: &'static str, width: f32, height: f32) {
    let b = bounds(cx, name);
    assert!(
        b.left() >= px(0.)
            && b.right() <= px(width + 1.)
            && b.top() >= px(0.)
            && b.bottom() <= px(height + 1.),
        "{name}: {b:?} outside {width} × {height}"
    );
}
#[gpui::test]
fn grid_selection_viewer_and_library_controls_work_at_multiple_window_sizes(
    cx: &mut TestAppContext,
) {
    cx.update(ui::init);
    let directory = tempfile::tempdir().unwrap();
    let vault = directory.path().join("library");
    std::fs::create_dir(&vault).unwrap();
    for i in 0..8 {
        image::RgbImage::from_pixel(40, 30, image::Rgb([i * 20, 100, 120]))
            .save(vault.join(format!("Photo {i}.png")))
            .unwrap();
    }
    let driver = Arc::new(storage::LocalPhotos::open(&vault).unwrap());
    driver.create_album("Weekend").unwrap();
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let v = cx.new(|cx| {
            let mut app = PhotosApp::new(directory.path().join("settings.json"), w, cx);
            app.preferences.bottom_to_top = false;
            app.driver = Some(driver.clone());
            app.library = driver.scan().unwrap();
            app.library_changed();
            app
        });
        entity = Some(v.clone());
        Root::new(v, w, cx)
    });
    let entity = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    for (width, height) in [
        (1440., 900.),
        (1060., 700.),
        (960., 600.),
        (820., 500.),
        (640., 480.),
        (480., 420.),
        (360., 360.),
    ] {
        cx.simulate_resize(size(px(width), px(height)));
        frame(cx);
        inside(cx, "photo-grid", width, height);
        // Debug bounds outlive their element, so absence is checked only
        // before the bar has ever been shown.
        if width == 1440. {
            assert!(
                cx.debug_bounds("selection-bar").is_none(),
                "no action bar without a selection"
            );
        }
        let tile = bounds(cx, "photo-tile-0");
        cx.simulate_click(
            point(tile.left() + px(20.), tile.top() + px(20.)),
            Modifiers::none(),
        );
        frame(cx);
        entity.read_with(cx, |app, _| {
            assert_eq!(app.selected.len(), 1, "selection at {width} × {height}")
        });
        inside(cx, "selection-bar", width, height);
        entity.update_in(cx, |app, w, cx| app.open_selected(w, cx));
        frame(cx);
        inside(cx, "photo-viewer", width, height);
        inside(cx, "next-photo", width, height);
        let next = bounds(cx, "next-photo");
        cx.simulate_click(next.center(), Modifiers::none());
        frame(cx);
        entity.read_with(cx, |app, _| {
            assert_eq!(app.anchor, Some(0), "the arrow slides before stepping");
            assert!(app.swipe_settling);
        });
        settle(cx);
        entity.read_with(cx, |app, _| assert_eq!(app.anchor, Some(1)));
        let back = bounds(cx, "close-viewer");
        cx.simulate_click(back.center(), Modifiers::none());
        settle(cx);
        inside(cx, "photo-grid", width, height);
        let settings = bounds(cx, "settings");
        cx.simulate_click(settings.center(), Modifiers::none());
        frame(cx);
        inside(cx, "modal-panel", width, height);
        let close = bounds(cx, "close-modal");
        cx.simulate_click(close.center(), Modifiers::none());
        frame(cx);
        entity.update_in(cx, |app, _, cx| {
            app.selected.clear();
            app.anchor = None;
            cx.notify();
        });
    }
    entity.update_in(cx, |app, w, cx| {
        app.select(0, false, Modifiers::none(), w, cx);
        app.select(
            3,
            false,
            Modifiers {
                shift: true,
                ..Modifiers::none()
            },
            w,
            cx,
        );
        assert_eq!(app.selected.len(), 4);
        app.select(
            2,
            false,
            Modifiers {
                platform: true,
                ..Modifiers::none()
            },
            w,
            cx,
        );
        assert_eq!(app.selected.len(), 3);
    });
}

#[gpui::test]
fn onboarding_exposes_service_logos_and_connection_forms(cx: &mut TestAppContext) {
    cx.update(ui::init);
    let directory = tempfile::tempdir().unwrap();
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let v = cx.new(|cx| PhotosApp::new(directory.path().join("settings.json"), w, cx));
        entity = Some(v.clone());
        Root::new(v, w, cx)
    });
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(360.), px(540.)));
    frame(cx);
    inside(cx, "topbar", 360., 540.);
    inside(cx, "app-brand", 360., 540.);
    let brand = bounds(cx, "app-brand");
    assert!(brand.size.width >= px(20.) && brand.left() < bounds(cx, "topbar").center().x);
    assert!(cx.debug_bounds("welcome").is_some());
    for name in [
        "connect-local-folder",
        "connect-apple-photos",
        "connect-immich",
        "connect-nextcloud",
    ] {
        assert!(cx.debug_bounds(name).is_some(), "missing {name}");
    }
    for (width, height) in [(360., 720.), (820., 740.), (1440., 900.)] {
        cx.simulate_resize(size(px(width), px(height)));
        frame(cx);
        inside(cx, "provider-grid", width, height);
        #[cfg(feature = "remote")]
        {
            for (id, provider) in [
                ("connect-immich", ProviderKind::Immich),
                ("connect-nextcloud", ProviderKind::Nextcloud),
            ] {
                let card = bounds(cx, id);
                cx.simulate_click(card.center(), Modifiers::none());
                frame(cx);
                entity
                    .as_ref()
                    .unwrap()
                    .read_with(cx, |app, _| assert_eq!(app.provider_step, Some(provider)));
                inside(cx, "connect-photo-service", width, height);
                // Invalid forms remain in onboarding and show a useful error.
                let button = bounds(cx, "connect-photo-service");
                cx.simulate_click(button.center(), Modifiers::none());
                frame(cx);
                entity.as_ref().unwrap().read_with(cx, |app, _| {
                    assert!(app.error.is_some() && app.driver.is_none())
                });
                entity.as_ref().unwrap().update_in(cx, |app, _, cx| {
                    app.provider_step = None;
                    app.error = None;
                    cx.notify();
                });
                frame(cx);
            }
        }
    }
}

#[gpui::test]
fn blank_click_marquee_viewer_zoom_delete_confirmation_and_album_spaces(cx: &mut TestAppContext) {
    cx.update(ui::init);
    cx.update(register_shortcuts);
    let directory = tempfile::tempdir().unwrap();
    for i in 0..6 {
        image::RgbImage::from_pixel(160, 120, image::Rgb([i * 30, 100, 150]))
            .save(directory.path().join(format!("Photo {i}.png")))
            .unwrap();
    }
    let driver = Arc::new(storage::LocalPhotos::open(directory.path()).unwrap());
    driver.create_album("Road trip").unwrap();
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let view = cx.new(|cx| {
            let mut app = PhotosApp::new(directory.path().join("profile/settings.json"), w, cx);
            app.preferences.bottom_to_top = false;
            app.driver = Some(driver.clone());
            app.library = driver.scan().unwrap();
            app.library_changed();
            app
        });
        entity = Some(view.clone());
        Root::new(view, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(700.)));
    frame(cx);
    let first = bounds(cx, "photo-tile-0");
    cx.simulate_click(first.center(), Modifiers::none());
    frame(cx);
    let grid = bounds(cx, "photo-grid");
    let blank = point(grid.right() - px(5.), grid.bottom() - px(10.));
    cx.simulate_click(blank, Modifiers::none());
    frame(cx);
    app.read_with(cx, |app, _| assert!(app.selected.is_empty()));
    let last = bounds(cx, "photo-tile-2");
    cx.simulate_event(MouseDownEvent {
        position: point(first.left() - px(5.), first.top() + px(4.)),
        button: MouseButton::Left,
        ..Default::default()
    });
    cx.simulate_event(MouseMoveEvent {
        position: last.center(),
        pressed_button: Some(MouseButton::Left),
        ..Default::default()
    });
    cx.simulate_event(MouseUpEvent {
        position: last.center(),
        button: MouseButton::Left,
        ..Default::default()
    });
    frame(cx);
    app.read_with(cx, |app, _| assert_eq!(app.selected.len(), 3));
    app.update_in(cx, |app, w, cx| app.open_modal(Modal::Album, w, cx));
    frame(cx);
    cx.simulate_keystrokes("m y space a l b u m");
    frame(cx);
    app.read_with(cx, |app, cx| {
        assert_eq!(app.name.read(cx).value().as_ref(), "my album")
    });
    app.update_in(cx, |app, w, cx| {
        app.modal = None;
        app.focus.focus(w);
        cx.notify();
    });
    frame(cx);
    let first = bounds(cx, "photo-tile-0");
    cx.simulate_event(MouseDownEvent {
        position: first.center(),
        button: MouseButton::Left,
        ..Default::default()
    });
    cx.simulate_event(MouseMoveEvent {
        position: first.center() + point(px(30.), px(0.)),
        pressed_button: Some(MouseButton::Left),
        ..Default::default()
    });
    frame(cx);
    assert!(cx.read(|cx| cx.has_active_drag()));
    let album = bounds(cx, "album-Road trip");
    cx.simulate_event(MouseMoveEvent {
        position: album.center(),
        pressed_button: Some(MouseButton::Left),
        ..Default::default()
    });
    cx.simulate_event(MouseUpEvent {
        position: album.center(),
        button: MouseButton::Left,
        ..Default::default()
    });
    frame(cx);
    assert_eq!(driver.scan().unwrap().albums["Road trip"].len(), 3);
    app.update_in(cx, |app, w, cx| {
        app.modal = None;
        app.select(0, true, Modifiers::none(), w, cx);
    });
    frame(cx);
    app.read_with(cx, |app, _| assert_eq!(app.selected.len(), 1));
    app.read_with(cx, |app, _| assert!(app.viewer.is_some()));
    let canvas = bounds(cx, "viewer-canvas");
    cx.simulate_event(ScrollWheelEvent {
        position: canvas.center(),
        delta: ScrollDelta::Lines(point(0., 3.)),
        ..Default::default()
    });
    frame(cx);
    app.read_with(cx, |app, _| {
        assert_eq!(app.viewport.pan, (0., 0.));
        assert_eq!(app.viewport.zoom, 1.);
    });
    cx.simulate_event(ScrollWheelEvent {
        position: canvas.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(100.))),
        modifiers: Modifiers {
            control: true,
            ..Modifiers::none()
        },
        ..Default::default()
    });
    frame(cx);
    app.read_with(cx, |app, _| assert!(app.viewport.zoom > 1.));
    app.update_in(cx, |app, w, cx| app.trash(w, cx));
    frame(cx);
    app.read_with(cx, |app, _| {
        assert!(app.modal == Some(Modal::DeleteConfirm))
    });
    assert!(driver.scan().unwrap().photos.iter().all(|p| !p.deleted));
    let cancel = bounds(cx, "cancel-delete");
    cx.simulate_click(cancel.center(), Modifiers::none());
    frame(cx);
    assert!(driver.scan().unwrap().photos.iter().all(|p| !p.deleted));
    app.update_in(cx, |app, w, cx| app.trash(w, cx));
    frame(cx);
    let confirm = bounds(cx, "confirm-delete");
    cx.simulate_click(confirm.center(), Modifiers::none());
    frame(cx);
    assert_eq!(
        driver
            .scan()
            .unwrap()
            .photos
            .iter()
            .filter(|p| p.deleted)
            .count(),
        1
    );
}

#[derive(Default)]
struct SourceCounters {
    pages: std::sync::atomic::AtomicUsize,
    thumbs: std::sync::atomic::AtomicUsize,
    previews: std::sync::atomic::AtomicUsize,
    originals: std::sync::atomic::AtomicUsize,
    fail_page: std::sync::atomic::AtomicBool,
    total: std::sync::atomic::AtomicUsize,
}
struct StressSource(Arc<SourceCounters>);
fn synthetic_photos(range: std::ops::Range<usize>) -> Vec<Arc<Photo>> {
    range
        .map(|i| {
            Arc::new(Photo {
                id: i.to_string(),
                path: PathBuf::new(),
                name: format!("Photo {i}.jpg"),
                modified: chrono::Utc::now(),
                bytes: 100,
                width: 4000,
                height: 3000,
                favorite: false,
                deleted: false,
                caption: String::new(),
                fingerprint: format!("{i}-v1"),
                duration: None,
            })
        })
        .collect()
}
impl desert_photos::sources::Provider for StressSource {
    fn page(&self, cursor: Option<&str>) -> anyhow::Result<PhotoPage> {
        use std::sync::atomic::Ordering::SeqCst;
        self.0.pages.fetch_add(1, SeqCst);
        if self.0.fail_page.load(SeqCst) {
            anyhow::bail!("Temporary connection failure");
        }
        let start: usize = cursor.unwrap_or("0").parse()?;
        let total = self.0.total.load(SeqCst);
        let total = if total == 0 { 1_000_000 } else { total };
        let end = (start + 200).min(total);
        Ok(PhotoPage {
            library: Library {
                photos: synthetic_photos(start..end),
                albums: Default::default(),
                synced: Default::default(),
            },
            next: (end < total).then(|| end.to_string()),
            total: Some(total),
        })
    }
    fn image(&self, _: &Photo, preview: bool, destination: &std::path::Path) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering::SeqCst;
        if preview {
            &self.0.previews
        } else {
            &self.0.thumbs
        }
        .fetch_add(1, SeqCst);
        image::RgbImage::new(32, 24).save_with_format(destination, image::ImageFormat::Png)?;
        Ok(())
    }
    fn original(&self, _: &str, _: &std::path::Path) -> anyhow::Result<String> {
        self.0
            .originals
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        anyhow::bail!("Scroll and preview must never request originals")
    }
}

#[gpui::test]
fn scrolling_a_million_photo_grid_only_loads_visible_thumbnails(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::SeqCst;
    cx.update(ui::init);
    let directory = tempfile::tempdir().unwrap();
    let counters = Arc::new(SourceCounters::default());
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: directory.path().join("source"),
            },
            Box::new(StressSource(counters.clone())),
        )
        .unwrap(),
    );
    let mut entity = None;
    let start = std::time::Instant::now();
    let window = cx.add_window(|w, cx| {
        let view = cx.new(|cx| {
            let mut app = PhotosApp::new(directory.path().join("settings.json"), w, cx);
            app.driver = Some(driver);
            app.library.photos = synthetic_photos(0..1_000_000);
            app.library_changed();
            app
        });
        entity = Some(view.clone());
        Root::new(view, w, cx)
    });
    let entity = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(700.)));
    frame(cx);
    println!(
        "one million photo GPUI setup and first frame: {:?}",
        start.elapsed()
    );
    let mut timings = vec![];
    let rows = [0, 1000]
        .into_iter()
        .chain((10_000..250_000).step_by(10_000))
        .chain([249_999, 50_000, 100, 0]);
    for row in rows {
        let start = std::time::Instant::now();
        entity.update_in(cx, |app, _, cx| {
            let a = app.visible(cx);
            let b = app.visible(cx);
            assert!(
                a.same_mapping(&b),
                "scrolling must reuse the filtered index"
            );
            scroll_to_row(app, row, cx);
            cx.notify();
        });
        frame(cx);
        timings.push(start.elapsed());
        entity.read_with(cx, |app, cx| {
            assert!(app.thumbnails.len() <= THUMBNAIL_CACHE);
            assert!(app.previews.is_empty());
            assert!(app.pending.len() <= THUMBNAIL_JOBS);
            let cache = app.render_cache.read(cx);
            assert!(cache.len() <= cache.capacity().max(counters.thumbs.load(SeqCst)));
        });
        assert_eq!(counters.previews.load(SeqCst), 0);
        assert_eq!(counters.originals.load(SeqCst), 0);
    }
    println!("one million photo GPUI scroll/frame durations: {timings:?}");
    entity.read_with(cx, |app, cx| {
        // Scrolling back must not refetch: every thumbnail seen stays known.
        assert_eq!(
            app.thumbnails.len(),
            counters.thumbs.load(SeqCst),
            "thumbnails must not be forgotten while scrolling"
        );
        assert!(
            app.render_cache.read(cx).len() > 96,
            "decoded thumbnails outlive one screen"
        );
    });
    // ~30 positions × (one screen + 1.5 prefetched screens either way).
    assert!(
        counters.thumbs.load(SeqCst) < 4000,
        "virtualization must not fetch an entire library"
    );
    entity.update_in(cx, |app, w, cx| {
        app.select(0, true, Modifiers::none(), w, cx)
    });
    frame(cx);
    assert_eq!(counters.previews.load(SeqCst), 1);
    assert_eq!(counters.originals.load(SeqCst), 0);
}

#[gpui::test]
fn scroll_pages_retry_and_request_limits_are_enforced(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::SeqCst;
    cx.update(ui::init);
    let directory = tempfile::tempdir().unwrap();
    let counters = Arc::new(SourceCounters::default());
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: directory.path().join("source"),
            },
            Box::new(StressSource(counters.clone())),
        )
        .unwrap(),
    );
    counters.total.store(1200, SeqCst);
    let first = driver.page(None).unwrap();
    counters.fail_page.store(true, SeqCst);
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let view = cx.new(|cx| {
            let mut app = PhotosApp::new(directory.path().join("settings.json"), w, cx);
            app.finish_connection(Ok((driver.clone(), first)), w, cx);
            app
        });
        entity = Some(view.clone());
        Root::new(view, w, cx)
    });
    let entity = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(700.)));
    frame(cx);
    assert_eq!(counters.pages.load(SeqCst), 2);
    assert_eq!(counters.previews.load(SeqCst), 0);
    assert_eq!(counters.originals.load(SeqCst), 0);
    entity.read_with(cx, |app, _| {
        assert!(app.page_failed && app.has_more);
        assert_eq!(app.total, Some(1200));
        assert_eq!(app.preferences.catalog_counts.values().next(), Some(&1200));
    });
    let pages = counters.pages.load(SeqCst);
    frame(cx);
    frame(cx);
    assert_eq!(
        pages,
        counters.pages.load(SeqCst),
        "a failed page must not spin on every frame"
    );
    counters.fail_page.store(false, SeqCst);
    let retry = bounds(cx, "retry-catalog");
    cx.simulate_click(retry.center(), Modifiers::none());
    frame(cx);
    entity.read_with(cx, |app, _| {
        assert!(!app.page_failed && !app.has_more && !app.loading);
        assert_eq!(app.library.photos.len(), 1200);
        assert_eq!(app.total, Some(1200));
    });
    assert!(cx.debug_bounds("load-more-photos").is_none());
    entity.update_in(cx, |app, _, cx| {
        app.pending.clear();
        for photo in synthetic_photos(100_000..100_100) {
            app.request_image(&photo, false, cx);
        }
        assert_eq!(app.pending.len(), THUMBNAIL_JOBS);
        for photo in synthetic_photos(100_000..100_100) {
            app.request_image(&photo, true, cx);
        }
        // The photo on screen never waits behind a neighbour's prefetch.
        assert_eq!(app.pending.len(), THUMBNAIL_JOBS + 2);
    });
    frame(cx);
    assert_eq!(counters.originals.load(SeqCst), 0);
}

#[gpui::test]
fn accent_settings_removal_gapless_gallery_and_side_navigation(cx: &mut TestAppContext) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    let library = data.path().join("library");
    std::fs::create_dir(&library).unwrap();
    for i in 0..24 {
        image::RgbImage::from_pixel(120, 80, image::Rgb([i * 8, 90, 140]))
            .save(library.join(format!("Photo {i}.png")))
            .unwrap();
    }
    let driver: Arc<dyn PhotosDriver> = Arc::new(storage::LocalPhotos::open(&library).unwrap());
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("profile/settings.json"), w, cx);
            let page = driver.page(None).unwrap();
            app.finish_connection(Ok((driver.clone(), page)), w, cx);
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(760.)));
    frame(cx);
    app.update_in(cx, |app, w, cx| app.open_modal(Modal::Sources, w, cx));
    frame(cx);
    click(cx, "settings-appearance");
    frame(cx);
    click(cx, "accent-purple");
    frame(cx);
    app.read_with(cx, |app, _| assert_eq!(app.preferences.accent, "purple"));
    assert_eq!(
        Preferences::load(&data.path().join("profile/settings.json"))
            .unwrap()
            .accent,
        "purple"
    );
    app.update_in(cx, |app, w, cx| {
        app.modal = None;
        app.focus.focus(w);
        cx.notify();
    });
    frame(cx);
    app.update_in(cx, |app, _, cx| {
        pinch(app, -0.6, false, cx);
        let z = app.gallery_zoom.as_ref().unwrap();
        let default = GalleryLayout::DEFAULT_LEVEL;
        assert_eq!(app.gallery_level, default);
        assert!((z.zoom.position - (default as f32 - 0.6)).abs() < 1e-3);
        let (a, b, t) = z.ladder.pair(z.zoom.position);
        assert_eq!(
            (a, b),
            (default - 1, default),
            "between the levels around the size"
        );
        assert!((t - 0.4).abs() < 1e-3);
        assert_eq!(z.zoom.phase, Phase::Held);
    });
    // Holding the fingers still never snaps back on its own.
    for _ in 0..30 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    app.read_with(cx, |app, _| {
        let z = zoom_state(app);
        assert_eq!(z.phase, Phase::Held);
        assert!((z.position - (GalleryLayout::DEFAULT_LEVEL as f32 - 0.6)).abs() < 1e-3);
    });
    // Lifting lands on the level the photos were already fading to.
    app.update_in(cx, |app, _, cx| {
        app.zoom_library(ZoomInput::Release, true, cx)
    });
    advance_gallery_animation(cx);
    app.read_with(cx, |app, _| {
        assert!(app.gallery_zoom.is_none());
        assert_eq!(app.gallery_level, GalleryLayout::DEFAULT_LEVEL - 1)
    });
    // Margined levels go out to 9 columns, then straight to month sections.
    for level in [6usize] {
        advance_gallery_animation(cx);
        app.read_with(cx, |app, cx| {
            assert!(
                app.gallery_layout(cx).gap > 0.,
                "level {} is margined",
                level + 1
            )
        });
        app.update_in(cx, |app, w, cx| app.gallery_step(false, w, cx));
        advance_gallery_animation(cx);
        app.read_with(cx, |app, _| assert_eq!(app.gallery_level, level));
    }
    let a = bounds(cx, "photo-tile-0");
    let heart = bounds(cx, "photo-heart-0");
    assert!((heart.size.width / a.size.width - 0.08).abs() < 0.025);
    assert!(
        (a.size.width / a.size.height - 120. / 80.).abs() < 0.05,
        "margined photos keep their aspect ratio"
    );
    app.read_with(cx, |app, cx| assert_eq!(app.gallery_layout(cx).columns, 9));
    app.update_in(cx, |app, w, cx| app.gallery_step(false, w, cx));
    advance_gallery_animation(cx);
    app.read_with(cx, |app, cx| {
        let layout = app.gallery_layout(cx);
        assert_eq!(layout.tier, Tier::Months);
        assert!(layout.full_bleed(), "month grids have no gutter");
    });
    inside(cx, "gallery-date-label", 1060., 760.);
    assert!(cx.debug_bounds("dense-grid").is_some());
    let scrollbar = bounds(cx, "gallery-scrollbar-layer");
    assert!((scrollbar.right() - bounds(cx, "photo-grid").right()).abs() < px(1.));
    app.update_in(cx, |app, w, cx| {
        app.select(0, true, Modifiers::none(), w, cx);
    });
    frame(cx);
    let canvas = bounds(cx, "viewer-canvas");
    let previous = bounds(cx, "previous-photo");
    let next = bounds(cx, "next-photo");
    assert!(previous.center().x < canvas.center().x && next.center().x > canvas.center().x);
    cx.simulate_click(next.center(), Modifiers::none());
    settle(cx);
    app.read_with(cx, |app, _| assert_eq!(app.anchor, Some(1)));
    click(cx, "previous-photo");
    settle(cx);
    app.read_with(cx, |app, _| assert_eq!(app.anchor, Some(0)));
    app.update_in(cx, |app, w, cx| {
        app.settings_page = 0;
        app.open_modal(Modal::Sources, w, cx);
    });
    frame(cx);
    click(cx, "remove-library-0");
    frame(cx);
    app.read_with(cx, |app, _| {
        assert!(app.preferences.sources.is_empty());
        assert!(app.driver.is_none());
        assert!(app.library.photos.is_empty());
    });
    assert!(library.join("Photo 0.png").exists());
    assert!(cx.debug_bounds("provider-grid").is_some());
}

#[gpui::test]
fn million_photo_catalog_finishes_automatically_without_fetching_offscreen_images(
    cx: &mut TestAppContext,
) {
    use std::sync::atomic::Ordering::SeqCst;
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    let counters = Arc::new(SourceCounters::default());
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: data.path().join("source"),
            },
            Box::new(StressSource(counters.clone())),
        )
        .unwrap(),
    );
    let first = driver.page(None).unwrap();
    let mut entity = None;
    let start = std::time::Instant::now();
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("profile/settings.json"), w, cx);
            app.finish_connection(Ok((driver, first)), w, cx);
            assert!(app.loading_status().is_some());
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(700.)));
    frame(cx);
    app.read_with(cx, |app, _| {
        assert_eq!(app.library.photos.len(), 1_000_000);
        assert_eq!(app.total, Some(1_000_000));
        assert!(!app.has_more && !app.loading && !app.page_failed);
        assert!(app.loading_status().is_none());
        assert!(
            app.visible_cache.borrow().is_none(),
            "unfiltered catalog growth must not clone the whole photo index"
        );
    });
    assert_eq!(counters.pages.load(SeqCst), 5000);
    assert!(counters.thumbs.load(SeqCst) < 100);
    assert_eq!(counters.previews.load(SeqCst), 0);
    assert_eq!(counters.originals.load(SeqCst), 0);
    assert!(cx.debug_bounds("load-more-photos").is_none());
    println!(
        "automatic million-photo metadata catalog plus initial thumbnails: {:?}",
        start.elapsed()
    );
}

#[gpui::test]
fn thumbnail_remains_while_viewer_upgrade_is_not_yet_decoded(cx: &mut TestAppContext) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    image::RgbImage::from_pixel(160, 100, image::Rgb([140, 80, 100]))
        .save(data.path().join("Photo.png"))
        .unwrap();
    let driver: Arc<dyn PhotosDriver> = Arc::new(storage::LocalPhotos::open(data.path()).unwrap());
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("profile/settings.json"), w, cx);
            app.finish_connection(Ok((driver.clone(), driver.page(None).unwrap())), w, cx);
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(760.)));
    frame(cx);
    app.update_in(cx, |app, w, cx| {
        app.select(0, true, Modifiers::none(), w, cx)
    });
    for _ in 0..4 {
        cx.update(|w, cx| {
            w.refresh();
            let _ = w.draw(cx);
        });
        cx.run_until_parked();
    }
    app.read_with(cx, |app, _| {
        assert!(
            app.decoded_preview.is_some(),
            "previews={}, failed={}, pending={}",
            app.previews.len(),
            app.failed.len(),
            app.pending.len()
        )
    });
    app.update_in(cx, |app, w, cx| {
        app.viewer_render_cache
            .update(cx, |cache, cx| cache.clear(w, cx));
        app.decoded_preview = None;
        assert!(app.decoded_preview.is_none());
        assert!(!app.thumbnails.is_empty());
        assert!(app.image_loading());
        cx.notify();
    });
    for _ in 0..4 {
        cx.update(|w, cx| {
            w.refresh();
            let _ = w.draw(cx);
        });
        cx.run_until_parked();
    }
    app.read_with(cx, |app, _| {
        assert!(app.decoded_preview.is_some());
        assert!(!app.image_loading());
    });
    assert!(cx.debug_bounds("viewer-image-layers").is_some());
}

fn anchor_screen_y(app: &PhotosApp, anchor: &GalleryAnchor, cx: &App) -> f32 {
    let photos = app.visible(cx);
    let index = photos.position(app.photo_index[&anchor.id]).unwrap();
    let layout = app.gallery_layout(cx);
    layout.cell(index).y
        + anchor.within * layout.edge
        + f32::from(app.scroll.0.borrow().base_handle.offset().y)
}
fn assert_at_bottom(app: &PhotosApp, cx: &App) {
    let layout = app.gallery_layout(cx);
    let h = &app.scroll.0.borrow().base_handle;
    let max = (layout.height - f32::from(h.bounds().size.height)).max(0.);
    assert!(
        (f32::from(-h.offset().y) - max).abs() < 2.,
        "offset={:?} max={max}",
        h.offset()
    );
}

#[gpui::test]
fn bottom_first_gallery_anchors_catalog_growth_resize_filters_and_direction(
    cx: &mut TestAppContext,
) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    let counters = Arc::new(SourceCounters::default());
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: data.path().join("source"),
            },
            Box::new(StressSource(counters)),
        )
        .unwrap(),
    );
    let mut app = None;
    let window = cx.add_window(|w, cx| {
        let entity = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("settings.json"), w, cx);
            app.driver = Some(driver);
            app.library.photos = synthetic_photos(0..1003);
            app.library_changed();
            app
        });
        app = Some(entity.clone());
        Root::new(entity, w, cx)
    });
    let app = app.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(760.)));
    frame(cx);
    app.read_with(cx, |app, cx| {
        assert!(app.preferences.bottom_to_top);
        assert_at_bottom(app, cx);
        assert_eq!(app.visible(cx).get(&app.library, 1002).unwrap().id, "0");
    });
    // Older remote metadata is inserted above the current view; the latest photos stay pinned.
    app.update_in(cx, |app, _, cx| {
        app.preserve_gallery(cx);
        app.library.photos.extend(synthetic_photos(1003..1200));
        app.library_changed();
        cx.notify();
    });
    frame(cx);
    app.read_with(cx, assert_at_bottom);
    // A user browsing older photos must never be pulled back to the bottom.
    app.update_in(cx, |app, _, cx| {
        app.scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.), px(-4000.)));
        cx.notify();
    });
    frame(cx);
    let anchor = app.read_with(cx, |app, cx| app.capture_gallery_anchor(cx).unwrap());
    app.update_in(cx, |app, _, cx| {
        app.preserve_gallery(cx);
        app.library.photos.extend(synthetic_photos(1200..1397));
        app.library_changed();
        cx.notify();
    });
    frame(cx);
    app.read_with(cx, |app, cx| {
        assert!((anchor_screen_y(app, &anchor, cx) - anchor.screen).abs() < 1.)
    });
    // The direction switch keeps the same image under the same point.
    app.update_in(cx, |app, _, cx| app.set_scroll_direction(false, cx));
    frame(cx);
    app.read_with(cx, |app, cx| {
        assert!((anchor_screen_y(app, &anchor, cx) - anchor.screen).abs() < 1.)
    });
    assert!(
        !Preferences::load(&data.path().join("settings.json"))
            .unwrap()
            .bottom_to_top
    );
    app.update_in(cx, |app, w, cx| {
        app.set_scroll_direction(true, cx);
        app.set_filter(Filter::All, w, cx);
    });
    frame(cx);
    app.read_with(cx, assert_at_bottom);
    cx.simulate_resize(size(px(960.), px(500.)));
    frame(cx);
    frame(cx);
    app.read_with(cx, assert_at_bottom);
    app.update_in(cx, |app, w, cx| app.gallery_step(false, w, cx));
    frame(cx);
    frame(cx);
    app.read_with(cx, assert_at_bottom);
    // New local arrivals while at the bottom remain visible, including incomplete rows.
    app.update_in(cx, |app, _, cx| {
        app.preserve_gallery(cx);
        app.library
            .photos
            .insert(0, synthetic_photos(9999..10000).pop().unwrap());
        app.library_changed();
        cx.notify();
    });
    frame(cx);
    app.read_with(cx, assert_at_bottom);
    // Empty filters and short libraries have a valid, bounded scroll position.
    app.update_in(cx, |app, w, cx| app.set_filter(Filter::Favorites, w, cx));
    frame(cx);
    app.read_with(cx, |app, cx| assert!(app.visible(cx).is_empty()));
    app.update_in(cx, |app, w, cx| {
        app.library.photos.truncate(3);
        app.library_changed();
        app.set_filter(Filter::All, w, cx);
    });
    frame(cx);
    app.read_with(cx, assert_at_bottom);
}

#[gpui::test]
fn viewer_swipes_settle_once_pan_only_when_zoomed_and_toolbars_frame_image(
    cx: &mut TestAppContext,
) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    for i in 0..8 {
        image::RgbImage::from_pixel(160, 100, image::Rgb([i * 20, 100, 90]))
            .save(data.path().join(format!("Photo {i}.png")))
            .unwrap();
    }
    let driver: Arc<dyn PhotosDriver> = Arc::new(storage::LocalPhotos::open(data.path()).unwrap());
    let mut app = None;
    let window = cx.add_window(|w, cx| {
        let entity = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("profile/settings.json"), w, cx);
            app.finish_connection(Ok((driver.clone(), driver.page(None).unwrap())), w, cx);
            app
        });
        app = Some(entity.clone());
        Root::new(entity, w, cx)
    });
    let app = app.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(760.)));
    frame(cx);
    app.update_in(cx, |app, w, cx| {
        app.select(3, true, Modifiers::none(), w, cx)
    });
    frame(cx);
    let canvas = bounds(cx, "viewer-canvas");
    let toolbar = bounds(cx, "viewer-toolbar");
    let bottom = bounds(cx, "photo-status");
    assert!((canvas.top() - toolbar.bottom()).abs() < px(1.));
    assert!((canvas.bottom() - bottom.top()).abs() < px(1.));
    assert!((bounds(cx, "viewer-date").center().x - canvas.center().x).abs() < px(1.));
    assert!((bounds(cx, "viewer-file-info").center().x - canvas.center().x).abs() < px(1.));
    app.read_with(cx, |app, _| assert!(!app.can_pan()));
    for (dx, phase) in [
        (-10., TouchPhase::Started),
        (-100., TouchPhase::Moved),
        (0., TouchPhase::Ended),
    ] {
        cx.simulate_event(ScrollWheelEvent {
            position: canvas.center(),
            delta: ScrollDelta::Pixels(point(px(dx), px(0.))),
            touch_phase: phase,
            ..Default::default()
        });
    }
    // Advance each frame of the timer-based settle animation.
    cx.run_until_parked();
    cx.run_until_parked();
    for _ in 0..14 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    app.read_with(cx, |app, _| {
        assert_eq!(app.anchor, Some(4));
        assert_eq!(app.swipe_offset, 0.);
        assert!(!app.swipe_settling);
    });
    // Momentum after touch-end must not advance a second image.
    app.update_in(cx, |app, w, cx| {
        app.viewer_swipe(-200., 0., TouchPhase::Moved, w, cx);
        assert_eq!(app.anchor, Some(4));
        assert_eq!(app.swipe_offset, 0.);
        app.zoom_by(1.2, w, cx);
        assert!(app.can_pan());
    });
    frame(cx);
    app.update_in(cx, |app, w, cx| {
        app.zoom_by(0.5, w, cx);
        assert!(!app.can_pan());
        app.select(0, true, Modifiers::none(), w, cx);
        app.viewer_swipe(200., 0., TouchPhase::Started, w, cx);
        app.viewer_swipe(0., 0., TouchPhase::Ended, w, cx);
    });
    cx.run_until_parked();
    for _ in 0..14 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    frame(cx);
    app.read_with(cx, |app, _| assert_eq!(app.anchor, Some(0)));
    // A small cancelled gesture settles back without changing selection.
    app.update_in(cx, |app, w, cx| {
        app.viewer_swipe(-12., 0., TouchPhase::Started, w, cx);
        app.viewer_swipe(0., 0., TouchPhase::Ended, w, cx);
    });
    cx.run_until_parked();
    for _ in 0..14 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    frame(cx);
    app.read_with(cx, |app, _| {
        assert_eq!(app.anchor, Some(0));
        assert_eq!(app.swipe_offset, 0.);
    });
}

fn paint_frames(cx: &mut VisualTestContext, count: usize) {
    for _ in 0..count {
        cx.update(|w, cx| {
            w.refresh();
            let _ = w.draw(cx);
        });
        cx.run_until_parked();
    }
}

#[gpui::test]
fn dense_gallery_keeps_every_visible_thumbnail_decoded_after_zoom_and_scroll(
    cx: &mut TestAppContext,
) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    let counters = Arc::new(SourceCounters::default());
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: data.path().join("source"),
            },
            Box::new(StressSource(counters.clone())),
        )
        .unwrap(),
    );
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("settings.json"), w, cx);
            app.driver = Some(driver);
            app.library.photos = synthetic_photos(0..3000);
            app.library_changed();
            app.gallery_level = 0;
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(640.), px(1200.)));
    frame(cx);
    paint_frames(cx, 8);
    app.read_with(cx, assert_dense_ready);
    app.update_in(cx, |app, _, cx| {
        scroll_to_row(app, 140, cx);
        cx.notify();
    });
    frame(cx);
    paint_frames(cx, 8);
    app.read_with(cx, assert_dense_ready);
    let pointer = bounds(cx, "photo-grid").center();
    app.update_in(cx, |app, _, cx| {
        pinch(app, 0.55, false, cx);
        assert!((zoom_state(app).position - 0.55).abs() < 0.001);
    });
    paint_frames(cx, 2);
    assert!(cx.debug_bounds("gallery-motion").is_some());
    // Scroll interrupts an active pinch; the real virtual grid must remain live.
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: ScrollDelta::Pixels(point(px(0.), px(-80.))),
        touch_phase: TouchPhase::Started,
        ..Default::default()
    });
    frame(cx);
    app.read_with(cx, |app, _| {
        assert!(app.gallery_zoom.is_none());
    });
    app.update_in(cx, |app, _, cx| pinch(app, 0.7, true, cx));
    cx.run_until_parked();
    for _ in 0..5 {
        cx.executor().advance_clock(Duration::from_millis(16));
        paint_frames(cx, 1);
    }
    app.update_in(cx, |app, _, cx| {
        let before = zoom_state(app).position;
        pinch(app, -0.15, false, cx);
        let after = zoom_state(app).position;
        assert!(
            (before - after - 0.15).abs() < 0.001,
            "grabbing a settling zoom must continue from what is on screen"
        );
        pinch(app, 0., true, cx);
    });
    advance_gallery_animation(cx);
    app.read_with(cx, |app, _| assert!(app.gallery_zoom.is_none()));
    app.update_in(cx, |app, _, cx| {
        let step = ZoomInput::Step(-(app.gallery_level as i32));
        app.zoom_library(step, false, cx)
    });
    advance_gallery_animation(cx);
    paint_frames(cx, 8);
    app.read_with(cx, assert_dense_ready);
    app.update_in(cx, |app, _, cx| {
        scroll_to_row(app, 20, cx);
        cx.notify();
    });
    frame(cx);
    paint_frames(cx, 8);
    app.read_with(cx, assert_dense_ready);
    app.read_with(cx, |app, cx| {
        assert!(app.thumbnails.len() <= app.render_cache.read(cx).capacity().max(THUMBNAIL_CACHE))
    });
    use std::sync::atomic::Ordering::SeqCst;
    assert_eq!(counters.previews.load(SeqCst), 0);
    assert_eq!(counters.originals.load(SeqCst), 0);
}

fn scroll_to_row(app: &PhotosApp, row: usize, cx: &App) {
    let layout = app.gallery_layout(cx);
    app.set_gallery_offset(row as f32 * layout.pitch);
}

#[gpui::test]
fn command_scroll_zooms_without_scrolling_and_chains_wheel_notches(cx: &mut TestAppContext) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: data.path().join("source"),
            },
            Box::new(StressSource(Arc::new(SourceCounters::default()))),
        )
        .unwrap(),
    );
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("settings.json"), w, cx);
            app.driver = Some(driver);
            app.library.photos = synthetic_photos(0..2000);
            app.library_changed();
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(760.)));
    frame(cx);
    app.update_in(cx, |app, _, cx| {
        scroll_to_row(app, 60, cx);
        cx.notify();
    });
    frame(cx);
    let pointer = bounds(cx, "photo-grid").center();
    let wheel = |cx: &mut VisualTestContext, y: f32| {
        cx.simulate_event(ScrollWheelEvent {
            position: pointer,
            delta: ScrollDelta::Lines(point(0., y)),
            modifiers: Modifiers {
                platform: true,
                ..Default::default()
            },
            touch_phase: TouchPhase::Moved,
        });
    };
    // One notch is one full level: it animates and settles rather than snapping back.
    wheel(cx, -1.);
    app.read_with(cx, |app, _| {
        let target = GalleryLayout::DEFAULT_LEVEL - 1;
        assert_eq!(zoom_state(app).phase, Phase::Settling { target });
    });
    advance_gallery_animation(cx);
    app.read_with(cx, |app, _| {
        assert!(app.gallery_zoom.is_none());
        assert_eq!(app.gallery_level, GalleryLayout::DEFAULT_LEVEL - 1);
    });
    // Fast notches retarget the running spring instead of cutting it short.
    let offset = app.read_with(cx, |app, _| app.scroll.0.borrow().base_handle.offset());
    wheel(cx, -1.);
    for _ in 0..3 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    let (value, velocity) = app.read_with(cx, |app, _| {
        let z = zoom_state(app);
        (z.position, z.velocity)
    });
    assert!(velocity < 0., "already zooming out");
    wheel(cx, -1.);
    app.read_with(cx, |app, _| {
        let z = zoom_state(app);
        assert!(
            z.position <= value + 1e-4,
            "a new notch must not rewind the zoom"
        );
        assert_eq!(z.velocity, velocity, "and keeps its momentum");
        let target = GalleryLayout::DEFAULT_LEVEL - 3;
        assert_eq!(z.phase, Phase::Settling { target });
    });
    for _ in 0..40 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    frame(cx);
    app.read_with(cx, |app, _| {
        assert!(app.gallery_zoom.is_none());
        assert_eq!(app.gallery_level, GalleryLayout::DEFAULT_LEVEL - 3);
    });
    // A plain scroll still scrolls; a ⌘-scroll never moves the list itself.
    let settled = app.read_with(cx, |app, _| app.scroll.0.borrow().base_handle.offset());
    assert_ne!(settled, offset, "zoom re-anchors the grid");
    wheel(cx, 0.);
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: ScrollDelta::Pixels(point(px(0.), px(30.))),
        modifiers: Modifiers {
            platform: true,
            ..Default::default()
        },
        touch_phase: TouchPhase::Started,
    });
    // A trackpad ⌘-scroll steps at once, animated like a notch, and then
    // only after a longer travel.
    app.read_with(cx, |app, _| {
        assert_eq!(app.scroll.0.borrow().base_handle.offset(), settled);
        let target = GalleryLayout::DEFAULT_LEVEL - 2;
        assert_eq!(zoom_state(app).phase, Phase::Settling { target });
    });
    let trackpad = |cx: &mut VisualTestContext, y: f32, touch_phase: TouchPhase| {
        cx.simulate_event(ScrollWheelEvent {
            position: pointer,
            delta: ScrollDelta::Pixels(point(px(0.), px(y))),
            modifiers: Modifiers {
                platform: true,
                ..Default::default()
            },
            touch_phase,
        });
    };
    for _ in 0..4 {
        trackpad(cx, 30., TouchPhase::Moved);
    }
    app.read_with(cx, |app, _| {
        let target = GalleryLayout::DEFAULT_LEVEL - 2;
        assert_eq!(zoom_state(app).phase, Phase::Settling { target });
    });
    for _ in 0..3 {
        trackpad(cx, 30., TouchPhase::Moved);
    }
    app.read_with(cx, |app, _| {
        let target = GalleryLayout::DEFAULT_LEVEL - 1;
        assert_eq!(zoom_state(app).phase, Phase::Settling { target });
    });
    // Turning around steps straight back.
    trackpad(cx, -20., TouchPhase::Moved);
    trackpad(cx, 0., TouchPhase::Ended);
    app.read_with(cx, |app, _| {
        let target = GalleryLayout::DEFAULT_LEVEL - 2;
        assert_eq!(zoom_state(app).phase, Phase::Settling { target });
    });
    advance_gallery_animation(cx);
    app.read_with(cx, |app, _| {
        assert!(app.gallery_zoom.is_none());
        assert_eq!(app.gallery_level, GalleryLayout::DEFAULT_LEVEL - 2);
    });
}

#[gpui::test]
fn zoomed_out_tiers_group_months_then_years_and_click_through_painted_tiles(
    cx: &mut TestAppContext,
) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: data.path().join("source"),
            },
            Box::new(StressSource(Arc::new(SourceCounters::default()))),
        )
        .unwrap(),
    );
    // Newest first, one photo every 3 days across roughly two and a half years.
    let start = chrono::Utc::now();
    let photos: Vec<_> = synthetic_photos(0..300)
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let mut p = (*p).clone();
            p.modified = start - chrono::Duration::days(3 * i as i64);
            Arc::new(p)
        })
        .collect();
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("settings.json"), w, cx);
            app.driver = Some(driver);
            app.library.photos = photos;
            app.library_changed();
            app.preferences.bottom_to_top = false;
            app.gallery_level = 4;
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(760.)));
    frame(cx);
    paint_frames(cx, 2);
    let layout = app.read_with(cx, |app, cx| app.gallery_layout(cx));
    assert_eq!(layout.tier, Tier::Months);
    assert!(layout.sections.len() >= 29, "one section per month");
    // Every month starts its own row under its own header.
    app.read_with(cx, |app, cx| {
        let photos = app.visible(cx);
        for s in layout.sections.iter() {
            let first = photos.get(&app.library, s.start).unwrap();
            let last = photos.get(&app.library, s.start + s.len - 1).unwrap();
            let month = |p: &Photo| {
                let l = p.modified.with_timezone(&chrono::Local);
                (l.year(), l.month())
            };
            assert_eq!(month(first), month(last));
            assert_eq!(layout.cell(s.start).x, 0.);
        }
    });
    inside(cx, "gallery-date-label", 1060., 760.);
    assert!(
        cx.debug_bounds("gallery-section-1").is_some(),
        "the next month's header is drawn inline"
    );
    // Painted tiles are hit-tested: a click selects, a double click opens.
    let grid = bounds(cx, "photo-rows");
    let cell = layout.cell(layout.sections[1].start + 2);
    let at = grid.origin + point(px(cell.x + cell.edge / 2.), px(cell.y + cell.edge / 2.));
    cx.simulate_click(at, Modifiers::none());
    frame(cx);
    app.read_with(cx, |app, cx| {
        let id = app
            .visible(cx)
            .get(&app.library, layout.sections[1].start + 2)
            .unwrap()
            .id
            .clone();
        assert!(app.selected.contains(&id) && app.selected.len() == 1);
    });
    // Further out, months merge into years with larger steps per notch.
    for _ in 0..2 {
        app.update_in(cx, |app, w, cx| app.gallery_step(false, w, cx));
        advance_gallery_animation(cx);
    }
    let years = app.read_with(cx, |app, cx| app.gallery_layout(cx));
    assert_eq!(years.tier, Tier::Years);
    assert!(years.sections.len() >= 3 && years.sections.len() <= 4);
    assert!(layout.edge / years.edge > 1.3);
    assert!(years.header > layout.header);
}

#[gpui::test]
fn five_thousand_painted_tiles_scroll_and_zoom_without_per_tile_cost(cx: &mut TestAppContext) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    let counters = Arc::new(SourceCounters::default());
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: data.path().join("source"),
            },
            Box::new(StressSource(counters.clone())),
        )
        .unwrap(),
    );
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("settings.json"), w, cx);
            app.driver = Some(driver);
            app.library.photos = synthetic_photos(0..60_000);
            app.library_changed();
            app.gallery_level = 0;
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1500.), px(980.)));
    frame(cx);
    paint_frames(cx, 30);
    let on_screen = app.read_with(cx, |app, cx| {
        let layout = app.gallery_layout(cx);
        let offset = app.gallery_offset();
        let height = f32::from(app.scroll_handle().bounds().size.height);
        layout
            .visible(offset, offset + height)
            .into_iter()
            .flatten()
            .count()
    });
    assert!(on_screen >= 5000, "only {on_screen} photos on screen");
    app.read_with(cx, assert_dense_ready);
    // Steady-state frames: scroll a row at a time, as momentum scrolling does.
    let mut times = vec![];
    // PHOTOS_BENCH_FRAMES lengthens the run for profiling.
    let frames = std::env::var("PHOTOS_BENCH_FRAMES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(40);
    eprintln!("steady frames start");
    for step in 0..frames {
        app.update_in(cx, |app, _, cx| {
            app.set_gallery_offset(app.gallery_offset() + 13. * (step % 7) as f32);
            cx.notify();
        });
        let start = std::time::Instant::now();
        cx.update(|w, cx| {
            w.refresh();
            let _ = w.draw(cx);
        });
        times.push(start.elapsed());
        cx.run_until_parked();
    }
    times.sort();
    let median = times[times.len() / 2];
    println!(
        "{on_screen} painted tiles: median frame {median:?}, max {:?}",
        times.last().unwrap()
    );
    // Release builds measure ~1 ms; leave headroom for slower machines.
    if !cfg!(debug_assertions) {
        assert!(
            median < Duration::from_millis(4),
            "a frame of {on_screen} tiles took {median:?}"
        );
    }
    // Mid-crossfade between two dense levels: both grids drawn, each scaled
    // whole to the size between them.
    app.update_in(cx, |app, _, cx| {
        pinch(app, 0.45, false, cx);
        let z = app.gallery_zoom.as_ref().unwrap();
        let (_, _, t) = z.ladder.pair(z.zoom.position);
        assert!(t > 0.4 && t < 0.6, "half way between two levels");
    });
    cx.executor().advance_clock(FADE / 2);
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        let z = app.gallery_zoom.as_ref().unwrap();
        let layers = z.content.layers(cx.background_executor().now());
        assert_eq!(layers.len(), 2, "both levels' photos are drawn");
    });
    let start = std::time::Instant::now();
    cx.update(|w, cx| {
        w.refresh();
        let _ = w.draw(cx);
    });
    let transition = start.elapsed();
    println!("transition frame: {transition:?}");
    if !cfg!(debug_assertions) {
        assert!(transition < Duration::from_millis(8));
    }
}

fn advance_gallery_animation(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..50 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    frame(cx);
}

fn assert_dense_ready(app: &PhotosApp, cx: &App) {
    let view = app.visible(cx);
    let layout = app.gallery_layout(cx);
    let handle = app.scroll_handle();
    let offset = f32::from(-handle.offset().y);
    let height = f32::from(handle.bounds().size.height);
    let visible: Vec<usize> = layout
        .visible(offset, offset + height)
        .into_iter()
        .flatten()
        .filter(|&i| {
            let c = layout.cell(i);
            c.y >= offset && c.y + c.edge <= offset + height
        })
        .collect();
    assert!(
        visible.len() > 96,
        "test must exceed the old decoded-cache capacity"
    );
    let ready = visible
        .iter()
        .filter(|&&i| app.tiles.ready(view.get(&app.library, i).unwrap()))
        .count();
    assert_eq!(
        ready,
        visible.len(),
        "all visible thumbnails must stay decoded, not evict each other"
    );
}

#[gpui::test]
fn final_catalog_page_keeps_latest_photo_frames_fixed_and_waits_for_pinch(cx: &mut TestAppContext) {
    cx.update(ui::init);
    for (initial, before_id, after_id) in [
        (3, "photo-tile-2", "photo-tile-5"),
        (5, "photo-tile-4", "photo-tile-7"),
        (200, "photo-tile-199", "photo-tile-202"),
    ] {
        let data = tempfile::tempdir().unwrap();
        let counters = Arc::new(SourceCounters::default());
        counters
            .total
            .store(initial + 3, std::sync::atomic::Ordering::SeqCst);
        let driver: Arc<dyn PhotosDriver> = Arc::new(
            desert_photos::sources::ConnectedPhotos::new(
                Source {
                    driver: "synthetic".into(),
                    path: data.path().join("source"),
                },
                Box::new(StressSource(counters)),
            )
            .unwrap(),
        );
        let mut entity = None;
        let window = cx.add_window(|w, cx| {
            let app = cx.new(|cx| {
                let mut app = PhotosApp::new(data.path().join("settings.json"), w, cx);
                app.library.photos = synthetic_photos(0..initial);
                app.driver = Some(driver);
                app.total = Some(initial + 3);
                app.next_page = Some(initial.to_string());
                app.has_more = true;
                app.library_changed();
                app
            });
            entity = Some(app.clone());
            Root::new(app, w, cx)
        });
        let app = entity.unwrap();
        let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
        cx.simulate_resize(size(px(1060.), px(700.)));
        frame(cx);
        let before = cx.debug_bounds(before_id).unwrap();
        // Short of committing to the next level, so it springs back.
        app.update_in(cx, |app, _, cx| {
            pinch(app, 0.1, false, cx);
            app.load_more(cx);
        });
        frame(cx);
        app.read_with(cx, |app, _| {
            assert_eq!(app.library.photos.len(), initial);
            assert!(app.gallery_zoom.is_some());
        });
        app.update_in(cx, |app, _, cx| pinch(app, 0., true, cx));
        advance_gallery_animation(cx);
        frame(cx);
        app.read_with(cx, |app, _| {
            assert_eq!(app.library.photos.len(), initial + 3);
            assert!(!app.has_more);
            assert!(app.gallery_zoom.is_none());
        });
        let after = cx.debug_bounds(after_id).unwrap();
        assert!(
            (before.origin.x - after.origin.x).abs() < px(1.),
            "latest photo changed column at catalog completion"
        );
        assert!(
            (before.origin.y - after.origin.y).abs() < px(1.),
            "latest photo dropped a row at catalog completion"
        );
        assert_eq!(before.size, after.size);
        // A resize during another pinch must cancel the old geometry, not retain a
        // surface tied to the old viewport and stale scroll metrics.
        app.update_in(cx, |app, _, cx| pinch(app, 0.6, false, cx));
        cx.simulate_resize(size(px(720.), px(620.)));
        frame(cx);
        app.read_with(cx, |app, _| assert!(app.gallery_zoom.is_none()));
    }
}

#[gpui::test]
fn viewer_stops_at_fit_and_zoom_out_returns_to_library(cx: &mut TestAppContext) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    for i in 0..8 {
        image::RgbImage::from_pixel(160, 100, image::Rgb([i * 20, 80, 100]))
            .save(data.path().join(format!("Photo {i}.png")))
            .unwrap();
    }
    let driver: Arc<dyn PhotosDriver> = Arc::new(storage::LocalPhotos::open(data.path()).unwrap());
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("profile/settings.json"), w, cx);
            app.finish_connection(Ok((driver.clone(), driver.page(None).unwrap())), w, cx);
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(700.)));
    frame(cx);
    app.update_in(cx, |app, w, cx| {
        app.select(3, true, Modifiers::none(), w, cx);
        app.set_viewer_zoom(0.2, (0., 0.), false, w, cx);
        assert_eq!(app.viewport.zoom, 1.);
        assert!(app.viewer.is_some());
        app.set_viewer_zoom(2., (0., 0.), true, w, cx);
        app.set_viewer_zoom(0.2, (0., 0.), true, w, cx);
        assert_eq!(app.viewport.zoom, 1.);
        assert!(
            app.viewer.is_some(),
            "zooming down from above fit stops at 100% first"
        );
        app.set_viewer_zoom(0.99, (0., 0.), true, w, cx);
        assert!(app.viewer.is_none());
        assert_eq!(app.viewport.zoom, 1.);
    });
    frame(cx);
    assert!(cx.debug_bounds("photo-tile-3").is_some());
    let grid = bounds(cx, "photo-grid");
    let before = app.read_with(cx, |app, _| app.gallery_level);
    cx.simulate_event(ScrollWheelEvent {
        position: grid.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-100.))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        touch_phase: TouchPhase::Moved,
    });
    frame(cx);
    app.read_with(cx, |app, _| {
        assert_eq!(app.gallery_level, before);
        assert!(
            app.gallery_zoom.is_none(),
            "closing gesture momentum must not zoom the gallery"
        );
    });
    app.update_in(cx, |app, w, cx| {
        app.select(3, true, Modifiers::none(), w, cx)
    });
    frame(cx);
    let canvas = bounds(cx, "viewer-canvas");
    cx.simulate_event(ScrollWheelEvent {
        position: canvas.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
        touch_phase: TouchPhase::Started,
        ..Default::default()
    });
    frame(cx);
    app.read_with(cx, |app, _| {
        assert!(app.viewer.is_some(), "scrolling never closes the viewer");
        assert_eq!(app.viewport.zoom, 1.);
    });
    app.update_in(cx, |app, w, cx| {
        // A pinch that starts zoomed in stops at 100% and cannot close.
        app.set_viewer_zoom(2., (0., 0.), false, w, cx);
        app.viewer_pinch_began();
        app.viewer_pinch(-0.6, (0., 0.), w, cx);
        app.viewer_pinch(-0.6, (0., 0.), w, cx);
        assert_eq!(app.viewport.zoom, 1.);
        assert!(app.dismiss_pinch.is_none());
        app.viewer_pinch_ended(w, cx);
        assert!(app.viewer.is_some());
        // At 100%, jitter alone neither closes nor shrinks the photo…
        app.set_viewer_zoom(1., (0., 0.), false, w, cx);
        app.viewer_pinch_began();
        app.viewer_pinch(-0.01, (0., 0.), w, cx);
        assert!(app.viewer.is_some());
        assert_eq!(app.viewport.zoom, 1.);
        // …but pinching in closes it at once, without waiting for release.
        app.viewer_pinch(-0.4, (0., 0.), w, cx);
        assert!(app.viewer.is_none(), "pinching in from fit closes at once");
        // The rest of the gesture is not a gallery zoom.
        app.viewer_pinch(-0.4, (0., 0.), w, cx);
        app.viewer_pinch_ended(w, cx);
    });
    frame(cx);
    app.read_with(cx, |app, _| {
        assert!(app.viewer.is_none());
        let closing = app.closing.as_ref().expect("the photo flies back");
        assert!(
            matches!(closing.target, Some(Some(_))),
            "it lands on its tile in the gallery"
        );
        assert!(app.gallery_zoom.is_none());
        assert_eq!(app.selected.len(), 1);
        assert_eq!(app.viewport.zoom, 1.);
    });
    assert!(cx.debug_bounds("closing-photo").is_some());
    assert!(cx.debug_bounds("photo-grid").is_some());
    settle(cx);
    app.read_with(cx, |app, _| assert!(app.closing.is_none()));
    app.update_in(cx, |app, w, cx| {
        app.open_selected(w, cx);
        app.zoom_by(1. / 1.2, w, cx);
        assert!(
            app.viewer.is_none(),
            "the zoom-out button at fit also returns to the library"
        );
    });
}

#[gpui::test]
fn viewer_exports_by_dragging_at_fit_and_keeps_dragging_for_panning_when_zoomed(
    cx: &mut TestAppContext,
) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    image::RgbImage::from_pixel(160, 100, image::Rgb([20, 80, 100]))
        .save(data.path().join("Photo.png"))
        .unwrap();
    let driver: Arc<dyn PhotosDriver> = Arc::new(storage::LocalPhotos::open(data.path()).unwrap());
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("profile/settings.json"), w, cx);
            app.finish_connection(Ok((driver.clone(), driver.page(None).unwrap())), w, cx);
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let app = entity.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(700.)));
    frame(cx);
    app.update_in(cx, |app, w, cx| {
        app.select(0, true, Modifiers::none(), w, cx)
    });
    frame(cx);
    let at = bounds(cx, "viewer-canvas").center();
    cx.simulate_event(MouseDownEvent {
        position: at,
        button: MouseButton::Left,
        ..Default::default()
    });
    cx.simulate_event(MouseMoveEvent {
        position: at + point(px(35.), px(0.)),
        pressed_button: Some(MouseButton::Left),
        ..Default::default()
    });
    frame(cx);
    assert!(
        cx.read(|cx| cx.has_active_drag()),
        "the fitted viewer needs a file-export drag source"
    );
    cx.simulate_event(MouseUpEvent {
        position: at,
        button: MouseButton::Left,
        ..Default::default()
    });
    frame(cx);
    app.update_in(cx, |app, w, cx| app.zoom_by(2., w, cx));
    frame(cx);
    let at = bounds(cx, "viewer-canvas").center();
    cx.simulate_event(MouseDownEvent {
        position: at,
        button: MouseButton::Left,
        ..Default::default()
    });
    cx.simulate_event(MouseMoveEvent {
        position: at + point(px(35.), px(0.)),
        pressed_button: Some(MouseButton::Left),
        ..Default::default()
    });
    frame(cx);
    assert!(
        !cx.read(|cx| cx.has_active_drag()),
        "a zoomed image should pan instead of exporting"
    );
    app.read_with(cx, |app, _| assert_ne!(app.viewport.pan, (0., 0.)));
}

fn synthetic_app(
    cx: &mut TestAppContext,
    photos: Vec<Arc<Photo>>,
) -> (Entity<PhotosApp>, &mut VisualTestContext, tempfile::TempDir) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: data.path().join("source"),
            },
            Box::new(StressSource(Arc::new(SourceCounters::default()))),
        )
        .unwrap(),
    );
    let mut entity = None;
    let settings = data.path().join("settings.json");
    let window = cx.add_window(|w, cx| {
        let app = cx.new(|cx| {
            let mut app = PhotosApp::new(settings, w, cx);
            app.driver = Some(driver);
            app.library.photos = photos;
            app.library_changed();
            app
        });
        entity = Some(app.clone());
        Root::new(app, w, cx)
    });
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(760.)));
    frame(cx);
    (entity.unwrap(), cx, data)
}

#[gpui::test]
fn zooming_is_centred_and_photos_crossfade_in_their_slots(cx: &mut TestAppContext) {
    let (app, cx, _data) = synthetic_app(cx, synthetic_photos(0..2000));
    for (from, delta) in [
        (9usize, -1f32),
        (8, -1.),
        (8, 1.),
        (7, -1.),
        (6, 1.),
        (6, -1.),
        (4, 1.),
    ] {
        let next = (from as isize + delta as isize) as usize;
        app.update(cx, |app, cx| {
            app.gallery_level = from;
            app.reset_gallery();
            cx.notify();
        });
        frame(cx);
        app.update(cx, |app, cx| {
            scroll_to_row(app, 20, cx);
            cx.notify();
        });
        frame(cx);
        let (anchor, center) = app.update(cx, |app, cx| {
            pinch(app, delta * 0.1, false, cx);
            let z = app.gallery_zoom.as_ref().unwrap();
            assert_eq!(
                (z.content.from, z.content.to),
                (from, next),
                "{from}→{next}: the photos head for the next level at once"
            );
            let ((anchor, within), center) = (z.anchor, z.center);
            assert!((center.0 - app.gallery_width / 2.).abs() < 0.01);
            for _ in 0..4 {
                pinch(app, delta * 0.2, false, cx);
                let z = app.gallery_zoom.as_ref().unwrap();
                let (ladder, position) = (z.ladder.clone(), z.zoom.position);
                let (a, b, _) = ladder.lattice_pair(position).unwrap();
                let la = Lattice::of(&app.zoom_frame(ladder.levels[a], cx), anchor);
                let lb = Lattice::of(&app.zoom_frame(ladder.levels[b], cx), anchor);
                let lattice =
                    zoom::lattice_at(&ladder, position, center, |s| if s == a { la } else { lb })
                        .unwrap();
                assert!(
                    (lattice.x - center.0).abs() < 0.01,
                    "{from}→{next}: rows widen and narrow about the centre"
                );
                assert!(
                    (lattice.y + (within - 0.5) * lattice.edge - center.1).abs() < 0.5,
                    "{from}→{next}: the spot at the centre stays there"
                );
            }
            (anchor, center)
        });
        // The crossfade runs on its own clock, not the gesture's.
        cx.executor().advance_clock(FADE);
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            let z = app.gallery_zoom.as_ref().unwrap();
            let now = cx.background_executor().now();
            assert_eq!(z.content.layers(now), vec![(next, 1.)]);
        });
        app.update(cx, |app, cx| app.zoom_library(ZoomInput::Release, true, cx));
        // The spring takes a little over half a second to come to rest.
        for _ in 0..4 {
            settle(cx);
        }
        app.read_with(cx, |app, cx| {
            assert!(app.gallery_zoom.is_none(), "the zoom settles");
            assert_eq!(app.gallery_level, next);
            // The photo zoomed on ends at the centre of the grid.
            let layout = app.gallery_layout(cx);
            let cell = layout.cell(anchor);
            let y = gallery_title(&layout) + cell.y - app.gallery_offset();
            assert!(
                y - 1. <= center.1 && center.1 <= y + cell.edge + 1.,
                "photo {anchor} at {y}..{} for centre {}",
                y + cell.edge,
                center.1
            );
        });
    }
}

#[gpui::test]
fn photos_grow_out_of_and_land_back_in_their_tiles(cx: &mut TestAppContext) {
    let (app, cx, _data) = synthetic_app(cx, synthetic_photos(0..40));
    app.update_in(cx, |app, w, cx| {
        app.preferences.bottom_to_top = false;
        app.reset_gallery();
        app.viewer_bounds = Bounds::default();
        cx.notify();
        let _ = w;
    });
    frame(cx);
    // A first open (no measured viewer yet) simply appears; later ones grow.
    for _ in 0..2 {
        let tile = bounds(cx, "photo-tile-5");
        app.update_in(cx, |app, w, cx| {
            app.select(5, true, Modifiers::none(), w, cx)
        });
        app.read_with(cx, |app, _| {
            let opening = app.opening.as_ref().expect("the photo grows from its tile");
            assert!((opening.from.center().x - tile.center().x).abs() < px(1.));
            assert!((opening.from.center().y - tile.center().y).abs() < px(1.));
        });
        frame(cx);
        settle(cx);
        app.read_with(cx, |app, _| assert!(app.opening.is_none()));
        click(cx, "close-viewer");
        frame(cx);
        app.read_with(cx, |app, _| {
            let closing = app.closing.as_ref().expect("the photo flies back");
            let (target, _) = closing.target.flatten().expect("to its tile");
            assert!((target.center().x - tile.center().x).abs() < px(1.));
        });
        settle(cx);
        app.read_with(cx, |app, _| assert!(app.closing.is_none()));
    }
}

#[gpui::test]
fn videos_tab_badges_and_library_switcher(cx: &mut TestAppContext) {
    let photos = synthetic_photos(0..30)
        .into_iter()
        .enumerate()
        .map(|(i, mut p)| {
            if i % 3 == 0 {
                Arc::make_mut(&mut p).duration = Some(75.);
            }
            p
        })
        .collect();
    let (app, cx, _data) = synthetic_app(cx, photos);
    app.read_with(cx, |app, _| assert_eq!(app.video_count, 10));
    click(cx, "videos");
    frame(cx);
    app.read_with(cx, |app, cx| {
        assert!(app.filter == Filter::Videos);
        assert_eq!(app.visible(cx).len(), 10);
    });
    assert_eq!(duration_label(75.), "1:15");
    assert_eq!(duration_label(3723.), "1:02:03");
    click(cx, "library-switcher");
    frame(cx);
    let menu = bounds(cx, "library-menu");
    let switcher = bounds(cx, "library-switcher");
    assert!(menu.top() >= switcher.bottom() && menu.left() >= px(0.));
    click(cx, "add-photo-library");
    frame(cx);
    app.read_with(cx, |app, _| {
        assert!(!app.library_menu);
        assert!(app.modal == Some(Modal::Sources) && app.settings_page == 0);
    });
}

#[test]
fn search_matching_folds_case_like_to_lowercase() {
    for (haystack, needle) in [
        ("IMG_0042.HEIC", "img_00"),
        ("Beach Day", "day"),
        ("Beach Day", "night"),
        ("", "a"),
        ("ab", "abc"),
        ("Ünïcode Straße", "straße"),
        ("ÉTÉ", "été"),
        ("anything", ""),
    ] {
        assert_eq!(
            super::contains_folded(haystack, needle),
            haystack.to_lowercase().contains(needle),
            "{haystack:?} / {needle:?}"
        );
    }
}

/// Each catalog page re-anchors the grid. The header is built before the grid
/// in the same frame, so it must already show the dates the grid settles on,
/// not those at the old scroll offset (which flickered for a frame per page).
#[gpui::test]
fn header_dates_do_not_flicker_while_cataloging(cx: &mut TestAppContext) {
    cx.update(ui::init);
    let data = tempfile::tempdir().unwrap();
    let driver: Arc<dyn PhotosDriver> = Arc::new(
        desert_photos::sources::ConnectedPhotos::new(
            Source {
                driver: "synthetic".into(),
                path: data.path().join("source"),
            },
            Box::new(StressSource(Arc::default())),
        )
        .unwrap(),
    );
    // Newest first, one day apart, so every row has its own dates.
    let dated = |range: std::ops::Range<usize>| {
        let base = chrono::Utc::now();
        synthetic_photos(range.clone())
            .into_iter()
            .zip(range)
            .map(|(p, i)| {
                Arc::new(Photo {
                    modified: base - chrono::Duration::days(i as i64),
                    ..p.as_ref().clone()
                })
            })
            .collect::<Vec<_>>()
    };
    let mut app = None;
    let window = cx.add_window(|w, cx| {
        let entity = cx.new(|cx| {
            let mut app = PhotosApp::new(data.path().join("settings.json"), w, cx);
            app.driver = Some(driver);
            app.library.photos = dated(0..600);
            app.library_changed();
            app
        });
        app = Some(entity.clone());
        Root::new(entity, w, cx)
    });
    let app = app.unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1060.), px(760.)));
    frame(cx);
    let mut pages = 600..800;
    // Pinned to the newest photos, then browsing older ones mid-library.
    for browse in [None, Some(3000.)] {
        if let Some(offset) = browse {
            app.update_in(cx, |app, _, cx| {
                app.set_gallery_offset(offset);
                cx.notify();
            });
            frame(cx);
        }
        for _ in 0..3 {
            let page = pages.start..pages.start + 200;
            pages.start += 200;
            let before_grid = app.update_in(cx, |app, _, cx| {
                app.preserve_gallery(cx);
                app.library.photos.extend(dated(page));
                app.library_changed();
                cx.notify();
                app.visible_dates(cx)
            });
            frame(cx);
            let settled = app.read_with(cx, |app, cx| app.visible_dates(cx));
            assert!(settled.is_some());
            assert_eq!(before_grid, settled);
        }
    }
}
