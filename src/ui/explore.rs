//! Explore (people and places), photos of a person or place, and the
//! source's own search (Immich's smart search) for the search field.
use super::*;
use std::collections::HashSet;

const PEOPLE_PREVIEW: usize = 24;
const SEARCH_DELAY: Duration = Duration::from_millis(350);

impl PhotosApp {
    pub(super) fn explore_supported(&self) -> bool {
        self.driver.as_ref().is_some_and(|d| d.explore())
    }
    /// Loads people and places when Explore opens (and each time after a
    /// reconnect, which clears them).
    pub(super) fn load_explore(&mut self, cx: &mut Context<Self>) {
        if self.explore.is_some() || self.explore_pending {
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        self.explore_pending = true;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let people = driver.people();
                    let places = driver.places();
                    (people, places)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.explore_pending = false;
                let (people, places) = result;
                if let Err(e) = people.as_ref().and(places.as_ref()) {
                    this.error = Some(format!("Cannot load Explore: {e}"));
                }
                this.explore = Some(Explore {
                    people: people.unwrap_or_default(),
                    places: places.unwrap_or_default(),
                });
                cx.notify();
            });
        })
        .detach();
    }
    /// Fetches the photo IDs of a person or place filter.
    pub(super) fn load_query(&mut self, filter: &Filter, cx: &mut Context<Self>) {
        let query = match filter {
            Filter::Person(id, _) => storage::Query::Person(id.clone()),
            Filter::Place(name) => storage::Query::Place(name.clone()),
            _ => return,
        };
        if self.query_ids.as_ref().is_some_and(|(f, _)| f == filter)
            || self.query_pending.as_ref() == Some(filter)
        {
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        self.query_pending = Some(filter.clone());
        let filter = filter.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { driver.matching(&query) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.query_pending.as_ref() != Some(&filter) {
                    return;
                }
                this.query_pending = None;
                match result {
                    Ok(ids) => {
                        this.query_ids = Some((filter, Arc::new(ids.into_iter().collect())));
                        this.revision += 1;
                        this.reset_gallery();
                    }
                    Err(e) => this.error = Some(format!("Cannot load these photos: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// Asks the source's own search (after a pause in typing); its matches
    /// join the name and description matches.
    pub(super) fn search_source(&mut self, cx: &mut Context<Self>) {
        self.search_epoch += 1;
        let epoch = self.search_epoch;
        let text = self.search.read(cx).value().trim().to_string();
        if text.is_empty() || !self.explore_supported() {
            if self.remote_search.take().is_some() {
                self.revision += 1;
            }
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            if this
                .update(cx, |this, _| this.search_epoch != epoch)
                .unwrap_or(true)
            {
                return;
            }
            let query = text.clone();
            let result = cx
                .background_executor()
                .spawn(async move { driver.search(&query) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.search_epoch != epoch {
                    return;
                }
                if let Ok(Some(ids)) = result {
                    this.remote_search = Some((
                        text.to_lowercase(),
                        Arc::new(ids.into_iter().collect::<HashSet<_>>()),
                    ));
                    this.revision += 1;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    /// A video in the system's player: the file itself, or a copy fetched
    /// from the server first.
    pub(super) fn play_externally(&mut self, photo: Photo, w: &mut Window, cx: &mut Context<Self>) {
        let Some(driver) = self.driver.clone() else {
            return;
        };
        self.notice = "Opening video…".into();
        cx.spawn_in(w, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { driver.video_file(&photo) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(path) => {
                        this.notice.clear();
                        cx.open_with_system(&path);
                    }
                    Err(e) => this.error = Some(format!("Cannot open this video: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn place_photo(&self, place: &storage::Place) -> Arc<Photo> {
        self.photo_index
            .get(&place.photo)
            .and_then(|&i| self.library.photos.get(i))
            .cloned()
            .unwrap_or_else(|| {
                Arc::new(Photo {
                    id: place.photo.clone(),
                    path: PathBuf::new(),
                    name: place.name.clone(),
                    modified: Default::default(),
                    bytes: 0,
                    width: 0,
                    height: 0,
                    favorite: false,
                    deleted: false,
                    caption: String::new(),
                    fingerprint: format!("place-{}", place.photo),
                    duration: None,
                })
            })
    }
    pub(super) fn explore_view(&mut self, c: Colors, cx: &mut Context<Self>) -> AnyElement {
        self.load_explore(cx);
        let mut content = div()
            .id("explore")
            .debug_selector(|| "explore".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .flex()
            .flex_col()
            .gap_5();
        let Some(explore) = self.explore.clone() else {
            return content
                .child(kit::empty("explore", "Loading people and places…", "", c))
                .into_any_element();
        };
        if explore.people.is_empty() && explore.places.is_empty() {
            return content
                .child(kit::empty(
                    "explore",
                    "Nothing to Explore Yet",
                    "People appear once your server recognises faces; places once photos have locations.",
                    c,
                ))
                .into_any_element();
        }
        let heading = |title: &str, action: Option<AnyElement>| {
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(17.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_owned()),
                )
                .children(action)
        };
        if !explore.people.is_empty() {
            let shown = if self.all_people {
                explore.people.len()
            } else {
                PEOPLE_PREVIEW.min(explore.people.len())
            };
            let toggle = (explore.people.len() > PEOPLE_PREVIEW).then(|| {
                ui::button("explore-all-people", c, false, true)
                    .child(if self.all_people {
                        "Show Less".to_owned()
                    } else {
                        format!("View All {}", explore.people.len())
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.all_people = !this.all_people;
                        cx.notify();
                    }))
                    .into_any_element()
            });
            let mut people = div().flex().flex_wrap().gap_4();
            for person in &explore.people[..shown] {
                let path = self.request_face(&person.id, cx);
                let (id, name) = (person.id.clone(), person.name.clone());
                people = people.child(
                    div()
                        .id(SharedString::from(format!("explore-person-{}", person.id)))
                        .w(px(96.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .rounded(px(10.))
                        .p_1()
                        .hover(move |s| s.bg(rgb(c.soft)))
                        .child(info::face(path, 80., c))
                        .child(
                            div()
                                .w_full()
                                .text_center()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(rgb(if person.name.is_empty() {
                                    c.muted
                                } else {
                                    c.text
                                }))
                                .child(if person.name.is_empty() {
                                    "Unnamed".to_owned()
                                } else {
                                    person.name.clone()
                                }),
                        )
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.set_filter(Filter::Person(id.clone(), name.clone()), w, cx)
                        })),
                );
            }
            content = content.child(heading("People", toggle)).child(people);
        }
        if !explore.places.is_empty() {
            let mut places = div().flex().flex_wrap().gap_3();
            for place in &explore.places {
                let photo = self.place_photo(place);
                let thumbnail = self.thumbnails.get(&photo.fingerprint).cloned();
                if thumbnail.is_none() {
                    self.request_image(&photo, false, cx);
                }
                let name = place.name.clone();
                places = places.child(
                    div()
                        .id(SharedString::from(format!("explore-place-{}", place.name)))
                        .relative()
                        .size(px(150.))
                        .rounded(px(10.))
                        .overflow_hidden()
                        .bg(rgb(c.soft))
                        .cursor_pointer()
                        .when_some(thumbnail, |d, path| {
                            d.child(
                                img(Arc::<std::path::Path>::from(path))
                                    .size_full()
                                    .object_fit(ObjectFit::Cover),
                            )
                        })
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .px_2()
                                .py_1()
                                .bg(rgba(0x00000088))
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(0xffffff))
                                .truncate()
                                .child(place.name.clone()),
                        )
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.set_filter(Filter::Place(name.clone()), w, cx)
                        })),
                );
            }
            content = content.child(heading("Places", None)).child(places);
        }
        content.into_any_element()
    }
    /// The bar above every view: what you are looking at (with a way back
    /// for a person or place), then the dates of the photos on screen.
    pub(super) fn view_header(&self, c: Colors, cx: &mut Context<Self>) -> Stateful<Div> {
        let searching = !self.search.read(cx).value().trim().is_empty();
        let title = if searching {
            "Search Results".to_owned()
        } else {
            match &self.filter {
                Filter::All => "Library".into(),
                Filter::Videos => "Videos".into(),
                Filter::Favorites => "Favorites".into(),
                Filter::Recent => "Last 30 Days".into(),
                Filter::Trash => "Recently Deleted".into(),
                Filter::Album(name) => name.clone(),
                Filter::Explore => "Explore".into(),
                Filter::Person(_, name) if !name.is_empty() => name.clone(),
                Filter::Person(..) => "Unnamed Person".into(),
                Filter::Place(name) => name.clone(),
            }
        };
        let back = matches!(self.filter, Filter::Person(..) | Filter::Place(_));
        let loading = self.query_pending.as_ref() == Some(&self.filter);
        let range = (self.filter != Filter::Explore)
            .then(|| self.visible_dates(cx))
            .flatten();
        div()
            .id("view-header")
            .debug_selector(|| "view-header".into())
            .h(px(40.))
            .flex_shrink_0()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(c.border))
            .when(back, |d| {
                d.child(
                    div()
                        .id("back-to-explore")
                        .debug_selector(|| "back-to-explore".into())
                        .size(px(24.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .child(kit::glyph("back", 16., c.muted))
                        .tooltip(|w, cx| {
                            gpui_component::tooltip::Tooltip::new("Back to Explore").build(w, cx)
                        })
                        .on_click(
                            cx.listener(|this, _, w, cx| this.set_filter(Filter::Explore, w, cx)),
                        ),
                )
            })
            .child(
                div()
                    .min_w_0()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(title),
            )
            .when_some(range, |d, range| {
                d.child(div().w(px(1.)).h(px(14.)).flex_shrink_0().bg(rgb(c.border)))
                    .child(
                        div()
                            .id("view-dates")
                            .debug_selector(|| "view-dates".into())
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child(range),
                    )
            })
            .when(loading, |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(c.muted))
                        .child("Loading…"),
                )
            })
    }
    /// "Oct 25th, 2024 – Nov 15th, 2024" for the photos on screen.
    fn visible_dates(&self, cx: &App) -> Option<String> {
        let photos = self.visible(cx);
        if photos.is_empty() {
            return None;
        }
        let layout = self.gallery_layout(cx);
        let scroll = self.scroll_handle();
        let height = f32::from(scroll.bounds().size.height);
        let top = f32::from(-scroll.offset().y).max(0.);
        // Before the grid's first layout, the top of the view stands in.
        let bottom = top
            + if height > 1. {
                height
            } else {
                self.gallery_height.max(400.)
            };
        let mut range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> =
            None;
        for span in layout.visible(top, bottom) {
            if span.is_empty() {
                continue;
            }
            // Photos are in date order, so each span's ends bound it.
            for i in [span.start, span.end - 1] {
                if let Some(photo) = photos.get(&self.library, i) {
                    let date = photo.modified;
                    range = Some(match range {
                        Some((a, b)) => (a.min(date), b.max(date)),
                        None => (date, date),
                    });
                }
            }
        }
        let (first, last) = range?;
        let day = |d: chrono::DateTime<chrono::Utc>| {
            let d = d.with_timezone(&chrono::Local);
            let n = d.day();
            let suffix = match (n % 10, n % 100) {
                (_, 11..=13) => "th",
                (1, _) => "st",
                (2, _) => "nd",
                (3, _) => "rd",
                _ => "th",
            };
            format!("{} {n}{suffix}, {}", d.format("%b"), d.year())
        };
        let (a, b) = (day(first), day(last));
        Some(if a == b { a } else { format!("{a} – {b}") })
    }
}
