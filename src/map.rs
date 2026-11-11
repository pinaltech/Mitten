//! Small static maps for the info panel, composed from raster tiles of an
//! open tile server. Only the few tiles around a photo's location are
//! fetched (nothing else leaves the device), and every tile is cached.
/// Open-source, key-free raster tile servers Settings offers. Users can
/// enter any other `{z}/{x}/{y}` template, including a self-hosted server.
pub const PRESETS: [(&str, &str); 3] = [
    (
        "OpenStreetMap",
        "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
    ),
    (
        "Humanitarian",
        "https://a.tile.openstreetmap.fr/hot/{z}/{x}/{y}.png",
    ),
    (
        "OpenTopoMap",
        "https://a.tile.opentopomap.org/{z}/{x}/{y}.png",
    ),
];
pub const TILE: u32 = 256;
/// Whether `template` is an HTTP(S) URL with {z}, {x} and {y} placeholders.
pub fn valid_template(template: &str) -> bool {
    let t = template.trim();
    (t.starts_with("https://") || t.starts_with("http://"))
        && ["{z}", "{x}", "{y}"].iter().all(|p| t.contains(p))
        && url::Url::parse(&t.replace(['{', '}'], "")).is_ok()
}
/// Web-Mercator world pixel of a coordinate at `zoom`.
pub fn world_pixel(lat: f64, lon: f64, zoom: u8) -> (f64, f64) {
    let size = f64::from(TILE) * f64::from(1u32 << zoom);
    let lat = lat.clamp(-85.0511, 85.0511).to_radians();
    let x = (lon + 180.) / 360. * size;
    let y = (1. - (lat.tan() + 1. / lat.cos()).ln() / std::f64::consts::PI) / 2. * size;
    (x, y)
}
/// The tiles (x, y) covering a `width`×`height` view centred on a pixel.
pub fn tiles_for(center: (f64, f64), width: u32, height: u32, zoom: u8) -> Vec<(i64, i64)> {
    let count = 1i64 << zoom;
    let left = center.0 - f64::from(width) / 2.;
    let top = center.1 - f64::from(height) / 2.;
    let tile = f64::from(TILE);
    let (x0, x1) = (
        (left / tile).floor() as i64,
        ((left + f64::from(width) - 1.) / tile).floor() as i64,
    );
    let (y0, y1) = (
        (top / tile).floor() as i64,
        ((top + f64::from(height) - 1.) / tile).floor() as i64,
    );
    let mut tiles = vec![];
    for y in y0.max(0)..=y1.min(count - 1) {
        for x in x0..=x1 {
            tiles.push((x, y));
        }
    }
    tiles
}

#[cfg(feature = "remote")]
pub use fetch::static_map;
#[cfg(feature = "remote")]
mod fetch {
    use super::*;
    use anyhow::{Context, Result, ensure};
    use std::{
        fs,
        io::Read,
        path::{Path, PathBuf},
        time::Duration,
    };

    const TILE_LIMIT: u64 = 2 * 1024 * 1024;
    fn tile(
        client: &reqwest::blocking::Client,
        template: &str,
        cache: &Path,
        (x, y, z): (i64, i64, u8),
    ) -> Result<image::RgbaImage> {
        let path = cache.join(format!("{z}-{x}-{y}.png"));
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                let url = template
                    .trim()
                    .replace("{z}", &z.to_string())
                    .replace("{x}", &x.to_string())
                    .replace("{y}", &y.to_string())
                    .replace("{r}", "");
                let response = client.get(url).send()?;
                ensure!(
                    response.status().is_success(),
                    "Map server returned {}",
                    response.status()
                );
                let mut bytes = vec![];
                response.take(TILE_LIMIT + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() as u64 <= TILE_LIMIT, "Map tile is too large");
                let temp = tempfile::NamedTempFile::new_in(cache)?;
                fs::write(temp.path(), &bytes)?;
                let _ = temp.persist(&path);
                bytes
            }
        };
        Ok(image::load_from_memory(&bytes)
            .context("Map server returned an invalid tile")?
            .to_rgba8())
    }
    /// A `width`×`height` PNG centred on (`lat`, `lon`), written under
    /// `cache` (tiles are kept there too) and reused while it exists.
    pub fn static_map(
        template: &str,
        (lat, lon): (f64, f64),
        zoom: u8,
        (width, height): (u32, u32),
        cache: &Path,
    ) -> Result<PathBuf> {
        ensure!(
            valid_template(template),
            "Enter a tile URL with {{z}}, {{x}} and {{y}}"
        );
        let key = crate::sources::digest(&format!(
            "{}:{lat:.5}:{lon:.5}:{zoom}:{width}x{height}",
            template.trim()
        ));
        let target = cache.join(format!("map-{key}.png"));
        if target.is_file() {
            return Ok(target);
        }
        let tiles = cache
            .join("tiles")
            .join(&crate::sources::digest(template.trim())[..16]);
        fs::create_dir_all(&tiles)?;
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            // Tile servers' usage policies ask clients to identify themselves.
            .user_agent(concat!("Mitten/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let center = world_pixel(lat, lon, zoom);
        let left = center.0 - f64::from(width) / 2.;
        let top = center.1 - f64::from(height) / 2.;
        let count = 1i64 << zoom;
        let mut canvas =
            image::RgbaImage::from_pixel(width, height, image::Rgba([224, 224, 224, 255]));
        for (x, y) in tiles_for(center, width, height, zoom) {
            let pixels = tile(&client, template, &tiles, (x.rem_euclid(count), y, zoom))?;
            let ox = (x as f64 * f64::from(TILE) - left).round() as i64;
            let oy = (y as f64 * f64::from(TILE) - top).round() as i64;
            image::imageops::overlay(&mut canvas, &pixels, ox, oy);
        }
        let temp = tempfile::NamedTempFile::new_in(cache)?;
        canvas.save_with_format(temp.path(), image::ImageFormat::Png)?;
        temp.persist(&target).map_err(|e| e.error)?;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mercator_tiles_cover_the_view() {
        let (x, y) = world_pixel(0., 0., 1);
        assert!((x - 256.).abs() < 1e-6 && (y - 256.).abs() < 1e-6);
        let center = world_pixel(51.5, -0.12, 14);
        let tiles = tiles_for(center, 440, 300, 14);
        assert!((4..=6).contains(&tiles.len()), "{tiles:?}");
        assert!(valid_template(PRESETS[0].1));
        assert!(!valid_template("https://example.com/tiles.png"));
        assert!(!valid_template("file:///{z}/{x}/{y}"));
    }
}
