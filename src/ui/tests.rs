use super::*;
use std::prelude::v1::test;

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
        inside(cx, "photo-status", width, height);
        let tile = bounds(cx, "photo-tile-0");
        cx.simulate_click(
            point(tile.left() + px(20.), tile.top() + px(20.)),
            Modifiers::none(),
        );
        frame(cx);
        entity.read_with(cx, |app, _| {
            assert_eq!(app.selected.len(), 1, "selection at {width} × {height}")
        });
        entity.update_in(cx, |app, w, cx| app.open_selected(w, cx));
        frame(cx);
        inside(cx, "photo-viewer", width, height);
        inside(cx, "next-photo", width, height);
        let next = bounds(cx, "next-photo");
        cx.simulate_click(next.center(), Modifiers::none());
        frame(cx);
        entity.read_with(cx, |app, _| assert_eq!(app.anchor, Some(1)));
        let back = bounds(cx, "close-viewer");
        cx.simulate_click(back.center(), Modifiers::none());
        frame(cx);
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
        delta: ScrollDelta::Lines(point(0., -3.)),
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
            app.scroll.scroll_to_item(row, ScrollStrategy::Top);
            cx.notify();
        });
        frame(cx);
        timings.push(start.elapsed());
        entity.read_with(cx, |app, cx| {
            assert!(app.thumbnails.len() <= THUMBNAIL_CACHE);
            assert!(app.previews.is_empty());
            assert!(app.pending.len() <= THUMBNAIL_JOBS);
            assert!(app.render_cache.read(cx).len() <= 96);
        });
        assert_eq!(counters.previews.load(SeqCst), 0);
        assert_eq!(counters.originals.load(SeqCst), 0);
    }
    println!("one million photo GPUI scroll/frame durations: {timings:?}");
    entity.read_with(cx, |app, cx| {
        assert_eq!(
            app.thumbnails.len(),
            THUMBNAIL_CACHE,
            "stress run should actually exercise thumbnail eviction"
        );
        assert_eq!(
            app.render_cache.read(cx).len(),
            96,
            "stress run should actually exercise decoded-image eviction"
        );
    });
    assert!(
        counters.thumbs.load(SeqCst) < 1000,
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
            app.request_image(photo.as_ref().clone(), false, cx);
        }
        assert_eq!(app.pending.len(), THUMBNAIL_JOBS);
        for photo in synthetic_photos(100_000..100_100) {
            app.request_image(photo.as_ref().clone(), true, cx);
        }
        assert_eq!(app.pending.len(), THUMBNAIL_JOBS + 1);
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
    let grid = bounds(cx, "photo-grid");
    app.update_in(cx, |app, _, cx| {
        app.pinch_gallery(-0.6, grid.center(), false, cx);
        assert_eq!(app.gallery_zoom, 2.);
        assert_eq!(app.gallery_pinch, Some(1.4));
    });
    cx.simulate_event(ScrollWheelEvent {
        position: grid.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        touch_phase: TouchPhase::Ended,
    });
    frame(cx);
    app.read_with(cx, |app, _| assert_eq!(app.gallery_zoom, 1.));
    app.update_in(cx, |app, w, cx| app.gallery_step(false, w, cx));
    frame(cx);
    inside(cx, "gallery-date-label", 1060., 760.);
    let a = bounds(cx, "photo-tile-0");
    let b = bounds(cx, "photo-tile-1");
    let heart = bounds(cx, "photo-heart-0");
    assert!((heart.size.width / a.size.width - 0.08).abs() < 0.025);
    let scrollbar = bounds(cx, "gallery-scrollbar-layer");
    assert!((scrollbar.right() - bounds(cx, "photo-grid").right()).abs() < px(1.));
    assert!((a.size.width - a.size.height).abs() < px(1.));
    assert!(
        (a.right() - b.left()).abs() < px(1.),
        "zoomed-out photos must have no gutter"
    );
    app.update_in(cx, |app, w, cx| {
        app.select(0, true, Modifiers::none(), w, cx);
    });
    frame(cx);
    let canvas = bounds(cx, "viewer-canvas");
    let previous = bounds(cx, "previous-photo");
    let next = bounds(cx, "next-photo");
    assert!(previous.center().x < canvas.center().x && next.center().x > canvas.center().x);
    cx.simulate_click(next.center(), Modifiers::none());
    frame(cx);
    app.read_with(cx, |app, _| assert_eq!(app.anchor, Some(1)));
    click(cx, "previous-photo");
    frame(cx);
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
    let layout = GalleryLayout::new(app.gallery_width, app.gallery_zoom)
        .with_count(app.visible(cx).len(), app.preferences.bottom_to_top);
    layout.cell(index).y
        + anchor.within * layout.edge
        + f32::from(app.scroll.0.borrow().base_handle.offset().y)
}
fn assert_at_bottom(app: &PhotosApp, cx: &App) {
    let layout = GalleryLayout::new(app.gallery_width, app.gallery_zoom)
        .with_count(app.visible(cx).len(), app.preferences.bottom_to_top);
    let h = &app.scroll.0.borrow().base_handle;
    let max = (layout.blocks(app.visible(cx).len()) as f32 * layout.block_height
        - f32::from(h.bounds().size.height))
    .max(0.);
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
            app.gallery_zoom = 0.;
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
        app.scroll.scroll_to_item(140, ScrollStrategy::Top);
        cx.notify();
    });
    frame(cx);
    paint_frames(cx, 8);
    app.read_with(cx, assert_dense_ready);
    let pointer = bounds(cx, "photo-grid").center();
    app.update_in(cx, |app, _, cx| {
        app.pinch_gallery(0.55, pointer, false, cx);
        assert!((app.gallery_motion.as_ref().unwrap().progress - 0.55).abs() < 0.001);
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
        assert!(app.gallery_motion.is_none());
        assert!(app.gallery_pinch.is_none());
    });
    app.update_in(cx, |app, _, cx| app.pinch_gallery(0.7, pointer, true, cx));
    cx.run_until_parked();
    for _ in 0..5 {
        cx.executor().advance_clock(Duration::from_millis(16));
        paint_frames(cx, 1);
    }
    app.update_in(cx, |app, _, cx| {
        let before = app.gallery_motion.as_ref().unwrap().progress;
        app.pinch_gallery(-0.15, pointer, false, cx);
        let after = app.gallery_motion.as_ref().unwrap().progress;
        assert!(
            (before - after - 0.15).abs() < 0.001,
            "reversing a settling pinch must resume the visible frame"
        );
        app.pinch_gallery(0., pointer, true, cx);
    });
    advance_gallery_animation(cx);
    app.read_with(cx, |app, _| assert!(app.gallery_motion.is_none()));
    app.update_in(cx, |app, _, cx| {
        app.zoom_gallery(-app.gallery_zoom, pointer, cx)
    });
    advance_gallery_animation(cx);
    paint_frames(cx, 8);
    app.read_with(cx, assert_dense_ready);
    app.update_in(cx, |app, _, cx| {
        app.scroll.scroll_to_item(20, ScrollStrategy::Top);
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

fn advance_gallery_animation(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..18 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    frame(cx);
}

fn assert_dense_ready(app: &PhotosApp, cx: &App) {
    let view = app.visible(cx);
    let layout = GalleryLayout::new(app.gallery_width, app.gallery_zoom)
        .with_count(app.visible(cx).len(), app.preferences.bottom_to_top);
    let handle = &app.scroll.0.borrow().base_handle;
    let offset = f32::from(-handle.offset().y);
    let height = f32::from(handle.bounds().size.height);
    let first = (offset / layout.block_height).ceil() as usize * layout.block_len;
    let last = (((offset + height) / layout.block_height).floor() as usize * layout.block_len)
        .min(view.len());
    assert!(
        last - first > 96,
        "test must exceed the old decoded-cache capacity"
    );
    assert!(app.render_cache.read(cx).capacity() >= last - first);
    assert!(app.render_cache.read(cx).len() <= app.render_cache.read(cx).capacity());
    let ready = (first..last)
        .filter(|&i| {
            let p = view.get(&app.library, i).unwrap();
            app.thumbnails
                .peek(&p.fingerprint)
                .and_then(|path| app.render_cache.read(cx).ready(path))
                .is_some()
        })
        .count();
    assert_eq!(
        ready,
        last - first,
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
        let pointer = bounds(cx, "photo-grid").center();
        app.update_in(cx, |app, _, cx| {
            app.pinch_gallery(0.3, pointer, false, cx);
            app.load_more(cx);
        });
        frame(cx);
        app.read_with(cx, |app, _| {
            assert_eq!(app.library.photos.len(), initial);
            assert!(app.gallery_motion.is_some());
        });
        app.update_in(cx, |app, _, cx| app.pinch_gallery(0., pointer, true, cx));
        advance_gallery_animation(cx);
        frame(cx);
        app.read_with(cx, |app, _| {
            assert_eq!(app.library.photos.len(), initial + 3);
            assert!(!app.has_more);
            assert!(app.gallery_motion.is_none());
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
        app.update_in(cx, |app, _, cx| app.pinch_gallery(0.6, pointer, false, cx));
        cx.simulate_resize(size(px(720.), px(620.)));
        frame(cx);
        app.read_with(cx, |app, _| assert!(app.gallery_motion.is_none()));
    }
}
