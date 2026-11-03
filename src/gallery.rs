//! Gallery density levels, month/year sections and zoom transitions.
//!
//! Three tiers, from most zoomed out: **Years** and **Months** pack square
//! photos edge to edge in date sections, each with its own header and a gap
//! before the next section; **All** is one continuous grid with gutters, each
//! photo at its own aspect ratio, in 9, 7, 5 or 3 columns. Steps are large
//! inside the date tiers, and the step from 9 columns into them is the
//! largest.
use std::{ops::Range, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Years,
    Months,
    All,
}

#[derive(Clone, Copy, Debug)]
enum Size {
    /// Target tile edge in points; the column count follows the width.
    Edge(f32),
    /// Fixed column count with a gutter/padding/corner scale.
    Columns(usize, f32),
}

/// Densest first. Dense levels multiply tile size ~1.35× per step. Margined
/// levels differ by two columns, one more each side, so every row widens
/// about the centre.
const LEVELS: [(Tier, Size); 10] = [
    (Tier::Years, Size::Edge(13.)),
    (Tier::Years, Size::Edge(18.)),
    (Tier::Years, Size::Edge(24.)),
    (Tier::Months, Size::Edge(32.)),
    (Tier::Months, Size::Edge(42.)),
    (Tier::Months, Size::Edge(56.)),
    (Tier::All, Size::Columns(9, 0.2)),
    (Tier::All, Size::Columns(7, 0.26)),
    (Tier::All, Size::Columns(5, 0.38)),
    (Tier::All, Size::Columns(3, 0.7)),
];
/// Gutter, outer padding and corner radius at spacing 1.0.
const GAP: f32 = 12.;
const PADDING: f32 = 24.;
const RADIUS: f32 = 10.;

/// A run of consecutive photos (in display order) sharing a month or year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    pub start: usize,
    pub len: usize,
    /// `year * 12 + month0` for months, the year for years.
    pub key: i32,
}

pub fn month_key(year: i32, month0: u32) -> i32 {
    year * 12 + month0 as i32
}
pub fn year_of(month_key: i32) -> i32 {
    month_key.div_euclid(12)
}

/// Extends `groups` with photos `from..keys.len()` (indices are positions in
/// `keys`), merging into the last run when the key continues.
pub fn extend_groups(groups: &mut Vec<Group>, from: usize, keys: impl Iterator<Item = i32>) {
    for (index, key) in (from..).zip(keys) {
        match groups.last_mut() {
            Some(last) if last.key == key && last.start + last.len == index => last.len += 1,
            _ => groups.push(Group {
                start: index,
                len: 1,
                key,
            }),
        }
    }
}
/// Groups built in source order, viewed in reverse (bottom-to-top browsing).
pub fn reverse_groups(groups: &[Group], count: usize) -> Vec<Group> {
    groups
        .iter()
        .rev()
        .map(|g| Group {
            start: count - g.start - g.len,
            ..*g
        })
        .collect()
}
/// Year runs derived from month runs without touching photos again.
pub fn years_from_months(months: &[Group]) -> Vec<Group> {
    let mut years: Vec<Group> = vec![];
    for g in months {
        let key = year_of(g.key);
        match years.last_mut() {
            Some(last) if last.key == key && last.start + last.len == g.start => last.len += g.len,
            _ => years.push(Group { key, ..*g }),
        }
    }
    years
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    pub start: usize,
    pub len: usize,
    pub key: i32,
    /// Top of the header (or of the first row when there is no header).
    pub top: f32,
    pub rows: usize,
}

#[derive(Clone, Debug)]
pub struct GalleryLayout {
    pub level: usize,
    pub tier: Tier,
    pub columns: usize,
    pub edge: f32,
    pub gap: f32,
    pub padding: f32,
    pub radius: f32,
    /// Vertical step from one row to the next.
    pub pitch: f32,
    /// Section header height (0 in the All tier).
    pub header: f32,
    /// Space between a section's last row and the next section header.
    pub section_gap: f32,
    /// Empty slots before the first photo (All tier, bottom-up).
    pub leading: usize,
    pub count: usize,
    pub sections: Arc<Vec<Section>>,
    pub height: f32,
    pub width: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    /// The largest rect of `aspect` centred inside this one.
    pub fn fit(self, aspect: f32) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0. {
            aspect
        } else {
            1.
        };
        let (w, h) = if aspect >= self.w / self.h {
            (self.w, self.w / aspect)
        } else {
            (self.h * aspect, self.h)
        };
        Self {
            x: self.x + (self.w - w) / 2.,
            y: self.y + (self.h - h) / 2.,
            w,
            h,
        }
    }
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
            w: self.w + (other.w - self.w) * t,
            h: self.h + (other.h - self.h) * t,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub x: f32,
    pub y: f32,
    pub edge: f32,
}

impl GalleryLayout {
    pub const MAX_LEVEL: usize = LEVELS.len() - 1;
    /// Five columns.
    pub const DEFAULT_LEVEL: usize = 8;

    pub fn tier_of(level: usize) -> Tier {
        LEVELS[level.min(Self::MAX_LEVEL)].0
    }
    /// A level with gutters and a fixed, odd column count.
    pub fn margined(level: usize) -> bool {
        matches!(LEVELS[level.min(Self::MAX_LEVEL)].1, Size::Columns(..))
    }
    fn columns_at(width: f32, level: usize) -> usize {
        let base = |level: usize| match LEVELS[level].1 {
            Size::Columns(columns, _) => columns,
            Size::Edge(edge) => (width / edge).round().max(1.) as usize,
        };
        // Stepping out never removes columns, and every dense step adds some,
        // even in very narrow windows. Leaving the margined levels for the
        // date grids is always a big step.
        let mut columns = base(LEVELS.len() - 1);
        for l in (level..LEVELS.len() - 1).rev() {
            columns = match (LEVELS[l].1, LEVELS[l + 1].1) {
                (Size::Edge(_), Size::Columns(..)) => base(l).max(columns * 3 / 2 + 1),
                (Size::Edge(_), _) => base(l).max(columns + 1),
                (Size::Columns(..), _) => base(l).max(columns),
            };
        }
        columns
    }
    /// Geometry for a level, laid out as one unsectioned grid. Call
    /// [`with_count`](Self::with_count) or [`with_groups`](Self::with_groups).
    pub fn new(width: f32, level: usize) -> Self {
        let level = level.min(Self::MAX_LEVEL);
        let width = width.max(1.);
        let (tier, size) = LEVELS[level];
        let spacing = match size {
            Size::Columns(_, spacing) => spacing,
            Size::Edge(_) => 0.,
        };
        let columns = Self::columns_at(width, level);
        let padding = PADDING * spacing;
        let gap = GAP * spacing;
        let edge = ((width - 2. * padding - (columns - 1) as f32 * gap) / columns as f32).max(1.);
        let (header, section_gap) = match tier {
            Tier::Years => (52., 22.),
            Tier::Months => (40., 14.),
            Tier::All => (0., 0.),
        };
        Self {
            level,
            tier,
            columns,
            edge,
            gap,
            padding,
            radius: RADIUS * spacing,
            pitch: if gap == 0. {
                edge
            } else {
                (edge + gap).round()
            },
            header,
            section_gap,
            leading: 0,
            count: 0,
            sections: Arc::new(vec![]),
            height: 0.,
            width,
        }
    }
    pub fn sectioned(&self) -> bool {
        self.tier != Tier::All
    }
    /// Gapless levels are painted directly instead of as interactive tiles.
    pub fn dense(&self) -> bool {
        self.full_bleed()
    }
    pub fn full_bleed(&self) -> bool {
        self.gap == 0.
    }
    /// One continuous grid. Bottom-up catalogs keep the newest row full: older
    /// photos only change the partial row at the top.
    pub fn with_count(mut self, count: usize, bottom_up: bool) -> Self {
        self.count = count;
        self.leading = if bottom_up && count > 0 {
            (self.columns - count % self.columns) % self.columns
        } else {
            0
        };
        let rows = (count + self.leading).div_ceil(self.columns);
        self.sections = Arc::new(if count == 0 {
            vec![]
        } else {
            vec![Section {
                start: 0,
                len: count,
                key: 0,
                top: 0.,
                rows,
            }]
        });
        self.height = self.rows_height(rows);
        self
    }
    /// Date sections in the Months/Years tiers; other tiers ignore the groups.
    pub fn with_groups(self, groups: &[Group], count: usize, bottom_up: bool) -> Self {
        if !self.sectioned() || groups.is_empty() {
            return self.with_count(count, bottom_up);
        }
        let mut layout = self;
        layout.count = count;
        layout.leading = 0;
        let mut y = 0.;
        let sections = groups
            .iter()
            .map(|g| {
                let rows = g.len.div_ceil(layout.columns);
                let section = Section {
                    start: g.start,
                    len: g.len,
                    key: g.key,
                    top: y,
                    rows,
                };
                y += layout.header + layout.rows_height(rows) + layout.section_gap;
                section
            })
            .collect();
        layout.sections = Arc::new(sections);
        layout.height = (y - layout.section_gap).max(0.);
        layout
    }
    fn rows_height(&self, rows: usize) -> f32 {
        if rows == 0 {
            0.
        } else {
            (rows - 1) as f32 * self.pitch + self.edge.max(self.pitch.min(self.edge + self.gap))
        }
    }
    fn section_index_of(&self, index: usize) -> usize {
        self.sections
            .partition_point(|s| s.start <= index)
            .saturating_sub(1)
    }
    fn section_at_y(&self, y: f32) -> usize {
        self.sections
            .partition_point(|s| s.top <= y)
            .saturating_sub(1)
    }
    pub fn section_of(&self, index: usize) -> Option<&Section> {
        self.sections.get(self.section_index_of(index))
    }
    /// The photo's drawn frame: the whole cell when gapless, else its aspect
    /// ratio (`width / height`) fitted and centred in the cell.
    pub fn frame(&self, index: usize, aspect: f32) -> Rect {
        let cell = self.cell(index);
        let rect = Rect {
            x: cell.x,
            y: cell.y,
            w: cell.edge,
            h: cell.edge,
        };
        if self.full_bleed() {
            rect
        } else {
            rect.fit(aspect)
        }
    }
    pub fn cell(&self, index: usize) -> Cell {
        let Some(section) = self.section_of(index) else {
            return Cell {
                x: self.padding,
                y: 0.,
                edge: self.edge,
            };
        };
        let local = index - section.start + if self.sectioned() { 0 } else { self.leading };
        Cell {
            x: self.padding + (local % self.columns) as f32 * (self.edge + self.gap),
            y: section.top + self.header + (local / self.columns) as f32 * self.pitch,
            edge: self.edge,
        }
    }
    /// The photo at a content point, if the point is on a photo.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let index = self.nearest(x, y)?;
        let cell = self.cell(index);
        (x >= cell.x && x < cell.x + cell.edge && y >= cell.y && y < cell.y + cell.edge)
            .then_some(index)
    }
    pub fn nearest(&self, x: f32, y: f32) -> Option<usize> {
        let section = self.sections.get(self.section_at_y(y))?;
        let leading = if self.sectioned() { 0 } else { self.leading };
        let row = ((y - section.top - self.header) / self.pitch)
            .floor()
            .clamp(0., section.rows.saturating_sub(1) as f32) as usize;
        let column = ((x - self.padding + self.gap / 2.) / (self.edge + self.gap))
            .floor()
            .clamp(0., (self.columns - 1) as f32) as usize;
        let local = (row * self.columns + column)
            .max(leading)
            .min(leading + section.len - 1);
        Some(section.start + local - leading)
    }
    /// Photo index ranges whose rows intersect `top..bottom`.
    pub fn visible(&self, top: f32, bottom: f32) -> Vec<Range<usize>> {
        let leading = if self.sectioned() { 0 } else { self.leading };
        let mut ranges = vec![];
        for section in &self.sections[self.section_at_y(top)..] {
            if section.top > bottom {
                break;
            }
            let rows_top = section.top + self.header;
            let first = ((top - rows_top) / self.pitch).floor().max(0.) as usize;
            let last = (((bottom - rows_top) / self.pitch).floor() + 1.).max(0.) as usize;
            let start = (first * self.columns)
                .saturating_sub(leading)
                .min(section.len);
            let end = (last.min(section.rows) * self.columns)
                .saturating_sub(leading)
                .min(section.len);
            if start < end {
                ranges.push(section.start + start..section.start + end);
            }
        }
        ranges
    }
    /// Sections whose header or rows intersect `top..bottom`.
    pub fn sections_in(&self, top: f32, bottom: f32) -> Range<usize> {
        let first = self.section_at_y(top);
        let last = self.sections.partition_point(|s| s.top <= bottom);
        first..last.max(first)
    }
    /// The header of the section at the top of the viewport stays pinned until
    /// the next section's header pushes it away. Returns (section, y in content).
    pub fn sticky(&self, offset: f32) -> Option<(usize, f32)> {
        if !self.sectioned() || self.sections.is_empty() {
            return None;
        }
        let i = self.section_at_y(offset);
        let section = &self.sections[i];
        let mut y = section.top.max(offset);
        if let Some(next) = self.sections.get(i + 1) {
            y = y.min(next.top - self.header);
        }
        Some((i, y))
    }
    /// The scroll offset that keeps the photo under `pointer` (viewport
    /// coordinates) under it in `next`.
    pub fn anchored_offset(&self, next: &Self, pointer: (f32, f32), offset: f32) -> f32 {
        let Some(index) = self.nearest(pointer.0, offset + pointer.1) else {
            return 0.;
        };
        let old = self.cell(index);
        let new = next.cell(index);
        let within = ((offset + pointer.1 - old.y) / old.edge).clamp(0., 1.);
        (new.y + within * new.edge - pointer.1).max(0.)
    }
}

/// One endpoint of a transition: a layout scrolled to `offset`, drawn with its
/// content starting `header` points below the top of the gallery surface.
#[derive(Clone)]
pub struct GridFrame {
    pub layout: GalleryLayout,
    pub offset: f32,
    pub header: f32,
    pub height: f32,
}
impl GridFrame {
    /// A photo's frame on the gallery surface.
    pub fn screen(&self, cell: Cell) -> Cell {
        Cell {
            y: cell.y - self.offset + self.header,
            ..cell
        }
    }
    pub fn screen_rect(&self, rect: Rect) -> Rect {
        Rect {
            y: rect.y - self.offset + self.header,
            ..rect
        }
    }
    /// `layout` on a gallery surface `area` tall, below a `title` row, and
    /// scrolled so the `anchor` photo's spot (its index and the y fraction
    /// inside its cell) is at `surface_y`. Bottom-up browsing that started at
    /// the newest row (`at_end`) stays there instead.
    pub fn anchored(
        layout: GalleryLayout,
        anchor: Option<(usize, f32)>,
        surface_y: f32,
        area: f32,
        title: f32,
        bottom_up: bool,
        at_end: bool,
    ) -> Self {
        let body = (area - title).max(1.);
        let height = layout.height.min(body);
        let header = title + if bottom_up { body - height } else { 0. };
        let max = (layout.height - height).max(0.);
        let offset = if bottom_up && at_end {
            max
        } else {
            anchor.map_or(0., |(index, within)| {
                let cell = layout.cell(index);
                (cell.y + within * cell.edge - (surface_y - header)).clamp(0., max)
            })
        };
        Self {
            layout,
            offset,
            header,
            height,
        }
    }
}

/// Library zoom.
///
/// The settled grid is always one of the [`LEVELS`]. Two things change while
/// zooming, on different clocks:
///
/// - **Size** follows the input. A pinch moves a continuous `position` on a
///   [`Ladder`] as the fingers move; ⌘-scroll, wheel notches and ⌘+/− move it
///   a whole level at a time on a spring. Margined levels share one centred
///   [`Lattice`] of slots: from 3 columns to 5, a slot appears at each end of
///   every row and the rows close up about the centre, so no photo travels
///   across the grid. Date grids are scaled whole about the same centre.
/// - **Photos** change level by level ([`Crossfade`]): each change is a
///   crossfade of fixed length that starts as soon as the zoom heads to the
///   next level, whatever the gesture then does. Each level's photos are laid
///   into the slots (or scaled grid) at the current size, so the incoming and
///   outgoing photos always share the same geometry.
///
/// Everything here is pure: input goes in through [`Zoom::apply`], time
/// through [`Zoom::tick`] and [`Crossfade::aim`], and drawing reads [`Layer`].
pub mod zoom {
    use super::{GalleryLayout, GridFrame, Rect, Tier};
    use std::{
        collections::VecDeque,
        ops::Range,
        time::{Duration, Instant},
    };

    /// Spring stiffness (rad/s) of the snap: critically damped, settling in
    /// roughly a third of a second while carrying the release velocity.
    pub const OMEGA: f32 = 17.;
    /// How far past either end a gesture can stretch, rubber-banded.
    pub const OVERSCROLL: f32 = 0.35;
    /// Input without gesture phases settles once it has been quiet this long.
    pub const IDLE_RELEASE: Duration = Duration::from_millis(240);
    /// How long one level's photos take to crossfade into the next level's.
    pub const FADE: Duration = Duration::from_millis(220);
    /// How far (in levels) a gesture moves before the photos start changing
    /// to the next level's.
    pub const START: f32 = 0.04;
    /// How far a gesture must come back before it counts as turning around.
    const TURN: f32 = 0.1;
    /// How far toward the next level a released gesture must have gone to
    /// land there rather than spring back.
    const COMMIT: f32 = 0.15;
    /// Gesture history used to measure release velocity.
    const SAMPLE_WINDOW: Duration = Duration::from_millis(100);
    /// Neighbouring levels whose photos differ by less than this are the
    /// same size on screen, so only one of them is on the ladder.
    const DISTINCT: f32 = 0.5;

    /// The levels reachable at one width, smallest photos first, with their
    /// photo edge. Only one of any levels the same size is kept, so photo
    /// size always grows along the ladder and a pinch always has somewhere to
    /// go.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Ladder {
        pub levels: Vec<usize>,
        pub edges: Vec<f32>,
    }
    impl Ladder {
        /// The ladder for `width`. `keep` (the level on screen) is the one
        /// kept among levels of the same size, so the zoom starts from it.
        pub fn for_width(width: f32, keep: usize) -> Self {
            let mut levels: Vec<usize> = vec![];
            let mut edges: Vec<f32> = vec![];
            for level in 0..=GalleryLayout::MAX_LEVEL {
                let edge = GalleryLayout::new(width, level).edge;
                match edges.last() {
                    Some(&last) if edge <= last + DISTINCT => {
                        let before = edges.len().checked_sub(2).map(|i| edges[i]);
                        if level == keep && before.is_none_or(|b| edge > b + DISTINCT) {
                            *levels.last_mut().unwrap() = level;
                            *edges.last_mut().unwrap() = edge;
                        }
                    }
                    _ => {
                        levels.push(level);
                        edges.push(edge);
                    }
                }
            }
            Self { levels, edges }
        }
        /// Highest position.
        pub fn max(&self) -> f32 {
            (self.levels.len() - 1) as f32
        }
        /// The ladder slot showing `level`, or the closest smaller one.
        pub fn index_of(&self, level: usize) -> usize {
            self.levels.partition_point(|&l| l <= level).saturating_sub(1)
        }
        /// The slots of margined levels (always the largest photos).
        pub fn margined(&self) -> Range<usize> {
            self.levels
                .partition_point(|&l| !GalleryLayout::margined(l))..self.levels.len()
        }
        /// The segment `position` lies on: its lower slot and how far along.
        fn segment(&self, position: f32) -> (usize, f32) {
            if self.levels.len() < 2 {
                return (0, 0.);
            }
            let i = (position.floor().max(0.) as usize).min(self.levels.len() - 2);
            (i, position - i as f32)
        }
        /// The photo edge at `position`: linear between neighbouring levels,
        /// extended past either end.
        pub fn edge(&self, position: f32) -> f32 {
            let (i, t) = self.segment(position);
            match self.edges.get(i + 1) {
                Some(&next) => (self.edges[i] + (next - self.edges[i]) * t).max(1.),
                None => self.edges[0],
            }
        }
        /// The position whose photo edge is `edge`; the inverse of
        /// [`edge`](Self::edge).
        pub fn locate(&self, edge: f32) -> f32 {
            if self.levels.len() < 2 {
                return 0.;
            }
            let i = self
                .edges
                .partition_point(|&e| e <= edge)
                .saturating_sub(1)
                .min(self.levels.len() - 2);
            let (low, high) = (self.edges[i], self.edges[i + 1]);
            i as f32 + (edge - low) / (high - low)
        }
        /// The two levels around `position` and how far from the first to
        /// the second (0..=1). Past either end it is the last pair, at rest.
        pub fn pair(&self, position: f32) -> (usize, usize, f32) {
            let clamped = position.clamp(0., self.max());
            let (i, t) = self.segment(clamped);
            let j = (i + 1).min(self.levels.len() - 1);
            (self.levels[i], self.levels[j], if i == j { 0. } else { t })
        }
        /// The margined slots whose lattices blend into the one at
        /// `position`, and how far from the first to the second. Outside the
        /// margined levels it is the nearest one, which
        /// [`lattice_at`] then scales to the photo size there.
        pub fn lattice_pair(&self, position: f32) -> Option<(usize, usize, f32)> {
            let margined = self.margined();
            let (low, high) = (margined.start, margined.end.checked_sub(1)?);
            if low >= high {
                return (low == high).then_some((low, low, 0.));
            }
            let p = position.clamp(low as f32, high as f32);
            let i = (p.floor() as usize).clamp(low, high - 1);
            Some((i, i + 1, p - i as f32))
        }
    }

    /// Gesture past either end meets increasing resistance.
    pub fn rubber_band(value: f32, delta: f32, max: f32) -> f32 {
        let past = if value < 0. {
            -value
        } else if value > max {
            value - max
        } else {
            0.
        };
        let resist = (1. - past / OVERSCROLL).clamp(0.05, 1.);
        (value + delta * resist).clamp(-OVERSCROLL, max + OVERSCROLL)
    }
    /// Where a released gesture settles. A gesture that went a little way
    /// toward the next level (or flicked there) lands on it; momentum never
    /// carries it more than one level beyond where the fingers left it.
    /// `heading` is the direction the gesture was moving (−1, 0 or 1).
    pub fn release_target(value: f32, velocity: f32, heading: f32, max: f32) -> f32 {
        let here = value.round();
        let projected = value + velocity * 0.12;
        let target = if heading < 0. {
            (projected + COMMIT).floor()
        } else if heading > 0. {
            (projected - COMMIT).ceil()
        } else {
            projected.round()
        };
        target.clamp(here - 1., here + 1.).clamp(0., max)
    }
    /// Advances a critically damped spring toward `target` by `dt` seconds
    /// (exact solution, so frame timing never changes the curve).
    pub fn spring(value: f32, velocity: f32, target: f32, dt: f32) -> (f32, f32) {
        let x = value - target;
        let b = velocity + OMEGA * x;
        let decay = (-OMEGA * dt).exp();
        let x = (x + b * dt) * decay;
        (
            target + x,
            (b - OMEGA * (x / decay.max(f32::MIN_POSITIVE))) * decay,
        )
    }
    pub fn at_rest(value: f32, velocity: f32, target: f32) -> bool {
        (value - target).abs() < 0.002 && velocity.abs() < 0.02
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Input {
        /// Scale the photos by this factor (1 = no change).
        Magnify(f32),
        /// Move this many levels (⌘-scroll, wheel notch, ⌘+/−).
        Step(i32),
        /// The fingers lifted.
        Release,
    }
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Phase {
        /// Following the fingers; holds still while they rest.
        Held,
        /// Springing to a ladder slot.
        Settling { target: usize },
    }
    /// A zoom interaction's size, from first input until it comes to rest.
    #[derive(Clone, Debug)]
    pub struct Zoom {
        /// Continuous position on the ladder.
        pub position: f32,
        /// Ladder slots per second.
        pub velocity: f32,
        pub phase: Phase,
        /// The input reports when the fingers lift (trackpad), so it holds
        /// until then; other input settles once it goes quiet.
        pub phased: bool,
        /// Which way a held gesture is going: −1 out, 1 in, 0 not yet.
        pub heading: f32,
        /// The furthest the gesture has gone in its heading.
        extreme: f32,
        last_input: Instant,
        clock: Instant,
        samples: VecDeque<(Instant, f32)>,
    }
    impl Zoom {
        pub fn new(slot: usize, now: Instant) -> Self {
            Self {
                position: slot as f32,
                velocity: 0.,
                phase: Phase::Settling { target: slot },
                phased: false,
                heading: 0.,
                extreme: slot as f32,
                last_input: now,
                clock: now,
                samples: VecDeque::new(),
            }
        }
        pub fn apply(&mut self, ladder: &Ladder, input: Input, phased: bool, now: Instant) {
            let max = ladder.max();
            match input {
                Input::Magnify(factor) => {
                    if !factor.is_finite() || factor <= 0. {
                        return;
                    }
                    if self.phase != Phase::Held {
                        // Grabbing a settling zoom continues from what is on screen.
                        self.phase = Phase::Held;
                        self.velocity = 0.;
                        self.samples.clear();
                        self.heading = 0.;
                        self.extreme = self.position;
                    }
                    self.phased |= phased;
                    let to = ladder.locate(ladder.edge(self.position) * factor);
                    self.position = rubber_band(self.position, to - self.position, max);
                    self.track();
                    self.last_input = now;
                    self.samples.push_back((now, self.position));
                    while self
                        .samples
                        .front()
                        .is_some_and(|(t, _)| now.duration_since(*t) > SAMPLE_WINDOW)
                    {
                        self.samples.pop_front();
                    }
                }
                Input::Step(steps) => {
                    // Steps during a step retarget the same spring and keep
                    // its momentum, so a run of them reads as one zoom.
                    let base = match self.phase {
                        Phase::Settling { target } => target as f32,
                        Phase::Held => self.position.round(),
                    };
                    let target = (base + steps as f32).clamp(0., max) as usize;
                    self.phase = Phase::Settling { target };
                    self.phased = false;
                }
                Input::Release => {
                    if self.phase != Phase::Held {
                        return;
                    }
                    if let (Some((t0, v0)), Some((t1, v1))) =
                        (self.samples.front(), self.samples.back())
                    {
                        let dt = t1.duration_since(*t0).as_secs_f32();
                        self.velocity = if dt > 0.004 { (v1 - v0) / dt } else { 0. };
                    }
                    let target =
                        release_target(self.position, self.velocity, self.heading, max) as usize;
                    self.phase = Phase::Settling { target };
                    self.phased = false;
                }
            }
        }
        /// Follows the gesture's heading. It is set once the gesture has
        /// moved [`START`], and turns only after coming back [`TURN`], so a
        /// trembling finger never flips the photos back and forth.
        fn track(&mut self) {
            let moved = self.position - self.extreme;
            if self.heading == 0. {
                if moved.abs() > START {
                    self.heading = moved.signum();
                    self.extreme = self.position;
                }
            } else if moved * self.heading > 0. {
                self.extreme = self.position;
            } else if -moved * self.heading > TURN {
                self.heading = -self.heading;
                self.extreme = self.position;
            }
        }
        /// The level whose photos should fill the grid, given the level
        /// shown now: where a step or release is going, or else the next
        /// level the fingers are heading to (from the moment they set off).
        pub fn goal(&self, shown: usize, ladder: &Ladder) -> usize {
            let goal = match self.phase {
                Phase::Settling { target } => return target,
                Phase::Held if self.heading < 0. => (self.position + START).floor(),
                Phase::Held if self.heading > 0. => (self.position - START).ceil(),
                Phase::Held => return shown,
            };
            goal.clamp(0., ladder.max()) as usize
        }
        /// Restarts the clock after the zoom has been still.
        pub fn resume(&mut self, now: Instant) {
            self.clock = now;
        }
        /// Advances time. Returns whether more frames are needed.
        pub fn tick(&mut self, ladder: &Ladder, now: Instant) -> bool {
            let dt = now.duration_since(self.clock).as_secs_f32().min(0.05);
            self.clock = now;
            match self.phase {
                Phase::Held if self.phased => false,
                Phase::Held => {
                    if now.duration_since(self.last_input) >= IDLE_RELEASE {
                        self.apply(ladder, Input::Release, false, now);
                    }
                    true
                }
                Phase::Settling { target } => {
                    let target = target as f32;
                    (self.position, self.velocity) =
                        spring(self.position, self.velocity, target, dt);
                    if at_rest(self.position, self.velocity, target) {
                        self.position = target;
                        self.velocity = 0.;
                        return false;
                    }
                    true
                }
            }
        }
        /// The ladder slot the zoom has come to rest on.
        pub fn settled(&self) -> Option<usize> {
            match self.phase {
                Phase::Settling { target }
                    if self.position == target as f32 && self.velocity == 0. =>
                {
                    Some(target)
                }
                _ => None,
            }
        }
    }

    /// Which levels' photos fill the grid: one at rest, or two while one
    /// crossfades into the next. A change takes [`FADE`] whatever the zoom
    /// does meanwhile, and goes one level at a time: a zoom past several
    /// levels fades through each in turn. Turning back mid-fade reverses it
    /// from where it is.
    #[derive(Clone, Debug)]
    pub struct Crossfade {
        /// Ladder slots: fading from `from` to `to` (equal at rest).
        pub from: usize,
        pub to: usize,
        started: Instant,
    }
    impl Crossfade {
        pub fn new(slot: usize, now: Instant) -> Self {
            Self {
                from: slot,
                to: slot,
                started: now,
            }
        }
        fn progress(&self, now: Instant) -> f32 {
            if self.from == self.to {
                return 1.;
            }
            (now.saturating_duration_since(self.started).as_secs_f32() / FADE.as_secs_f32()).min(1.)
        }
        /// Whether a fade is still running.
        pub fn fading(&self, now: Instant) -> bool {
            self.progress(now) < 1.
        }
        /// The level shown alone, once no fade is running.
        pub fn shown(&self, now: Instant) -> Option<usize> {
            (!self.fading(now)).then_some(self.to)
        }
        /// Moves the photos toward level `goal`: reverses a running fade if
        /// `goal` is back where it came from, else starts the next level's
        /// fade once the running one has finished.
        pub fn aim(&mut self, goal: usize, now: Instant) {
            let progress = self.progress(now);
            if progress < 1. {
                if goal != self.to && (goal < self.to) == (self.from < self.to) {
                    std::mem::swap(&mut self.from, &mut self.to);
                    self.started = now
                        .checked_sub(FADE.mul_f32(1. - progress))
                        .unwrap_or(now);
                }
                return;
            }
            self.from = self.to;
            if goal != self.to {
                self.to = if goal > self.to {
                    self.to + 1
                } else {
                    self.to - 1
                };
                self.started = now;
            }
        }
        /// The levels to draw, the outgoing one first, with their opacity.
        /// Easing is symmetric, so a reversed fade keeps its opacities.
        pub fn layers(&self, now: Instant) -> Vec<(usize, f32)> {
            let p = self.progress(now);
            if p >= 1. {
                return vec![(self.to, 1.)];
            }
            let mix = p * p * (3. - 2. * p);
            vec![(self.from, 1. - mix), (self.to, mix)]
        }
    }

    /// A margined grid's slots on the gallery surface. Slots are counted from
    /// the centre column (`u`) and from the zoom anchor's row (`v`); slot
    /// (u, v) is the square of side `edge` centred at
    /// (`x + u * column`, `y + v * row`). Every margined level is one of
    /// these, so two of them blend into a third, and any level's photos can
    /// be laid into any of them.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Lattice {
        pub x: f32,
        pub y: f32,
        pub column: f32,
        pub row: f32,
        pub edge: f32,
    }
    impl Lattice {
        /// `frame`'s grid as drawn, with `anchor`'s row as row 0.
        pub fn of(frame: &GridFrame, anchor: usize) -> Self {
            let l = &frame.layout;
            let column = l.edge + l.gap;
            let local = (l.count.max(1) - 1).min(anchor) + l.leading;
            Self {
                x: l.padding + (l.columns - 1) as f32 / 2. * column + l.edge / 2.,
                y: frame.header - frame.offset + (local / l.columns) as f32 * l.pitch + l.edge / 2.,
                column,
                row: l.pitch,
                edge: l.edge,
            }
        }
        pub fn lerp(self, other: Self, t: f32) -> Self {
            let mix = |a: f32, b: f32| a + (b - a) * t;
            Self {
                x: mix(self.x, other.x),
                y: mix(self.y, other.y),
                column: mix(self.column, other.column),
                row: mix(self.row, other.row),
                edge: mix(self.edge, other.edge),
            }
        }
        /// Scaled by `s` about `center`.
        pub fn scaled(self, (cx, cy): (f32, f32), s: f32) -> Self {
            Self {
                x: cx + (self.x - cx) * s,
                y: cy + (self.y - cy) * s,
                column: self.column * s,
                row: self.row * s,
                edge: self.edge * s,
            }
        }
        /// Slot (u, v).
        pub fn slot(&self, u: f32, v: f32) -> Rect {
            Rect {
                x: self.x + u * self.column - self.edge / 2.,
                y: self.y + v * self.row - self.edge / 2.,
                w: self.edge,
                h: self.edge,
            }
        }
    }
    /// The lattice margined photos are laid into at ladder `position`: the
    /// blend of the margined levels around it, or past them, the nearest
    /// one's scaled about `center` to the photo size there. `lattice` gives
    /// a margined slot's own lattice.
    pub fn lattice_at(
        ladder: &Ladder,
        position: f32,
        center: (f32, f32),
        mut lattice: impl FnMut(usize) -> Lattice,
    ) -> Option<Lattice> {
        let (a, b, t) = ladder.lattice_pair(position)?;
        let blend = if a == b {
            lattice(a)
        } else {
            lattice(a).lerp(lattice(b), t)
        };
        let s = ladder.edge(position) / blend.edge;
        Some(if (s - 1.).abs() < 1e-5 {
            blend
        } else {
            blend.scaled(center, s)
        })
    }

    /// How one level's photos are placed at the zoom's current size.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Placement {
        /// Margined levels: each photo in its slot of this lattice.
        Slots(Lattice),
        /// Date grids: the whole grid scaled by this about the zoom's centre.
        Scaled(f32),
    }

    /// A section header as drawn during a zoom.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Header {
        pub rect: Rect,
        pub tier: Tier,
        pub key: i32,
        /// Text scale (1 at natural size).
        pub scale: f32,
    }

    /// One level's photos while zooming: where each is drawn, which can be
    /// on screen, and the level's section headers.
    pub struct Layer<'a> {
        pub frame: &'a GridFrame,
        pub placement: Placement,
        /// The point the zoom scales about, on the gallery surface.
        pub center: (f32, f32),
        /// The anchor photo's row in this level (slot row 0).
        anchor_row: usize,
    }
    impl<'a> Layer<'a> {
        pub fn new(
            frame: &'a GridFrame,
            placement: Placement,
            center: (f32, f32),
            anchor: usize,
        ) -> Self {
            let l = &frame.layout;
            let anchor_row = ((l.count.max(1) - 1).min(anchor) + l.leading) / l.columns;
            Self {
                frame,
                placement,
                center,
                anchor_row,
            }
        }
        fn place(&self, r: Rect, s: f32) -> Rect {
            if s == 1. {
                return r;
            }
            let (cx, cy) = self.center;
            Rect {
                x: cx + (r.x - cx) * s,
                y: cy + (r.y - cy) * s,
                w: r.w * s,
                h: r.h * s,
            }
        }
        /// Where photo `index` (of `width / height` aspect) is drawn on the
        /// gallery surface.
        pub fn frame(&self, index: usize, aspect: f32) -> Rect {
            let l = &self.frame.layout;
            match self.placement {
                Placement::Slots(lattice) => {
                    let local = index + l.leading;
                    let u = (local % l.columns) as f32 - (l.columns - 1) as f32 / 2.;
                    let v = (local / l.columns) as f32 - self.anchor_row as f32;
                    let slot = lattice.slot(u, v);
                    if l.full_bleed() {
                        slot
                    } else {
                        slot.fit(aspect)
                    }
                }
                Placement::Scaled(s) => {
                    self.place(self.frame.screen_rect(l.frame(index, aspect)), s)
                }
            }
        }
        /// Corner radius of a photo as drawn.
        pub fn radius(&self) -> f32 {
            let l = &self.frame.layout;
            match self.placement {
                Placement::Slots(lattice) => l.radius * lattice.edge / l.edge,
                Placement::Scaled(s) => l.radius * s,
            }
        }
        /// The content rows of a scaled grid that can reach the surface
        /// `0..area`.
        fn span(&self, s: f32, area: f32) -> (f32, f32) {
            let (_, cy) = self.center;
            let f = self.frame;
            let margin = f.layout.pitch;
            (
                (cy - cy / s - f.header + f.offset - margin).max(0.),
                cy + (area - cy) / s - f.header + f.offset + margin,
            )
        }
        /// Photos that can be on the surface `0..area`, in order.
        pub fn visible(&self, area: f32) -> Vec<Range<usize>> {
            let l = &self.frame.layout;
            match self.placement {
                Placement::Slots(lattice) => {
                    if l.count == 0 || lattice.row <= 0.01 {
                        return vec![];
                    }
                    let rows = (l.count + l.leading).div_ceil(l.columns) as f32;
                    let half = lattice.edge / 2.;
                    let anchor = self.anchor_row as f32;
                    let first = (anchor + ((-half - lattice.y) / lattice.row).floor()).max(0.);
                    let last = (anchor + ((area + half - lattice.y) / lattice.row).ceil())
                        .min(rows - 1.);
                    if last < first {
                        return vec![];
                    }
                    let start = (first as usize * l.columns).saturating_sub(l.leading);
                    let end = ((last as usize + 1) * l.columns)
                        .saturating_sub(l.leading)
                        .min(l.count);
                    if start < end { vec![start..end] } else { vec![] }
                }
                Placement::Scaled(s) => {
                    let (top, bottom) = self.span(s, area);
                    l.visible(top, bottom)
                }
            }
        }
        /// Section headers on the surface, scaled with their photos; the
        /// one pinned at the top stays where it is pinned.
        pub fn headers(&self, area: f32) -> Vec<Header> {
            let l = &self.frame.layout;
            let Placement::Scaled(s) = self.placement else {
                return vec![];
            };
            if !l.sectioned() {
                return vec![];
            }
            let (top, bottom) = self.span(s, area);
            let sticky = l.sticky(self.frame.offset);
            l.sections_in(top, bottom)
                .map(|i| {
                    let y = match sticky {
                        Some((pinned, y)) if pinned == i => y,
                        _ => l.sections[i].top,
                    };
                    let rect = self.frame.screen_rect(Rect {
                        x: 0.,
                        y,
                        w: l.width,
                        h: l.header,
                    });
                    Header {
                        rect: self.place(rect, s),
                        tier: l.tier,
                        key: l.sections[i].key,
                        scale: s,
                    }
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn months(lens: &[usize]) -> Vec<Group> {
        let mut start = 0;
        lens.iter()
            .enumerate()
            .map(|(i, &len)| {
                let g = Group {
                    start,
                    len,
                    key: month_key(2026, 0) - i as i32,
                };
                start += len;
                g
            })
            .collect()
    }
    #[test]
    fn levels_step_out_three_five_seven_nine_then_jump_into_date_sections() {
        for width in [312., 640., 844., 1224., 1800.] {
            let mut last = 0;
            for level in (0..=GalleryLayout::MAX_LEVEL).rev() {
                let l = GalleryLayout::new(width, level);
                assert!(l.columns > last, "{width}: level {level} must add columns");
                last = l.columns;
                assert!(l.edge > 0. && l.padding * 2. + l.edge <= width + 1.);
                assert_eq!(
                    l.tier,
                    match level {
                        0..=2 => Tier::Years,
                        3..=5 => Tier::Months,
                        _ => Tier::All,
                    }
                );
                assert_eq!(l.full_bleed(), level <= 5);
                assert_eq!(l.dense(), level <= 5);
                assert_eq!(GalleryLayout::margined(level), level >= 6);
            }
            let columns = |level| GalleryLayout::new(width, level).columns;
            assert_eq!((6..=9).map(columns).collect::<Vec<_>>(), [9, 7, 5, 3]);
            assert!(columns(5) > 13, "{width}: leaving 9 columns is a big step");
        }
        let default = GalleryLayout::new(1000., GalleryLayout::DEFAULT_LEVEL);
        assert_eq!(default.columns, 5);
        assert!(default.gap > 4. && default.padding > 8.);
        let ratio =
            |a: usize, b: usize| GalleryLayout::new(1000., a).edge / GalleryLayout::new(1000., b).edge;
        assert!(
            ratio(4, 3) > 1.25 && ratio(1, 0) > 1.25 && ratio(6, 5) > 1.6,
            "dense tiers and the step into them zoom in big steps"
        );
    }
    #[test]
    fn bottom_rows_never_reflow_when_older_catalog_pages_arrive() {
        for level in 6..=GalleryLayout::MAX_LEVEL {
            for count in 20..120 {
                let before = GalleryLayout::new(844., level).with_count(count, true);
                for added in [1, 2, 17, 197, 200, 3200] {
                    let after =
                        GalleryLayout::new(844., level).with_count(count + added, true);
                    for age in 0..20 {
                        let a = before.cell(count - 1 - age);
                        let b = after.cell(count + added - 1 - age);
                        assert!((a.x - b.x).abs() < 0.01);
                        assert!(
                            ((a.y - before.height) - (b.y - after.height)).abs() < 0.1,
                            "latest row must not drop: {count}, {added}, {level}"
                        );
                    }
                    let all = after
                        .visible(0., after.height)
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>();
                    assert_eq!(all, (0..count + added).collect::<Vec<_>>());
                }
            }
        }
    }
    #[test]
    fn month_sections_start_new_rows_with_headers_and_gaps() {
        let groups = months(&[3, 40, 1, 25]);
        let l = GalleryLayout::new(1000., 4).with_groups(&groups, 69, false);
        assert_eq!(l.tier, Tier::Months);
        assert_eq!(l.sections.len(), 4);
        for (g, s) in groups.iter().zip(l.sections.iter()) {
            let first = l.cell(g.start);
            assert_eq!(first.x, 0., "every month starts a new row");
            assert!((first.y - s.top - l.header).abs() < 0.01);
            let last = l.cell(g.start + g.len - 1);
            if let Some(next) = l.sections.iter().find(|n| n.start == g.start + g.len) {
                assert!((next.top - (last.y + l.edge) - l.section_gap).abs() < 0.1);
            }
        }
        assert!((l.height - (l.cell(68).y + l.edge)).abs() < 0.1);
        // Visible ranges, hits and nearest stay inside sections.
        let all = l
            .visible(0., l.height)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(all, (0..69).collect::<Vec<_>>());
        for i in [0, 2, 3, 42, 43, 44, 68] {
            let c = l.cell(i);
            assert_eq!(l.hit(c.x + 1., c.y + 1.), Some(i));
            assert_eq!(l.nearest(c.x + c.edge / 2., c.y + c.edge / 2.), Some(i));
        }
        let header = l.sections[1];
        assert_eq!(l.hit(500., header.top + 5.), None, "headers are not photos");
        // Years merge consecutive months of the same year.
        let years = years_from_months(&[
            Group {
                start: 0,
                len: 3,
                key: month_key(2026, 4),
            },
            Group {
                start: 3,
                len: 2,
                key: month_key(2026, 1),
            },
            Group {
                start: 5,
                len: 4,
                key: month_key(2025, 11),
            },
        ]);
        assert_eq!(
            years,
            vec![
                Group {
                    start: 0,
                    len: 5,
                    key: 2026
                },
                Group {
                    start: 5,
                    len: 4,
                    key: 2025
                }
            ]
        );
    }
    #[test]
    fn margined_photos_keep_their_aspect_ratio_and_gapless_ones_are_square() {
        let margined = GalleryLayout::new(1000., GalleryLayout::DEFAULT_LEVEL).with_count(10, false);
        let cell = margined.cell(3);
        let wide = margined.frame(3, 1.5);
        assert!((wide.w - cell.edge).abs() < 0.01 && (wide.w / wide.h - 1.5).abs() < 0.01);
        assert!(
            (wide.y + wide.h / 2. - (cell.y + cell.edge / 2.)).abs() < 0.01,
            "centred"
        );
        let tall = margined.frame(3, 0.75);
        assert!((tall.h - cell.edge).abs() < 0.01 && (tall.w / tall.h - 0.75).abs() < 0.01);
        let gapless = GalleryLayout::new(1000., 5).with_count(10, false);
        let square = gapless.frame(3, 1.5);
        assert_eq!(square.w, square.h);
    }
    #[test]
    fn sticky_header_is_pushed_off_by_the_next_section() {
        let l = GalleryLayout::new(1000., 3).with_groups(&months(&[60, 60]), 120, false);
        let next = l.sections[1].top;
        assert_eq!(l.sticky(0.), Some((0, 0.)));
        assert_eq!(l.sticky(10.), Some((0, 10.)));
        let (i, y) = l.sticky(next - 10.).unwrap();
        assert_eq!(i, 0);
        assert!((y - (next - l.header)).abs() < 0.01);
        assert_eq!(l.sticky(next + 1.), Some((1, next + 1.)));
    }
    #[test]
    fn incremental_and_reversed_groups_match_a_full_rebuild() {
        let keys: Vec<i32> = (0..500).map(|i| 30_000 - (i / 37)).collect();
        let mut groups = vec![];
        extend_groups(&mut groups, 0, keys[..200].iter().copied());
        extend_groups(&mut groups, 200, keys[200..].iter().copied());
        let mut full = vec![];
        extend_groups(&mut full, 0, keys.iter().copied());
        assert_eq!(groups, full);
        let reversed = reverse_groups(&full, 500);
        let mut rebuilt = vec![];
        extend_groups(&mut rebuilt, 0, keys.iter().rev().copied());
        assert_eq!(reversed, rebuilt);
    }
    #[test]
    fn ladder_grows_photos_at_every_width_and_keeps_the_level_on_screen() {
        use zoom::*;
        for width in [312., 480., 640., 844., 1224., 1800.] {
            for keep in 0..=GalleryLayout::MAX_LEVEL {
                let ladder = Ladder::for_width(width, keep);
                assert_eq!(ladder.levels, (0..=GalleryLayout::MAX_LEVEL).collect::<Vec<_>>());
                assert!(
                    ladder.edges.windows(2).all(|w| w[1] > w[0] + 0.5),
                    "{width}: photos grow along the ladder"
                );
                assert_eq!(ladder.margined(), 6..10);
                // Edge and position are inverses; a pinch scales the photos
                // by exactly its factor.
                for step in 0..=(ladder.max() * 10.) as usize {
                    let p = step as f32 / 10.;
                    assert!((ladder.locate(ladder.edge(p)) - p).abs() < 1e-3);
                }
            }
        }
        let ladder = Ladder::for_width(1000., GalleryLayout::DEFAULT_LEVEL);
        let now = std::time::Instant::now();
        let mut z = Zoom::new(8, now);
        let before = ladder.edge(z.position);
        z.apply(&ladder, Input::Magnify(1.1), true, now);
        assert!((ladder.edge(z.position) / before - 1.1).abs() < 1e-3);
        assert_eq!(z.phase, Phase::Held);
        assert_eq!(ladder.pair(7.25), (7, 8, 0.25));
        assert_eq!(ladder.pair(9.), (8, 9, 1.));
        assert_eq!(ladder.pair(-0.2), (0, 1, 0.));
        assert_eq!(ladder.lattice_pair(7.25), Some((7, 8, 0.25)));
        assert_eq!(ladder.lattice_pair(3.), Some((6, 7, 0.)));
        assert_eq!(ladder.lattice_pair(9.3), Some((8, 9, 1.)));
    }
    fn close(p: Rect, q: Rect) -> bool {
        // f32 positions far down a long grid carry a little rounding.
        (p.x - q.x).abs() < 0.01
            && (p.y - q.y).abs() < 0.01 + q.y.abs() * 1e-6
            && (p.w - q.w).abs() < 0.01
            && (p.h - q.h).abs() < 0.01
    }
    #[test]
    fn margined_levels_share_one_centred_lattice_and_rows_widen_about_the_centre() {
        use zoom::*;
        let (w, area, count, anchor) = (1000., 700., 4000, 1500);
        let center = (w / 2., 380.);
        for bottom_up in [false, true] {
            let frame = |level: usize| {
                let layout = GalleryLayout::new(w, level).with_count(count, bottom_up);
                let title = if layout.full_bleed() { 0. } else { 84. };
                GridFrame::anchored(layout, Some((anchor, 0.5)), center.1, area, title, bottom_up, false)
            };
            // Each margined level's own lattice is exactly its grid.
            for level in 6..=9 {
                let f = frame(level);
                let lattice = Lattice::of(&f, anchor);
                assert!((lattice.x - center.0).abs() < 0.01, "the middle column is centred");
                assert!((lattice.y - center.1).abs() < 0.5, "the anchor's row is at the centre");
                let layer = Layer::new(&f, Placement::Slots(lattice), center, anchor);
                for i in [0, 1, 1499, 1500, 1501, 3998, 3999] {
                    let (p, q) = (layer.frame(i, 1.5), f.screen_rect(f.layout.frame(i, 1.5)));
                    assert!(close(p, q), "{level} {bottom_up} {i}: {p:?} {q:?} {lattice:?}");
                }
                // Everything that reaches the surface is listed, in order.
                let drawn: Vec<usize> = layer.visible(area).into_iter().flatten().collect();
                assert!(drawn.windows(2).all(|w| w[0] < w[1]));
                for i in 0..count {
                    let r = layer.frame(i, 1.);
                    if r.y + r.h > 0. && r.y < area {
                        assert!(drawn.contains(&i), "{level}: {i} is on screen");
                    }
                }
            }
            // From 3 columns to 5, the 5-column grid's middle slots are the
            // 3-column grid's: a slot appears at each end of every row.
            let (three, five) = (frame(9), frame(8));
            let (l3, l5) = (Lattice::of(&three, anchor), Lattice::of(&five, anchor));
            let row_start = |f: &GridFrame| {
                let l = &f.layout;
                ((anchor + l.leading) / l.columns * l.columns).saturating_sub(l.leading)
            };
            let (s3, s5) = (row_start(&three), row_start(&five));
            for t in [0., 0.25, 0.5, 0.75, 1.] {
                let lattice = l3.lerp(l5, t);
                assert!((lattice.x - center.0).abs() < 0.01);
                assert!((lattice.y - center.1).abs() < 0.5, "rows close up about the centre");
                let a = Layer::new(&three, Placement::Slots(lattice), center, anchor);
                let b = Layer::new(&five, Placement::Slots(lattice), center, anchor);
                for column in 0..3 {
                    let (r3, r5) = (a.frame(s3 + column, 1.), b.frame(s5 + column + 1, 1.));
                    assert!((r3.x - r5.x).abs() < 0.01 && (r3.y - r5.y).abs() < 0.01);
                }
            }
            // Past the margined levels the lattice scales about the centre.
            let ladder = Ladder::for_width(w, 9);
            let lattice = |slot: usize| Lattice::of(&frame(ladder.levels[slot]), anchor);
            let out = lattice_at(&ladder, 5.5, center, lattice).unwrap();
            assert!((out.edge - ladder.edge(5.5)).abs() < 0.01);
            assert!((out.x - center.0).abs() < 0.01 && (out.y - center.1).abs() < 0.5);
            let stretched = lattice_at(&ladder, 9.2, center, lattice).unwrap();
            assert!(stretched.edge > l3.edge);
            let between = lattice_at(&ladder, 8.5, center, lattice).unwrap();
            assert!(close(between.slot(1., 2.), l5.lerp(l3, 0.5).slot(1., 2.)));
        }
    }
    #[test]
    fn date_grids_scale_whole_about_the_centre_with_their_headers() {
        use zoom::*;
        let (w, area, center) = (1000., 700., (500., 350.));
        let groups = months(&[370, 800, 30, 1200, 1600]);
        let layout = GalleryLayout::new(w, 4).with_groups(&groups, 4000, false);
        let f = GridFrame::anchored(layout, Some((1500, 0.5)), center.1, area, 0., false, false);
        let rest = Layer::new(&f, Placement::Scaled(1.), center, 1500);
        let grown = Layer::new(&f, Placement::Scaled(2.), center, 1500);
        for i in [0, 1499, 1500, 3999] {
            let (a, b) = (rest.frame(i, 1.), grown.frame(i, 1.));
            assert!(close(a, f.screen_rect(f.layout.frame(i, 1.))));
            assert!((b.w - 2. * a.w).abs() < 0.01);
            assert!((b.x - (center.0 + (a.x - center.0) * 2.)).abs() < 0.01);
            assert!((b.y - (center.1 + (a.y - center.1) * 2.)).abs() < 0.01);
        }
        let all: Vec<usize> = rest.visible(area).into_iter().flatten().collect();
        let fewer: Vec<usize> = grown.visible(area).into_iter().flatten().collect();
        assert!(fewer.len() < all.len() && fewer.iter().all(|i| all.contains(i)));
        let headers = grown.headers(area);
        assert!(!headers.is_empty() && headers.iter().all(|h| h.scale == 2. && h.tier == Tier::Months));
        assert!(rest.radius() == 0. && grown.radius() == 0.);
    }
    #[test]
    fn photos_crossfade_level_by_level_on_a_fixed_clock() {
        use std::time::{Duration, Instant};
        use zoom::*;
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let fade = FADE.as_millis() as u64;
        let mut f = Crossfade::new(8, t0);
        assert_eq!(f.layers(t0), vec![(8, 1.)]);
        // Two levels away: one level at a time, each waiting for the last.
        f.aim(6, ms(0));
        assert_eq!((f.from, f.to), (8, 7));
        f.aim(6, ms(fade / 2));
        assert_eq!((f.from, f.to), (8, 7));
        let mid = f.layers(ms(fade / 2));
        assert_eq!((mid[0].0, mid[1].0), (8, 7), "outgoing drawn first");
        assert!((mid[1].1 - 0.5).abs() < 0.01 && (mid[0].1 + mid[1].1 - 1.).abs() < 1e-5);
        f.aim(6, ms(fade));
        assert_eq!((f.from, f.to), (7, 6));
        // Turning back mid-fade reverses it without a jump in opacity.
        let at = ms(fade + fade / 3);
        let before = f.layers(at);
        f.aim(7, at);
        let after = f.layers(at);
        assert_eq!((after[0].0, after[1].0), (6, 7));
        assert!((after[1].1 - before[0].1).abs() < 1e-3);
        assert!(f.fading(at) && f.shown(at).is_none());
        f.aim(7, ms(10 * fade));
        assert_eq!(f.shown(ms(10 * fade)), Some(7));
        assert_eq!(f.layers(ms(10 * fade)), vec![(7, 1.)]);
    }
    #[test]
    fn gestures_head_to_the_next_level_at_once_and_release_commits_past_a_little() {
        use std::time::{Duration, Instant};
        use zoom::*;
        let ladder = Ladder::for_width(1000., 8);
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        // A phased pinch holds wherever the fingers rest, and the photos head
        // to the next level as soon as it sets off.
        let mut z = Zoom::new(8, t0);
        z.apply(&ladder, Input::Magnify(0.95), true, ms(0));
        let held = z.position;
        assert!(held < 8. && held > 7.);
        assert_eq!((z.heading, z.goal(8, &ladder)), (-1., 7));
        for n in 1..60 {
            assert!(!z.tick(&ladder, ms(n * 16)));
        }
        assert_eq!(z.position, held);
        z.apply(&ladder, Input::Release, false, ms(1000));
        assert_eq!(z.phase, Phase::Settling { target: 7 }, "went far enough to land");
        // A tremor changes nothing; a short pinch springs back.
        let mut z = Zoom::new(8, t0);
        z.apply(&ladder, Input::Magnify(0.995), true, ms(0));
        assert_eq!((z.heading, z.goal(8, &ladder)), (0., 8));
        let mut z = Zoom::new(8, t0);
        z.apply(&ladder, Input::Magnify(0.98), true, ms(0));
        assert_eq!(z.goal(8, &ladder), 7);
        z.apply(&ladder, Input::Release, false, ms(1000));
        assert_eq!((z.phase, z.goal(7, &ladder)), (Phase::Settling { target: 8 }, 8));
        // Turning around heads back; going on past a level heads to the next.
        let mut z = Zoom::new(8, t0);
        z.apply(&ladder, Input::Magnify(0.85), true, ms(0));
        assert_eq!(z.goal(7, &ladder), 7);
        z.apply(&ladder, Input::Magnify(1.08), true, ms(10));
        assert_eq!((z.heading, z.goal(7, &ladder)), (1., 8));
        let mut z = Zoom::new(8, t0);
        z.apply(&ladder, Input::Magnify(0.7), true, ms(0));
        assert!(z.position < 6.95);
        assert_eq!(z.goal(7, &ladder), 6);
        // A flick carries on to the next level.
        let mut flick = Zoom::new(8, t0);
        for n in 0..3 {
            flick.apply(&ladder, Input::Magnify(0.995), true, ms(n * 10));
        }
        flick.apply(&ladder, Input::Release, false, ms(30));
        assert!(flick.velocity < 0.);
        assert_eq!(flick.phase, Phase::Settling { target: 7 });
        // Unphased input settles once quiet.
        let mut wheel = Zoom::new(8, t0);
        wheel.apply(&ladder, Input::Magnify(0.9), false, ms(0));
        assert!(wheel.tick(&ladder, ms(100)));
        assert_eq!(wheel.phase, Phase::Held);
        wheel.tick(&ladder, ms(250));
        assert!(matches!(wheel.phase, Phase::Settling { .. }));
        // The spring settles in about a third of a second without overshoot.
        let (mut v, mut vel) = (7.6, 0.);
        let mut t = 0.;
        while !at_rest(v, vel, 8.) {
            (v, vel) = spring(v, vel, 8., 1. / 120.);
            assert!(v <= 8.0001, "no overshoot");
            t += 1. / 120.;
        }
        assert!(t > 0.15 && t < 0.6, "settled in {t}s");
        // Steps animate; a second one retargets without losing momentum.
        let mut z = Zoom::new(8, t0);
        z.apply(&ladder, Input::Step(-1), false, ms(0));
        assert_eq!(z.goal(8, &ladder), 7, "the photos change at once");
        let mut n = 0;
        while z.tick(&ladder, ms(16 * (n + 1))) {
            n += 1;
            if n == 3 {
                let velocity = z.velocity;
                z.apply(&ladder, Input::Step(-1), false, ms(16 * n));
                assert_eq!(z.velocity, velocity);
                assert_eq!(z.phase, Phase::Settling { target: 6 });
            }
        }
        assert_eq!(z.settled(), Some(6));
        // Grabbing a settling zoom continues from what is on screen.
        let mut z = Zoom::new(8, t0);
        z.apply(&ladder, Input::Step(-2), false, ms(0));
        z.tick(&ladder, ms(16));
        z.tick(&ladder, ms(32));
        let here = z.position;
        z.apply(&ladder, Input::Magnify(1.), true, ms(40));
        assert_eq!((z.position, z.velocity, z.phase), (here, 0., Phase::Held));
        // Never closer than three columns: gestures only stretch a little.
        let mut v = ladder.max();
        for _ in 0..100 {
            v = rubber_band(v, 0.1, ladder.max());
        }
        assert!((ladder.max()..=ladder.max() + OVERSCROLL).contains(&v));
        assert_eq!(release_target(9.3, 0., 1., 9.), 9.);
        assert_eq!(release_target(7.6, 0., 0., 9.), 8.);
        assert_eq!(release_target(7.6, -40., 0., 9.), 7.);
        assert_eq!(release_target(7.8, 0., -1., 9.), 7.);
        assert_eq!(release_target(7.9, 0., -1., 9.), 8.);
    }
}
