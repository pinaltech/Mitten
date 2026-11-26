//! The Pinal Photos API's records, shared by the desktop source (which signs
//! in with Pinal) and the browser one (which uses the page's session).
use super::*;
use serde_json::Value;

/// A Pinal Photos ID: letters, digits, `-` and `_`, 6–80 of them.
pub fn valid(id: &str) -> Result<&str> {
    ensure!(
        (6..=80).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "Invalid Pinal Photos ID"
    );
    Ok(id)
}

/// One page of `GET /api/photos`.
pub fn parse_page(bytes: &[u8]) -> Result<PhotoPage> {
    let value: Value = serde_json::from_slice(bytes).context("Pinal Photos sent an unreadable library")?;
    let items = value["photos"].as_array().context("Pinal Photos sent no photos")?;
    let mut photos = Vec::with_capacity(items.len());
    for p in items {
        let Some(id) = p["id"].as_str().and_then(|id| valid(id).ok()) else {
            continue;
        };
        let kind = p["type"].as_str().unwrap_or_default();
        let taken = p["taken"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_default();
        let bytes = p["size"].as_u64().unwrap_or(0);
        photos.push(Arc::new(Photo {
            fingerprint: digest(&format!("{id}:{bytes}:{taken}")),
            duration: kind.starts_with("video/").then_some(0.),
            id: id.to_owned(),
            path: PathBuf::new(),
            name: p["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("Photo").into(),
            modified: taken,
            bytes,
            width: p["width"].as_u64().unwrap_or(0) as u32,
            height: p["height"].as_u64().unwrap_or(0) as u32,
            favorite: false,
            deleted: false,
            caption: String::new(),
            placeholder: None,
        }));
    }
    Ok(PhotoPage {
        library: Library { photos, albums: Default::default(), synced: Default::default() },
        next: value["cursor"].as_str().filter(|s| !s.is_empty()).map(str::to_owned),
        total: None,
    })
}

