use std::error::Error;

use cosmic_text::{
    Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
    Wrap,
};
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

use crate::config::StyleConfig;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Other(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug)]
pub struct ClickContext<'a> {
    pub button_left: i32,
    pub surface_width: u32,
    pub output: Option<&'a str>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenderContent {
    Text(String),
    Buttons(Vec<RenderButton>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct RenderButton {
    pub text: Option<String>,
    pub icon: Option<RenderIcon>,
    pub padding: Option<u32>,
    pub bold: Option<bool>,
    pub color: Option<[u8; 4]>,
    pub background: Option<[u8; 4]>,
    pub indicator: Option<([u8; 4], u32, bool)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderIcon {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub trait RenderWidget {
    fn content(&mut self) -> RenderContent;
    fn content_spacing(&self) -> Option<u32> {
        None
    }
    fn set_monitor_name(&mut self, _monitor_name: Option<&str>) {}
    fn on_click(&mut self, _button: MouseButton, _item: usize) {}
    fn on_click_at(&mut self, button: MouseButton, item: usize, _context: ClickContext<'_>) {
        self.on_click(button, item);
    }
    fn on_scroll(&mut self, _direction: ScrollDirection, _item: usize) {}
}

/// Renderer shared by window-system backends.
///
/// The first version only paints a solid background. `cosmic-text` is kept here
/// rather than in the Wayland code so text layout/rasterization can later be
/// reused by an X11 backend without changing the backend contract.
pub struct BarRenderer<'a> {
    font_system: FontSystem,
    swash_cache: SwashCache,
    config: &'a StyleConfig,
    hitboxes: Vec<Hitbox>,
}

struct RenderMetrics {
    width: u32,
    height: u32,
    font_size: f32,
    text_color: TextColor,
}

type RenderItem = (usize, usize, RenderButton, i32, i32, Option<i32>);

impl<'a> BarRenderer<'a> {
    pub fn new(config: &'a StyleConfig) -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            config,
            hitboxes: Vec::new(),
        }
    }

    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        left: &mut [Box<dyn RenderWidget>],
        center: &mut [Box<dyn RenderWidget>],
        right: &mut [Box<dyn RenderWidget>],
    ) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).expect("valid bar dimensions");
        let background = parse_color(&self.config.colors.bg).expect("validated background color");
        let text = parse_rgba(&self.config.colors.text).expect("validated text color");
        let text_color = TextColor::rgba(text[0], text[1], text[2], text[3]);
        let metrics = RenderMetrics {
            width,
            height,
            font_size: self.config.font_size.max(1.0),
            text_color,
        };
        pixmap.fill(background);
        self.hitboxes.clear();

        self.draw_group(&mut pixmap, left, &metrics, Alignment::Left);
        self.draw_group(&mut pixmap, center, &metrics, Alignment::Center);
        self.draw_group(&mut pixmap, right, &metrics, Alignment::Right);

        // tiny-skia stores RGBA pixels. On little-endian machines Wayland's
        // ARGB8888 shm format is laid out as B,G,R,A, so convert explicitly.
        let (pixels, _) = pixmap.data().as_chunks::<4>();
        pixels
            .iter()
            .flat_map(|rgba| [rgba[2], rgba[1], rgba[0], rgba[3]])
            .collect()
    }

    pub fn handle_click(
        &self,
        x: f64,
        button: MouseButton,
        output: Option<&str>,
        left: &mut [Box<dyn RenderWidget>],
        center: &mut [Box<dyn RenderWidget>],
        right: &mut [Box<dyn RenderWidget>],
    ) {
        if let Some(hitbox) = self.hitboxes.iter().find(|hitbox| hitbox.contains(x)) {
            hitbox.dispatch_click(button, output, left, center, right);
        }
    }

    pub fn handle_scroll(
        &self,
        x: f64,
        direction: ScrollDirection,
        left: &mut [Box<dyn RenderWidget>],
        center: &mut [Box<dyn RenderWidget>],
        right: &mut [Box<dyn RenderWidget>],
    ) -> bool {
        if let Some(hitbox) = self.hitboxes.iter().find(|hitbox| hitbox.contains(x)) {
            hitbox.dispatch_scroll(direction, left, center, right);
            true
        } else {
            false
        }
    }

    fn draw_group(
        &mut self,
        pixmap: &mut Pixmap,
        widgets: &mut [Box<dyn RenderWidget>],
        metrics: &RenderMetrics,
        alignment: Alignment,
    ) {
        let mut items = Vec::with_capacity(widgets.len());
        for (index, widget) in widgets.iter_mut().enumerate() {
            let content = widget.content();
            let buttons = match content {
                RenderContent::Text(text) => vec![RenderButton {
                    text: Some(text),
                    icon: None,
                    padding: None,
                    bold: None,
                    color: None,
                    background: None,
                    indicator: None,
                }],
                RenderContent::Buttons(buttons) => buttons,
            };
            let content_spacing = widget.content_spacing().map(|spacing| spacing as i32);
            for (button_index, button) in buttons.into_iter().enumerate() {
                let text_width = button.text.as_deref().map_or(0, |text| {
                    self.measure_text(text, metrics, button.bold.unwrap_or(self.config.bold))
                });
                let button_padding = button.padding.unwrap_or(self.config.button_padding) as i32;
                items.push((
                    index,
                    button_index,
                    button,
                    text_width,
                    button_padding,
                    content_spacing,
                ));
            }
        }

        let spacing = self.config.spacing as i32;
        let total_width = items
            .iter()
            .map(|(_, _, button, width, padding, _)| {
                let icon_width = button.icon.as_ref().map_or(0, |icon| icon.width as i32);
                (*width).max(icon_width) + padding * 2
            })
            .sum::<i32>()
            + items
                .windows(2)
                .map(|pair| next_spacing(pair[0].0, pair[1].0, pair[0].5, spacing))
                .sum::<i32>();
        let mut cursor = match alignment {
            Alignment::Left => self.config.padding as i32,
            Alignment::Center => (metrics.width as i32 - total_width) / 2,
            Alignment::Right => metrics.width.saturating_sub(self.config.padding) as i32,
        };

        let items = if matches!(alignment, Alignment::Right) {
            items.into_iter().rev().collect::<Vec<_>>()
        } else {
            items
        };
        for (position, (index, button_index, button, text_width, button_padding, _)) in
            items.iter().enumerate()
        {
            let index = *index;
            let button_index = *button_index;
            let text_width = *text_width;
            let button_padding = *button_padding;
            let icon_width = button.icon.as_ref().map_or(0, |icon| icon.width as i32);
            let content_width = text_width.max(icon_width);
            let button_width = content_width + button_padding * 2;
            let bold = button.bold.unwrap_or(self.config.bold);
            let button_color = button
                .color
                .map(|[r, g, b, a]| TextColor::rgba(r, g, b, a))
                .unwrap_or(metrics.text_color);
            match alignment {
                Alignment::Right => {
                    self.draw_button_background(
                        pixmap,
                        cursor - button_width,
                        cursor,
                        metrics.height,
                        button.background,
                    );
                    self.draw_button_indicator(
                        pixmap,
                        cursor - button_width,
                        cursor,
                        metrics.height,
                        button.indicator,
                    );
                    self.hitboxes.push(Hitbox::new(
                        cursor - button_width,
                        cursor,
                        metrics.width,
                        alignment,
                        index,
                        button_index,
                    ));
                    if let Some(text) = button.text.as_deref() {
                        self.draw_text(
                            pixmap,
                            text,
                            cursor - button_padding,
                            metrics,
                            button_color,
                            bold,
                        );
                    }
                    if let Some(icon) = button.icon.as_ref() {
                        self.draw_icon(pixmap, icon, cursor - button_padding - icon_width);
                    }
                    cursor -= button_width + next_spacing_for_position(&items, position, spacing);
                }
                Alignment::Left | Alignment::Center => {
                    self.draw_button_background(
                        pixmap,
                        cursor,
                        cursor + button_width,
                        metrics.height,
                        button.background,
                    );
                    self.draw_button_indicator(
                        pixmap,
                        cursor,
                        cursor + button_width,
                        metrics.height,
                        button.indicator,
                    );
                    self.hitboxes.push(Hitbox::new(
                        cursor,
                        cursor + button_width,
                        metrics.width,
                        alignment,
                        index,
                        button_index,
                    ));
                    if let Some(text) = button.text.as_deref() {
                        self.draw_text(
                            pixmap,
                            text,
                            cursor + button_padding + text_width,
                            metrics,
                            button_color,
                            bold,
                        );
                    }
                    if let Some(icon) = button.icon.as_ref() {
                        self.draw_icon(pixmap, icon, cursor + button_padding);
                    }
                    cursor += button_width + next_spacing_for_position(&items, position, spacing);
                }
            }
        }
    }

    fn draw_button_background(
        &self,
        pixmap: &mut Pixmap,
        left: i32,
        right: i32,
        height: u32,
        background: Option<[u8; 4]>,
    ) {
        let Some([r, g, b, a]) = background else {
            return;
        };
        if right <= left || height == 0 {
            return;
        }

        let mut paint = Paint::default();
        paint.set_color_rgba8(r, g, b, a);

        let left = left.max(0) as f32;
        let right = right.min(pixmap.width() as i32) as f32;
        let width = right - left;
        if width <= 0.0 {
            return;
        }

        let rect = Rect::from_xywh(left, 0.0, width, height as f32)
            .expect("valid button background rectangle");
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }

    fn draw_button_indicator(
        &self,
        pixmap: &mut Pixmap,
        left: i32,
        right: i32,
        height: u32,
        indicator: Option<([u8; 4], u32, bool)>,
    ) {
        let Some(([r, g, b, a], indicator_height, at_top)) = indicator else {
            return;
        };
        let indicator_height = indicator_height.min(height);
        if right <= left || indicator_height == 0 || height == 0 {
            return;
        }
        let mut paint = Paint::default();
        paint.set_color_rgba8(r, g, b, a);
        let left = left.max(0) as f32;
        let right = right.min(pixmap.width() as i32) as f32;
        let width = right - left;
        if width <= 0.0 {
            return;
        }
        let y = if at_top {
            0.0
        } else {
            (height - indicator_height) as f32
        };
        let rect = Rect::from_xywh(left, y, width, indicator_height as f32)
            .expect("valid button indicator rectangle");
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }

    fn measure_text(&mut self, text: &str, metrics: &RenderMetrics, bold: bool) -> i32 {
        let line_height = metrics.font_size * 1.2857;
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(metrics.font_size, line_height),
        );
        buffer.set_size(Some(metrics.width as f32), Some(metrics.height as f32));
        buffer.set_wrap(Wrap::None);
        let mut attrs = Attrs::new().color(metrics.text_color);
        if !self.config.font_family.is_empty() {
            attrs = attrs.family(Family::Name(&self.config.font_family));
        }
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
            .ceil() as i32
    }

    fn draw_text(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        right: i32,
        metrics: &RenderMetrics,
        text_color: TextColor,
        bold: bool,
    ) -> i32 {
        let line_height = metrics.font_size * 1.2857;
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(metrics.font_size, line_height),
        );
        buffer.set_size(Some(pixmap.width() as f32), Some(metrics.height as f32));
        buffer.set_wrap(Wrap::None);
        let mut attrs = Attrs::new().color(text_color);
        if !self.config.font_family.is_empty() {
            attrs = attrs.family(Family::Name(&self.config.font_family));
        }
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let text_width = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
            .ceil() as i32;
        let x = right - text_width;

        let text_y = ((metrics.height as f32 - line_height) / 2.0).max(0.0) as i32;
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

    fn draw_icon(&self, pixmap: &mut Pixmap, icon: &RenderIcon, x: i32) {
        if icon.width == 0
            || icon.height == 0
            || icon.pixels.len()
                != (icon.width as usize)
                    .saturating_mul(icon.height as usize)
                    .saturating_mul(4)
        {
            return;
        }
        let y = ((pixmap.height() as i32 - icon.height as i32) / 2).max(0);
        for iy in 0..icon.height {
            for ix in 0..icon.width {
                let px = x + ix as i32;
                let py = y + iy as i32;
                if px < 0 || py < 0 || px >= pixmap.width() as i32 || py >= pixmap.height() as i32 {
                    continue;
                }
                let source = (iy as usize * icon.width as usize + ix as usize) * 4;
                let destination = (py as usize * pixmap.width() as usize + px as usize) * 4;
                let [sr, sg, sb, sa] = [
                    icon.pixels[source],
                    icon.pixels[source + 1],
                    icon.pixels[source + 2],
                    icon.pixels[source + 3],
                ];
                let da = pixmap.data()[destination + 3];
                let alpha = sa as u16 + ((da as u16 * (255 - sa as u16)) / 255);
                if alpha == 0 {
                    continue;
                }
                let inverse = 255 - sa as u16;
                let data = pixmap.data_mut();
                let dr = data[destination];
                let dg = data[destination + 1];
                let db = data[destination + 2];
                data[destination..destination + 4].copy_from_slice(&[
                    (sr as u16 + dr as u16 * inverse / 255).min(255) as u8,
                    (sg as u16 + dg as u16 * inverse / 255).min(255) as u8,
                    (sb as u16 + db as u16 * inverse / 255).min(255) as u8,
                    alpha.min(255) as u8,
                ]);
            }
        }
    }
}

fn next_spacing(
    current_index: usize,
    next_index: usize,
    content_spacing: Option<i32>,
    spacing: i32,
) -> i32 {
    if current_index == next_index {
        content_spacing.unwrap_or(spacing)
    } else {
        spacing
    }
}

fn next_spacing_for_position(items: &[RenderItem], position: usize, spacing: i32) -> i32 {
    items.get(position + 1).map_or(0, |next| {
        next_spacing(items[position].0, next.0, items[position].5, spacing)
    })
}

#[derive(Clone, Copy)]
enum Alignment {
    Left,
    Center,
    Right,
}

struct Hitbox {
    left: i32,
    right: i32,
    surface_width: u32,
    alignment: Alignment,
    index: usize,
    item: usize,
}

impl Hitbox {
    fn new(
        left: i32,
        right: i32,
        surface_width: u32,
        alignment: Alignment,
        index: usize,
        item: usize,
    ) -> Self {
        Self {
            left,
            right,
            surface_width,
            alignment,
            index,
            item,
        }
    }

    fn contains(&self, x: f64) -> bool {
        x >= self.left as f64 && x < self.right as f64
    }

    fn dispatch_click(
        &self,
        button: MouseButton,
        output: Option<&str>,
        left: &mut [Box<dyn RenderWidget>],
        center: &mut [Box<dyn RenderWidget>],
        right: &mut [Box<dyn RenderWidget>],
    ) {
        let context = ClickContext {
            button_left: self.left,
            surface_width: self.surface_width,
            output,
        };
        match self.alignment {
            Alignment::Left => left[self.index].on_click_at(button, self.item, context),
            Alignment::Center => center[self.index].on_click_at(button, self.item, context),
            Alignment::Right => right[self.index].on_click_at(button, self.item, context),
        }
    }

    fn dispatch_scroll(
        &self,
        direction: ScrollDirection,
        left: &mut [Box<dyn RenderWidget>],
        center: &mut [Box<dyn RenderWidget>],
        right: &mut [Box<dyn RenderWidget>],
    ) {
        match self.alignment {
            Alignment::Left => left[self.index].on_scroll(direction, self.item),
            Alignment::Center => center[self.index].on_scroll(direction, self.item),
            Alignment::Right => right[self.index].on_scroll(direction, self.item),
        }
    }
}

fn parse_color(value: &str) -> Result<Color, Box<dyn Error>> {
    let [r, g, b, a] = parse_rgba(value)?;
    Ok(Color::from_rgba8(r, g, b, a))
}

pub(crate) fn parse_rgba(value: &str) -> Result<[u8; 4], Box<dyn Error>> {
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
