use std::error::Error;

use cosmic_text::{
    Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};
use tiny_skia::{Color, Pixmap};

use crate::config::StyleConfig;
use crate::widget::Widget;

/// Renderer shared by window-system backends.
///
/// The first version only paints a solid background. `cosmic-text` is kept here
/// rather than in the Wayland code so text layout/rasterization can later be
/// reused by an X11 backend without changing the backend contract.
pub struct BarRenderer<'a> {
    font_system: FontSystem,
    swash_cache: SwashCache,
    config: &'a StyleConfig,
}

impl<'a> BarRenderer<'a> {
    pub fn new(config: &'a StyleConfig) -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            config,
        }
    }

    pub fn height(&self) -> u32 {
        self.config.height.max(1)
    }

    pub fn render(&mut self, width: u32, height: u32, widgets: &mut [Box<dyn Widget>]) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).expect("valid bar dimensions");
        let background = parse_color(&self.config.colors.bg).expect("validated background color");
        let text = parse_rgba(&self.config.colors.text).expect("validated text color");
        let text_color = TextColor::rgba(text[0], text[1], text[2], text[3]);
        let font_size = self.config.font_size.max(1.0);
        pixmap.fill(background);

        let mut right = width.saturating_sub(self.config.padding) as i32;
        for widget in widgets.iter_mut() {
            let widget_width = self.draw_text(
                &mut pixmap,
                &widget.text(),
                right,
                height,
                font_size,
                text_color,
            );
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

    fn draw_text(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        right: i32,
        height: u32,
        font_size: f32,
        text_color: TextColor,
    ) -> i32 {
        let line_height = font_size * 1.2857;
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
        buffer.set_size(
            &mut self.font_system,
            Some(pixmap.width() as f32),
            Some(height as f32),
        );
        buffer.set_wrap(&mut self.font_system, Wrap::None);
        buffer.set_text(
            &mut self.font_system,
            text,
            &Attrs::new().color(text_color),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);
        let text_width = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
            .ceil() as i32;
        let x = right - text_width;

        let text_y = ((height as f32 - line_height) / 2.0).max(0.0) as i32;
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color,
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

fn parse_color(value: &str) -> Result<Color, Box<dyn Error>> {
    let [r, g, b, a] = parse_rgba(value)?;
    Ok(Color::from_rgba8(r, g, b, a))
}

fn parse_rgba(value: &str) -> Result<[u8; 4], Box<dyn Error>> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let (rgb, alpha) = match hex.len() {
        6 => (hex, 255),
        8 => (&hex[..6], u8::from_str_radix(&hex[6..], 16)?),
        _ => return Err(format!("invalid color `{value}`; expected #RRGGBB or #RRGGBBAA").into()),
    };
    Ok([
        u8::from_str_radix(&rgb[0..2], 16)?,
        u8::from_str_radix(&rgb[2..4], 16)?,
        u8::from_str_radix(&rgb[4..6], 16)?,
        alpha,
    ])
}
