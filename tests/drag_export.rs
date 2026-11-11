use mitten::{
    drag_export,
    storage::{LocalPhotos, PhotosDriver},
};
use std::fs;

#[test]
fn promised_names_are_safe_and_unique_for_multiple_albums() {
    let names = drag_export::filenames([
        "album/Photo.jpg",
        "album2/Photo.jpg",
        r"other\PHOTO.JPG",
        "..",
        "",
        "bad\0name.png",
    ]);
    assert_eq!(
        &names[..3],
        &["Photo.jpg", "Photo (2).jpg", "PHOTO (3).JPG"]
    );
    let unique = names
        .iter()
        .map(|s| s.to_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), names.len());
    assert!(names.iter().all(|name| !name.is_empty()
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
        && name != ".."));
}
#[test]
fn promised_local_files_preserve_original_bytes_and_refuse_overwrites() {
    let source = tempfile::tempdir().unwrap();
    let receiver = tempfile::tempdir().unwrap();
    let original = source.path().join("Photo.png");
    image::RgbImage::from_pixel(40, 30, image::Rgb([20, 100, 50]))
        .save(&original)
        .unwrap();
    let bytes = fs::read(&original).unwrap();
    let driver = LocalPhotos::open(source.path()).unwrap();
    let photo = driver.scan().unwrap().photos[0].clone();
    let destination = receiver
        .path()
        .join("Receiving app chooses a unique name.png");
    drag_export::write_original(&driver, &photo.id, &destination).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    fs::write(&destination, b"existing file").unwrap();
    assert!(drag_export::write_original(&driver, &photo.id, &destination).is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"existing file");
    assert_eq!(fs::read(&original).unwrap(), bytes);
}
#[test]
fn failed_promises_leave_no_partial_files_or_staging_directories() {
    let source = tempfile::tempdir().unwrap();
    let receiver = tempfile::tempdir().unwrap();
    let driver = LocalPhotos::open(source.path()).unwrap();
    assert!(
        drag_export::write_original(&driver, "missing", &receiver.path().join("Failed.png"))
            .is_err()
    );
    assert_eq!(fs::read_dir(receiver.path()).unwrap().count(), 0);
}
