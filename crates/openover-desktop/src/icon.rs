use std::sync::Arc;

pub fn window_image() -> Arc<image::RgbaImage> {
    Arc::new(decode(include_bytes!("../assets/openover-64.png")))
}

pub fn tray_image() -> image::RgbaImage {
    decode(tray_bytes())
}

/// The existing mark, plus a small red dot while `unread` is set.
#[cfg(windows)]
pub(crate) fn tray_image_with_dot(unread: bool) -> image::RgbaImage {
    let mut image = tray_image();
    if unread {
        paint_unread_dot(&mut image);
    }
    image
}

#[cfg(windows)]
fn paint_unread_dot(image: &mut image::RgbaImage) {
    let width = image.width();
    let height = image.height();
    if width < 4 || height < 4 {
        return;
    }
    let radius = (width.min(height) / 6).clamp(2, 10);
    let center_x = width - radius - 1;
    let center_y = height - radius - 1;
    let limit = radius * radius;
    let x0 = center_x.saturating_sub(radius);
    let y0 = center_y.saturating_sub(radius);
    let x1 = (center_x + radius).min(width - 1);
    let y1 = (center_y + radius).min(height - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x.abs_diff(center_x);
            let dy = y.abs_diff(center_y);
            if dx * dx + dy * dy <= limit {
                image.put_pixel(x, y, image::Rgba([220, 32, 32, 255]));
            }
        }
    }
}

fn decode(bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory(bytes)
        .expect("Openover icon")
        .into_rgba8()
}

#[cfg(windows)]
fn tray_bytes() -> &'static [u8] {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};

    // The metric is a process-wide integer and takes no pointer.
    let slot = unsafe { GetSystemMetrics(SM_CXSMICON) };
    match slot {
        1..=16 => include_bytes!("../assets/openover-16.png"),
        17..=40 => include_bytes!("../assets/openover-32.png"),
        41.. => include_bytes!("../assets/openover-64.png"),
        _ => include_bytes!("../assets/openover-32.png"),
    }
}

#[cfg(not(windows))]
fn tray_bytes() -> &'static [u8] {
    include_bytes!("../assets/openover-32.png")
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn png_assets_match_their_pixel_size() {
        for (bytes, size) in [
            (include_bytes!("../assets/openover-16.png").as_slice(), 16),
            (include_bytes!("../assets/openover-32.png").as_slice(), 32),
            (include_bytes!("../assets/openover-64.png").as_slice(), 64),
        ] {
            let image = decode(bytes);
            assert_eq!(image.dimensions(), (size, size));
            assert_eq!(image.get_pixel(0, 0).0[3], 0);
            let center = image.get_pixel(size / 2, size / 2).0;
            assert_eq!(center[3], 255);
            assert!(center[2] > center[0]);
        }
    }

    #[test]
    fn windows_icon_contains_16_32_and_64() {
        let bytes = include_bytes!("../assets/openover.ico");
        assert_eq!(&bytes[..4], &[0, 0, 1, 0]);
        let count = u16::from_le_bytes([bytes[4], bytes[5]]);
        assert_eq!(count, 3);
        let mut sizes = Vec::new();
        for index in 0..usize::from(count) {
            let entry = 6 + index * 16;
            sizes.push(bytes[entry]);
            assert_eq!(bytes[entry], bytes[entry + 1]);
            assert_eq!(u16::from_le_bytes([bytes[entry + 4], bytes[entry + 5]]), 1);
            assert_eq!(u16::from_le_bytes([bytes[entry + 6], bytes[entry + 7]]), 32);
        }
        assert_eq!(sizes, [16, 32, 64]);
    }
}
