//! Stable gallery rows and gesture-driven frame/content transitions.
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
    pub leading: usize,
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
            leading: 0,
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
    /// Bottom-up catalogs keep the newest row full. Unknown/older photos only
    /// change the partial row at the top, never the grouping of the latest photos.
    pub fn with_count(mut self, count: usize, bottom_up: bool) -> Self {
        self.leading = if bottom_up && count > 0 {
            (self.block_len - count % self.block_len) % self.block_len
        } else {
            0
        };
        self
    }
    pub fn indices(&self, block: usize, count: usize) -> std::ops::Range<usize> {
        (block * self.block_len)
            .saturating_sub(self.leading)
            .min(count)
            ..((block + 1) * self.block_len)
                .saturating_sub(self.leading)
                .min(count)
    }
    pub fn full_bleed(&self) -> bool {
        self.zoom <= 1.
    }
    pub fn cell(&self, index: usize) -> Cell {
        let index = index + self.leading;
        Cell {
            x: self.padding + (index % self.block_len) as f32 * (self.edge + self.gap),
            y: (index / self.block_len) as f32 * self.block_height,
            edge: self.edge,
        }
    }
    pub fn blocks(&self, count: usize) -> usize {
        (count + self.leading).div_ceil(self.block_len)
    }
    pub fn nearest(&self, x: f32, y: f32, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        let block = (y.max(0.) / self.block_height).floor() as usize;
        let first = (block.saturating_sub(1) * self.block_len).saturating_sub(self.leading);
        let last = ((block + 2) * self.block_len)
            .saturating_sub(self.leading)
            .min(count);
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

/// A grid slot, not a moving photo identity. Its frame follows the pinch while
/// its two image contents dissolve. This avoids diagonal photo migrations when
/// a column count changes (e.g. five columns become three).
#[derive(Clone, Copy, Debug)]
pub struct FramePair {
    pub from: Cell,
    pub to: Cell,
    pub source: Option<usize>,
    pub target: Option<usize>,
}
impl FramePair {
    pub fn at(&self, progress: f32) -> Cell {
        let t = progress.clamp(0., 1.);
        Cell {
            x: self.from.x + (self.to.x - self.from.x) * t,
            y: self.from.y + (self.to.y - self.from.y) * t,
            edge: self.from.edge + (self.to.edge - self.from.edge) * t,
        }
    }
}
#[derive(Clone, Copy)]
pub struct GridFrame {
    pub layout: GalleryLayout,
    pub offset: f32,
    pub header: f32,
    pub height: f32,
}
impl GridFrame {
    pub fn slots_to(self, next: Self, anchor: usize, count: usize) -> Vec<FramePair> {
        if count == 0 {
            return vec![];
        }
        let old = self.layout;
        let new = next.layout;
        let a = old.cell(anchor);
        let b = new.cell(anchor);
        let a_slot = anchor + old.leading;
        let b_slot = anchor + new.leading;
        let rows = (self.height.max(next.height) / old.block_height.min(new.block_height)).ceil()
            as i64
            + 2;
        let cols = old.block_len.max(new.block_len) as i64;
        let index = |layout: GalleryLayout, slot: usize, dr: i64, dc: i64| {
            let row = (slot / layout.block_len) as i64 + dr;
            let col = (slot % layout.block_len) as i64 + dc;
            if row < 0 || col < 0 || col >= layout.block_len as i64 {
                return None;
            }
            let slot = row as usize * layout.block_len + col as usize;
            slot.checked_sub(layout.leading).filter(|&i| i < count)
        };
        let visible = |cell: Cell, frame: Self| {
            cell.x + cell.edge > 0.
                && cell.x
                    < (frame.layout.edge + frame.layout.gap) * frame.layout.block_len as f32
                        + frame.layout.padding
                && cell.y + cell.edge > frame.header
                && cell.y < frame.header + frame.height
        };
        let mut pairs = vec![];
        for dr in -rows..=rows {
            for dc in -cols..=cols {
                let from = Cell {
                    x: a.x + dc as f32 * (old.edge + old.gap),
                    y: a.y + dr as f32 * old.block_height - self.offset + self.header,
                    edge: old.edge,
                };
                let to = Cell {
                    x: b.x + dc as f32 * (new.edge + new.gap),
                    y: b.y + dr as f32 * new.block_height - next.offset + next.header,
                    edge: new.edge,
                };
                let source = index(old, a_slot, dr, dc);
                let target = index(new, b_slot, dr, dc);
                if (source.is_some() || target.is_some())
                    && (visible(from, self) || visible(to, next))
                {
                    pairs.push(FramePair {
                        from,
                        to,
                        source,
                        target,
                    });
                }
            }
        }
        pairs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bottom_rows_never_reflow_when_older_catalog_pages_arrive() {
        for zoom in 0..=4 {
            for count in 20..120 {
                let before = GalleryLayout::new(844., zoom as f32).with_count(count, true);
                for added in [1, 2, 17, 197, 200, 3200] {
                    let after =
                        GalleryLayout::new(844., zoom as f32).with_count(count + added, true);
                    for age in 0..20 {
                        let a = before.cell(count - 1 - age);
                        let b = after.cell(count + added - 1 - age);
                        assert!((a.x - b.x).abs() < 0.01);
                        let ay = a.y - before.blocks(count) as f32 * before.block_height;
                        let by = b.y - after.blocks(count + added) as f32 * after.block_height;
                        assert!(
                            (ay - by).abs() < 0.1,
                            "latest row must not drop: {count}, {added}, {zoom}"
                        );
                    }
                    let all = (0..after.blocks(count + added))
                        .flat_map(|row| after.indices(row, count + added))
                        .collect::<Vec<_>>();
                    assert_eq!(all, (0..count + added).collect::<Vec<_>>());
                }
            }
        }
    }
    #[test]
    fn transition_moves_slot_frames_and_dissolves_contents_with_a_stable_anchor() {
        let count = 1_000_003;
        for level in 0..4 {
            let old = GalleryLayout::new(844., level as f32).with_count(count, true);
            let new = GalleryLayout::new(844., level as f32 + 1.).with_count(count, true);
            let anchor = 500_001;
            let offset = old.cell(anchor).y + old.edge / 2. - 300.;
            let next_offset = old.anchored_offset(
                &new,
                (old.cell(anchor).x + old.edge / 2., 300.),
                offset,
                count,
            );
            let from = GridFrame {
                layout: old,
                offset,
                header: 0.,
                height: 600.,
            };
            let to = GridFrame {
                layout: new,
                offset: next_offset,
                header: 0.,
                height: 600.,
            };
            let pairs = from.slots_to(to, anchor, count);
            assert!(
                pairs.len() < 500,
                "transition must be bounded by the viewport"
            );
            assert!(
                pairs
                    .iter()
                    .any(|p| p.source == Some(anchor) && p.target == Some(anchor))
            );
            assert!(
                pairs
                    .iter()
                    .any(|p| p.source.is_some() && p.target.is_some() && p.source != p.target)
            );
            for pair in pairs {
                let frame = pair.at(0.5);
                assert!((frame.x - (pair.from.x + pair.to.x) / 2.).abs() < 0.1);
                assert!((frame.y - (pair.from.y + pair.to.y) / 2.).abs() < 0.1);
                assert!((frame.edge - (pair.from.edge + pair.to.edge) / 2.).abs() < 0.1);
            }
        }
    }
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
