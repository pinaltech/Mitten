//! The window frame when Mitten draws its own decorations (Linux and BSD).
//!
//! GNOME never offers server-side decorations, and KDE honors a request for
//! client-side ones, so on those platforms the topbar becomes the title bar:
//! it moves the window, opens the compositor's window menu, and carries the
//! minimize, maximize and close controls. The root surface is rounded and
//! outlined to match Adwaita and Breeze. GPUI-component's `Root` supplies the
//! shadow and resize edges. macOS and Windows windows are left unchanged.
use desert_ui::{self as ui, Colors};
use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;

/// Corner radius of a client-decorated window, matching Breeze and Adwaita.
const WINDOW_RADIUS: f32 = 10.;

/// Whether this window draws its own title bar.
fn is_client_decorated(w: &Window) -> bool {
    cfg!(any(target_os = "linux", target_os = "freebsd"))
        && matches!(w.window_decorations(), Decorations::Client { .. })
}

/// Radii for the window's outer corners: rounded while Mitten draws its own
/// decorations, square on edges the compositor has tiled or maximized.
pub(super) fn window_corners(w: &Window) -> Corners<Pixels> {
    if !cfg!(any(target_os = "linux", target_os = "freebsd")) {
        return Corners::default();
    }
    let Decorations::Client { tiling } = w.window_decorations() else {
        return Corners::default();
    };
    let r = |a: bool, b: bool| px(if a || b { 0. } else { WINDOW_RADIUS });
    Corners {
        top_left: r(tiling.top, tiling.left),
        top_right: r(tiling.top, tiling.right),
        bottom_left: r(tiling.bottom, tiling.left),
        bottom_right: r(tiling.bottom, tiling.right),
    }
}

/// Round and outline the root surface when it is the window frame. GPUI clips
/// children only to rectangles, so a full-bleed child that paints its own
/// background into a corner must apply the matching [`window_corners`] radius.
pub(super) fn window_frame<E: Styled>(root: E, c: Colors, w: &Window) -> E {
    if !is_client_decorated(w) {
        return root;
    }
    let Decorations::Client { tiling } = w.window_decorations() else {
        return root;
    };
    let corners = window_corners(w);
    let mut root = root
        .rounded_tl(corners.top_left)
        .rounded_tr(corners.top_right)
        .rounded_bl(corners.bottom_left)
        .rounded_br(corners.bottom_right)
        .border_color(rgb(c.border));
    if !tiling.top {
        root = root.border_t_1();
    }
    if !tiling.bottom {
        root = root.border_b_1();
    }
    if !tiling.left {
        root = root.border_l_1();
    }
    if !tiling.right {
        root = root.border_r_1();
    }
    root
}

/// Make a finished bar the window's title bar when Mitten draws its own
/// decorations, with minimize, maximize and close at the trailing edge; return
/// it unchanged otherwise.
///
/// Buttons in the bar must stop left mouse-down propagation so clicking them
/// never moves the window. `on_close` runs for the close button, so the app
/// can refuse while work is in flight.
pub(super) fn client_titlebar(
    bar: Stateful<Div>,
    c: Colors,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    w: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    if !is_client_decorated(w) {
        return bar;
    }
    // Start the compositor move only once the pointer moves while pressed, so
    // plain clicks and double-clicks on the bar still arrive.
    let pressed = w.use_keyed_state("mitten-titlebar-pressed", cx, |_, _| false);
    let set = |value: bool| {
        let pressed = pressed.clone();
        move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            pressed.update(cx, |p, _| *p = value)
        }
    };
    let release = pressed.clone();
    let moving = pressed.clone();
    let available = w.window_controls();
    let maximized = w.is_maximized();
    bar.on_mouse_down(MouseButton::Left, set(true))
        .on_mouse_down_out(set(false))
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            release.update(cx, |p, _| *p = false)
        })
        .on_mouse_move(move |_, w, cx| {
            if *moving.read(cx) {
                moving.update(cx, |p, _| *p = false);
                w.start_window_move();
            }
        })
        .on_mouse_down(MouseButton::Right, |event, w, _| {
            w.show_window_menu(event.position)
        })
        .when(available.minimize, |bar| {
            bar.child(
                control("window-minimize", "window-minimize", "Minimize", c)
                    .on_click(|_, w, _| w.minimize_window()),
            )
        })
        .when(available.maximize, |bar| {
            bar.child(
                control(
                    "window-maximize",
                    if maximized {
                        "window-restore"
                    } else {
                        "window-maximize"
                    },
                    if maximized { "Restore" } else { "Maximize" },
                    c,
                )
                .on_click(|_, w, _| w.zoom_window()),
            )
        })
        .child(control("window-close", "window-close", "Close", c).on_click(on_close))
}

/// A window control the size of the topbar's small icon buttons.
fn control(id: &'static str, icon: &str, label: &'static str, c: Colors) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(7.))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(c.raised)))
        .active(|s| s.opacity(0.75))
        .tooltip(move |w, cx| Tooltip::new(label).build(w, cx))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(ui::icon(format!("icons/{icon}.svg"), 16., c.muted))
}
