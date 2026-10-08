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

pub fn capture() -> Result<Screen, String> {
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let monitor = cursor_position()
        .and_then(|(x, y)| xcap::Monitor::from_point(x, y).ok())
        .or_else(|| {
            monitors
                .into_iter()
                .find(|m| m.is_primary().unwrap_or(false))
        })
        .ok_or("No display available")?;
    Ok(Screen {
        pixels: monitor.capture_image().map_err(|e| e.to_string())?,
        origin: [
            monitor.x().map_err(|e| e.to_string())? as f32,
            monitor.y().map_err(|e| e.to_string())? as f32,
        ],
        scale: monitor.scale_factor().map_err(|e| e.to_string())?,
    })
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
#[cfg(test)]
mod tests {
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
