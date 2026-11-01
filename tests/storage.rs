use desert_photos::storage::{self, LocalPhotos, PhotosDriver, Preferences, Source};
use std::{fs, path::Path};
fn image(path: &Path, color: u8) {
    image::RgbImage::from_pixel(80, 60, image::Rgb([color, 90, 40]))
        .save(path)
        .unwrap();
}

#[test]
fn library_references_originals_and_ignores_its_private_cache() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("Vacation")).unwrap();
    image(&dir.path().join("Vacation/sunset.png"), 140);
    let driver = LocalPhotos::open(dir.path()).unwrap();
    let library = driver.scan().unwrap();
    assert_eq!(library.photos.len(), 1);
    assert_eq!(
        (library.photos[0].width, library.photos[0].height),
        (80, 60)
    );
    assert_eq!(library.photos[0].id, "Vacation/sunset.png");
    let thumb = driver.thumbnail(&library.photos[0]).unwrap();
    assert!(thumb.is_file());
    assert_eq!(driver.scan().unwrap().photos.len(), 1);
    assert!(dir.path().join("Vacation/sunset.png").is_file());
}

#[test]
fn imports_copy_and_deduplicate_by_content_including_existing_library_files() {
    let dir = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    image(&dir.path().join("existing.png"), 100);
    fs::copy(
        dir.path().join("existing.png"),
        input.path().join("renamed.png"),
    )
    .unwrap();
    image(&input.path().join("new.png"), 200);
    fs::write(input.path().join("not-a-photo.png"), b"invalid").unwrap();
    let driver = LocalPhotos::open(dir.path()).unwrap();
    let report = driver.import(&[input.path().to_owned()]).unwrap();
    assert_eq!(
        (report.imported, report.duplicates, report.unsupported),
        (1, 1, 1)
    );
    assert_eq!(driver.scan().unwrap().photos.len(), 2);
    assert!(input.path().join("new.png").exists());
    assert_eq!(
        driver
            .import(&[input.path().join("new.png")])
            .unwrap()
            .duplicates,
        1
    );
}

#[test]
fn albums_captions_favorites_and_recoverable_deletion_persist_without_changing_originals() {
    let dir = tempfile::tempdir().unwrap();
    image(&dir.path().join("photo.png"), 120);
    let before = fs::read(dir.path().join("photo.png")).unwrap();
    let driver = LocalPhotos::open(dir.path()).unwrap();
    let ids = vec!["photo.png".to_owned()];
    driver.create_album("Weekend").unwrap();
    driver.add_to_album("Weekend", &ids).unwrap();
    driver.favorite(&ids, true).unwrap();
    driver.caption(&ids[0], "A quiet afternoon").unwrap();
    driver.trash(&ids, true).unwrap();
    let restarted = LocalPhotos::open(dir.path()).unwrap();
    let library = restarted.scan().unwrap();
    assert!(library.photos[0].favorite && library.photos[0].deleted);
    assert_eq!(library.photos[0].caption, "A quiet afternoon");
    assert!(library.albums["Weekend"].contains(&ids[0]));
    restarted.trash(&ids, false).unwrap();
    restarted.remove_from_album("Weekend", &ids).unwrap();
    let library = restarted.scan().unwrap();
    assert!(!library.photos[0].deleted);
    assert!(library.albums["Weekend"].is_empty());
    assert_eq!(fs::read(dir.path().join("photo.png")).unwrap(), before);
}

#[test]
fn export_keeps_original_bytes_and_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    image(&dir.path().join("photo.png"), 50);
    let driver = LocalPhotos::open(dir.path()).unwrap();
    let ids = vec!["photo.png".into()];
    assert_eq!(driver.export(&ids, output.path()).unwrap(), 1);
    assert_eq!(
        fs::read(dir.path().join("photo.png")).unwrap(),
        fs::read(output.path().join("photo.png")).unwrap()
    );
    assert!(driver.export(&ids, output.path()).is_err());
}

#[test]
fn unsupported_driver_and_damaged_metadata_fail_without_replacing_files() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        storage::open_source(&Source {
            driver: "future-cloud".into(),
            path: dir.path().into()
        })
        .is_err()
    );
    let _driver = LocalPhotos::open(dir.path()).unwrap();
    let metadata = dir.path().join(".desert-photos/library.json");
    fs::write(&metadata, "{broken").unwrap();
    assert!(LocalPhotos::open(dir.path()).is_err());
    assert_eq!(fs::read_to_string(metadata).unwrap(), "{broken");
}

#[test]
fn traversal_and_duplicate_album_names_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let driver = LocalPhotos::open(dir.path()).unwrap();
    assert!(
        driver
            .export(&["../outside.png".into()], output.path())
            .is_err()
    );
    assert!(driver.trash(&["/tmp/photo.png".into()], true).is_err());
    assert!(
        driver
            .caption(".desert-photos/library.json", "bad")
            .is_err()
    );
    assert!(driver.create_album(" ").is_err());
    driver.create_album("Trip").unwrap();
    assert!(driver.create_album("Trip").is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_are_neither_indexed_nor_used_for_storage() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    image(&outside.path().join("outside.png"), 1);
    std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
    let driver = LocalPhotos::open(dir.path()).unwrap();
    assert!(driver.scan().unwrap().photos.is_empty());
    assert!(driver.caption("linked/outside.png", "bad").is_err());
    let other = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), other.path().join(".desert-photos")).unwrap();
    assert!(LocalPhotos::open(other.path()).is_err());
}

#[test]
fn preview_generation_is_bounded_and_content_updates_invalidate_cached_images() {
    let dir = tempfile::tempdir().unwrap();
    image::RgbImage::from_pixel(2800, 1800, image::Rgb([50, 100, 150]))
        .save(dir.path().join("large.png"))
        .unwrap();
    let driver = LocalPhotos::open(dir.path()).unwrap();
    let photo = driver.scan().unwrap().photos.remove(0);
    let thumb = driver.thumbnail(&photo).unwrap();
    assert_eq!(image::image_dimensions(thumb).unwrap().0, 512);
    let preview = driver.preview(&photo).unwrap();
    assert_eq!(image::image_dimensions(preview).unwrap().0, 2560);
    image(&dir.path().join("large.png"), 2);
    let updated = driver.scan().unwrap().photos.remove(0);
    assert_ne!(updated.fingerprint, photo.fingerprint);
}

#[test]
fn local_source_registry_roundtrips_without_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let mut prefs = Preferences::default();
    let source = Source {
        driver: "local-folder".into(),
        path: dir.path().into(),
    };
    prefs.connect(source.clone());
    prefs.connect(source.clone());
    prefs.save(&dir.path().join("settings.json")).unwrap();
    let read = Preferences::load(&dir.path().join("settings.json")).unwrap();
    assert_eq!(read.sources, vec![source.clone()]);
    assert_eq!(read.active, Some(source.path));
}

#[test]
fn concurrent_album_updates_are_serialized() {
    let dir = tempfile::tempdir().unwrap();
    let driver = std::sync::Arc::new(LocalPhotos::open(dir.path()).unwrap());
    let threads = (0..12)
        .map(|i| {
            let driver = driver.clone();
            std::thread::spawn(move || driver.create_album(&format!("Album {i}")).unwrap())
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(driver.scan().unwrap().albums.len(), 12);
}
