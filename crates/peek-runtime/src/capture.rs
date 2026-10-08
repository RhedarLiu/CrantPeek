//! User-triggered capture. Images remain in memory and are never written to disk.
//!
//! Geometry is expressed with plain `[f32; 2]` points and an owned [`Rect`]
//! rather than a GUI toolkit's types, so both the egui shell and the GPUI shell
//! can share the same capture and coordinate math. Callers convert at the edge.

/// A point or vector in display coordinates: `[x, y]`.
pub type Point = [f32; 2];

/// An axis-aligned rectangle in display coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Rect {
    pub fn from_min_max(min: Point, max: Point) -> Self {
        Self { min, max }
    }

    pub fn from_min_size(min: Point, size: Point) -> Self {
        Self {
            min,
            max: [min[0] + size[0], min[1] + size[1]],
        }
    }

    /// The rectangle spanned by a drag between two corners, in either direction.
    pub fn from_drag(anchor: Point, cursor: Point) -> Self {
        Self {
            min: [anchor[0].min(cursor[0]), anchor[1].min(cursor[1])],
            max: [anchor[0].max(cursor[0]), anchor[1].max(cursor[1])],
        }
    }

    pub fn width(&self) -> f32 {
        self.max[0] - self.min[0]
    }

    pub fn height(&self) -> f32 {
        self.max[1] - self.min[1]
    }

    pub fn size(&self) -> Point {
        [self.width(), self.height()]
    }
}

/// A captured display: pixel data plus the display coordinates it covers.
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

/// Where the peek panel should appear: just below-right of the pointer, clamped
/// inside the monitor that holds it.
pub fn popup_position(size: Point) -> Option<Point> {
    let (x, y) = cursor_position()?;
    let monitor = xcap::Monitor::from_point(x, y).ok()?;
    let scale = monitor.scale_factor().ok()?;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let origin = [monitor.x().ok()? as f32, monitor.y().ok()? as f32];
    let cursor = [x as f32, y as f32];
    // macOS global screen coordinates are points; Windows cursor coordinates are pixels.
    let divisor = coordinate_divisor(cfg!(windows), scale);
    let origin = [origin[0] / divisor, origin[1] / divisor];
    let cursor = [cursor[0] / divisor, cursor[1] / divisor];
    let extent = [
        monitor.width().ok()? as f32 / divisor,
        monitor.height().ok()? as f32 / divisor,
    ];
    Some(clamp_popup(
        [cursor[0] + 14.0, cursor[1] + 14.0],
        Rect::from_min_size(origin, extent),
        size,
    ))
}

fn coordinate_divisor(physical_coordinates: bool, scale: f32) -> f32 {
    if physical_coordinates { scale } else { 1.0 }
}

fn clamp_popup(position: Point, bounds: Rect, size: Point) -> Point {
    let margin = 8.0;
    let min = [bounds.min[0] + margin, bounds.min[1] + margin];
    let max = [
        (bounds.max[0] - size[0] - margin).max(min[0]),
        (bounds.max[1] - size[1] - margin).max(min[1]),
    ];
    [
        position[0].clamp(min[0], max[0]),
        position[1].clamp(min[1], max[1]),
    ]
}
/// Diagnostic: what the platform reports for every display.
///
/// `Monitor::from_point` fails on this macOS with "Monitor is not active" even
/// while the cursor position is readable, so bounds are resolved by comparing
/// rectangles instead. This shows the raw data behind that decision, and
/// creates no window.
pub fn monitor_report() -> Vec<String> {
    match xcap::Monitor::all() {
        Ok(monitors) => monitors
            .iter()
            .map(|monitor| {
                format!(
                    "id={:?} origin=({:?},{:?}) size={:?}x{:?} scale={:?} primary={:?}",
                    monitor.id(),
                    monitor.x(),
                    monitor.y(),
                    monitor.width(),
                    monitor.height(),
                    monitor.scale_factor(),
                    monitor.is_primary()
                )
            })
            .collect(),
        Err(err) => vec![format!("Monitor::all failed: {err}")],
    }
}

/// Index of the display containing `point`, or of the primary one when no
/// rectangle contains it. Entries are `(x, y, width, height, is_primary)`.
///
/// Pure so it can be tested without a display attached.
pub(crate) fn pick_monitor(
    monitors: &[(i32, i32, u32, u32, bool)],
    point: (i32, i32),
) -> Option<usize> {
    let mut primary = None;
    for (index, (x, y, width, height, is_primary)) in monitors.iter().enumerate() {
        let (x, y) = (*x as i64, *y as i64);
        let inside = (point.0 as i64) >= x
            && (point.1 as i64) >= y
            && (point.0 as i64) < x + *width as i64
            && (point.1 as i64) < y + *height as i64;
        if inside {
            return Some(index);
        }
        if primary.is_none() && *is_primary {
            primary = Some(index);
        }
    }
    primary
}

fn monitor_containing(point: (i32, i32)) -> Option<xcap::Monitor> {
    let monitors = xcap::Monitor::all().ok()?;
    let shapes: Vec<(i32, i32, u32, u32, bool)> = monitors
        .iter()
        .map(|monitor| {
            (
                monitor.x().unwrap_or(0),
                monitor.y().unwrap_or(0),
                monitor.width().unwrap_or(0),
                monitor.height().unwrap_or(0),
                monitor.is_primary().unwrap_or(false),
            )
        })
        .collect();
    let index = pick_monitor(&shapes, point)?;
    monitors.into_iter().nth(index)
}

/// Logical bounds of the monitor holding the cursor, in display coordinates.
///
/// Enumerating monitors needs no system permission (only `capture_image` does),
/// so the region-selection overlay can size itself before the user has granted
/// Screen Recording.
pub fn monitor_bounds() -> Option<Rect> {
    let (x, y) = match cursor_position() {
        Some(position) => position,
        None => {
            eprintln!("[capture] cursor position unavailable");
            return None;
        }
    };
    let monitor = match monitor_containing((x, y)) {
        Some(monitor) => monitor,
        None => {
            eprintln!("[capture] no monitor contains ({x},{y})");
            return None;
        }
    };
    let scale = match monitor.scale_factor() {
        Ok(scale) => scale,
        Err(err) => {
            eprintln!("[capture] scale factor unavailable: {err}");
            return None;
        }
    };
    if !scale.is_finite() || scale <= 0.0 {
        eprintln!("[capture] implausible scale factor {scale}");
        return None;
    }
    let divisor = coordinate_divisor(cfg!(windows), scale);
    let (mx, my) = match (monitor.x(), monitor.y()) {
        (Ok(x), Ok(y)) => (x as f32, y as f32),
        _ => {
            eprintln!("[capture] monitor origin unavailable");
            return None;
        }
    };
    let (mw, mh) = match (monitor.width(), monitor.height()) {
        (Ok(w), Ok(h)) => (w as f32, h as f32),
        _ => {
            eprintln!("[capture] monitor size unavailable");
            return None;
        }
    };
    Some(Rect::from_min_size(
        [mx / divisor, my / divisor],
        [mw / divisor, mh / divisor],
    ))
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
/// Accounts for interface zoom independently of OS DPI, and keeps both axes proportional.
pub fn selection_to_logical(rect: Rect, rendered: Point, logical: Point) -> Rect {
    // A zero rendered extent would divide by zero; treat it as 1:1.
    let ratio = [
        if rendered[0] == 0.0 {
            1.0
        } else {
            logical[0] / rendered[0]
        },
        if rendered[1] == 0.0 {
            1.0
        } else {
            logical[1] / rendered[1]
        },
    ];
    Rect::from_min_max(
        [rect.min[0] * ratio[0], rect.min[1] * ratio[1]],
        [rect.max[0] * ratio[0], rect.max[1] * ratio[1]],
    )
}

/// Convert logical UI coordinates into clamped image pixels; reject tiny selections.
pub fn crop(screen: &Screen, rect: Rect) -> Option<image::RgbaImage> {
    if !screen.scale.is_finite()
        || screen.scale <= 0.0
        || !rect.min[0].is_finite()
        || !rect.min[1].is_finite()
        || !rect.max[0].is_finite()
        || !rect.max[1].is_finite()
    {
        return None;
    }
    let w = screen.pixels.width();
    let h = screen.pixels.height();
    let x = (rect.min[0] * screen.scale).floor().max(0.0).min(w as f32) as u32;
    let y = (rect.min[1] * screen.scale).floor().max(0.0).min(h as f32) as u32;
    let right = (rect.max[0] * screen.scale).ceil().max(0.0).min(w as f32) as u32;
    let bottom = (rect.max[1] * screen.scale).ceil().max(0.0).min(h as f32) as u32;
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
    use super::*;

    #[test]
    fn monitor_containment_handles_negative_origins_and_fallback() {
        let monitors = [(-1920, 0, 1920, 1080, false), (0, 0, 2560, 1440, true)];
        assert_eq!(pick_monitor(&monitors, (-100, 100)), Some(0));
        assert_eq!(pick_monitor(&monitors, (1200, 700)), Some(1));
        // Right and bottom edges are exclusive.
        assert_eq!(pick_monitor(&monitors, (2560, 700)), Some(1));
        // Outside every display falls back to the primary.
        assert_eq!(pick_monitor(&monitors, (99_999, 99_999)), Some(1));
        // No primary and no hit.
        assert_eq!(pick_monitor(&[(-10, -10, 5, 5, false)], (500, 500)), None);
        assert_eq!(pick_monitor(&[], (0, 0)), None);
    }

    #[test]
    fn retina_logical_bounds_are_not_scaled_twice() {
        let extent = [1440.0, 900.0];
        let divided = |value: f32, physical: bool| value / coordinate_divisor(physical, 2.0);
        assert_eq!(
            [divided(extent[0], false), divided(extent[1], false)],
            extent
        );
        assert_eq!([divided(2880.0, true), divided(1800.0, true)], extent);
    }

    #[test]
    fn selection_respects_interface_zoom_before_pixel_crop() {
        let logical = [1000.0, 500.0];
        let rendered = [logical[0] / 1.5, logical[1] / 1.5];
        let rect = selection_to_logical(
            Rect::from_min_max([100.0, 50.0], [300.0, 150.0]),
            rendered,
            logical,
        );
        assert!((rect.min[0] - 150.0).abs() < 0.001);
        assert!((rect.max[1] - 225.0).abs() < 0.001);
    }

    #[test]
    fn selection_to_logical_tolerates_zero_extent() {
        let rect = selection_to_logical(
            Rect::from_min_max([1.0, 2.0], [3.0, 4.0]),
            [0.0, 0.0],
            [10.0, 10.0],
        );
        assert_eq!(rect.min, [1.0, 2.0]);
        assert_eq!(rect.max, [3.0, 4.0]);
    }

    #[test]
    fn popup_stays_on_negative_coordinate_monitor() {
        let bounds = Rect::from_min_size([-1920.0, 0.0], [1920.0, 1080.0]);
        assert_eq!(
            clamp_popup([-10.0, 1050.0], bounds, [480.0, 560.0]),
            [-488.0, 512.0]
        );
        assert_eq!(
            clamp_popup([-2500.0, -100.0], bounds, [480.0, 560.0]),
            [-1912.0, 8.0]
        );
    }

    #[test]
    fn drag_rectangle_normalises_direction() {
        let forward = Rect::from_drag([10.0, 20.0], [110.0, 70.0]);
        let backward = Rect::from_drag([110.0, 70.0], [10.0, 20.0]);
        assert_eq!(forward, backward);
        assert_eq!(forward.min, [10.0, 20.0]);
        assert_eq!(forward.max, [110.0, 70.0]);
        assert_eq!(forward.size(), [100.0, 50.0]);
    }

    #[test]
    fn large_regions_shrink_and_transparency_is_composited() {
        let result = prepare_region(image::RgbaImage::from_pixel(
            4000,
            2000,
            image::Rgba([0, 0, 0, 0]),
        ));
        assert_eq!(result.dimensions(), (2000, 1000));
        assert_eq!(result.get_pixel(0, 0).0, [255, 255, 255, 255]);
        let result = prepare_region(image::RgbaImage::from_pixel(
            10,
            10,
            image::Rgba([0, 0, 0, 128]),
        ));
        assert_eq!(result.get_pixel(0, 0).0, [127, 127, 127, 255]);
    }

    #[test]
    fn invalid_selection_and_scale_rejected() {
        let mut screen = Screen {
            pixels: image::RgbaImage::new(10, 10),
            origin: [0.0, 0.0],
            scale: 1.0,
        };
        assert!(crop(&screen, Rect::from_min_max([0.0, 0.0], [f32::NAN, 5.0])).is_none());
        assert!(crop(&screen, Rect::from_min_max([5.0, 5.0], [1.0, 1.0])).is_none());
        screen.scale = 0.0;
        assert!(crop(&screen, Rect::from_min_max([0.0, 0.0], [5.0, 5.0])).is_none());
    }

    #[test]
    fn crop_scaled_and_clamped() {
        let screen = Screen {
            pixels: image::RgbaImage::new(100, 80),
            origin: [0.0, 0.0],
            scale: 2.0,
        };
        let result = crop(&screen, Rect::from_min_max([-3.0, 5.0], [60.0, 30.0])).unwrap();
        assert_eq!(result.dimensions(), (100, 50));
        assert!(crop(&screen, Rect::from_min_max([60.0, 5.0], [70.0, 30.0])).is_none());
    }
}
