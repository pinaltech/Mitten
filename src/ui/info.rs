//! The info panel: a description, the people in a photo, every capture
//! detail its metadata has (with icons, after Immich's) and a map.
use super::*;
use storage::PhotoDetails;

pub(super) const INSPECTOR_WIDTH: f32 = 300.;
/// The map's size in the panel (it is fetched at this many pixels, so the
/// tiles' labels stay readable).
const MAP_SIZE: (u32, u32) = (268, 180);
const MAP_ZOOM: u8 = 14;
const FACE_JOBS: usize = 6;

fn row(icon: &str, title: String, detail: Option<String>, c: Colors) -> Div {
    div()
        .flex()
        .gap_3()
        .items_start()
        .child(
            div()
                .pt(px(1.))
                .flex_shrink_0()
                .child(kit::glyph(icon, 18., c.muted)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(13.)).child(title))
                .when_some(detail, |d, detail| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child(detail),
                    )
                }),
        )
}
fn megabytes(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.)
    } else {
        format!("{} KB", (bytes as f64 / 1_000.).round().max(1.))
    }
}
/// "48.85661° N, 2.35222° E"
fn coordinates((lat, lon): (f64, f64)) -> String {
    format!(
        "{:.5}° {}, {:.5}° {}",
        lat.abs(),
        if lat >= 0. { "N" } else { "S" },
        lon.abs(),
        if lon >= 0. { "E" } else { "W" }
    )
}
/// A round face, or the person glyph until (or unless) it loads.
pub(super) fn face(path: Option<PathBuf>, edge: f32, c: Colors) -> Div {
    div()
        .size(px(edge))
        .flex_shrink_0()
        .rounded_full()
        .overflow_hidden()
        .bg(rgb(c.soft))
        .flex()
        .items_center()
        .justify_center()
        .child(match path {
            Some(path) => img(Arc::<std::path::Path>::from(path))
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => kit::glyph("person", edge * 0.5, c.muted).into_any_element(),
        })
}
impl PhotosApp {
    /// Keeps the description field on the shown photo and fetches its
    /// details (and then its map) when they are not loaded yet.
    pub(super) fn prepare_info(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(photo) = self.current() else {
            return;
        };
        if self.caption_for.as_deref() != Some(&photo.id) {
            // An unsaved edit belongs to the photo shown before.
            self.save_caption(cx);
            self.caption_for = Some(photo.id.clone());
            self.caption
                .update(cx, |e, cx| e.set_value(photo.caption.clone(), w, cx));
        }
        let loaded = self.details.as_ref().is_some_and(|(id, _)| *id == photo.id);
        if !loaded && self.details_pending.as_deref() != Some(&photo.id) {
            let Some(driver) = self.driver.clone() else {
                return;
            };
            self.details_pending = Some(photo.id.clone());
            let id = photo.id.clone();
            let lookup = photo.clone();
            cx.spawn(async move |this, cx| {
                let details = cx
                    .background_executor()
                    .spawn(async move { driver.details(&lookup) })
                    .await
                    .unwrap_or_default();
                let _ = this.update(cx, |this, cx| {
                    if this.details_pending.as_deref() == Some(&id) {
                        this.details_pending = None;
                    }
                    this.details = Some((id, Arc::new(details)));
                    cx.notify();
                });
            })
            .detach();
        }
        if let Some((_, details)) = self.details.as_ref().filter(|(id, _)| *id == photo.id)
            && let Some(at) = details.coordinates()
            && self.preferences.show_map
        {
            self.request_map(at, cx);
        }
    }
    fn map_key(&self, (lat, lon): (f64, f64)) -> String {
        format!("{lat:.5},{lon:.5},{}", self.preferences.map_tiles)
    }
    fn request_map(&mut self, at: (f64, f64), cx: &mut Context<Self>) {
        let key = self.map_key(at);
        if self.map.as_ref().is_some_and(|(k, _)| *k == key)
            || self.map_pending.as_deref() == Some(&key)
        {
            return;
        }
        #[cfg(feature = "remote")]
        {
            let cache = self.config_path.with_file_name("map-cache");
            let template = self.preferences.map_tiles.clone();
            self.map_pending = Some(key.clone());
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        std::fs::create_dir_all(&cache)?;
                        map::static_map(&template, at, MAP_ZOOM, MAP_SIZE, &cache)
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.map_pending.as_deref() == Some(&key) {
                        this.map_pending = None;
                    }
                    this.map = Some((key, result.ok()));
                    cx.notify();
                });
            })
            .detach();
        }
        #[cfg(not(feature = "remote"))]
        {
            self.map = Some((key, None));
            let _ = cx;
        }
    }
    /// Loads a person's face for the info panel and Explore, a few at once.
    pub(super) fn request_face(&mut self, id: &str, cx: &mut Context<Self>) -> Option<PathBuf> {
        if let Some(face) = self.faces.get(id) {
            return face.clone();
        }
        if self.faces_pending.contains(id) || self.faces_pending.len() >= FACE_JOBS {
            return None;
        }
        let driver = self.driver.clone()?;
        let id = id.to_owned();
        self.faces_pending.insert(id.clone());
        cx.spawn(async move |this, cx| {
            let lookup = id.clone();
            let face = cx
                .background_executor()
                .spawn(async move { driver.person_thumbnail(&lookup).ok() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.faces_pending.remove(&id);
                this.faces.insert(id, face);
                cx.notify();
            });
        })
        .detach();
        None
    }
    /// Saves the description when it changed (a private annotation).
    pub(super) fn save_caption(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.caption_for.clone() else {
            return;
        };
        let caption = self.caption.read(cx).value().trim().to_string();
        let Some(&index) = self.photo_index.get(&id) else {
            return;
        };
        if self.library.photos[index].caption == caption {
            return;
        }
        let Some(driver) = self.driver.clone() else {
            return;
        };
        Arc::make_mut(&mut self.library.photos[index]).caption = caption.clone();
        self.revision += 1;
        *self.visible_cache.borrow_mut() = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { driver.caption(&id, &caption) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.notice = "Description saved".into(),
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn inspector_view(
        &mut self,
        w: &mut Window,
        c: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.prepare_info(w, cx);
        let mut info = div()
            .id("photo-inspector")
            .debug_selector(|| "photo-inspector".into())
            .w(px(INSPECTOR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(rgb(c.border))
            .bg(rgb(c.surface))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(48.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(c.border))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Info"),
                    )
                    .child(kit::tool("close-info", "close", "Close info", c).on_click(
                        cx.listener(|this, _, w, cx| {
                            this.save_caption(cx);
                            this.inspector = false;
                            this.focus.focus(w);
                            cx.notify();
                        }),
                    )),
            );
        let Some(photo) = self.current() else {
            return info
                .child(kit::empty("info", "Select a photo", "", c))
                .into_any_element();
        };
        let details = self
            .details
            .as_ref()
            .filter(|(id, _)| *id == photo.id)
            .map(|(_, d)| d.clone());
        let loading = details.is_none();
        let details = details.unwrap_or_else(|| Arc::new(PhotoDetails::default()));
        let mut body = div()
            .id("inspector-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .flex()
            .flex_col()
            .gap_4()
            // Description first: an underline with its placeholder.
            .child(
                div()
                    .id("photo-description")
                    .debug_selector(|| "photo-description".into())
                    .border_b_1()
                    .border_color(rgb(c.border))
                    .child(
                        Input::new(&self.caption)
                            .appearance(false)
                            .bordered(false)
                            .text_size(px(13.))
                            .px_0(),
                    ),
            );
        if !details.people.is_empty() {
            let mut people = div().flex().flex_wrap().gap_3();
            for person in details.people.iter().take(24) {
                let face_path = self.request_face(&person.id, cx);
                let (id, name) = (person.id.clone(), person.name.clone());
                people = people.child(
                    div()
                        .id(SharedString::from(format!("info-person-{}", person.id)))
                        .w(px(56.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .cursor_pointer()
                        .child(face(face_path, 48., c))
                        .child(
                            div()
                                .w_full()
                                .text_size(px(11.))
                                .text_center()
                                .truncate()
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
                            this.close_viewer(false, w, cx);
                            this.set_filter(Filter::Person(id.clone(), name.clone()), w, cx);
                        })),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(settings::section("People", c))
                    .child(people),
            );
        }
        let mut rows = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(settings::section("Details", c));
        // When it was taken: the camera's own clock where known.
        let (date, time) = match details.taken {
            Some(taken) => (
                taken.format("%b %-d, %Y").to_string(),
                format!(
                    "{}{}",
                    taken.format("%a, %-I:%M %p"),
                    details
                        .zone
                        .as_deref()
                        .map(|z| format!(" · {z}"))
                        .unwrap_or_default()
                ),
            ),
            None => {
                let local = photo.modified.with_timezone(&chrono::Local);
                (
                    local.format("%b %-d, %Y").to_string(),
                    local.format("%a, %-I:%M %p").to_string(),
                )
            }
        };
        rows = rows.child(row("calendar", date, Some(time), c));
        let (width, height) = match (details.width, details.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
            _ => (photo.width, photo.height),
        };
        let mut file = vec![];
        if width > 0 && height > 0 {
            let megapixels = f64::from(width) * f64::from(height) / 1_000_000.;
            if photo.duration.is_none() && megapixels >= 0.1 {
                file.push(format!("{megapixels:.1} MP"));
            }
            file.push(format!("{width} × {height}"));
        }
        if let Some(bytes) = details.bytes.or((photo.bytes > 0).then_some(photo.bytes))
            && bytes > 0
        {
            file.push(megabytes(bytes));
        }
        if let Some(seconds) = photo.duration.filter(|s| *s > 0.) {
            file.push(duration_label(seconds));
        }
        rows = rows.child(row(
            if photo.duration.is_some() {
                "video"
            } else {
                "image"
            },
            photo.name.clone(),
            (!file.is_empty()).then(|| file.join(" · ")),
            c,
        ));
        if let Some(camera) = details.camera() {
            rows = rows.child(row("camera", camera, details.exposure_line(), c));
        } else if let Some(exposure) = details.exposure_line() {
            rows = rows.child(row("camera", exposure, None, c));
        }
        if let Some(lens) = &details.lens {
            rows = rows.child(row("aperture", lens.clone(), None, c));
        }
        let at = details.coordinates();
        if let Some((first, rest)) = details.place.split_first() {
            rows = rows.child(row(
                "location",
                first.clone(),
                Some(if rest.is_empty() {
                    at.map(coordinates).unwrap_or_default()
                } else {
                    rest.join(", ")
                })
                .filter(|s| !s.is_empty()),
                c,
            ));
        } else if let Some(at) = at {
            rows = rows.child(row("location", coordinates(at), None, c));
        }
        let albums: Vec<String> = self
            .library
            .albums
            .iter()
            .filter(|(_, ids)| ids.contains(&photo.id))
            .map(|(name, _)| name.clone())
            .collect();
        if !albums.is_empty() {
            rows = rows.child(row("folder", albums.join(", "), None, c));
        }
        if loading {
            rows = rows.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c.muted))
                    .child("Loading details…"),
            );
        }
        body = body.child(rows);
        if let Some(at) = at.filter(|_| self.preferences.show_map) {
            let key = self.map_key(at);
            let image = self
                .map
                .as_ref()
                .filter(|(k, _)| *k == key)
                .map(|(_, path)| path.clone());
            let (mw, mh) = (MAP_SIZE.0 as f32, MAP_SIZE.1 as f32);
            body = body.child(
                div()
                    .id("photo-map")
                    .debug_selector(|| "photo-map".into())
                    .relative()
                    .w(px(mw))
                    .h(px(mh))
                    .flex_shrink_0()
                    .rounded(px(10.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(c.border))
                    .bg(rgb(c.soft))
                    .cursor_pointer()
                    .child(match image {
                        Some(Some(path)) => img(Arc::<std::path::Path>::from(path))
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .into_any_element(),
                        Some(None) => div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child("Map unavailable")
                            .into_any_element(),
                        None => div().into_any_element(),
                    })
                    .child(
                        // The pin, centred on the location.
                        div()
                            .absolute()
                            .left(px(mw / 2. - 8.))
                            .top(px(mh / 2. - 8.))
                            .size(px(16.))
                            .rounded_full()
                            .border_2()
                            .border_color(rgb(0xffffff))
                            .bg(rgb(c.accent))
                            .shadow_md(),
                    )
                    .tooltip(|w, cx| {
                        gpui_component::tooltip::Tooltip::new("Open in OpenStreetMap").build(w, cx)
                    })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let (lat, lon) = at;
                        cx.open_url(&format!(
                            "https://www.openstreetmap.org/?mlat={lat:.5}&mlon={lon:.5}#map=16/{lat:.5}/{lon:.5}"
                        ));
                    })),
            );
        }
        if !photo.path.as_os_str().is_empty() {
            body = body.child(
                ui::button("reveal-photo", c, false, true)
                    .border_1()
                    .border_color(rgb(c.border))
                    .child(kit::glyph("folder", 14., c.muted))
                    .child("Show in Folder")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(photo) = this.current()
                            && !photo.path.as_os_str().is_empty()
                        {
                            cx.reveal_path(&photo.path);
                        }
                    })),
            );
        }
        info = info.child(body);
        info.into_any_element()
    }
}
