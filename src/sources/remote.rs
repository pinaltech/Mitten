//! Immich REST and Nextcloud WebDAV, using reqwest_dav for DAV transport/auth.
use super::*;
use crate::loading::PAGE_SIZE;
use chrono::{DateTime, Utc};
use reqwest::{Method, blocking::Client, redirect::Policy};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    sync::{Arc, OnceLock},
    time::Duration,
};
use url::Url;

const METADATA_LIMIT: u64 = 8 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 32 * 1024 * 1024;

pub fn credential(source: &Source) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(
        "com.pinal.DesertPhotos",
        &digest(&source.path.to_string_lossy()),
    )?)
}
pub fn save_connection(config: &Connection, secret: &str, data: &Path) -> Result<Source> {
    ensure!(
        !secret.trim().is_empty(),
        "Enter an API key or app password"
    );
    let source = config.source(data)?;
    fs::create_dir_all(&source.path)?;
    // A platform keystore failure must not silently fall back to plaintext settings.
    credential(&source)?
        .set_password(secret)
        .context("Cannot save credentials in the system keyring")?;
    let temp = tempfile::NamedTempFile::new_in(&source.path)?;
    serde_json::to_writer(temp.as_file(), config)?;
    temp.persist(source.path.join("connection.json"))
        .map_err(|e| e.error)?;
    Ok(source)
}
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("network runtime"))
}

pub struct Remote {
    config: Connection,
    base: Url,
    http: Client,
    secret: String,
    dav: reqwest_dav::Client,
    /// Immich album IDs by the names shown in the sidebar.
    album_ids: std::sync::Mutex<std::collections::HashMap<String, String>>,
}
impl Remote {
    pub fn new(config: Connection, secret: &str) -> Result<Self> {
        let mut base = config.validate()?;
        if config.provider == "immich" && !base.path().trim_end_matches('/').ends_with("/api") {
            base = base.join("api/")?;
        }
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(Policy::none())
            .build()?;
        let agent = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(Policy::none())
            .build()?;
        let dav = reqwest_dav::ClientBuilder::new()
            .set_agent(agent)
            .set_host(base.to_string())
            .set_auth(reqwest_dav::Auth::Basic(
                config.username.clone(),
                secret.to_owned(),
            ))
            .build()?;
        Ok(Self {
            config,
            base,
            http,
            secret: secret.to_owned(),
            dav,
            album_ids: Default::default(),
        })
    }
    fn immich_request(&self, method: Method, url: Url) -> reqwest::blocking::RequestBuilder {
        self.http
            .request(method, url)
            .header("x-api-key", &self.secret)
    }
    fn immich_page(&self, cursor: Option<&str>) -> Result<PhotoPage> {
        // Photos and videos together (no type filter), newest first.
        let mut body =
            json!({"size": PAGE_SIZE, "withExif": true, "order": "desc", "isTrashed": false});
        match cursor {
            Some(c) if c.starts_with("cursor:") => body["cursor"] = json!(&c[7..]),
            Some(c) => body["page"] = json!(c.parse::<usize>().context("Invalid Immich page")?),
            None => body["page"] = json!(1),
        }
        let response = self
            .immich_request(Method::POST, self.base.join("search/metadata")?)
            .json(&body)
            .send()?;
        let data: Value = serde_json::from_slice(&read_response(response, METADATA_LIMIT)?)?;
        let assets = data
            .get("assets")
            .context("Immich response has no assets")?;
        let items = assets["items"]
            .as_array()
            .context("Invalid Immich assets response")?;
        ensure!(items.len() <= PAGE_SIZE, "Immich ignored the page limit");
        let mut photos = Vec::with_capacity(items.len());
        for a in items {
            let video = a["type"].as_str() == Some("VIDEO");
            if a["type"]
                .as_str()
                .is_some_and(|t| t != "IMAGE" && t != "VIDEO")
            {
                continue;
            }
            let id = a["id"]
                .as_str()
                .context("Immich asset has no ID")?
                .to_owned();
            uuid::Uuid::parse_str(&id).context("Invalid Immich asset ID")?;
            let modified = date(a["fileCreatedAt"].as_str().or(a["createdAt"].as_str()));
            photos.push(Arc::new(Photo {
                fingerprint: digest(&format!("{id}:{}:{}", a["updatedAt"], a["checksum"])),
                duration: video.then(|| a["duration"].as_str().map_or(0., clock_seconds)),
                id,
                path: PathBuf::new(),
                name: a["originalFileName"].as_str().unwrap_or("Photo").into(),
                modified,
                bytes: a["exifInfo"]["fileSizeInByte"].as_u64().unwrap_or(0),
                width: a["width"]
                    .as_u64()
                    .or(a["exifInfo"]["exifImageWidth"].as_u64())
                    .unwrap_or(0) as u32,
                height: a["height"]
                    .as_u64()
                    .or(a["exifInfo"]["exifImageHeight"].as_u64())
                    .unwrap_or(0) as u32,
                favorite: a["isFavorite"].as_bool().unwrap_or(false),
                deleted: false,
                caption: a["exifInfo"]["description"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                placeholder: a["thumbhash"]
                    .as_str()
                    .and_then(crate::loading::base64_decode)
                    .filter(|h| (5..=64).contains(&h.len()))
                    .map(Into::into),
            }));
        }
        let next = assets["nextCursor"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| format!("cursor:{s}"))
            .or_else(|| {
                assets["nextPage"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            });
        let total = assets["total"]
            .as_u64()
            .map(|n| n as usize)
            .filter(|&n| n >= photos.len());
        Ok(PhotoPage {
            library: Library {
                photos,
                albums: Default::default(),
                synced: Default::default(),
            },
            next,
            total,
        })
    }
    /// A write, explaining the API key permission it needs when refused.
    fn immich_write(
        &self,
        method: Method,
        path: &str,
        body: Value,
        permission: &str,
    ) -> Result<()> {
        let response = self
            .immich_request(method, self.base.join(path)?)
            .json(&body)
            .send()?;
        ensure!(
            response.status() != reqwest::StatusCode::FORBIDDEN
                && response.status() != reqwest::StatusCode::UNAUTHORIZED,
            "Immich refused the change. Give this library's API key the “{permission}” permission."
        );
        checked(response)?;
        Ok(())
    }
    fn album_id(&self, name: &str) -> Result<String> {
        self.album_ids
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .with_context(|| format!("“{name}” is not an Immich album"))
    }
    fn immich_json(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self.immich_request(method, self.base.join(path)?);
        if let Some(body) = body {
            request = request.json(&body);
        }
        Ok(serde_json::from_slice(&read_response(
            request.send()?,
            METADATA_LIMIT,
        )?)?)
    }
    /// Asset IDs from an Immich search, following its pages up to `limit`.
    fn immich_ids(&self, path: &str, mut body: Value, limit: usize) -> Result<Vec<String>> {
        let mut ids = vec![];
        let mut page = 1;
        loop {
            body["page"] = json!(page);
            body["size"] = json!(PAGE_SIZE);
            let data = self.immich_json(Method::POST, path, Some(body.clone()))?;
            let assets = &data["assets"];
            ids.extend(
                assets["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a["id"].as_str().map(str::to_owned)),
            );
            let next = assets["nextPage"]
                .as_str()
                .and_then(|s| s.parse::<usize>().ok())
                .or_else(|| assets["nextPage"].as_u64().map(|n| n as usize));
            match next {
                Some(n) if n > page && ids.len() < limit => page = n,
                _ => break,
            }
        }
        ids.truncate(limit);
        Ok(ids)
    }
    fn basic_auth(&self) -> String {
        format!(
            "Basic {}",
            base64(format!("{}:{}", self.config.username, self.secret).as_bytes())
        )
    }
    fn scope(&self) -> Result<Url> {
        let mut url = self.base.join("remote.php/dav/files/")?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid server URL"))?;
            segments.pop_if_empty().push(&self.config.username);
            for part in self.config.folder.split('/').filter(|s| !s.is_empty()) {
                segments.push(part);
            }
            segments.push("");
        }
        Ok(url)
    }
    fn nextcloud_page(&self, cursor: Option<&str>) -> Result<PhotoPage> {
        let offset: usize = cursor
            .unwrap_or("0")
            .parse()
            .context("Invalid WebDAV page")?;
        let scope = self.scope()?;
        let dav_prefix = self.base.join("remote.php/dav/")?;
        let relative_scope = scope
            .path()
            .strip_prefix(dav_prefix.path())
            .context("Invalid WebDAV scope")?;
        let body = search_body(relative_scope, offset);
        let bytes = runtime().block_on(async {
            let response = self
                .dav
                .start_request(Method::from_bytes(b"SEARCH")?, "remote.php/dav/")
                .await?
                .header("content-type", "application/xml; charset=utf-8")
                .body(body)
                .send()
                .await?;
            read_async(response, METADATA_LIMIT).await
        })?;
        parse_nextcloud(&bytes, &scope, offset)
    }
    /// Resolve server hrefs only inside this user's selected DAV folder.
    fn asset_url(&self, id: &str) -> Result<Url> {
        let scope = self.scope()?;
        let url = self.base.join(id)?;
        ensure!(
            url.origin() == self.base.origin()
                && url.path().starts_with(scope.path())
                && url.query().is_none()
                && url.fragment().is_none(),
            "Photo is outside the connected folder"
        );
        Ok(url)
    }
    fn dav_relative(&self, url: &Url) -> Result<String> {
        Ok(url
            .path()
            .strip_prefix(self.base.path())
            .context("Photo is outside this server")?
            .to_owned())
    }
}
impl Provider for Remote {
    fn page(&self, cursor: Option<&str>) -> Result<PhotoPage> {
        if self.config.provider == "immich" {
            self.immich_page(cursor)
        } else {
            self.nextcloud_page(cursor)
        }
    }
    fn image(&self, photo: &Photo, preview: bool, destination: &Path) -> Result<()> {
        self.image_at(photo, if preview { 2560 } else { 512 }, destination)
    }
    /// Immich serves fixed renditions: every grid size up to 512 px is its
    /// ~250 px thumbnail and every viewer size up to 2560 px its ~1440 px
    /// preview, so those sizes are the same image (and the same cache file).
    fn rendition(&self, edge: u32) -> u32 {
        if self.config.provider != "immich" {
            return edge;
        }
        match edge {
            0..=512 => 512,
            1025..=2560 => 2560,
            _ => edge,
        }
    }
    fn image_at(&self, photo: &Photo, edge: u32, destination: &Path) -> Result<()> {
        let bytes = if self.config.provider == "immich" {
            uuid::Uuid::parse_str(&photo.id)?;
            // Immich keeps three renditions: a ~250 px thumbnail, a ~1440 px
            // preview and (newer servers) a full-size JPEG.
            // Grid sizes up to 512 px use the thumbnail, as they always
            // have; automatic sizing measures whether the preview is worth it.
            let sizes: &[&str] = match edge {
                0..=512 => &["thumbnail"],
                513..=2560 => &["preview"],
                _ => &["fullsize", "preview"],
            };
            let mut result = Err(anyhow::anyhow!("No Immich rendition"));
            for size in sizes {
                let mut url = self.base.join(&format!("assets/{}/thumbnail", photo.id))?;
                url.query_pairs_mut().append_pair("size", size);
                result = self
                    .immich_request(Method::GET, url)
                    .send()
                    .map_err(anyhow::Error::from)
                    .and_then(|r| read_response(r, IMAGE_LIMIT));
                if result.is_ok() {
                    break;
                }
            }
            result?
        } else {
            let url = self.asset_url(&photo.id)?;
            let scope_root = self.base.join("remote.php/dav/files/")?;
            let mut root = scope_root.clone();
            root.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid URL"))?
                .pop_if_empty()
                .push(&self.config.username)
                .push("");
            let file = url
                .path()
                .strip_prefix(root.path())
                .context("Invalid photo path")?;
            let file = percent_decode(file)?;
            let edge = edge.to_string();
            let mut url = self.base.join("index.php/core/preview.png")?;
            url.query_pairs_mut()
                .append_pair("file", &format!("/{file}"))
                .append_pair("x", &edge)
                .append_pair("y", &edge)
                .append_pair("a", "1")
                .append_pair("forceIcon", "0");
            read_response(
                self.http
                    .get(url)
                    .basic_auth(&self.config.username, Some(&self.secret))
                    .send()?,
                IMAGE_LIMIT,
            )?
        };
        fs::write(destination, bytes)?;
        Ok(())
    }
    fn video(&self, photo: &Photo) -> Option<VideoSource> {
        photo.duration?;
        if self.config.provider == "immich" {
            uuid::Uuid::parse_str(&photo.id).ok()?;
            let url = self
                .base
                .join(&format!("assets/{}/video/playback", photo.id))
                .ok()?;
            Some(VideoSource::Remote {
                url: url.to_string(),
                headers: vec![("x-api-key".into(), self.secret.clone())],
            })
        } else {
            Some(VideoSource::Remote {
                url: self.asset_url(&photo.id).ok()?.to_string(),
                headers: vec![("Authorization".into(), self.basic_auth())],
            })
        }
    }
    fn details(&self, photo: &Photo) -> Result<PhotoDetails> {
        if self.config.provider != "immich" {
            return Ok(PhotoDetails {
                bytes: (photo.bytes > 0).then_some(photo.bytes),
                ..Default::default()
            });
        }
        uuid::Uuid::parse_str(&photo.id)?;
        let a = self.immich_json(Method::GET, &format!("assets/{}", photo.id), None)?;
        Ok(immich_details(&a))
    }
    fn explore(&self) -> bool {
        self.config.provider == "immich"
    }
    fn albums(&self) -> Result<BTreeMap<String, BTreeSet<String>>> {
        if self.config.provider != "immich" {
            return Ok(BTreeMap::new());
        }
        let mut albums = BTreeMap::new();
        let mut ids = std::collections::HashMap::new();
        // Owned albums and albums shared with this user.
        let mut listed = vec![];
        for path in ["albums", "albums?shared=true"] {
            let data = self.immich_json(Method::GET, path, None)?;
            for album in data.as_array().into_iter().flatten() {
                let Some(id) = album["id"].as_str() else {
                    continue;
                };
                if !listed.iter().any(|(i, _): &(String, String)| i == id) {
                    let name = album["albumName"].as_str().unwrap_or("").trim();
                    listed.push((id.to_owned(), name.to_owned()));
                }
            }
        }
        for (id, base) in listed {
            let base = if base.is_empty() {
                "Untitled Album".to_owned()
            } else {
                base
            };
            let mut name = base.clone();
            for n in 2.. {
                if !albums.contains_key(&name) {
                    break;
                }
                name = format!("{base} ({n})");
            }
            // Album responses carry their assets on older servers; newer
            // ones are searched by album instead.
            let detail = self.immich_json(Method::GET, &format!("albums/{id}"), None)?;
            let members: BTreeSet<String> = match detail["assets"].as_array() {
                Some(assets) if !assets.is_empty() || detail["assetCount"].as_u64() == Some(0) => {
                    assets
                        .iter()
                        .filter_map(|a| a["id"].as_str().map(str::to_owned))
                        .collect()
                }
                _ => self
                    .immich_ids("search/metadata", json!({"albumIds": [id]}), 100_000)?
                    .into_iter()
                    .collect(),
            };
            ids.insert(name.clone(), id);
            albums.insert(name, members);
        }
        *self.album_ids.lock().unwrap() = ids;
        Ok(albums)
    }
    fn writable(&self) -> bool {
        self.config.provider == "immich"
    }
    fn set_favorite(&self, ids: &[String], value: bool) -> Result<()> {
        ensure!(self.writable(), "This source is read-only");
        for id in ids {
            uuid::Uuid::parse_str(id)?;
        }
        for chunk in ids.chunks(500) {
            self.immich_write(
                Method::PUT,
                "assets",
                json!({"ids": chunk, "isFavorite": value}),
                "asset.update",
            )?;
        }
        Ok(())
    }
    fn set_description(&self, id: &str, description: &str) -> Result<()> {
        ensure!(self.writable(), "This source is read-only");
        uuid::Uuid::parse_str(id)?;
        self.immich_write(
            Method::PUT,
            &format!("assets/{id}"),
            json!({"description": description}),
            "asset.update",
        )
    }
    fn create_album(&self, name: &str) -> Result<()> {
        ensure!(self.writable(), "This source is read-only");
        let response = self
            .immich_request(Method::POST, self.base.join("albums")?)
            .json(&json!({"albumName": name}))
            .send()?;
        ensure!(
            response.status() != reqwest::StatusCode::FORBIDDEN,
            "Immich refused the change. Give this library's API key the “album.create” permission."
        );
        let album: Value = serde_json::from_slice(&read_response(response, METADATA_LIMIT)?)?;
        let id = album["id"]
            .as_str()
            .context("Immich did not return the album")?;
        self.album_ids
            .lock()
            .unwrap()
            .insert(name.to_owned(), id.to_owned());
        Ok(())
    }
    fn add_to_album(&self, name: &str, ids: &[String]) -> Result<()> {
        let album = self.album_id(name)?;
        for chunk in ids.chunks(500) {
            self.immich_write(
                Method::PUT,
                &format!("albums/{album}/assets"),
                json!({"ids": chunk}),
                "albumAsset.create",
            )?;
        }
        Ok(())
    }
    fn remove_from_album(&self, name: &str, ids: &[String]) -> Result<()> {
        let album = self.album_id(name)?;
        for chunk in ids.chunks(500) {
            self.immich_write(
                Method::DELETE,
                &format!("albums/{album}/assets"),
                json!({"ids": chunk}),
                "albumAsset.delete",
            )?;
        }
        Ok(())
    }
    fn people(&self) -> Result<Vec<Person>> {
        if self.config.provider != "immich" {
            return Ok(vec![]);
        }
        let data = self.immich_json(Method::GET, "people?withHidden=false&size=1000", None)?;
        let mut people: Vec<Person> = data["people"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| !p["isHidden"].as_bool().unwrap_or(false))
            .filter_map(|p| {
                Some(Person {
                    id: p["id"].as_str()?.to_owned(),
                    name: p["name"].as_str().unwrap_or_default().to_owned(),
                })
            })
            .collect();
        // Named people first, keeping the server's order (most photos first).
        people.sort_by_key(|p| p.name.is_empty());
        Ok(people)
    }
    fn person_thumbnail(&self, id: &str, destination: &Path) -> Result<()> {
        ensure!(
            self.config.provider == "immich",
            "This source has no people"
        );
        uuid::Uuid::parse_str(id)?;
        let bytes = read_response(
            self.immich_request(
                Method::GET,
                self.base.join(&format!("people/{id}/thumbnail"))?,
            )
            .send()?,
            IMAGE_LIMIT,
        )?;
        fs::write(destination, bytes)?;
        Ok(())
    }
    fn places(&self) -> Result<Vec<Place>> {
        if self.config.provider != "immich" {
            return Ok(vec![]);
        }
        let data = self.immich_json(Method::GET, "search/explore", None)?;
        Ok(data
            .as_array()
            .into_iter()
            .flatten()
            .filter(|f| f["fieldName"].as_str() == Some("exifInfo.city"))
            .flat_map(|f| f["items"].as_array().into_iter().flatten())
            .filter_map(|item| {
                Some(Place {
                    name: item["value"].as_str()?.to_owned(),
                    photo: item["data"]["id"].as_str()?.to_owned(),
                })
            })
            .collect())
    }
    fn matching(&self, query: &Query) -> Result<Vec<String>> {
        if self.config.provider != "immich" {
            return Ok(vec![]);
        }
        let body = match query {
            Query::Person(id) => json!({"personIds": [id]}),
            Query::Place(city) => json!({"city": city}),
        };
        self.immich_ids("search/metadata", body, 20_000)
    }
    fn search(&self, text: &str) -> Result<Option<Vec<String>>> {
        if self.config.provider != "immich" || text.trim().is_empty() {
            return Ok(None);
        }
        // Smart (CLIP) search needs the server's machine learning; without it
        // fall back to searching descriptions and file names.
        let smart = self.immich_ids("search/smart", json!({"query": text}), 1_000);
        let ids = match smart {
            Ok(ids) => ids,
            Err(_) => {
                let mut ids =
                    self.immich_ids("search/metadata", json!({"description": text}), 1_000)?;
                if let Ok(names) =
                    self.immich_ids("search/metadata", json!({"originalFileName": text}), 1_000)
                {
                    ids.extend(names);
                }
                ids
            }
        };
        Ok(Some(ids))
    }
    fn original(&self, id: &str, destination: &Path) -> Result<String> {
        if self.config.provider == "immich" {
            uuid::Uuid::parse_str(id)?;
            let metadata = read_response(
                self.immich_request(Method::GET, self.base.join(&format!("assets/{id}"))?)
                    .send()?,
                METADATA_LIMIT,
            )?;
            let metadata: Value = serde_json::from_slice(&metadata)?;
            let name = metadata["originalFileName"]
                .as_str()
                .context("Original filename unavailable")?
                .to_owned();
            let mut response = checked(
                self.immich_request(
                    Method::GET,
                    self.base.join(&format!("assets/{id}/original"))?,
                )
                .send()?,
            )?;
            std::io::copy(&mut response, &mut fs::File::create(destination)?)?;
            Ok(name)
        } else {
            let url = self.asset_url(id)?;
            let name = percent_decode(
                url.path_segments()
                    .and_then(|mut p| p.next_back())
                    .context("Invalid filename")?,
            )?;
            let path = self.dav_relative(&url)?;
            runtime().block_on(async {
                let mut response = self.dav.get(&path).await?;
                ensure!(
                    response.status().is_success(),
                    "WebDAV download failed: {}",
                    response.status()
                );
                let mut file = fs::File::create(destination)?;
                while let Some(chunk) = response.chunk().await? {
                    file.write_all(&chunk)?;
                }
                Ok::<_, anyhow::Error>(())
            })?;
            Ok(name)
        }
    }
}
/// Immich's video length, "H:MM:SS.ffffff", in seconds.
pub fn clock_seconds(s: &str) -> f32 {
    s.split(':')
        .try_fold(0f32, |total, part| {
            Some(total * 60. + part.parse::<f32>().ok()?)
        })
        .filter(|s| s.is_finite() && *s >= 0.)
        .unwrap_or(0.)
}
/// Details from an Immich asset response (`GET /assets/{id}`).
pub fn immich_details(a: &Value) -> PhotoDetails {
    let exif = &a["exifInfo"];
    let text = |v: &Value| {
        v.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let number = |v: &Value| v.as_f64().map(|n| n as f32);
    let exposure = exif["exposureTime"]
        .as_str()
        .and_then(|s| match s.split_once('/') {
            Some((n, d)) => Some(n.trim().parse::<f32>().ok()? / d.trim().parse::<f32>().ok()?),
            None => s.trim().parse().ok(),
        });
    // `localDateTime` is the camera's wall-clock time (sent with a Z).
    let taken = a["localDateTime"]
        .as_str()
        .or(exif["dateTimeOriginal"].as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.naive_utc());
    let mut people: Vec<Person> = a["people"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| !p["isHidden"].as_bool().unwrap_or(false))
        .filter_map(|p| {
            Some(Person {
                id: p["id"].as_str()?.to_owned(),
                name: p["name"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .collect();
    people.sort_by_key(|p| p.name.is_empty());
    PhotoDetails {
        description: text(&exif["description"]),
        taken,
        zone: text(&exif["timeZone"]),
        make: text(&exif["make"]),
        model: text(&exif["model"]),
        lens: text(&exif["lensModel"]),
        f_number: number(&exif["fNumber"]),
        exposure,
        iso: exif["iso"].as_u64().map(|n| n as u32),
        focal_length: number(&exif["focalLength"]),
        latitude: exif["latitude"].as_f64(),
        longitude: exif["longitude"].as_f64(),
        place: [&exif["city"], &exif["state"], &exif["country"]]
            .into_iter()
            .filter_map(text)
            .collect(),
        bytes: exif["fileSizeInByte"].as_u64(),
        width: exif["exifImageWidth"].as_u64().map(|n| n as u32),
        height: exif["exifImageHeight"].as_u64().map(|n| n as u32),
        people,
    }
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
fn date(s: Option<&str>) -> DateTime<Utc> {
    s.and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_default()
}
fn checked(response: reqwest::blocking::Response) -> Result<reqwest::blocking::Response> {
    ensure!(
        response.status().is_success(),
        "Photo server returned {}. Check the address, credentials and read permissions.",
        response.status()
    );
    Ok(response)
}
fn read_response(response: reqwest::blocking::Response, limit: u64) -> Result<Vec<u8>> {
    let response = checked(response)?;
    ensure!(
        response.content_length().is_none_or(|n| n <= limit),
        "Photo server response exceeds the size limit"
    );
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Photo server response exceeds the size limit"
    );
    Ok(bytes)
}
async fn read_async(mut response: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    ensure!(
        response.status().is_success(),
        "WebDAV server returned {}. Check the address and app password.",
        response.status()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            (bytes.len() + chunk.len()) as u64 <= limit,
            "WebDAV response exceeds the page size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn search_body(scope: &str, offset: usize) -> String {
    format!(
        r#"<?xml version="1.0"?><d:searchrequest xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:ns="https://github.com/icewind1991/SearchDAV/ns"><d:basicsearch><d:select><d:prop><d:displayname/><d:getcontenttype/><d:getcontentlength/><d:getetag/><d:getlastmodified/><oc:favorite/></d:prop></d:select><d:from><d:scope><d:href>/{}</d:href><d:depth>infinity</d:depth></d:scope></d:from><d:where><d:or><d:like><d:prop><d:getcontenttype/></d:prop><d:literal>image/%</d:literal></d:like><d:like><d:prop><d:getcontenttype/></d:prop><d:literal>video/%</d:literal></d:like></d:or></d:where><d:orderby><d:order><d:prop><d:getlastmodified/></d:prop><d:descending/></d:order><d:order><d:prop><d:displayname/></d:prop><d:ascending/></d:order></d:orderby><d:limit><d:nresults>{PAGE_SIZE}</d:nresults><ns:firstresult>{offset}</ns:firstresult></d:limit></d:basicsearch></d:searchrequest>"#,
        xml(scope)
    )
}
fn percent_decode(s: &str) -> Result<String> {
    // Url's query parser provides standards-tested percent decoding (preserve '+').
    let encoded = format!("v={}", s.replace('+', "%2B"));
    Ok(url::form_urlencoded::parse(encoded.as_bytes())
        .next()
        .context("Invalid encoded path")?
        .1
        .into_owned())
}
pub fn parse_nextcloud(bytes: &[u8], scope: &Url, offset: usize) -> Result<PhotoPage> {
    let text = std::str::from_utf8(bytes)?;
    let doc = roxmltree::Document::parse(text).context("Invalid WebDAV XML")?;
    ensure!(
        doc.root_element().has_tag_name(("DAV:", "multistatus")),
        "Invalid WebDAV response"
    );
    let responses: Vec<_> = doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name(("DAV:", "response")))
        .collect();
    ensure!(
        responses.len() <= PAGE_SIZE,
        "Nextcloud ignored the page limit"
    );
    let next = (responses.len() == PAGE_SIZE).then(|| (offset + responses.len()).to_string());
    let mut photos = vec![];
    for response in responses {
        let href = response
            .children()
            .find(|n| n.has_tag_name(("DAV:", "href")))
            .and_then(|n| n.text())
            .context("WebDAV item has no href")?;
        let url = scope.join(href)?;
        ensure!(
            url.origin() == scope.origin()
                && url.path().starts_with(scope.path())
                && url.query().is_none()
                && url.fragment().is_none(),
            "WebDAV returned a photo outside your folder"
        );
        let props: Vec<_> = response
            .children()
            .filter(|n| n.has_tag_name(("DAV:", "propstat")))
            .filter(|n| {
                n.children().any(|c| {
                    c.has_tag_name(("DAV:", "status"))
                        && c.text()
                            .is_some_and(|s| s.split_whitespace().nth(1) == Some("200"))
                })
            })
            .flat_map(|n| {
                n.children()
                    .filter(|c| c.has_tag_name(("DAV:", "prop")))
                    .flat_map(|n| n.children())
            })
            .collect();
        let prop = |namespace, name| {
            props
                .iter()
                .find(|n| n.has_tag_name((namespace, name)))
                .and_then(|n| n.text())
        };
        let kind = prop("DAV:", "getcontenttype").unwrap_or_default();
        let video = kind.starts_with("video/");
        if !kind.starts_with("image/") && !video {
            continue;
        }
        let id = url.path().to_owned();
        let name = prop("DAV:", "displayname")
            .map(str::to_owned)
            .unwrap_or(percent_decode(
                url.path_segments()
                    .and_then(|mut p| p.next_back())
                    .unwrap_or("Photo"),
            )?);
        let modified = prop("DAV:", "getlastmodified")
            .and_then(|s| DateTime::parse_from_rfc2822(s).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_default();
        let bytes = prop("DAV:", "getcontentlength")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        photos.push(Arc::new(Photo {
            fingerprint: digest(&format!(
                "{id}:{}:{bytes}:{modified}",
                prop("DAV:", "getetag").unwrap_or_default()
            )),
            id,
            path: PathBuf::new(),
            name,
            modified,
            bytes,
            width: 0,
            height: 0,
            favorite: prop("http://owncloud.org/ns", "favorite") == Some("1"),
            deleted: false,
            caption: String::new(),
            // WebDAV has no length; zero marks a video of unknown length.
            duration: video.then_some(0.),
            placeholder: None,
        }));
    }
    Ok(PhotoPage {
        library: Library {
            photos,
            albums: Default::default(),
            synced: Default::default(),
        },
        next,
        total: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immich_details_and_video_lengths() {
        assert_eq!(clock_seconds("0:01:05.500000"), 65.5);
        assert_eq!(clock_seconds("bad"), 0.);
        assert_eq!(base64(b"alice:secret"), "YWxpY2U6c2VjcmV0");
        assert_eq!(base64(b"ab"), "YWI=");
        let details = immich_details(&json!({
            "localDateTime": "2025-06-01T18:30:00.000Z",
            "exifInfo": {"make": "Apple", "model": "iPhone 15 Pro", "fNumber": 1.78,
                "exposureTime": "1/120", "iso": 80, "focalLength": 6.86,
                "latitude": 48.85, "longitude": 2.35, "city": "Paris", "state": "Île-de-France",
                "country": "France", "timeZone": "Europe/Paris", "fileSizeInByte": 2048},
            "people": [{"id": "b", "name": ""}, {"id": "a", "name": "Ana"}]
        }));
        assert_eq!(details.camera().as_deref(), Some("Apple iPhone 15 Pro"));
        assert_eq!(
            details.exposure_line().as_deref(),
            Some("ƒ/1.8 · 1/120 s · 6.9 mm · ISO 80")
        );
        assert_eq!(details.place, ["Paris", "Île-de-France", "France"]);
        assert_eq!(details.coordinates(), Some((48.85, 2.35)));
        assert_eq!(details.people[0].name, "Ana");
        assert_eq!(details.taken.unwrap().format("%H:%M").to_string(), "18:30");
    }
}
