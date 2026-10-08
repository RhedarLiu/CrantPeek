//! User-triggered capture. Images remain in memory and are never written to disk.
use eframe::egui;

pub struct Screen {
    pub pixels: image::RgbaImage,
    pub origin: [f32; 2],
    pub scale: f32,
}
pub fn capture() -> Result<Screen, String> {
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    // Initial implementation uses primary display; secondary-display selection is pending.
    let monitor = monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .ok_or("No primary display")?;
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
