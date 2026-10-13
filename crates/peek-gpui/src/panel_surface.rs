//! Draw a small, consistent shadow rather than AppKit's stronger key-window
//! shadow. Transparent insets hold the blur outside the content area.
use super::*;

pub const INSET: f32 = 8.;

pub fn wrap(page: AnyElement, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let alpha = if theme.is_dark() { 0.20 } else { 0.10 };
    div()
        .size_full()
        .p(ui_px(INSET))
        .child(
            div()
                .size_full()
                .rounded(ui_px(16.))
                .bg(theme.background)
                .shadow(vec![
                    BoxShadow::new(px(0.), ui_px(2.), hsla(0., 0., 0., alpha))
                        .blur_radius(ui_px(6.)),
                ])
                .child(
                    div()
                        .size_full()
                        .rounded(ui_px(16.))
                        .overflow_hidden()
                        .child(page),
                ),
        )
        .into_any_element()
}
