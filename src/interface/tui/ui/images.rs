//! Issue images, drawn over rows the Markdown renderer leaves free: sixel,
//! kitty, iTerm2, or half blocks.

use std::collections::HashMap;

use image::DynamicImage;
use ratatui::{
    Frame,
    layout::{Rect, Size},
};
use ratatui_image::picker::Picker;
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};

use crate::core::store::Image;

/// Tallest image, in rows.
const MAX_ROWS: u16 = 30;

#[derive(Default)]
pub struct Images {
    /// `None`: no images (headless).
    picker: Option<Picker>,
    /// By URL; `None` if not decodable.
    decoded: HashMap<String, Option<DynamicImage>>,
    /// Encoded per drawn size.
    encoded: HashMap<(String, Size), SlicedProtocol>,
}

impl std::fmt::Debug for Images {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Images")
            .field("picker", &self.picker)
            .field("decoded", &self.decoded.len())
            .field("encoded", &self.encoded.len())
            .finish()
    }
}

impl Images {
    pub fn enable(&mut self, picker: Picker) {
        self.picker = Some(picker);
    }

    /// Keep the picker across a session restart.
    pub fn carry_over(&mut self) -> Self {
        Self {
            picker: self.picker.take(),
            ..Self::default()
        }
    }

    /// Cells the image takes within `width`; `None` until drawable.
    // ponytail: decodes on the UI thread, once per image; move off-thread if it stutters.
    pub fn size(&mut self, url: &str, files: &HashMap<String, Image>, width: u16) -> Option<Size> {
        let font = self.picker.as_ref()?.font_size();
        let Some(Image::Loaded(file)) = files.get(url) else {
            return None;
        };
        let image = self
            .decoded
            .entry(url.to_owned())
            .or_insert_with(|| {
                image::load_from_memory(file)
                    .inspect_err(|e| tracing::warn!(%url, "image not decoded: {e}"))
                    .ok()
            })
            .as_ref()?;
        fit(
            (image.width(), image.height()),
            (font.width, font.height),
            width,
        )
    }

    /// Draw at `position` in `area`, clipped to it.
    pub fn draw(
        &mut self,
        f: &mut Frame,
        url: &str,
        size: Size,
        area: Rect,
        position: SignedPosition,
    ) {
        let (Some(picker), Some(Some(image))) = (&self.picker, self.decoded.get(url)) else {
            return;
        };
        let key = (url.to_owned(), size);
        if !self.encoded.contains_key(&key) {
            match SlicedProtocol::new(picker, image.clone(), Some(size)) {
                Ok(encoded) => {
                    self.encoded.insert(key.clone(), encoded);
                }
                Err(e) => {
                    tracing::warn!(%url, "image not encoded: {e}");
                    return;
                }
            }
        }
        f.render_widget(SlicedImage::new(&self.encoded[&key], position), area);
        unwrap_tmux(f, area);
    }
}

/// Send sixel to tmux unwrapped: tmux 3.4 keeps it in the pane, so it
/// clears on a window switch and scrolls with the text. Passthrough would not.
fn unwrap_tmux(f: &mut Frame, area: Rect) {
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if let Some(inner) = cell.symbol().strip_prefix("\x1bPtmux;")
                && let Some(inner) = inner.strip_suffix("\x1b\\")
            {
                let raw = inner.replace("\x1b\x1b", "\x1b");
                cell.set_symbol(&raw);
            }
        }
    }
}

/// Cells for `pixels` in a `font`-sized cell, within `width` x [`MAX_ROWS`], aspect kept.
fn fit(pixels: (u32, u32), font: (u16, u16), width: u16) -> Option<Size> {
    let (w, h) = (u64::from(pixels.0), u64::from(pixels.1));
    let (fw, fh) = (u64::from(font.0.max(1)), u64::from(font.1.max(1)));
    if w == 0 || h == 0 || width == 0 {
        return None;
    }
    let mut cols = w.div_ceil(fw).min(u64::from(width));
    let mut rows = (h * cols * fw).div_ceil(w * fh);
    if rows > u64::from(MAX_ROWS) {
        rows = u64::from(MAX_ROWS);
        cols = (w * rows * fh).div_ceil(h * fw).clamp(1, u64::from(width));
    }
    Some(Size::new(cols as u16, rows.max(1) as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small stays natural; wide fits width; tall caps at MAX_ROWS.
    #[test]
    fn an_image_is_scaled_down_to_fit_keeping_its_shape() {
        // 10x20 px cells: 100x100 px is 10 columns by 5 rows.
        assert_eq!(fit((100, 100), (10, 20), 80), Some(Size::new(10, 5)));
        // 2000x1000 px into 50 columns: 500 px wide, so 250 px or 13 rows.
        assert_eq!(fit((2000, 1000), (10, 20), 50), Some(Size::new(50, 13)));
        // 100x2000 px: 100 rows at natural size, held to 30.
        let tall = fit((100, 2000), (10, 20), 80).unwrap();
        assert_eq!(tall.height, MAX_ROWS);
        assert!(tall.width < 10);
        assert_eq!(fit((0, 10), (10, 20), 80), None);
    }
}
