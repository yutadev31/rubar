use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use tiny_skia::{Color, Pixmap};

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

    pub fn render(&mut self, width: u32, height: u32) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).expect("valid bar dimensions");
        pixmap.fill(Color::from_rgba8(31, 35, 43, 255));

        // Keep the text pipeline initialized and ready for modules. No text is
        // painted yet: this milestone intentionally presents a single-color bar.
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
        buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(height as f32),
        );
        buffer.set_wrap(&mut self.font_system, Wrap::None);
        buffer.set_text(&mut self.font_system, "", &Attrs::new(), Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let _ = (&mut self.swash_cache, buffer);

        // tiny-skia stores RGBA pixels. On little-endian machines Wayland's
        // ARGB8888 shm format is laid out as B,G,R,A, so convert explicitly.
        pixmap
            .data()
            .chunks_exact(4)
            .flat_map(|rgba| [rgba[2], rgba[1], rgba[0], rgba[3]])
            .collect()
    }
}
