//! Pinal Photos: the photo library of a Pinal account, stored by Pinal
//! (photos.pinal.tech, in Cloudflare R2) and shown here like any other source.
//!
//! Signing in follows Picacho's Pinal Mail sign-in: Mitten is an ordinary
//! public developer app at Pinal (`MITTEN_PINAL_CLIENT_ID`), signed in to in
//! the system browser with PKCE. Answers come back on a loopback address
//! (RFC 8252 §7.3), or, for the bundled macOS app, through Pinal's
//! `/native/callback` page as a `mitten://` URL (§7.1). Pinal
//! issues access tokens addressed to Pinal Photos (JWTs it verifies) and a
//! refresh token, kept in the system keyring.
//!
//! Pinal Photos is zero-access storage: Pinal stores only ciphertext. At
//! sign-in Mitten sends a public key (`zk_pub`) and the person's browser
//! seals the Photos keys to it (see `pinal_za`); those keys are kept in the
//! keyring too, and every photo is decrypted here.
use super::*;
use super::pinal_za::{self, Folder, Keys};
use std::collections::HashMap;
pub use super::pinal_api::parse_page;
use reqwest::blocking::Client;
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use url::Url;

pub const DRIVER: &str = "pinal-photos";
const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 64 * 1024 * 1024;
/// What Mitten asks Pinal for: who you are, seeing and adding photos, albums,
/// and staying signed in.
const SCOPES: &str = "openid profile email offline_access photos.read photos.upload photos.albums";

/// `MITTEN_PINAL_*` override these for development, at run or build time.
fn setting(name: &str, built: Option<&'static str>, default: &str) -> String {
    std::env::var(name)
        .ok()
        .or(built.map(str::to_owned))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_owned())
        .trim_end_matches('/')
        .to_owned()
}
pub fn accounts() -> String {
    setting("MITTEN_PINAL_ACCOUNTS", option_env!("MITTEN_PINAL_ACCOUNTS"), "https://accounts.pinal.tech")
}
pub fn server() -> String {
    setting("MITTEN_PINAL_PHOTOS", option_env!("MITTEN_PINAL_PHOTOS"), "https://photos.pinal.tech")
}
/// Mitten's developer app at Pinal (public; redirect URIs
/// `http://127.0.0.1/callback` and `https://accounts.pinal.tech/native/callback`).
fn client_id() -> String {
    setting("MITTEN_PINAL_CLIENT_ID", option_env!("MITTEN_PINAL_CLIENT_ID"), "pnl_xlabeTOTOtwogPMJ")
}

/// The bundled macOS app, which has the `mitten://` scheme to be handed back to.
fn bundled() -> bool {
    cfg!(target_os = "macos")
        && std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
}

/// `mitten://` URLs the system opened Mitten with, claimed by the sign-in
/// whose state they carry.
static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
pub fn opened(urls: Vec<String>) {
    OPENED.lock().unwrap().extend(urls.into_iter().filter(|u| u.starts_with("mitten:")));
}

fn http() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

#[derive(Clone)]
struct Endpoints {
    authorize: Url,
    token: Url,
    userinfo: Url,
}
fn endpoints(http: &Client) -> Result<Endpoints> {
    static FOUND: Mutex<Option<(String, Endpoints)>> = Mutex::new(None);
    let origin = accounts();
    if let Some((o, found)) = FOUND.lock().unwrap().clone()
        && o == origin
    {
        return Ok(found);
    }
    let config: Value = http
        .get(format!("{origin}/.well-known/openid-configuration"))
        .send()
        .and_then(|r| r.error_for_status())
        .context("Could not reach Pinal Accounts. Check your connection.")?
        .json()
        .context("Pinal Accounts is not answering sign-in requests right now.")?;
    let endpoint = |key: &str| -> Result<Url> {
        let url = Url::parse(config[key].as_str().context("Pinal Accounts is misconfigured")?)?;
        // Codes and tokens go nowhere but Pinal Accounts.
        ensure!(
            url.origin().ascii_serialization() == origin,
            "Pinal Accounts returned an unexpected sign-in address."
        );
        Ok(url)
    };
    let found = Endpoints {
        authorize: endpoint("authorization_endpoint")?,
        token: endpoint("token_endpoint")?,
        userinfo: endpoint("userinfo_endpoint")?,
    };
    *FOUND.lock().unwrap() = Some((origin, found.clone()));
    Ok(found)
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}
fn random_token() -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    base64url(&bytes)
}

/// Who signed in: shown under the library's name.
#[derive(Clone, Debug, Default)]
pub struct Who {
    pub email: String,
    pub name: String,
}

/// A Pinal sign-in in progress: open [`SignIn::url`] in the browser, then
/// [`SignIn::finish`] waits (off the UI thread) for its answer.
pub struct SignIn {
    pub url: Url,
    /// None when Pinal's page hands the answer back as a `mitten://` URL.
    listener: Option<TcpListener>,
    redirect: String,
    verifier: String,
    state: String,
    endpoints: Endpoints,
    /// The X25519 key Pinal seals the Photos keys to (spec §7.5).
    device: [u8; 32],
    device_public: [u8; 32],
}
impl SignIn {
    pub fn start() -> Result<Self> {
        let endpoints = endpoints(&http()?)?;
        let (listener, redirect) = if bundled() {
            OPENED.lock().unwrap().clear();
            (None, format!("{}/native/callback", accounts()))
        } else {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            listener.set_nonblocking(true)?;
            let port = listener.local_addr()?.port();
            (Some(listener), format!("http://127.0.0.1:{port}/callback"))
        };
        let verifier = random_token();
        // `mitten.` tells Pinal's /native/callback page which app to open.
        let state = format!("mitten.{}", random_token());
        let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
        let (device, device_public) = pinal_za::keypair()?;
        let mut url = endpoints.authorize.clone();
        url.query_pairs_mut().extend_pairs([
            ("client_id", client_id().as_str()),
            ("redirect_uri", &redirect),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("state", &state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("resource", &server()),
            // Pinal's consent screen seals the Photos keys to this.
            ("zk_pub", &pinal_za::b64(&device_public)),
        ]);
        Ok(Self { url, listener, redirect, verifier, state, endpoints, device, device_public })
    }

    /// Short code Pinal's consent screen shows next to the keys it hands over.
    pub fn confirmation(&self) -> String {
        let d = Sha256::digest(self.device_public);
        let n = ((d[0] as u32) << 16 | (d[1] as u32) << 8 | d[2] as u32) % 1_000_000;
        format!("{:03} {:03}", n / 1000, n % 1000)
    }

    /// Waits up to five minutes for the browser, then redeems the code.
    pub fn finish(self, cancel: &AtomicBool) -> Result<(String, Who, Keys)> {
        let code = self.wait(cancel)?;
        let response = http()?
            .post(self.endpoints.token.clone())
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id().as_str()),
                ("code", &code),
                ("redirect_uri", &self.redirect),
                ("code_verifier", &self.verifier),
                ("resource", &server()),
            ])
            .send()
            .context("Could not reach Pinal Accounts. Check your connection.")?;
        let tokens = token_response(response)?;
        let refresh = tokens["refresh_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("Pinal did not let Mitten stay signed in. Please try again.")?
            .to_owned();
        let access = tokens["access_token"].as_str().unwrap_or_default();
        let info: Value = http()?
            .get(self.endpoints.userinfo.clone())
            .bearer_auth(access)
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
            .and_then(|r| r.json().ok())
            .unwrap_or(Value::Null);
        let text = |key: &str| info[key].as_str().unwrap_or_default().to_owned();
        // The Photos keys, sealed to this sign-in's key by the person's browser.
        let grant: Value = http()?
            .get(format!("{}/api/keys/grants/{}", accounts(), pinal_za::grant_id(&self.device_public)))
            .send()
            .and_then(|r| r.error_for_status())
            .context("Pinal didn’t hand over your Photos keys. Unlock your data in your Pinal account and try again.")?
            .json()?;
        let keys = pinal_za::open_grant(&self.device, &pinal_za::unb64(grant["payload"].as_str().unwrap_or(""))?, &client_id())?;
        ensure!(info["sub"].as_str().is_none_or(|sub| sub == keys.user), "Pinal handed over keys for another account.");
        Ok((refresh, Who { email: text("email"), name: text("name") }, keys))
    }

    fn wait(&self, cancel: &AtomicBool) -> Result<String> {
        let start = Instant::now();
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Sign-in cancelled.");
            ensure!(start.elapsed() < Duration::from_secs(300), "Sign-in timed out. Please try again.");
            {
                // A mitten:// hand-back answering this request (it answers once).
                let mut opened = OPENED.lock().unwrap();
                let found = opened.iter().position(|u| Url::parse(u).ok().and_then(|u| self.answer(&u)).is_some());
                if let Some(i) = found {
                    let url = Url::parse(&opened.remove(i))?;
                    return self.answer(&url).unwrap();
                }
            }
            let Some(listener) = &self.listener else {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            };
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false)?;
                    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                    let mut request = [0u8; 8192];
                    let n = stream.read(&mut request).unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..n]);
                    let target = request
                        .lines()
                        .next()
                        .and_then(|s| s.split_whitespace().nth(1))
                        .unwrap_or("/");
                    let callback = Url::parse(&format!("http://127.0.0.1{target}"))?;
                    let answer = (callback.path() == "/callback").then(|| self.answer(&callback)).flatten();
                    let Some(answer) = answer else {
                        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    };
                    // Like the bundled app's hand-back, the browser ends on Pinal's
                    // own page (which carries only the outcome), not this one.
                    let done = match &answer {
                        Ok(_) => "ok",
                        Err(e) if e.to_string() == "Sign-in was cancelled." => "cancelled",
                        Err(_) => "failed",
                    };
                    let location = format!("{}/native/callback?done={done}&app=mitten", accounts());
                    let _ = stream.write_all(
                        format!(
                            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    );
                    return answer;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// The code or error a callback carries, if it answers this request.
    fn answer(&self, callback: &Url) -> Option<Result<String>> {
        let params: std::collections::HashMap<_, _> = callback.query_pairs().into_owned().collect();
        if params.get("state") != Some(&self.state) {
            return None;
        }
        Some(match (params.get("code"), params.get("error")) {
            (Some(code), None) => Ok(code.clone()),
            (_, Some(e)) if e == "access_denied" => Err(anyhow::anyhow!("Sign-in was cancelled.")),
            (_, error) => Err(anyhow::anyhow!(
                "Pinal declined the sign-in: {}",
                error.map(String::as_str).unwrap_or("no code")
            )),
        })
    }
}

fn token_response(response: reqwest::blocking::Response) -> Result<Value> {
    let status = response.status();
    let value: Value = response.json().unwrap_or(Value::Null);
    if !status.is_success() {
        // Only the error code: the body may carry codes or tokens.
        let code = value["error"].as_str().unwrap_or("unknown");
        if code == "invalid_grant" {
            bail!("Your Pinal sign-in has expired. Remove Pinal Photos and add it again.");
        }
        bail!("Pinal Accounts refused the sign-in ({code}).");
    }
    ensure!(value["access_token"].is_string(), "Pinal Accounts sent no access token.");
    Ok(value)
}

/// The keyring entry for a Pinal Photos library's keys (beside its refresh token).
fn keys_credential(source: &Source) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new("tech.pinal.mitten", &format!("{}:za", digest(&source.path.to_string_lossy())))?)
}

/// Saves a signed-in Pinal Photos library: its connection (no secrets), and
/// the refresh token and Photos keys in the system keyring.
pub fn save(refresh: &str, who: &Who, keys: &Keys, data: &Path) -> Result<Source> {
    let config = connection(who);
    let source = config.source(data)?;
    fs::create_dir_all(&source.path)?;
    remote::credential(&source)?
        .set_password(refresh)
        .context("Cannot save your Pinal sign-in in the system keyring")?;
    keys_credential(&source)?
        .set_password(&serde_json::to_string(keys)?)
        .context("Cannot save your Pinal Photos keys in the system keyring")?;
    let temp = tempfile::NamedTempFile::new_in(&source.path)?;
    serde_json::to_writer(temp.as_file(), &config)?;
    temp.persist(source.path.join("connection.json")).map_err(|e| e.error)?;
    Ok(source)
}
pub fn connection(who: &Who) -> Connection {
    Connection {
        provider: DRIVER.into(),
        server: server(),
        username: if who.email.is_empty() { who.name.clone() } else { who.email.clone() },
        folder: String::new(),
    }
}

struct Token {
    access: String,
    expires: Instant,
}

/// An encrypted photo, opened: its content key and details.
#[derive(Clone)]
struct Opened {
    key: [u8; 32],
    meta: Value,
}

pub struct PinalPhotos {
    base: Url,
    http: Client,
    source: Option<Source>,
    refresh: Mutex<String>,
    token: Mutex<Option<Token>>,
    keys: Option<Keys>,
    folders: Mutex<HashMap<u64, Folder>>,
    opened: Mutex<HashMap<String, Opened>>,
}

const REKEY: &str = "Pinal Photos’ keys changed (a device was removed or the storage mode changed). Remove Pinal Photos and add it again.";

impl PinalPhotos {
    /// `source` is where a rotated refresh token is saved (None while
    /// connecting); `keys` are the Photos keys (else read from the keyring).
    pub fn new(config: &Connection, refresh: &str, source: Option<Source>, keys: Option<Keys>) -> Result<Self> {
        let keys = keys.or_else(|| {
            let saved = keys_credential(source.as_ref()?).ok()?.get_password().ok()?;
            serde_json::from_str(&saved).ok()
        });
        Ok(Self {
            base: config.validate()?,
            http: http()?,
            source,
            refresh: Mutex::new(refresh.to_owned()),
            token: Mutex::new(None),
            keys,
            folders: Mutex::new(HashMap::new()),
            opened: Mutex::new(HashMap::new()),
        })
    }

    fn keys(&self) -> Result<&Keys> {
        self.keys
            .as_ref()
            .context("Pinal Photos now encrypts your library. Remove Pinal Photos and add it again to get your keys.")
    }

    /// The folder key a photo was sealed to (records come from Pinal Photos, checked here).
    fn folder(&self, version: u64) -> Result<Folder> {
        if let Some(f) = self.folders.lock().unwrap().get(&version) {
            return Ok(f.clone());
        }
        let keys = self.keys()?;
        ensure!(version <= keys.newest(), REKEY);
        let records: Value = serde_json::from_slice(&read(self.get("api/keys/folders")?, METADATA_LIMIT)?)?;
        let mut folders = self.folders.lock().unwrap();
        for record in records["folders"].as_array().into_iter().flatten() {
            if let Ok(folder) = pinal_za::open_folder(keys, record) {
                folders.insert(folder.version, folder);
            }
        }
        folders.get(&version).cloned().context(REKEY)
    }

    /// Opens a listing entry; an encrypted one becomes the plain fields Mitten reads.
    fn open_entry(&self, item: &Value) -> Result<Value> {
        if item["za"] != 1 {
            return Ok(item.clone());
        }
        let id = item["id"].as_str().context("Photo without an ID")?;
        let folder = self.folder(item["fv"].as_u64().unwrap_or(0))?;
        let key = pinal_za::open_content_key(&folder, &pinal_za::unb64(item["ek"].as_str().unwrap_or(""))?, id)?;
        let meta = pinal_za::open_meta(&key, id, &pinal_za::unb64(item["meta"].as_str().unwrap_or(""))?)?;
        self.opened.lock().unwrap().insert(id.to_owned(), Opened { key, meta: meta.clone() });
        Ok(serde_json::json!({
            "id": id, "name": meta["name"], "taken": meta["taken"], "width": meta["width"],
            "height": meta["height"], "size": meta["size"], "type": meta["type"]
        }))
    }

    /// An encrypted photo's key and details (from the listing, else asked for).
    fn opened(&self, id: &str) -> Result<Option<Opened>> {
        if let Some(o) = self.opened.lock().unwrap().get(id) {
            return Ok(Some(o.clone()));
        }
        let mut item: Value = serde_json::from_slice(&read(self.get(&format!("api/photos/{id}/key"))?, METADATA_LIMIT)?)?;
        if item["za"] != 1 {
            return Ok(None);
        }
        item["id"] = Value::String(id.to_owned());
        self.open_entry(&item)?;
        Ok(self.opened.lock().unwrap().get(id).cloned())
    }

    /// A photo file (`original` or `thumb`), decrypted when it is encrypted.
    fn fetch(&self, id: &str, thumb: bool, limit: u64) -> Result<Vec<u8>> {
        let path = if thumb { format!("api/photos/{id}/thumb") } else { format!("api/photos/{id}") };
        let opened = self.opened(id)?;
        let response = match self.get(&path) {
            // An encrypted photo without a thumbnail: its original stands in.
            Err(_) if thumb && opened.is_some() => return self.fetch(id, false, limit),
            r => r?,
        };
        let bytes = read(response, limit)?;
        match opened {
            Some(o) if bytes.starts_with(b"PZA1") => pinal_za::decrypt_file(&o.key, if thumb { "thumb" } else { "original" }, &bytes),
            _ => Ok(bytes),
        }
    }

    /// A current access token, renewed at Pinal a minute before it expires.
    fn access(&self) -> Result<String> {
        let mut token = self.token.lock().unwrap();
        if let Some(t) = token.as_ref()
            && t.expires > Instant::now() + Duration::from_secs(60)
        {
            return Ok(t.access.clone());
        }
        let endpoints = endpoints(&self.http)?;
        let mut refresh = self.refresh.lock().unwrap();
        let response = self
            .http
            .post(endpoints.token)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id().as_str()),
                ("refresh_token", refresh.as_str()),
                ("resource", &server()),
            ])
            .send()
            .context("Could not reach Pinal Accounts. Check your connection.")?;
        let value = token_response(response)?;
        if let Some(next) = value["refresh_token"].as_str().filter(|s| !s.is_empty() && *s != refresh.as_str()) {
            *refresh = next.to_owned();
            if let Some(source) = &self.source {
                let _ = remote::credential(source).and_then(|c| Ok(c.set_password(next)?));
            }
        }
        let access = value["access_token"].as_str().unwrap_or_default().to_owned();
        let lifetime = value["expires_in"].as_u64().unwrap_or(900);
        *token = Some(Token { access: access.clone(), expires: Instant::now() + Duration::from_secs(lifetime) });
        Ok(access)
    }

    fn get(&self, path: &str) -> Result<reqwest::blocking::Response> {
        let url = self.base.join(path.trim_start_matches('/'))?;
        let response = self.http.get(url).bearer_auth(self.access()?).send()?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            // The token may have been revoked early: renew once.
            *self.token.lock().unwrap() = None;
            let url = self.base.join(path.trim_start_matches('/'))?;
            return checked(self.http.get(url).bearer_auth(self.access()?).send()?);
        }
        checked(response)
    }

    fn valid(id: &str) -> Result<&str> {
        super::pinal_api::valid(id)
    }
}

fn checked(response: reqwest::blocking::Response) -> Result<reqwest::blocking::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        bail!("Pinal Photos refused this sign-in. Remove Pinal Photos and add it again.");
    }
    bail!("Pinal Photos answered {status}")
}
fn read(response: reqwest::blocking::Response, limit: u64) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response.take(limit + 1).read_to_end(&mut body)?;
    ensure!(body.len() as u64 <= limit, "Pinal Photos sent more than expected");
    Ok(body)
}

impl Provider for PinalPhotos {
    fn page(&self, cursor: Option<&str>) -> Result<PhotoPage> {
        let mut path = format!("api/photos?limit={}", crate::loading::PAGE_SIZE);
        if let Some(c) = cursor {
            path.push_str("&cursor=");
            path.extend(url::form_urlencoded::byte_serialize(c.as_bytes()));
        }
        let mut page: Value = serde_json::from_slice(&read(self.get(&path)?, METADATA_LIMIT)?)
            .context("Pinal Photos sent an unreadable library")?;
        if let Some(items) = page["photos"].as_array_mut() {
            for item in items.iter_mut() {
                *item = self.open_entry(item)?;
            }
        }
        parse_page(&serde_json::to_vec(&page)?)
    }
    fn image(&self, photo: &Photo, preview: bool, destination: &Path) -> Result<()> {
        self.image_at(photo, if preview { 2560 } else { 512 }, destination)
    }
    /// Thumbnails are 512 px; larger sizes come from the original.
    fn rendition(&self, edge: u32) -> u32 {
        if edge <= 512 { 512 } else { 2560 }
    }
    fn image_at(&self, photo: &Photo, edge: u32, destination: &Path) -> Result<()> {
        let id = Self::valid(&photo.id)?;
        // Videos are only drawn from their thumbnail (a frame the uploader took).
        let thumb = edge <= 512 || photo.duration.is_some();
        fs::write(destination, self.fetch(id, thumb, IMAGE_LIMIT)?)?;
        Ok(())
    }
    fn video(&self, photo: &Photo) -> Option<VideoSource> {
        photo.duration?;
        let id = Self::valid(&photo.id).ok()?;
        // Encrypted: decrypted here into a private temporary file, since a
        // stream from Pinal is ciphertext.
        if self.opened(id).ok().flatten().is_some() {
            let dir = std::env::temp_dir().join("mitten-pinal-photos");
            fs::create_dir_all(&dir).ok()?;
            let ext = Path::new(&photo.name).extension().and_then(|e| e.to_str()).unwrap_or("mp4");
            let file = dir.join(format!("{id}.{ext}"));
            if !file.exists() {
                fs::write(&file, self.fetch(id, false, 4 * IMAGE_LIMIT).ok()?).ok()?;
            }
            return Some(VideoSource::File(file));
        }
        Some(VideoSource::Remote {
            url: self.base.join(&format!("api/photos/{id}")).ok()?.to_string(),
            headers: vec![("Authorization".into(), format!("Bearer {}", self.access().ok()?))],
        })
    }
    fn details(&self, photo: &Photo) -> Result<PhotoDetails> {
        Ok(PhotoDetails {
            bytes: (photo.bytes > 0).then_some(photo.bytes),
            ..Default::default()
        })
    }
    fn original(&self, id: &str, destination: &Path) -> Result<String> {
        let id = Self::valid(id)?;
        if let Some(o) = self.opened(id)? {
            fs::write(destination, self.fetch(id, false, 4 * IMAGE_LIMIT)?)?;
            let name = o.meta["name"].as_str().unwrap_or_default().to_owned();
            let safe = !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\']) && !name.chars().any(char::is_control);
            return Ok(if safe { name } else { format!("{id}.jpg") });
        }
        let mut response = self.get(&format!("api/photos/{id}"))?;
        // `inline; filename*=UTF-8''name.jpg`
        let name = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split("filename*=UTF-8''").nth(1))
            .map(|n| url::form_urlencoded::parse(format!("n={}", n.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default())
            .filter(|n| {
                !n.is_empty()
                    && n != "."
                    && n != ".."
                    && !n.contains(['/', '\\'])
                    && !n.chars().any(char::is_control)
            })
            .unwrap_or_else(|| format!("{id}.jpg"));
        std::io::copy(&mut response, &mut fs::File::create(destination)?)?;
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_newest_first_with_videos() {
        let page = parse_page(
            br#"{"photos":[
                {"id":"2x231bapr-abc","name":"beach.jpg","taken":"2026-01-01T00:00:00.000Z","width":4032,"height":3024,"size":2400000,"type":"image/jpeg"},
                {"id":"2xpdtvuvz-def","name":"clip.mov","taken":"2024-05-23T23:29:16.000Z","width":null,"height":null,"size":9000000,"type":"video/quicktime"},
                {"id":"../escape","name":"x","taken":"2024-01-01T00:00:00Z","size":1,"type":"image/png"}
            ],"cursor":"next-page"}"#,
        )
        .unwrap();
        assert_eq!(page.library.photos.len(), 2, "invalid IDs are skipped");
        let [beach, clip] = [&page.library.photos[0], &page.library.photos[1]];
        assert_eq!((beach.width, beach.height, beach.duration), (4032, 3024, None));
        assert_eq!(clip.duration, Some(0.));
        assert!(beach.modified > clip.modified);
        assert_eq!(page.next.as_deref(), Some("next-page"));
    }

    #[test]
    fn claims_only_its_own_hand_back() {
        let endpoint = Url::parse("https://accounts.example/x").unwrap();
        let sign_in = SignIn {
            url: endpoint.clone(),
            listener: None,
            redirect: "https://accounts.example/native/callback".into(),
            verifier: random_token(),
            state: "mitten.abc".into(),
            endpoints: Endpoints { authorize: endpoint.clone(), token: endpoint.clone(), userinfo: endpoint },
            device: [1; 32],
            device_public: [2; 32],
        };
        opened(vec![
            "picacho://callback?code=nope&state=mitten.abc".into(),
            "mitten://callback?code=other&state=mitten.zzz".into(),
            "mitten://callback?code=c0de&state=mitten.abc".into(),
        ]);
        assert_eq!(sign_in.wait(&AtomicBool::new(false)).unwrap(), "c0de");
        // Another sign-in's answer stays for it.
        assert_eq!(OPENED.lock().unwrap().len(), 1);
        let denied = Url::parse("mitten://callback?error=access_denied&state=mitten.abc").unwrap();
        assert!(sign_in.answer(&denied).unwrap().is_err());
    }

    #[test]
    fn pkce_encoding() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(base64url(&Sha256::digest(verifier.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(random_token().len(), 43);
    }
}
