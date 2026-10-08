//! Native egui design tokens inspired by understated monochrome card interfaces.
use eframe::egui;

pub fn apply(ctx: &egui::Context) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.style_mut_of(theme, |style| {
            let dark = theme == egui::Theme::Dark;
            let background = if dark {
                egui::Color32::from_rgb(20, 20, 22)
            } else {
                egui::Color32::from_rgb(250, 250, 250)
            };
            let surface = if dark {
                egui::Color32::from_rgb(30, 30, 33)
            } else {
                egui::Color32::WHITE
            };
            let muted = if dark {
                egui::Color32::from_rgb(41, 41, 45)
            } else {
                egui::Color32::from_rgb(242, 242, 243)
            };
            let border = if dark {
                egui::Color32::from_rgb(60, 60, 65)
            } else {
                egui::Color32::from_rgb(224, 224, 227)
            };
            let text = if dark {
                egui::Color32::from_rgb(244, 244, 245)
            } else {
                egui::Color32::from_rgb(24, 24, 27)
            };
            style.spacing.item_spacing = egui::vec2(10.0, 12.0);
            style.spacing.button_padding = egui::vec2(14.0, 8.0);
            style.spacing.interact_size = egui::vec2(36.0, 36.0);
            style.spacing.text_edit_width = 280.0;
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(21.0));
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
            style.visuals.panel_fill = background;
            style.visuals.window_fill = surface;
            style.visuals.extreme_bg_color = muted;
            style.visuals.faint_bg_color = muted;
            style.visuals.window_corner_radius = egui::CornerRadius::same(20);
            style.visuals.window_stroke = egui::Stroke::new(1.0, border);
            style.visuals.selection.bg_fill = if dark {
                egui::Color32::from_gray(78)
            } else {
                egui::Color32::from_gray(215)
            };
            for widget in [
                &mut style.visuals.widgets.noninteractive,
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
                &mut style.visuals.widgets.open,
            ] {
                widget.corner_radius = egui::CornerRadius::same(12);
                widget.bg_stroke = egui::Stroke::new(1.0, border);
                widget.fg_stroke.color = text;
            }
            style.visuals.widgets.inactive.bg_fill = surface;
            style.visuals.widgets.hovered.bg_fill = muted;
            style.visuals.widgets.active.bg_fill = border;
        });
    }
}
pub fn primary(ui: &mut egui::Ui, text: String) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let fill = if dark {
        egui::Color32::from_gray(245)
    } else {
        egui::Color32::from_gray(24)
    };
    let foreground = if dark {
        egui::Color32::from_gray(24)
    } else {
        egui::Color32::WHITE
    };
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(foreground))
            .fill(fill)
            .corner_radius(18),
    )
}
pub fn icon(ui: &mut egui::Ui, glyph: &str, label: String) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(glyph).size(18.0))
            .frame(false)
            .min_size(egui::vec2(34.0, 34.0)),
    )
    .on_hover_text(label)
}
pub fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(ui.visuals().window_stroke)
        .corner_radius(16)
        .inner_margin(18)
}
