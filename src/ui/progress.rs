use desert_ui::Colors;
use gpui::{prelude::*, *};
use gpui_component::{Sizable, spinner::Spinner};

pub fn indicator(
    label: String,
    progress: Option<f32>,
    show_label: bool,
    c: Colors,
) -> Stateful<Div> {
    div()
        .id("loading-progress")
        .debug_selector(|| "loading-progress".into())
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(11.))
        .text_color(rgb(c.muted))
        .child(if let Some(progress) = progress {
            canvas(
                |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let radius = f32::from(bounds.size.width) / 2. - 2.;
                    for (amount, color) in [(1., c.border), (progress.clamp(0., 1.), c.accent)] {
                        let mut path = PathBuilder::stroke(px(2.));
                        for i in 0..=64 {
                            let angle = -std::f32::consts::FRAC_PI_2
                                + std::f32::consts::TAU * amount * i as f32 / 64.;
                            let p = bounds.center()
                                + point(px(radius * angle.cos()), px(radius * angle.sin()));
                            if i == 0 {
                                path.move_to(p);
                            } else {
                                path.line_to(p);
                            }
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, rgb(color));
                        }
                    }
                },
            )
            .size(px(20.))
            .into_any_element()
        } else {
            Spinner::new()
                .color(rgb(c.accent).into())
                .with_size(px(18.))
                .into_any_element()
        })
        .when(show_label, |d| d.child(label))
}
