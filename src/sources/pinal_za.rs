//! Pinal Photos' zero-access encryption, the Rust side of Pinal's `za.js`
//! (Pinal Technologies/zero-access/za.js): the same formats, byte for byte.
//!
//! Mitten is an app with a key grant: at sign-in it sends Pinal an X25519
//! public key (`zk_pub`), and the person's own browser, which holds their
//! root key, seals the Photos service keys to it. Pinal's servers relay that
//! grant and store only ciphertext, so everything is opened here:
//!
//! ```text
//! service key (from the grant) ─opens→ folder key (record from Pinal Photos)
//!     ─opens→ content key (per photo, bound to its ID) ─opens→ details, file
//! ```
//!
//! Formats: HPKE is RFC 9180 base mode with DHKEM(X25519, HKDF-SHA256),
//! HKDF-SHA256 and AES-256-GCM. Files are "PZA1" | 01 | prefix(7) | chunk
//! size u32, then AES-256-GCM chunks with nonce prefix | index u32 | last.
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use anyhow::{Context, Result, bail, ensure};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use hpke::{Deserializable, Kem as _, OpModeR, Serializable, aead::AesGcm256, kdf::HkdfSha256, kem::X25519HkdfSha256};
use serde_json::Value;
use sha2::{Digest, Sha256};

type Kem = X25519HkdfSha256;
type PrivateKey = <Kem as hpke::Kem>::PrivateKey;
type EncappedKey = <Kem as hpke::Kem>::EncappedKey;

const HEADER: usize = 16;
const TAG: usize = 16;

// ── Encoding ─────────────────────────────────────────────────────────────────

pub fn b64(bytes: &[u8]) -> String {
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

pub fn unb64(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => bail!("Not base64url"),
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

fn array32(bytes: &[u8]) -> Result<[u8; 32]> {
    bytes.try_into().ok().context("Expected 32 bytes")
}

/// JSON with sorted keys and no spaces: the bytes `za.js` signs and MACs.
pub fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canonical(&map[k])))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => format!("[{}]", items.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

// ── Primitives ───────────────────────────────────────────────────────────────

pub fn hkdf(ikm: &[u8], info: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(None, ikm).expand(info.as_bytes(), &mut out).expect("32 bytes");
    out
}

fn gcm_open(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    Aes256Gcm::new(key.into())
        .decrypt(nonce.into(), Payload { msg: ciphertext, aad })
        .ok()
        .context("This data could not be unlocked with your keys")
}

/// `Wrap` from za.js: 01 | nonce(12) | AES-256-GCM.
pub fn unwrap(key: &[u8; 32], wrapped: &[u8], aad: &str) -> Result<Vec<u8>> {
    ensure!(wrapped.len() > 13 + TAG && wrapped[0] == 1, "Unknown wrapping");
    gcm_open(key, &wrapped[1..13], &wrapped[13..], aad.as_bytes())
}

/// HPKE single-shot open (base mode).
pub fn open(private: &[u8; 32], sealed: &[u8], info: &str, aad: &str) -> Result<Vec<u8>> {
    ensure!(sealed.len() >= 32 + TAG, "Sealed data too short");
    let sk = PrivateKey::from_bytes(private).ok().context("Bad private key")?;
    let enc = EncappedKey::from_bytes(&sealed[..32]).ok().context("Bad sealed key")?;
    hpke::single_shot_open::<AesGcm256, HkdfSha256, Kem>(&OpModeR::Base, &sk, &enc, info.as_bytes(), &sealed[32..], aad.as_bytes())
        .ok()
        .context("This data could not be unlocked with your keys")
}

/// A fresh X25519 keypair: (private, public).
pub fn keypair() -> Result<([u8; 32], [u8; 32])> {
    let mut ikm = [0u8; 32];
    getrandom::fill(&mut ikm).ok().context("No randomness")?;
    let (sk, pk) = Kem::derive_keypair(&ikm);
    Ok((array32(&sk.to_bytes())?, array32(&pk.to_bytes())?))
}

pub fn public_of(private: &[u8; 32]) -> Result<[u8; 32]> {
    let sk = PrivateKey::from_bytes(private).ok().context("Bad private key")?;
    array32(&<Kem as hpke::Kem>::sk_to_pk(&sk).to_bytes())
}

fn verify(identity: &[u8; 32], record: &Value) -> bool {
    let Some(sig) = record["sig"].as_str().and_then(|s| unb64(s).ok()) else { return false };
    let mut body = record.clone();
    body.as_object_mut().map(|m| m.remove("sig"));
    let (Ok(key), Ok(sig)) = (ed25519_dalek::VerifyingKey::from_bytes(identity), ed25519_dalek::Signature::from_slice(&sig)) else {
        return false;
    };
    key.verify_strict(canonical(&body).as_bytes(), &sig).is_ok()
}

// ── Grants (spec §7.5) ───────────────────────────────────────────────────────

/// The ID Mitten looks its grant up by (derived from its public key).
pub fn grant_id(public: &[u8; 32]) -> String {
    let mut h = Sha256::new();
    h.update(b"pinal-za/v1 grant-id:");
    h.update(public);
    b64(&h.finalize())
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ServiceKey {
    pub version: u64,
    pub public: String,
    pub private: String,
}

/// The Photos keys Mitten was granted, as saved in the system keyring.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Keys {
    pub user: String,
    pub identity: String,
    pub keys: Vec<ServiceKey>,
}

impl Keys {
    fn key(&self, version: u64) -> Option<([u8; 32], [u8; 32])> {
        let k = self.keys.iter().find(|k| k.version == version)?;
        Some((array32(&unb64(&k.private).ok()?).ok()?, array32(&unb64(&k.public).ok()?).ok()?))
    }
    pub fn newest(&self) -> u64 {
        self.keys.iter().map(|k| k.version).max().unwrap_or(0)
    }
}

/// Opens a grant sealed to `device` and checks every signature in it.
pub fn open_grant(device: &[u8; 32], sealed: &[u8], client_id: &str) -> Result<Keys> {
    let plain = open(device, sealed, "pinal-za/v1 grant", &format!("grant:{client_id}"))?;
    let payload: Value = serde_json::from_slice(&plain).context("Unreadable grant")?;
    ensure!(payload["v"] == 1, "Unknown grant version");
    let user = payload["user"].as_str().context("Grant without an account")?.to_owned();
    let identity_b64 = payload["identity"].as_str().context("Grant without an identity key")?.to_owned();
    let identity = array32(&unb64(&identity_b64)?)?;
    ensure!(verify(&identity, &payload), "This grant isn’t signed by your Pinal account");
    let photos = &payload["services"]["photos"];
    let mut keys = Vec::new();
    for k in photos["keys"].as_array().context("No Photos keys in this grant")? {
        let (version, public, private) = (k["version"].as_u64().unwrap_or(0), k["pub"].as_str().unwrap_or(""), k["priv"].as_str().unwrap_or(""));
        let r = &k["record"];
        ensure!(
            r["user"] == user.as_str() && r["service"] == "photos" && r["version"] == version && r["publicKey"] == public && verify(&identity, r),
            "A Photos key in this grant isn’t signed by your Pinal account"
        );
        ensure!(public_of(&array32(&unb64(private)?)?)? == array32(&unb64(public)?)?, "A Photos key doesn’t match its public half");
        keys.push(ServiceKey { version, public: public.into(), private: private.into() });
    }
    ensure!(!keys.is_empty(), "No Photos keys in this grant");
    Ok(Keys { user, identity: identity_b64, keys })
}

// ── Folders and photos ───────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Folder {
    pub version: u64,
    pub public: [u8; 32],
    private: [u8; 32],
}

/// Opens a folder record after checking its MAC (made with a key derived
/// from the service key, so the server can't make one up).
pub fn open_folder(keys: &Keys, record: &Value) -> Result<Folder> {
    let version = record["version"].as_u64().context("Folder without a version")?;
    let (private, public) = keys.key(version).context("No key for this folder")?;
    ensure!(record["folder"] == "library", "Unknown folder");
    let mac = unb64(record["mac"].as_str().context("Folder without a MAC")?)?;
    let mut body = record.clone();
    body.as_object_mut().map(|m| m.remove("mac"));
    let mut check = <Hmac<Sha256> as Mac>::new_from_slice(&hkdf(&private, "pinal-za/v1 folder-mac")).expect("any key size");
    check.update(canonical(&body).as_bytes());
    check.verify_slice(&mac).ok().context("A folder key isn’t one of yours")?;
    let _ = public;
    let folder_private = open(
        &private,
        &unb64(record["sealed"].as_str().unwrap_or(""))?,
        "pinal-za/v1 folder",
        &format!("folder:photos:library:{version}"),
    )?;
    Ok(Folder { version, public: array32(&unb64(record["pub"].as_str().unwrap_or(""))?)?, private: array32(&folder_private)? })
}

/// A photo's content key, bound to its ID.
pub fn open_content_key(folder: &Folder, sealed: &[u8], id: &str) -> Result<[u8; 32]> {
    array32(&open(&folder.private, sealed, "pinal-za/v1 content-key", &format!("item:{id}"))?)
}

/// A photo's details: name, type, date taken, size, width and height.
pub fn open_meta(content_key: &[u8; 32], id: &str, sealed: &[u8]) -> Result<Value> {
    let plain = unwrap(&hkdf(content_key, "pinal-za/v1 meta"), sealed, &format!("meta:{id}"))?;
    serde_json::from_slice(&plain).context("Unreadable photo details")
}

/// Decrypts a whole file part (`original` or `thumb`).
pub fn decrypt_file(content_key: &[u8; 32], part: &str, data: &[u8]) -> Result<Vec<u8>> {
    ensure!(data.len() >= HEADER && &data[..4] == b"PZA1" && data[4] == 1, "Not an encrypted Pinal file");
    let header = &data[..HEADER];
    let prefix = &data[5..12];
    let chunk = u32::from_be_bytes(data[12..16].try_into()?) as usize;
    ensure!((1024..=16 << 20).contains(&chunk), "Bad chunk size");
    let key = hkdf(content_key, &format!("pinal-za/v1 file:{part}"));
    let body = &data[HEADER..];
    let count = body.len().div_ceil(chunk + TAG).max(1);
    let mut out = Vec::with_capacity(body.len());
    for (i, sealed) in body.chunks(chunk + TAG).enumerate() {
        let mut nonce = [0u8; 12];
        nonce[..7].copy_from_slice(prefix);
        nonce[7..11].copy_from_slice(&(i as u32).to_be_bytes());
        nonce[11] = (i + 1 == count) as u8;
        out.extend(gcm_open(&key, &nonce, sealed, header)?);
    }
    if body.is_empty() {
        bail!("Truncated file");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made by za.js (tests/fixtures/za-vectors.json, see its generator):
    /// Rust must open exactly what the browser seals.
    #[test]
    fn opens_what_za_js_seals() {
        let v: Value = serde_json::from_str(include_str!("../../tests/fixtures/za-vectors.json")).unwrap();
        let device = array32(&unb64(v["device"]["priv"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(b64(&public_of(&device).unwrap()), v["device"]["pub"].as_str().unwrap());
        let keys = open_grant(&device, &unb64(v["grant"].as_str().unwrap()).unwrap(), "test-client").unwrap();
        assert_eq!(keys.user, "user-1");
        assert_eq!(keys.newest(), 2);
        let folder = open_folder(&keys, &v["folder"]).unwrap();
        let item = &v["item"];
        let id = item["id"].as_str().unwrap();
        let ck = open_content_key(&folder, &unb64(item["ek"].as_str().unwrap()).unwrap(), id).unwrap();
        let meta = open_meta(&ck, id, &unb64(item["meta"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(meta["name"], "beach.jpg");
        let plain = decrypt_file(&ck, "original", &unb64(item["file"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(b64(&plain), item["plain"].as_str().unwrap());
        // Bound to its ID and part.
        assert!(open_content_key(&folder, &unb64(item["ek"].as_str().unwrap()).unwrap(), "other").is_err());
        assert!(decrypt_file(&ck, "thumb", &unb64(item["file"].as_str().unwrap()).unwrap()).is_err());
        // A tampered grant (a server-made key) is refused.
        let forged = open_grant(&device, &unb64(v["forged_grant"].as_str().unwrap()).unwrap(), "test-client");
        assert!(forged.is_err());
        // So is a folder record whose MAC doesn't check out.
        let mut bad = v["folder"].clone();
        bad["pub"] = Value::String(b64(&[7u8; 32]));
        assert!(open_folder(&keys, &bad).is_err());
    }

    #[test]
    fn base64url_round_trip_and_canonical_json() {
        for n in 0..40 {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(unb64(&b64(&bytes)).unwrap(), bytes);
        }
        let v: Value = serde_json::from_str(r#"{"b":1,"a":{"d":[1,"x"],"c":true}}"#).unwrap();
        assert_eq!(canonical(&v), r#"{"a":{"c":true,"d":[1,"x"]},"b":1}"#);
    }
}
