use anyhow::Result;
use desert_photos::{
    loading::PAGE_SIZE,
    sources::{ConnectedPhotos, Connection, PhotoPage, Provider},
    storage::{Library, Photo, PhotosDriver, Source},
};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct MillionPhotos {
    thumbnails: Arc<AtomicUsize>,
    previews: Arc<AtomicUsize>,
    originals: Arc<AtomicUsize>,
}
impl Provider for MillionPhotos {
    fn page(&self, cursor: Option<&str>) -> Result<PhotoPage> {
        let start: usize = cursor.unwrap_or("0").parse()?;
        let end = (start + PAGE_SIZE).min(1_000_000);
        let photos = (start..end)
            .map(|i| {
                Arc::new(Photo {
                    id: format!("{i}"),
                    name: format!("Photo {i}.png"),
                    path: Default::default(),
                    modified: Default::default(),
                    bytes: 10,
                    width: 64,
                    height: 48,
                    favorite: false,
                    deleted: false,
                    caption: String::new(),
                    fingerprint: format!("asset-{i}-v1"),
                    duration: None,
                })
            })
            .collect();
        Ok(PhotoPage {
            library: Library {
                photos,
                albums: Default::default(),
                synced: Default::default(),
            },
            next: (end < 1_000_000).then(|| end.to_string()),
            total: Some(1_000_000),
        })
    }
    fn image(&self, _: &Photo, preview: bool, destination: &Path) -> Result<()> {
        if preview {
            &self.previews
        } else {
            &self.thumbnails
        }
        .fetch_add(1, Ordering::SeqCst);
        image::RgbImage::from_pixel(64, 48, image::Rgb([90, 100, 120]))
            .save_with_format(destination, image::ImageFormat::Png)?;
        Ok(())
    }
    fn original(&self, _: &str, destination: &Path) -> Result<String> {
        self.originals.fetch_add(1, Ordering::SeqCst);
        fs::write(destination, b"original bytes")?;
        Ok("Photo.png".into())
    }
}
#[test]
fn a_million_asset_source_fetches_only_requested_pages_and_images() {
    let dir = tempfile::tempdir().unwrap();
    let thumbnails = Arc::new(AtomicUsize::new(0));
    let previews = Arc::new(AtomicUsize::new(0));
    let originals = Arc::new(AtomicUsize::new(0));
    let driver = ConnectedPhotos::new(
        Source {
            driver: "synthetic".into(),
            path: dir.path().join("source"),
        },
        Box::new(MillionPhotos {
            thumbnails: thumbnails.clone(),
            previews: previews.clone(),
            originals: originals.clone(),
        }),
    )
    .unwrap();
    let start = std::time::Instant::now();
    let mut cursor = None;
    let mut count = 0;
    // Enumerate all one million metadata entries without ever requesting pixels.
    loop {
        let page = driver.page(cursor.as_deref()).unwrap();
        assert!(page.library.photos.len() <= PAGE_SIZE);
        count += page.library.photos.len();
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(count, 1_000_000);
    assert_eq!(thumbnails.load(Ordering::SeqCst), 0);
    assert_eq!(previews.load(Ordering::SeqCst), 0);
    assert_eq!(originals.load(Ordering::SeqCst), 0);
    println!("one million paged metadata entries: {:?}", start.elapsed());
    let page = driver.page(Some("999800")).unwrap();
    assert_eq!(page.library.photos[0].id, "999800");
    assert!(page.next.is_none());
    let photo = &page.library.photos[0];
    assert!(driver.thumbnail(photo).unwrap().is_file());
    driver.thumbnail(photo).unwrap();
    assert_eq!(
        thumbnails.load(Ordering::SeqCst),
        1,
        "disk cache must prevent refetching"
    );
    assert_eq!(previews.load(Ordering::SeqCst), 0);
    assert_eq!(originals.load(Ordering::SeqCst), 0);
    let preview = driver.preview(photo).unwrap();
    assert!(preview.is_file());
    assert_eq!(previews.load(Ordering::SeqCst), 1);
    assert_eq!(originals.load(Ordering::SeqCst), 0);
    let output = tempfile::tempdir().unwrap();
    driver
        .export(std::slice::from_ref(&photo.id), output.path())
        .unwrap();
    assert_eq!(originals.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read(output.path().join("Photo.png")).unwrap(),
        b"original bytes"
    );
    assert!(
        driver
            .export(std::slice::from_ref(&photo.id), output.path())
            .is_err()
    );
}
#[test]
fn connected_annotations_persist_without_mutating_a_provider() {
    let dir = tempfile::tempdir().unwrap();
    let driver = ConnectedPhotos::new(
        Source {
            driver: "synthetic".into(),
            path: dir.path().join("source"),
        },
        Box::new(MillionPhotos {
            thumbnails: Arc::default(),
            previews: Arc::default(),
            originals: Arc::default(),
        }),
    )
    .unwrap();
    let ids = vec!["0".into(), "999999".into()];
    driver.create_album("My trip").unwrap();
    driver.add_to_album("My trip", &ids).unwrap();
    driver.favorite(&ids, true).unwrap();
    driver.trash(&ids, true).unwrap();
    driver.caption("0", "Caption").unwrap();
    let first = driver.page(None).unwrap();
    assert!(first.library.photos[0].favorite && first.library.photos[0].deleted);
    assert_eq!(first.library.photos[0].caption, "Caption");
    assert_eq!(first.library.albums["My trip"].len(), 2);
    driver.favorite(&ids, false).unwrap();
    driver.trash(&ids, false).unwrap();
    let refreshed = driver.refresh_library(first.library).unwrap();
    assert!(!refreshed.photos[0].favorite && !refreshed.photos[0].deleted);
    assert!(driver.import(&[]).is_err());
    assert!(!driver.can_import());
}
#[test]
fn connection_validation_and_credential_free_settings() {
    let mut connection = Connection {
        provider: "nextcloud".into(),
        server: "https://cloud.example.com/nextcloud".into(),
        username: "alice".into(),
        folder: "Family Photos/2026".into(),
    };
    assert_eq!(
        connection.validate().unwrap().as_str(),
        "https://cloud.example.com/nextcloud/"
    );
    let json = serde_json::to_string(&connection).unwrap();
    assert!(!json.contains("password") && !json.contains("secret"));
    connection.server = "https://alice:password@example.com".into();
    assert!(connection.validate().is_err());
    connection.server = "file:///etc".into();
    assert!(connection.validate().is_err());
    connection.server = "https://cloud.example.com".into();
    connection.folder = "../private".into();
    assert!(connection.validate().is_err());
    connection.folder.clear();
    connection.username.clear();
    assert!(connection.validate().is_err());
}

#[cfg(feature = "remote")]
mod remote {
    use super::*;
    use desert_photos::sources::remote::{Remote, parse_nextcloud, search_body};
    use serde_json::json;
    use std::{
        io::{BufRead, Read, Write},
        net::{TcpListener, TcpStream},
        sync::{Mutex, atomic::AtomicBool},
        thread,
        time::Duration,
    };
    #[derive(Clone)]
    struct Request {
        method: String,
        path: String,
        headers: String,
        body: Vec<u8>,
    }
    struct Server {
        address: String,
        requests: Arc<Mutex<Vec<Request>>>,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Server {
        fn new(handler: impl Fn(&Request) -> (u16, Vec<u8>) + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let r = requests.clone();
            let s = stop.clone();
            let worker = thread::spawn(move || {
                while !s.load(Ordering::SeqCst) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(s) => s,
                        Err(_) => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                    };
                    // The listener polls nonblocking, but each accepted HTTP
                    // connection uses blocking BufRead/write_all. Inheriting a
                    // nonblocking socket can truncate headers or a response.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut headers = String::new();
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        headers.push_str(&line);
                    }
                    if headers.is_empty() {
                        continue;
                    }
                    let first = headers
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .collect::<Vec<_>>();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let request = Request {
                        method: first[0].into(),
                        path: first[1].into(),
                        headers,
                        body,
                    };
                    let (status, body) = handler(&request);
                    r.lock().unwrap().push(request);
                    write!(
                        stream,
                        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(&body).unwrap();
                }
            });
            Self {
                address,
                requests,
                stop,
                worker: Some(worker),
            }
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = TcpStream::connect(self.address.trim_start_matches("http://"));
            self.worker.take().unwrap().join().unwrap();
        }
    }
    fn config(server: &Server, provider: &str) -> Connection {
        Connection {
            provider: provider.into(),
            server: server.address.clone(),
            username: "alice".into(),
            folder: String::new(),
        }
    }
    fn fixture(id: &str) -> serde_json::Value {
        json!({"id": id, "type": "IMAGE", "originalFileName": "Sunset.jpg", "fileCreatedAt": "2026-01-01T00:00:00Z", "updatedAt": "v1", "isFavorite": true, "width": 4000, "height": 3000 })
    }
    fn png() -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(20, 10))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }
    #[test]
    fn immich_paging_authentication_and_separate_image_routes() {
        let id = "00000000-0000-4000-8000-000000000001";
        let server = Server::new(move |r| {
            if r.path == "/api/search/metadata" {
                let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
                assert_eq!(r.method, "POST");
                assert_eq!(body["size"], PAGE_SIZE);
                if body["page"] == 1 {
                    (
                        200,
                        json!({"assets": {"items": [fixture(id)], "nextPage": "2", "nextCursor": null, "total": 2}})
                            .to_string()
                            .into_bytes(),
                    )
                } else {
                    assert_eq!(body["page"], 2);
                    (
                        200,
                        json!({"assets": {"items": [], "nextPage": null}})
                            .to_string()
                            .into_bytes(),
                    )
                }
            } else if r.path.contains("/thumbnail?") {
                (200, png())
            } else if r.path.ends_with("/original") {
                (200, b"original".to_vec())
            } else {
                (
                    200,
                    json!({"originalFileName":"Sunset.jpg"})
                        .to_string()
                        .into_bytes(),
                )
            }
        });
        let remote = Remote::new(config(&server, "immich"), "fixture-key").unwrap();
        let page = remote.page(None).unwrap();
        assert_eq!(page.library.photos.len(), 1);
        assert_eq!(page.next.as_deref(), Some("2"));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        let second = remote.page(page.next.as_deref()).unwrap();
        assert!(second.next.is_none());
        let photo = &page.library.photos[0];
        let dir = tempfile::tempdir().unwrap();
        remote
            .image(photo, false, &dir.path().join("thumb"))
            .unwrap();
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .path
                .ends_with("size=thumbnail")
        );
        assert!(
            !server
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.path.ends_with("/original"))
        );
        remote
            .image(photo, true, &dir.path().join("preview"))
            .unwrap();
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .path
                .ends_with("size=preview")
        );
        remote.original(id, &dir.path().join("original")).unwrap();
        assert_eq!(fs::read(dir.path().join("original")).unwrap(), b"original");
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|r| r.headers.to_lowercase().contains("x-api-key: fixture-key"))
        );
    }
    #[test]
    fn immich_supports_new_opaque_cursors_and_rejects_oversized_pages() {
        let server = Server::new(|r| {
            let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
            if body.get("cursor").is_none() {
                (
                    200,
                    json!({"assets":{"items":[],"nextCursor":"opaque-token","nextPage":null}})
                        .to_string()
                        .into_bytes(),
                )
            } else {
                assert_eq!(body["cursor"], "opaque-token");
                (200, json!({"assets":{"items": vec![fixture("00000000-0000-4000-8000-000000000001"); PAGE_SIZE+1],"nextPage":null}}).to_string().into_bytes())
            }
        });
        let remote = Remote::new(config(&server, "immich"), "key").unwrap();
        let first = remote.page(None).unwrap();
        assert_eq!(first.next.as_deref(), Some("cursor:opaque-token"));
        assert!(remote.page(first.next.as_deref()).is_err());
    }
    fn dav_response(href: &str, etag: &str) -> String {
        format!(
            r#"<x:multistatus xmlns:x="DAV:" xmlns:oc="http://owncloud.org/ns"><x:response><x:href>{href}</x:href><x:propstat><x:prop><x:displayname>Sun &amp; sky.jpg</x:displayname><x:getcontenttype>image/jpeg</x:getcontenttype><x:getetag>{etag}</x:getetag><x:getcontentlength>1000</x:getcontentlength><x:getlastmodified>Thu, 01 Jan 2026 00:00:00 GMT</x:getlastmodified><oc:favorite>1</oc:favorite></x:prop><x:status>HTTP/1.1 200 OK</x:status></x:propstat><x:propstat><x:prop><x:getcontentlength>999</x:getcontentlength></x:prop><x:status>HTTP/1.1 404 Not Found</x:status></x:propstat></x:response></x:multistatus>"#
        )
    }
    #[test]
    fn nextcloud_uses_established_dav_auth_bounded_search_and_preview_service() {
        let server = Server::new(|r| {
            if r.method == "SEARCH" {
                let xml = std::str::from_utf8(&r.body).unwrap();
                assert!(xml.contains("<d:nresults>200</d:nresults>"));
                assert!(xml.contains("<ns:firstresult>200</ns:firstresult>"));
                assert!(xml.contains("/files/alice/"));
                (
                    207,
                    dav_response("/remote.php/dav/files/alice/Sun%20%26%20sky.jpg", "v1")
                        .into_bytes(),
                )
            } else {
                (200, png())
            }
        });
        let remote = Remote::new(config(&server, "nextcloud"), "fixture-password").unwrap();
        let page = remote.page(Some("200")).unwrap();
        let photo = &page.library.photos[0];
        assert_eq!(photo.name, "Sun & sky.jpg");
        assert_eq!(photo.bytes, 1000);
        assert!(photo.favorite);
        let dir = tempfile::tempdir().unwrap();
        remote
            .image(photo, false, &dir.path().join("thumb"))
            .unwrap();
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests[0].path, "/remote.php/dav/");
        assert!(
            requests
                .iter()
                .all(|r| r.headers.to_lowercase().contains("authorization: basic "))
        );
        assert!(requests[1].path.starts_with("/index.php/core/preview.png?"));
        assert!(requests[1].path.contains("x=512") && requests[1].path.contains("forceIcon=0"));
        assert!(requests[1].path.contains("Sun+%26+sky.jpg"));
    }
    #[test]
    fn webdav_namespaces_statuses_etags_subpaths_and_scope_are_checked() {
        let scope = url::Url::parse(
            "https://cloud.example.com/nextcloud/remote.php/dav/files/alice/Photos/",
        )
        .unwrap();
        let body = dav_response("/nextcloud/remote.php/dav/files/alice/Photos/a.jpg", "v1");
        let first = parse_nextcloud(body.as_bytes(), &scope, 0).unwrap();
        assert_eq!(first.library.photos[0].bytes, 1000);
        let second = parse_nextcloud(body.replace("v1", "v2").as_bytes(), &scope, 0).unwrap();
        assert_ne!(
            first.library.photos[0].fingerprint,
            second.library.photos[0].fingerprint
        );
        for href in [
            "https://evil.example.com/steal",
            "/nextcloud/remote.php/dav/files/bob/private.jpg",
            "/nextcloud/remote.php/dav/files/alice/Photos/../private.jpg",
        ] {
            assert!(parse_nextcloud(dav_response(href, "v1").as_bytes(), &scope, 0).is_err());
        }
        assert!(parse_nextcloud(b"<html>Login</html>", &scope, 0).is_err());
        let query = search_body("files/alice/Family & pets", 400);
        assert!(query.contains("Family &amp; pets"));
        assert!(query.contains("<ns:firstresult>400</ns:firstresult>"));
    }
    #[test]
    fn authentication_and_thumbnail_failures_never_fall_back_to_originals() {
        for status in [401, 403, 404, 429, 500, 302] {
            let server = Server::new(move |_| (status, b"error".to_vec()));
            let remote = Remote::new(config(&server, "immich"), "key").unwrap();
            assert!(remote.page(None).is_err());
            let photo = Photo {
                id: "00000000-0000-4000-8000-000000000001".into(),
                name: "Photo".into(),
                path: Default::default(),
                modified: Default::default(),
                bytes: 0,
                width: 0,
                height: 0,
                favorite: false,
                deleted: false,
                caption: String::new(),
                fingerprint: String::new(),
                duration: None,
            };
            let dir = tempfile::tempdir().unwrap();
            assert!(
                remote
                    .image(&photo, false, &dir.path().join("thumb"))
                    .is_err()
            );
            assert!(
                server
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|r| !r.path.ends_with("/original"))
            );
        }
    }
}

#[test]
fn promised_connected_originals_are_fetched_only_on_fulfillment() {
    let source = tempfile::tempdir().unwrap();
    let receiver = tempfile::tempdir().unwrap();
    let originals = Arc::new(AtomicUsize::new(0));
    let driver = ConnectedPhotos::new(
        Source {
            driver: "synthetic".into(),
            path: source.path().to_owned(),
        },
        Box::new(MillionPhotos {
            thumbnails: Arc::default(),
            previews: Arc::default(),
            originals: originals.clone(),
        }),
    )
    .unwrap();
    let photos = driver.page(None).unwrap().library.photos;
    assert!(photos[0].path.as_os_str().is_empty());
    let names = desert_photos::drag_export::filenames(photos[..2].iter().map(|p| p.name.as_str()));
    assert_eq!(names.len(), 2);
    assert_eq!(originals.load(Ordering::SeqCst), 0);
    let destination = receiver.path().join("Receiver's renamed file.png");
    desert_photos::drag_export::write_original(&driver, &photos[0].id, &destination).unwrap();
    assert_eq!(originals.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(&destination).unwrap(), b"original bytes");
    assert!(
        desert_photos::drag_export::write_original(&driver, &photos[0].id, &destination).is_err()
    );
    assert_eq!(
        originals.load(Ordering::SeqCst),
        1,
        "an occupied destination must fail before downloading another original"
    );
    assert_eq!(fs::read_dir(receiver.path()).unwrap().count(), 1);
}

/// A provider with its own albums, as Apple Photos has.
struct AlbumSource;
impl Provider for AlbumSource {
    fn page(&self, _: Option<&str>) -> Result<PhotoPage> {
        let photos = (0..3)
            .map(|i| {
                Arc::new(Photo {
                    id: format!("{i}"),
                    name: if i == 2 {
                        "Clip.mov".into()
                    } else {
                        format!("Photo {i}.png")
                    },
                    path: Default::default(),
                    modified: Default::default(),
                    bytes: 10,
                    width: 64,
                    height: 48,
                    favorite: false,
                    deleted: false,
                    caption: String::new(),
                    fingerprint: format!("asset-{i}"),
                    duration: (i == 2).then_some(12.5),
                })
            })
            .collect();
        Ok(PhotoPage {
            library: Library {
                photos,
                ..Default::default()
            },
            next: None,
            total: Some(3),
        })
    }
    fn image(&self, _: &Photo, _: bool, _: &Path) -> Result<()> {
        anyhow::bail!("unused")
    }
    fn original(&self, _: &str, _: &Path) -> Result<String> {
        anyhow::bail!("unused")
    }
    fn albums(
        &self,
    ) -> Result<std::collections::BTreeMap<String, std::collections::BTreeSet<String>>> {
        Ok([
            ("Trip".to_owned(), ["0".to_owned(), "2".to_owned()].into()),
            ("Shared".to_owned(), ["1".to_owned()].into()),
        ]
        .into())
    }
}
#[test]
fn provider_albums_sync_read_only_and_merge_with_local_albums() {
    let dir = tempfile::tempdir().unwrap();
    let driver = ConnectedPhotos::new(
        Source {
            driver: "synthetic".into(),
            path: dir.path().join("source"),
        },
        Box::new(AlbumSource),
    )
    .unwrap();
    driver.create_album("Shared").unwrap();
    driver.add_to_album("Shared", &["0".into()]).unwrap();
    let library = driver.page(None).unwrap().library;
    assert_eq!(library.albums["Trip"].len(), 2);
    assert!(library.synced.contains("Trip"));
    // A local album of the same name merges, and stays editable here.
    assert_eq!(library.albums["Shared"].len(), 2);
    assert!(!library.synced.contains("Shared"));
    assert!(driver.add_to_album("Trip", &["1".into()]).is_err());
    assert!(driver.remove_from_album("Trip", &["0".into()]).is_err());
    assert!(driver.remove_from_album("Shared", &["1".into()]).is_err());
    driver.remove_from_album("Shared", &["0".into()]).unwrap();
    // Annotation refreshes (after favoriting, say) keep the synced albums.
    driver.favorite(&["0".into()], true).unwrap();
    let refreshed = driver.refresh_library(library).unwrap();
    assert!(refreshed.synced.contains("Trip"));
    assert_eq!(refreshed.albums["Shared"].len(), 1);
    assert_eq!(refreshed.photos[2].duration, Some(12.5));
}
