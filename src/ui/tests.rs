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
            app.driver = Some(driver.clone());
            app.library = driver.scan().unwrap();
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
            app.driver = Some(driver.clone());
            app.library = driver.scan().unwrap();
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
        let end = (start + 200).min(1_000_000);
        Ok(PhotoPage {
            library: Library {
                photos: synthetic_photos(start..end),
                albums: Default::default(),
            },
            next: (end < 1_000_000).then(|| end.to_string()),
            total: Some(1_000_000),
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
                Arc::ptr_eq(&a, &b),
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
    let mut entity = None;
    let window = cx.add_window(|w, cx| {
        let view = cx.new(|cx| {
            let mut app = PhotosApp::new(directory.path().join("settings.json"), w, cx);
            app.driver = Some(driver.clone());
            let page = driver.page(None).unwrap();
            app.library = page.library;
            app.next_page = page.next;
            app.has_more = true;
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
    assert_eq!(counters.pages.load(SeqCst), 1);
    for _ in 0..10 {
        let before = entity.read_with(cx, |app, _| app.library.photos.len());
        entity.update_in(cx, |app, _, cx| {
            app.scroll
                .scroll_to_item(before.div_ceil(4) - 1, ScrollStrategy::Bottom);
            cx.notify();
        });
        frame(cx);
        entity.read_with(cx, |app, _| assert!(app.library.photos.len() > before));
    }
    assert_eq!(counters.previews.load(SeqCst), 0);
    assert_eq!(counters.originals.load(SeqCst), 0);
    counters.fail_page.store(true, SeqCst);
    entity.update_in(cx, |app, _, cx| app.load_more(cx));
    frame(cx);
    entity.read_with(cx, |app, _| {
        assert!(app.page_failed && app.has_more && app.next_page.is_some())
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
    let retry = bounds(cx, "load-more-photos");
    cx.simulate_click(retry.center(), Modifiers::none());
    frame(cx);
    entity.read_with(cx, |app, _| assert!(!app.page_failed));
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
