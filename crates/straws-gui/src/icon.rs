//! The window icon: a pink crazy straw on a blue disc (icons/straws.svg,
//! drawn by icons/make_icon.py; the PNGs are rendered from it).

use eframe::egui::IconData;

/// The icon, decoded from the 256-pixel PNG built into the program.
pub fn icon() -> IconData {
    decode(include_bytes!("../icons/256x256.png")).unwrap_or_else(|| IconData {
        rgba: vec![0; 4],
        width: 1,
        height: 1,
    })
}

fn decode(bytes: &[u8]) -> Option<IconData> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => return None,
    };
    Some(IconData {
        rgba,
        width: info.width,
        height: info.height,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icon_decodes() {
        let icon = super::icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        // See-through corner, solid middle.
        assert_eq!(icon.rgba[3], 0);
        assert_eq!(icon.rgba[(128 * 256 + 128) * 4 + 3], 255);
    }
}
