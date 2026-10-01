use std::error::Error;

use cosmic_text::{
    Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
    Wrap,
};
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

use crate::config::StyleConfig;
use crate::widget::{
    MouseButton, ScrollDirection, Widget, WidgetButton, WidgetContent, WidgetGroups,
};

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

impl<'a> BarRenderer<'a> {
    pub fn new(config: &'a StyleConfig) -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            config,
            hitboxes: Vec::new(),
        }
    }

    pub fn height(&self) -> u32 {
        self.config.height.max(1)
    }

    pub fn render(&mut self, width: u32, height: u32, widgets: &mut WidgetGroups) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).expect("valid bar dimensions");
        let background = parse_color(&self.config.colors.bg).expect("validated background color");
        let text = parse_rgba(&self.config.colors.text).expect("validated text color");
        let text_color = TextColor::rgba(text[0], text[1], text[2], text[3]);
        let font_size = self.config.font_size.max(1.0);
        pixmap.fill(background);
        self.hitboxes.clear();

        self.draw_group(
            &mut pixmap,
            &mut widgets.left,
            width,
            height,
            font_size,
            text_color,
            Alignment::Left,
        );
        self.draw_group(
            &mut pixmap,
            &mut widgets.center,
            width,
            height,
            font_size,
            text_color,
            Alignment::Center,
        );
        self.draw_group(
            &mut pixmap,
            &mut widgets.right,
            width,
            height,
            font_size,
            text_color,
            Alignment::Right,
        );

        // tiny-skia stores RGBA pixels. On little-endian machines Wayland's
        // ARGB8888 shm format is laid out as B,G,R,A, so convert explicitly.
        pixmap
            .data()
            .chunks_exact(4)
            .flat_map(|rgba| [rgba[2], rgba[1], rgba[0], rgba[3]])
            .collect()
    }

    pub fn handle_click(&self, x: f64, button: MouseButton, widgets: &mut WidgetGroups) {
        if let Some(hitbox) = self.hitboxes.iter().find(|hitbox| hitbox.contains(x)) {
            hitbox.dispatch_click(button, widgets);
        }
    }

    pub fn handle_scroll(
        &self,
        x: f64,
        direction: ScrollDirection,
        widgets: &mut WidgetGroups,
    ) -> bool {
        if let Some(hitbox) = self.hitboxes.iter().find(|hitbox| hitbox.contains(x)) {
            hitbox.dispatch_scroll(direction, widgets);
            true
        } else {
            false
        }
    }

    fn draw_group(
        &mut self,
        pixmap: &mut Pixmap,
        widgets: &mut [Box<dyn Widget>],
        width: u32,
        height: u32,
        font_size: f32,
        text_color: TextColor,
        alignment: Alignment,
    ) {
        let mut items = Vec::with_capacity(widgets.len());
        for (index, widget) in widgets.iter_mut().enumerate() {
            let content = widget.content();
            let buttons = match content {
                WidgetContent::Text(text) => vec![WidgetButton {
                    text,
                    padding: None,
                    bold: None,
                    color: None,
                    background: None,
                }],
                WidgetContent::Buttons(buttons) => buttons,
            };
            for (button_index, button) in buttons.into_iter().enumerate() {
                let text_width = self.measure_text(
                    &button.text,
                    width,
                    height,
                    font_size,
                    text_color,
                    button.bold.unwrap_or(self.config.bold),
                );
                let button_padding = button.padding.unwrap_or(self.config.button_padding) as i32;
                items.push((index, button_index, button, text_width, button_padding));
            }
        }

        let spacing = self.config.spacing as i32;
        let button_vertical_padding = self.config.button_vertical_padding;
        let button_spacing = self.config.button_spacing as i32;
        let total_width = items
            .iter()
            .map(|(_, _, _, width, padding)| *width + padding * 2)
            .sum::<i32>()
            + items
                .windows(2)
                .map(|pair| {
                    if pair[0].0 == pair[1].0 {
                        button_spacing
                    } else {
                        spacing
                    }
                })
                .sum::<i32>();
        let mut cursor = match alignment {
            Alignment::Left => self.config.padding as i32,
            Alignment::Center => (width as i32 - total_width) / 2,
            Alignment::Right => width.saturating_sub(self.config.padding) as i32,
        };

        let items = if matches!(alignment, Alignment::Right) {
            items.into_iter().rev().collect::<Vec<_>>()
        } else {
            items
        };
        for (position, (index, button_index, button, text_width, button_padding)) in
            items.iter().enumerate()
        {
            let index = *index;
            let button_index = *button_index;
            let text_width = *text_width;
            let button_padding = *button_padding;
            let button_width = text_width + button_padding * 2;
            let bold = button.bold.unwrap_or(self.config.bold);
            let button_color = button
                .color
                .map(|[r, g, b, a]| TextColor::rgba(r, g, b, a))
                .unwrap_or(text_color);
            match alignment {
                Alignment::Right => {
                    self.draw_button_background(
                        pixmap,
                        cursor - button_width,
                        cursor,
                        height,
                        button_vertical_padding,
                        button.background,
                    );
                    self.hitboxes.push(Hitbox::new(
                        cursor - button_width,
                        cursor,
                        alignment,
                        index,
                        button_index,
                    ));
                    self.draw_text(
                        pixmap,
                        &button.text,
                        cursor - button_padding,
                        height,
                        font_size,
                        button_color,
                        bold,
                    );
                    cursor -=
                        button_width + next_spacing(&items, position, spacing, button_spacing);
                }
                Alignment::Left | Alignment::Center => {
                    self.draw_button_background(
                        pixmap,
                        cursor,
                        cursor + button_width,
                        height,
                        button_vertical_padding,
                        button.background,
                    );
                    self.hitboxes.push(Hitbox::new(
                        cursor,
                        cursor + button_width,
                        alignment,
                        index,
                        button_index,
                    ));
                    self.draw_text(
                        pixmap,
                        &button.text,
                        cursor + button_padding + text_width,
                        height,
                        font_size,
                        button_color,
                        bold,
                    );
                    cursor +=
                        button_width + next_spacing(&items, position, spacing, button_spacing);
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
        vertical_padding: u32,
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

        let vertical_padding = vertical_padding.min(height / 2);
        let y = vertical_padding as f32;
        let rect_height = height.saturating_sub(vertical_padding * 2) as f32;
        if rect_height <= 0.0 {
            return;
        }
        let rect = Rect::from_xywh(left, y, width, rect_height)
            .expect("valid button background rectangle");
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }

    fn measure_text(
        &mut self,
        text: &str,
        width: u32,
        height: u32,
        font_size: f32,
        text_color: TextColor,
        bold: bool,
    ) -> i32 {
        let line_height = font_size * 1.2857;
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
        buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(height as f32),
        );
        buffer.set_wrap(&mut self.font_system, Wrap::None);
        let mut attrs = Attrs::new().color(text_color);
        if !self.config.font_family.is_empty() {
            attrs = attrs.family(Family::Name(&self.config.font_family));
        }
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced);
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
        height: u32,
        font_size: f32,
        text_color: TextColor,
        bold: bool,
    ) -> i32 {
        let line_height = font_size * 1.2857;
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
        buffer.set_size(
            &mut self.font_system,
            Some(pixmap.width() as f32),
            Some(height as f32),
        );
        buffer.set_wrap(&mut self.font_system, Wrap::None);
        let mut attrs = Attrs::new().color(text_color);
        if !self.config.font_family.is_empty() {
            attrs = attrs.family(Family::Name(&self.config.font_family));
        }
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        buffer.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced);
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

fn next_spacing(
    items: &[(usize, usize, WidgetButton, i32, i32)],
    position: usize,
    spacing: i32,
    button_spacing: i32,
) -> i32 {
    items
        .get(position + 1)
        .map(|next| {
            if items[position].0 == next.0 {
                button_spacing
            } else {
                spacing
            }
        })
        .unwrap_or(0)
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
    alignment: Alignment,
    index: usize,
    item: usize,
}

impl Hitbox {
    fn new(left: i32, right: i32, alignment: Alignment, index: usize, item: usize) -> Self {
        Self {
            left,
            right,
            alignment,
            index,
            item,
        }
    }

    fn contains(&self, x: f64) -> bool {
        x >= self.left as f64 && x < self.right as f64
    }

    fn dispatch_click(&self, button: MouseButton, widgets: &mut WidgetGroups) {
        match self.alignment {
            Alignment::Left => widgets.left[self.index].on_click(button, self.item),
            Alignment::Center => widgets.center[self.index].on_click(button, self.item),
            Alignment::Right => widgets.right[self.index].on_click(button, self.item),
        }
    }

    fn dispatch_scroll(&self, direction: ScrollDirection, widgets: &mut WidgetGroups) {
        match self.alignment {
            Alignment::Left => widgets.left[self.index].on_scroll(direction, self.item),
            Alignment::Center => widgets.center[self.index].on_scroll(direction, self.item),
            Alignment::Right => widgets.right[self.index].on_scroll(direction, self.item),
        }
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
