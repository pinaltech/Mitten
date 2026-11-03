#![cfg(all(target_os = "macos", feature = "apple-photos"))]

use desert_photos::sources::{open, verify_photokit_configuration};
use desert_photos::storage::{Photo, Source};

#[test]
fn missing_usage_description_is_reported_before_requesting_photos_access() {
    // Cargo's integration test executable has no embedded app plist. Reaching a
    // PhotoKit access request here would cause the reported TCC process abort.
    let error = verify_photokit_configuration().unwrap_err().to_string();
    assert!(error.contains("NSPhotoLibraryUsageDescription"));
    let data = tempfile::tempdir().unwrap();
    let driver = open(&Source {
        driver: "apple-photos".into(),
        path: data.path().to_owned(),
    })
    .unwrap();
    let error = driver
        .page(None)
        .err()
        .expect("must reject a missing plist");
    assert!(error.to_string().contains("NSPhotoLibraryUsageDescription"));
    let photo = Photo {
        id: "fixture".into(),
        path: Default::default(),
        name: "Fixture.jpg".into(),
        modified: Default::default(),
        bytes: 0,
        width: 0,
        height: 0,
        favorite: false,
        deleted: false,
        caption: String::new(),
        fingerprint: "fixture".into(),
        duration: None,
    };
    for result in [driver.thumbnail(&photo), driver.preview(&photo)] {
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("NSPhotoLibraryUsageDescription")
        );
    }
    assert!(
        driver
            .export(&[photo.id], data.path())
            .unwrap_err()
            .to_string()
            .contains("NSPhotoLibraryUsageDescription")
    );
}
