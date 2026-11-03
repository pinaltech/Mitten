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
        })
    }
    fn immich_request(&self, method: Method, url: Url) -> reqwest::blocking::RequestBuilder {
        self.http
            .request(method, url)
            .header("x-api-key", &self.secret)
    }
    fn immich_page(&self, cursor: Option<&str>) -> Result<PhotoPage> {
        let mut body = json!({"size": PAGE_SIZE, "type": "IMAGE", "withExif": true, "order": "desc", "isTrashed": false});
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
            if a["type"].as_str().is_some_and(|t| t != "IMAGE") {
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
                duration: None,
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
        let bytes = if self.config.provider == "immich" {
            uuid::Uuid::parse_str(&photo.id)?;
            let mut url = self.base.join(&format!("assets/{}/thumbnail", photo.id))?;
            url.query_pairs_mut()
                .append_pair("size", if preview { "preview" } else { "thumbnail" });
            read_response(self.immich_request(Method::GET, url).send()?, IMAGE_LIMIT)?
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
            let edge = if preview { "2560" } else { "512" };
            let mut url = self.base.join("index.php/core/preview.png")?;
            url.query_pairs_mut()
                .append_pair("file", &format!("/{file}"))
                .append_pair("x", edge)
                .append_pair("y", edge)
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
        r#"<?xml version="1.0"?><d:searchrequest xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:ns="https://github.com/icewind1991/SearchDAV/ns"><d:basicsearch><d:select><d:prop><d:displayname/><d:getcontenttype/><d:getcontentlength/><d:getetag/><d:getlastmodified/><oc:favorite/></d:prop></d:select><d:from><d:scope><d:href>/{}</d:href><d:depth>infinity</d:depth></d:scope></d:from><d:where><d:like><d:prop><d:getcontenttype/></d:prop><d:literal>image/%</d:literal></d:like></d:where><d:orderby><d:order><d:prop><d:getlastmodified/></d:prop><d:descending/></d:order><d:order><d:prop><d:displayname/></d:prop><d:ascending/></d:order></d:orderby><d:limit><d:nresults>{PAGE_SIZE}</d:nresults><ns:firstresult>{offset}</ns:firstresult></d:limit></d:basicsearch></d:searchrequest>"#,
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
        if !prop("DAV:", "getcontenttype").is_some_and(|s| s.starts_with("image/")) {
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
            duration: None,
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
