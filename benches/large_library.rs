// Compile the production loading module directly so this benchmark needs no
// window server, Metal compiler, or platform-specific UI dependencies.
#[allow(dead_code)]
#[path = "../src/loading.rs"]
mod loading;
use loading::{BoundedCache, THUMBNAIL_CACHE, visible_range};
use std::{hint::black_box, time::Instant};
fn main() {
    for count in [100_000, 1_000_000, 10_000_000] {
        let start = Instant::now();
        let mut cache = BoundedCache::new(THUMBNAIL_CACHE);
        let mut tiles = 0;
        for row in (0..count / 5).step_by(7) {
            for i in visible_range(row..row + 7, 5, count) {
                cache.insert(i.to_string(), black_box(i));
                tiles += 1;
            }
        }
        println!(
            "{count} photos: {tiles} visible-tile visits in {:?}; retained {} paths; 0 preview/original requests",
            start.elapsed(),
            cache.len()
        );
        assert!(cache.len() <= THUMBNAIL_CACHE);
    }
}
