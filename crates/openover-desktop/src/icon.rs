use std::sync::Arc;

pub fn window_image() -> Arc<image::RgbaImage> {
    Arc::new(decode(include_bytes!("../assets/openover-64.png")))
}

pub fn tray_image() -> image::RgbaImage {
    decode(tray_bytes())
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
