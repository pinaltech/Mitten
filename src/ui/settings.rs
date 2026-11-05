//! Settings pages: Appearance and Behavior (browsing, image sizes, maps),
//! laid out like the mail client's settings.
use super::*;
use desert_photos::loading::{self, ImageSizes};
use std::rc::Rc;

/// A small uppercase section title with a rule beneath it.
pub(super) fn section(title: &str, c: Colors) -> Div {
    div()
        .pt_2()
        .pb_1()
        .border_b_1()
        .border_color(rgb(c.border))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c.muted))
        .child(title.to_uppercase())
}
/// A titled setting with an optional caption above its control.
pub(super) fn setting(title: &str, caption: &str, control: impl IntoElement, c: Colors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title.to_owned()),
                )
                .when(!caption.is_empty(), |d| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child(caption.to_owned()),
                    )
                }),
        )
        .child(control)
}
/// A setting whose control (a switch) sits to the right of its text.
pub(super) fn toggle_setting(
    title: &str,
    caption: &str,
    control: impl IntoElement,
    c: Colors,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title.to_owned()),
                )
                .when(!caption.is_empty(), |d| {
                    d.child(
                        div()
                            .mt(px(2.))
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child(caption.to_owned()),
                    )
                }),
        )
        .child(control)
}
type OnPick<T> = Rc<dyn Fn(&mut PhotosApp, T, &mut Window, &mut Context<PhotosApp>)>;
impl PhotosApp {
    /// A segmented control: one rounded track, the active option raised.
    /// `dimmed` shows the choice without implying it is in force.
    pub(super) fn segmented<T: Copy + 'static>(
        &self,
        id: &str,
        options: Vec<(String, T, bool)>,
        dimmed: bool,
        c: Colors,
        cx: &mut Context<Self>,
        on: OnPick<T>,
    ) -> Div {
        div()
            .w_full()
            .min_w_0()
            .p(px(3.))
            .flex()
            .flex_wrap()
            .gap(px(3.))
            .rounded(px(10.))
            .bg(rgb(c.bg))
            .border_1()
            .border_color(rgb(c.border))
            .when(dimmed, |d| d.opacity(0.6))
            .children(
                options
                    .into_iter()
                    .enumerate()
                    .map(|(i, (label, value, active))| {
                        let selector = format!("{id}-{i}");
                        let on = on.clone();
                        div()
                            .id(SharedString::from(selector.clone()))
                            .debug_selector(move || selector)
                            .flex_1()
                            .min_w(px(64.))
                            .h(px(28.))
                            .px_2()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(7.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .when(active, |d| {
                                d.bg(rgb(c.raised)).text_color(rgb(c.text)).shadow_sm()
                            })
                            .when(!active, |d| {
                                d.text_color(rgb(c.muted))
                                    .hover(move |s| s.text_color(rgb(c.text)))
                            })
                            .child(label)
                            .on_click(cx.listener(move |this, _, w, cx| on(this, value, w, cx)))
                    }),
            )
    }
    pub(super) fn appearance_page(&self, w: &Window, c: Colors, cx: &mut Context<Self>) -> Div {
        let theme = self.segmented(
            "appearance",
            [
                ("system", "Automatic"),
                ("light", "Light"),
                ("dark", "Dark"),
            ]
            .into_iter()
            .map(|(key, label)| {
                (
                    label.to_owned(),
                    key,
                    self.preferences.appearance == key
                        || key == "system"
                            && !matches!(self.preferences.appearance.as_str(), "light" | "dark"),
                )
            })
            .collect(),
            false,
            c,
            cx,
            Rc::new(|this, key: &'static str, w, cx| {
                this.preferences.appearance = key.into();
                if let Err(e) = this.preferences.save(&this.config_path) {
                    this.error = Some(e.to_string());
                }
                this.theme(w, cx);
            }),
        );
        let dark = matches!(self.preferences.appearance.as_str(), "dark")
            || self.preferences.appearance != "light" && ui::system_dark(w);
        let swatches = div()
            .flex()
            .flex_wrap()
            .gap_3()
            .children(Accent::ALL.map(|accent| {
                let color = Colors::new(dark, accent);
                let active = self.accent() == accent;
                let id = format!("accent-{}", accent.key());
                div()
                    .id(SharedString::from(id.clone()))
                    .debug_selector(move || id)
                    .size(px(30.))
                    .rounded_full()
                    .p(px(3.))
                    .border_2()
                    .border_color(rgb(if active { color.accent } else { c.surface }))
                    .cursor_pointer()
                    .focusable()
                    .tab_index(0)
                    .child(
                        div()
                            .size_full()
                            .rounded_full()
                            .bg(rgb(color.accent))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(active, |d| {
                                d.child(kit::glyph("check", 12., color.on_accent))
                            }),
                    )
                    .tooltip(move |w, cx| {
                        gpui_component::tooltip::Tooltip::new(accent.label()).build(w, cx)
                    })
                    .on_click(cx.listener(move |this, _, w, cx| this.set_accent(accent, w, cx)))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, w, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.set_accent(accent, w, cx);
                        }
                    }))
            }));
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(section("Appearance", c))
            .child(setting(
                "Theme",
                "Automatic follows your system's light and dark setting.",
                theme,
                c,
            ))
            .child(setting(
                "Theme color",
                &format!(
                    "Used for buttons, selection and highlights. {}",
                    self.accent().label()
                ),
                swatches,
                c,
            ))
    }
    pub(super) fn behavior_page(&self, c: Colors, cx: &mut Context<Self>) -> Div {
        let auto = self.preferences.automatic_sizes;
        let current = loading::image_sizes();
        let level = |this: &Self,
                     id: &str,
                     choices: &[u32],
                     value: u32,
                     cx: &mut Context<Self>,
                     set: fn(&mut ImageSizes, u32)| {
            this.segmented(
                id,
                choices
                    .iter()
                    .map(|&edge| (format!("{edge} px"), edge, edge == value))
                    .collect(),
                auto,
                c,
                cx,
                Rc::new(move |this: &mut Self, edge: u32, w, cx| {
                    // Choosing a size by hand turns automatic sizing off,
                    // starting from the sizes in use.
                    if this.preferences.automatic_sizes {
                        this.preferences.automatic_sizes = false;
                        this.preferences.image_sizes = loading::image_sizes();
                    }
                    set(&mut this.preferences.image_sizes, edge);
                    this.save_preferences();
                    this.apply_sizes(w, cx);
                }),
            )
        };
        let measured = match (auto, self.measuring) {
            (true, true) => "Measuring how quickly this library loads each size…".to_owned(),
            (true, false) => match self
                .driver
                .as_ref()
                .and_then(|d| self.preferences.measured_sizes.get(&d.source().path))
            {
                Some(sizes) => format!(
                    "Measured for this library: {} px in the grid, {} px in the viewer.",
                    sizes.grid, sizes.viewer
                ),
                None => "Not measured yet: measuring starts once the library loads.".into(),
            },
            (false, _) => "Sizes below are chosen by hand.".into(),
        };
        let map_choice = map::PRESETS
            .iter()
            .position(|(_, url)| *url == self.preferences.map_tiles.trim());
        let mut tiles: Vec<(String, Option<usize>, bool)> = map::PRESETS
            .iter()
            .enumerate()
            .map(|(i, (name, _))| ((*name).to_owned(), Some(i), map_choice == Some(i)))
            .collect();
        tiles.push((
            "Custom".into(),
            None,
            map_choice.is_none() || self.custom_map,
        ));
        if self.custom_map {
            for t in &mut tiles[..map::PRESETS.len()] {
                t.2 = false;
            }
        }
        let map_servers = self.segmented(
            "map-server",
            tiles,
            !self.preferences.show_map,
            c,
            cx,
            Rc::new(|this: &mut Self, choice: Option<usize>, w, cx| {
                match choice {
                    Some(i) => {
                        this.custom_map = false;
                        this.preferences.map_tiles = map::PRESETS[i].1.into();
                        let url = this.preferences.map_tiles.clone();
                        this.map_url.update(cx, |e, cx| e.set_value(url, w, cx));
                        this.save_preferences();
                        this.map = None;
                    }
                    None => {
                        this.custom_map = true;
                        this.map_url.update(cx, |e, cx| e.focus(w, cx));
                    }
                }
                cx.notify();
            }),
        );
        let custom = self.custom_map || map_choice.is_none();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(section("Browsing", c))
            .child(toggle_setting(
                "Browse from bottom to top",
                "Start at the newest photos at the bottom. Scroll up to go back in time.",
                div()
                    .id("bottom-to-top-setting")
                    .debug_selector(|| "bottom-to-top-setting".into())
                    .child(
                        Switch::new("bottom-to-top")
                            .checked(self.preferences.bottom_to_top)
                            .on_click(cx.listener(|this, value: &bool, _, cx| {
                                this.set_scroll_direction(*value, cx)
                            })),
                    ),
                c,
            ))
            .child(section("Thumbnails", c))
            .child(
                toggle_setting(
                    "Choose sizes automatically",
                    "Times how fast this library delivers each size, then uses the sharpest that still loads quickly, so the largest zoom fills fast.",
                    div().id("automatic-sizes-setting").child(
                        Switch::new("automatic-sizes")
                            .checked(auto)
                            .on_click(cx.listener(|this, value: &bool, w, cx| {
                                this.preferences.automatic_sizes = *value;
                                if !*value {
                                    this.preferences.image_sizes = loading::image_sizes();
                                }
                                this.save_preferences();
                                this.apply_sizes(w, cx);
                                this.measure_sizes(false, cx);
                            })),
                    ),
                    c,
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(rgb(c.muted))
                            .child(measured),
                    )
                    .when(auto && self.driver.is_some(), |d| {
                        d.child(
                            ui::button("measure-sizes", c, false, !self.measuring)
                                .border_1()
                                .border_color(rgb(c.border))
                                .child(kit::glyph("refresh", 14., c.muted))
                                .child("Measure Again")
                                .on_click(cx.listener(|this, _, _, cx| this.measure_sizes(true, cx))),
                        )
                    }),
            )
            .child(setting(
                "Largest zoom",
                "Thumbnails for the three- to nine-per-row grid. Larger is sharper but slower to load.",
                level(self, "size-grid", &ImageSizes::GRID, current.grid, cx, |s, v| s.grid = v),
                c,
            ))
            .child(setting(
                "Months",
                "Squares in the gapless and month grids.",
                level(self, "size-mini", &ImageSizes::MINI, current.mini, cx, |s, v| s.mini = v),
                c,
            ))
            .child(setting(
                "Years",
                "The smallest squares, in the year grid.",
                level(self, "size-micro", &ImageSizes::MICRO, current.micro, cx, |s, v| s.micro = v),
                c,
            ))
            .child(setting(
                "Viewer",
                "The preview shown when a photo is open.",
                level(self, "size-viewer", &ImageSizes::VIEWER, current.viewer, cx, |s, v| s.viewer = v),
                c,
            ))
            .child(section("Map", c))
            .child(toggle_setting(
                "Show where photos were taken",
                "A map at the bottom of the info panel. Only the map tiles around a photo's location are requested from the map server.",
                div().id("show-map-setting").child(
                    Switch::new("show-map")
                        .checked(self.preferences.show_map)
                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                            this.preferences.show_map = *value;
                            this.save_preferences();
                            cx.notify();
                        })),
                ),
                c,
            ))
            .child(setting(
                "Map server",
                "Open-source tile servers that need no account. Choose Custom for your own server.",
                map_servers,
                c,
            ))
            .when(custom, |d| {
                d.child(setting(
                    "Tile address",
                    "A raster tile URL with {z}, {x} and {y}. Press Return to use it.",
                    super::field(Input::new(&self.map_url), c),
                    c,
                ))
            })
    }
    pub(super) fn save_preferences(&mut self) {
        if let Err(e) = self.preferences.save(&self.config_path) {
            self.error = Some(e.to_string());
        }
    }
    /// Applies the image sizes for the active library; a change drops every
    /// image cached at the old sizes so tiles reload at the new ones.
    pub(super) fn apply_sizes(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let path = self.driver.as_ref().map(|d| d.source().path);
        let sizes = self.preferences.sizes_for(path.as_deref());
        if sizes == loading::image_sizes() {
            return;
        }
        loading::set_image_sizes(sizes);
        self.thumbnails.clear();
        self.previews.clear();
        self.tiles.clear();
        self.pending.clear();
        self.failed.clear();
        self.failed_order.clear();
        self.decoded_preview = None;
        self.generation += 1;
        self.render_cache.update(cx, |cache, cx| cache.clear(w, cx));
        self.viewer_render_cache
            .update(cx, |cache, cx| cache.clear(w, cx));
        cx.notify();
    }
    /// Times the active library at each size (once per library unless
    /// `force`) and switches to the sizes it picks.
    pub(super) fn measure_sizes(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(driver) = self.driver.clone() else {
            return;
        };
        let path = driver.source().path;
        // UI tests use synthetic sources whose image counters assert that
        // browsing never fetches previews; they measure only when asked.
        if cfg!(test) && !force {
            return;
        }
        if self.measuring
            || !self.preferences.automatic_sizes
            || self.library.photos.is_empty()
            || !force && self.preferences.measured_sizes.contains_key(&path)
        {
            return;
        }
        self.measuring = true;
        let photos: Vec<_> = self.library.photos.iter().take(600).cloned().collect();
        cx.spawn(async move |this, cx| {
            let sizes = cx
                .background_executor()
                .spawn(async move { storage::measure_sizes(driver.as_ref(), &photos) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.measuring = false;
                this.preferences.measured_sizes.insert(path, sizes);
                this.save_preferences();
                this.sizes_dirty = true;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
