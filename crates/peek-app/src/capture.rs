//! User-triggered capture. Images remain in memory and are never written to disk.
use eframe::egui;

pub struct Screen {
    pub pixels: image::RgbaImage,
    pub origin: [f32; 2],
    pub scale: f32,
}
#[cfg(target_os = "macos")]
fn cursor_position() -> Option<(i32, i32)> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    let event =
        CGEvent::new(CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?).ok()?;
    let point = event.location();
    Some((point.x as i32, point.y as i32))
}
#[cfg(windows)]
fn cursor_position() -> Option<(i32, i32)> {
    use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&mut point).ok()?;
    }
    Some((point.x, point.y))
}

pub fn popup_position(size: egui::Vec2) -> Option<egui::Pos2> {
    let (x, y) = cursor_position()?;
    let monitor = xcap::Monitor::from_point(x, y).ok()?;
    let scale = monitor.scale_factor().ok()?;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let origin = egui::pos2(monitor.x().ok()? as f32, monitor.y().ok()? as f32);
    let cursor = egui::pos2(x as f32, y as f32);
    // macOS global screen coordinates are points; Windows cursor coordinates are pixels.
    let divisor = coordinate_divisor(cfg!(windows), scale);
    let origin = origin / divisor;
    let cursor = cursor / divisor;
    let extent = egui::vec2(
        monitor.width().ok()? as f32 / divisor,
        monitor.height().ok()? as f32 / divisor,
    );
    Some(clamp_popup(
        cursor + egui::vec2(14.0, 14.0),
        egui::Rect::from_min_size(origin, extent),
        size,
    ))
}
fn coordinate_divisor(physical_coordinates: bool, scale: f32) -> f32 {
    if physical_coordinates { scale } else { 1.0 }
}
fn clamp_popup(position: egui::Pos2, bounds: egui::Rect, size: egui::Vec2) -> egui::Pos2 {
    let margin = 8.0;
    let min = bounds.min + egui::vec2(margin, margin);
    let max = egui::pos2(
        (bounds.max.x - size.x - margin).max(min.x),
        (bounds.max.y - size.y - margin).max(min.y),
    );
    egui::pos2(
        position.x.clamp(min.x, max.x),
        position.y.clamp(min.y, max.y),
    )
}

pub fn capture() -> Result<Screen, String> {
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let monitor = cursor_position()
        .and_then(|(x, y)| xcap::Monitor::from_point(x, y).ok())
        .or_else(|| {
            monitors
                .into_iter()
                .find(|m| m.is_primary().unwrap_or(false))
        })
        .ok_or("error-no-display")?;
    Ok(Screen {
        pixels: monitor.capture_image().map_err(|e| e.to_string())?,
        origin: [
            monitor.x().map_err(|e| e.to_string())? as f32,
            monitor.y().map_err(|e| e.to_string())? as f32,
        ],
        scale: monitor.scale_factor().map_err(|e| e.to_string())?,
    })
}
/// Convert coordinates inside a rendered screenshot back to display logical coordinates.
/// Accounts for egui zoom independently of OS DPI, and keeps both axes proportional.
pub fn selection_to_logical(
    rect: egui::Rect,
    rendered: egui::Vec2,
    logical: egui::Vec2,
) -> egui::Rect {
    let ratio = logical / rendered;
    egui::Rect::from_min_max(
        egui::pos2(rect.min.x * ratio.x, rect.min.y * ratio.y),
        egui::pos2(rect.max.x * ratio.x, rect.max.y * ratio.y),
    )
}
/// Convert logical UI coordinates into clamped image pixels; reject tiny selections.
pub fn crop(screen: &Screen, rect: egui::Rect) -> Option<image::RgbaImage> {
    if !screen.scale.is_finite()
        || screen.scale <= 0.0
        || !rect.min.x.is_finite()
        || !rect.min.y.is_finite()
        || !rect.max.x.is_finite()
        || !rect.max.y.is_finite()
    {
        return None;
    }
    let w = screen.pixels.width();
    let h = screen.pixels.height();
    let x = (rect.min.x * screen.scale).floor().max(0.0).min(w as f32) as u32;
    let y = (rect.min.y * screen.scale).floor().max(0.0).min(h as f32) as u32;
    let right = (rect.max.x * screen.scale).ceil().max(0.0).min(w as f32) as u32;
    let bottom = (rect.max.y * screen.scale).ceil().max(0.0).min(h as f32) as u32;
    if right <= x + 2 || bottom <= y + 2 {
        return None;
    }
    Some(image::imageops::crop_imm(&screen.pixels, x, y, right - x, bottom - y).to_image())
}
/// Normalize only the selected region, never the full display. Preserve aspect ratio.
/// 2000px fits Windows OCR's common 2600px maximum and stays below Clef's pixel cap.
pub fn prepare_region(image: image::RgbaImage) -> image::RgbaImage {
    let mut image = if image.width().max(image.height()) > 2000 {
        let ratio = 2000.0 / image.width().max(image.height()) as f64;
        let width = (image.width() as f64 * ratio).round().max(1.0) as u32;
        let height = (image.height() as f64 * ratio).round().max(1.0) as u32;
        image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    for pixel in image.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
        pixel[3] = 255;
    }
    image
}

#[cfg(test)]
mod tests {
    #[test]
    fn retina_logical_bounds_are_not_scaled_twice() {
        let extent = egui::vec2(1440.0, 900.0);
        assert_eq!(extent / super::coordinate_divisor(false, 2.0), extent);
        assert_eq!(
            egui::vec2(2880.0, 1800.0) / super::coordinate_divisor(true, 2.0),
            extent
        );
    }
    #[test]
    fn selection_respects_interface_zoom_before_pixel_crop() {
        let logical = egui::vec2(1000.0, 500.0);
        let rendered = logical / 1.5;
        let rect = super::selection_to_logical(
            egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(300.0, 150.0)),
            rendered,
            logical,
        );
        assert!((rect.min.x - 150.0).abs() < 0.001);
        assert!((rect.max.y - 225.0).abs() < 0.001);
    }
    #[test]
    fn popup_stays_on_negative_coordinate_monitor() {
        let bounds =
            egui::Rect::from_min_size(egui::pos2(-1920.0, 0.0), egui::vec2(1920.0, 1080.0));
        let pos = super::clamp_popup(egui::pos2(-10.0, 1050.0), bounds, egui::vec2(480.0, 560.0));
        assert_eq!(pos, egui::pos2(-488.0, 512.0));
        let pos = super::clamp_popup(
            egui::pos2(-2500.0, -100.0),
            bounds,
            egui::vec2(480.0, 560.0),
        );
        assert_eq!(pos, egui::pos2(-1912.0, 8.0));
    }
    #[test]
    fn large_regions_shrink_and_transparency_is_composited() {
        let result = super::prepare_region(image::RgbaImage::from_pixel(
            4000,
            2000,
            image::Rgba([0, 0, 0, 0]),
        ));
        assert_eq!(result.dimensions(), (2000, 1000));
        assert_eq!(result.get_pixel(0, 0).0, [255, 255, 255, 255]);
        let result = super::prepare_region(image::RgbaImage::from_pixel(
            10,
            10,
            image::Rgba([0, 0, 0, 128]),
        ));
        assert_eq!(result.get_pixel(0, 0).0, [127, 127, 127, 255]);
    }
    use super::*;
    #[test]
    fn invalid_selection_and_scale_rejected() {
        let mut screen = Screen {
            pixels: image::RgbaImage::new(10, 10),
            origin: [0.0, 0.0],
            scale: 1.0,
        };
        assert!(
            crop(
                &screen,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(f32::NAN, 5.0))
            )
            .is_none()
        );
        assert!(
            crop(
                &screen,
                egui::Rect::from_min_max(egui::pos2(5.0, 5.0), egui::pos2(1.0, 1.0))
            )
            .is_none()
        );
        screen.scale = 0.0;
        assert!(
            crop(
                &screen,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(5.0, 5.0))
            )
            .is_none()
        );
    }
    #[test]
    fn crop_scaled_and_clamped() {
        let screen = Screen {
            pixels: image::RgbaImage::new(100, 80),
            origin: [0.0, 0.0],
            scale: 2.0,
        };
        let result = crop(
            &screen,
            egui::Rect::from_min_max(egui::pos2(-3.0, 5.0), egui::pos2(60.0, 30.0)),
        )
        .unwrap();
        assert_eq!(result.dimensions(), (100, 50));
        assert!(
            crop(
                &screen,
                egui::Rect::from_min_max(egui::pos2(60.0, 5.0), egui::pos2(70.0, 30.0))
            )
            .is_none()
        );
    }
}
