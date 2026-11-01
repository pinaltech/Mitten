//! Discrete gallery density levels. Pinches commit a level, then the UI crossfades.
//! Dense, zoomed-out levels use edge-to-edge squares and floating dates.
#[derive(Clone, Copy, Debug)]
pub struct GalleryLayout {
    pub zoom: f32,
    pub columns_a: usize,
    pub columns_b: usize,
    pub block_len: usize,
    pub block_height: f32,
    pub edge: f32,
    pub caption: f32,
    pub gap: f32,
    pub padding: f32,
    pub radius: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub x: f32,
    pub y: f32,
    pub edge: f32,
}
impl GalleryLayout {
    pub const MAX_ZOOM: f32 = 4.;
    pub fn new(width: f32, zoom: f32) -> Self {
        let zoom = if zoom.is_finite() {
            zoom.clamp(0., Self::MAX_ZOOM)
        } else {
            2.
        };
        let zoom = zoom.round();
        let width = width.max(1.);
        let columns: usize = [12, 8, 5, 3, 1][zoom as usize];
        let columns = if width < 600. {
            columns.div_ceil(2)
        } else {
            columns
        };
        let spacing = ((zoom - 1.) / 3.).max(0.);
        let padding = 20. * spacing;
        let gap = 10. * spacing;
        let edge = ((width - 2. * padding - (columns - 1) as f32 * gap) / columns as f32).max(1.);
        Self {
            zoom,
            columns_a: columns,
            columns_b: columns,
            block_len: columns,
            block_height: if gap == 0. {
                edge.floor().max(1.)
            } else {
                (edge + gap).round()
            },
            edge,
            caption: 0.,
            gap,
            padding,
            radius: 9. * spacing,
        }
    }
    pub fn full_bleed(&self) -> bool {
        self.zoom <= 1.
    }
    pub fn cell(&self, index: usize) -> Cell {
        Cell {
            x: self.padding + (index % self.block_len) as f32 * (self.edge + self.gap),
            y: (index / self.block_len) as f32 * self.block_height,
            edge: self.edge,
        }
    }
    pub fn blocks(&self, count: usize) -> usize {
        count.div_ceil(self.block_len)
    }
    pub fn nearest(&self, x: f32, y: f32, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        let block = (y.max(0.) / self.block_height).floor() as usize;
        let first = block.saturating_sub(1) * self.block_len;
        let last = ((block + 2) * self.block_len).min(count);
        (first.min(count - 1)..last.max(first.min(count - 1) + 1))
            .min_by(|&a, &b| {
                let distance = |i| {
                    let cell = self.cell(i);
                    (cell.x + cell.edge / 2. - x).powi(2) + (cell.y + cell.edge / 2. - y).powi(2)
                };
                distance(a).total_cmp(&distance(b))
            })
            .unwrap_or(0)
    }
    pub fn anchored_offset(
        &self,
        next: &Self,
        pointer: (f32, f32),
        offset: f32,
        count: usize,
    ) -> f32 {
        if count == 0 {
            return 0.;
        }
        let index = self.nearest(pointer.0, offset + pointer.1, count);
        let old = self.cell(index);
        let new = next.cell(index);
        let within = ((offset + pointer.1 - old.y) / old.edge).clamp(0., 1.);
        (new.y + within * new.edge - pointer.1).max(0.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn density_levels_snap_and_only_zoomed_out_photos_are_gapless() {
        for width in [312., 640., 844., 1224.] {
            for level in 0..=4 {
                let layout = GalleryLayout::new(width, level as f32);
                assert_eq!(layout.caption, 0.);
                assert_eq!(layout.full_bleed(), level <= 1);
                if level <= 1 {
                    assert_eq!((layout.gap, layout.padding, layout.radius), (0., 0., 0.));
                } else {
                    assert!(layout.gap > 0. && layout.padding > 0.);
                }
                assert_eq!(
                    GalleryLayout::new(width, level as f32 + 0.2).edge,
                    layout.edge
                );
                for i in [0, 1, 10, 41, 1000] {
                    let cell = layout.cell(i);
                    assert!(cell.edge > 0. && cell.x >= 0. && cell.x + cell.edge <= width + 1.);
                }
            }
        }
    }
    #[test]
    fn virtual_blocks_match_absolute_positions_and_preserve_zoom_anchor() {
        for zoom in [0., 1.2, 2.5, 3., 3.8, 4.] {
            let layout = GalleryLayout::new(844., zoom);
            for i in 0..layout.block_len {
                let a = layout.cell(i);
                let b = layout.cell(i + layout.block_len);
                assert!((a.x - b.x).abs() < 0.01);
                assert!((b.y - a.y - layout.block_height).abs() < 0.1);
            }
            let next = GalleryLayout::new(844., (zoom + 1.).min(4.));
            let cell = layout.cell(1000);
            let pointer = (cell.x + cell.edge / 2., 200.);
            let old_offset = cell.y + cell.edge / 2. - pointer.1;
            let anchored = layout.nearest(pointer.0, old_offset + pointer.1, 1_000_000);
            let before = layout.cell(anchored);
            let within = ((old_offset + pointer.1 - before.y) / before.edge).clamp(0., 1.);
            let offset = layout.anchored_offset(&next, pointer, old_offset, 1_000_000);
            let after = next.cell(anchored);
            assert!((after.y + within * after.edge - offset - pointer.1).abs() < 0.2);
        }
    }
}
