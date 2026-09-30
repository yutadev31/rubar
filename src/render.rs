use cosmic_text::{
    Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};
use tiny_skia::{Color, Pixmap};

use crate::widget::Widget;

pub const BAR_HEIGHT: u32 = 28;

/// Renderer shared by window-system backends.
///
/// The first version only paints a solid background. `cosmic-text` is kept here
/// rather than in the Wayland code so text layout/rasterization can later be
/// reused by an X11 backend without changing the backend contract.
pub struct BarRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl BarRenderer {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    pub fn render(&mut self, width: u32, height: u32, widgets: &mut [Box<dyn Widget>]) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).expect("valid bar dimensions");
        pixmap.fill(Color::from_rgba8(31, 35, 43, 255));

        let mut right = width as i32;
        for widget in widgets.iter() {
            let widget_width = self.draw_text(&mut pixmap, &widget.text(), right, height);
            right -= widget_width;
        }

        // tiny-skia stores RGBA pixels. On little-endian machines Wayland's
        // ARGB8888 shm format is laid out as B,G,R,A, so convert explicitly.
        pixmap
            .data()
            .chunks_exact(4)
            .flat_map(|rgba| [rgba[2], rgba[1], rgba[0], rgba[3]])
            .collect()
    }

    fn draw_text(&mut self, pixmap: &mut Pixmap, text: &str, right: i32, height: u32) -> i32 {
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
        buffer.set_size(
            &mut self.font_system,
            Some(pixmap.width() as f32),
            Some(height as f32),
        );
        buffer.set_wrap(&mut self.font_system, Wrap::None);
        buffer.set_text(
            &mut self.font_system,
            text,
            &Attrs::new().color(TextColor::rgb(0xFF, 0xFF, 0xFF)),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);
        let text_width = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
            .ceil() as i32;
        let x = right - text_width;

        let text_y = ((height as f32 - 18.0) / 2.0).max(0.0) as i32;
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            TextColor::rgb(0xFF, 0xFF, 0xFF),
            |glyph_x, glyph_y, _w, _h, color| {
                let px = x + glyph_x;
                let py = text_y + glyph_y;
                if px < 0 || py < 0 || px >= pixmap.width() as i32 || py >= pixmap.height() as i32 {
                    return;
                }
                let pixel = (py as u32 * pixmap.width() + px as u32) as usize * 4;
                let [r, g, b, coverage] = color.as_rgba();
                let data = pixmap.data_mut();
                let inverse = 255 - coverage as u16;
                let blend = |source: u8, destination: u8| {
                    ((source as u16 * coverage as u16 + destination as u16 * inverse) / 255) as u8
                };
                let destination = [data[pixel], data[pixel + 1], data[pixel + 2]];
                data[pixel..pixel + 4].copy_from_slice(&[
                    blend(r, destination[0]),
                    blend(g, destination[1]),
                    blend(b, destination[2]),
                    255,
                ]);
            },
        );
        text_width
    }
}
