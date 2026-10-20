use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
    Wrap,
};
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    color::parse_rgba,
    config::{Config, StyleConfig},
};
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
    /// Maximum width of the whole button, including padding, in pixels.
    pub max_width: Option<u32>,
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
    fn reload_config(&mut self, _config: &Config) {}
    fn poll_changed(&mut self) -> bool {
        false
    }
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
pub struct BarRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    config: StyleConfig,
    background: Color,
    text_color: TextColor,
    hitboxes: HashMap<Option<String>, Vec<Hitbox>>,
    current_hitboxes: Vec<Hitbox>,
}

struct RenderMetrics {
    width: u32,
    height: u32,
    font_size: f32,
    text_color: TextColor,
}

struct PreparedItem {
    widget_index: usize,
    button_index: usize,
    button: RenderButton,
    text_width: i32,
    padding: i32,
    content_spacing: Option<i32>,
}

impl PreparedItem {
    fn width(&self) -> i32 {
        let icon_width = self
            .button
            .icon
            .as_ref()
            .map_or(0, |icon| icon.width as i32);
        self.text_width
            .max(icon_width)
            .saturating_add(self.padding.saturating_mul(2))
    }
}

impl BarRenderer {
    pub fn new(config: &StyleConfig) -> Result<Self, String> {
        let [r, g, b, a] = parse_rgba(&config.colors.bg)?;
        let text = parse_rgba(&config.colors.text)?;
        Ok(Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            config: config.clone(),
            background: Color::from_rgba8(r, g, b, a),
            text_color: TextColor::rgba(text[0], text[1], text[2], text[3]),
            hitboxes: HashMap::new(),
            current_hitboxes: Vec::new(),
        })
    }

    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        output: Option<&str>,
        left: &mut [Box<dyn RenderWidget>],
        center: &mut [Box<dyn RenderWidget>],
        right: &mut [Box<dyn RenderWidget>],
    ) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).expect("valid bar dimensions");
        let metrics = RenderMetrics {
            width,
            height,
            font_size: self.config.font_size.max(1.0),
            text_color: self.text_color,
        };
        pixmap.fill(self.background);
        self.current_hitboxes.clear();

        self.draw_group(&mut pixmap, left, &metrics, Alignment::Left);
        self.draw_group(&mut pixmap, center, &metrics, Alignment::Center);
        self.draw_group(&mut pixmap, right, &metrics, Alignment::Right);
        self.hitboxes.insert(
            output.map(str::to_owned),
            std::mem::take(&mut self.current_hitboxes),
        );

        // tiny-skia stores RGBA pixels; shell-surface expects native ARGB8888.
        let (pixels, _) = pixmap.data().as_chunks::<4>();
        pixels
            .iter()
            .flat_map(|rgba| native_argb8888(*rgba))
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
        if let Some(hitbox) = self
            .hitboxes
            .get(&output.map(str::to_owned))
            .and_then(|hitboxes| hitboxes.iter().find(|hitbox| hitbox.contains(x)))
        {
            hitbox.dispatch_click(button, output, left, center, right);
        }
    }

    pub fn handle_scroll(
        &self,
        x: f64,
        direction: ScrollDirection,
        output: Option<&str>,
        left: &mut [Box<dyn RenderWidget>],
        center: &mut [Box<dyn RenderWidget>],
        right: &mut [Box<dyn RenderWidget>],
    ) -> bool {
        if let Some(hitbox) = self
            .hitboxes
            .get(&output.map(str::to_owned))
            .and_then(|hitboxes| hitboxes.iter().find(|hitbox| hitbox.contains(x)))
        {
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
        let mut items = self.prepare_items(widgets, metrics);
        let module_spacing = self.config.module_spacing as i32;
        let item_spacing = self.config.item_spacing as i32;
        let total_width = items
            .iter()
            .map(PreparedItem::width)
            .chain(items.windows(2).map(|pair| {
                next_spacing(
                    pair[0].widget_index,
                    pair[1].widget_index,
                    pair[0].content_spacing,
                    module_spacing,
                    item_spacing,
                )
            }))
            .fold(0_i32, i32::saturating_add);
        let mut cursor = match alignment {
            Alignment::Left => self.config.bar_padding as i32,
            Alignment::Center => (metrics.width as i32 - total_width) / 2,
            Alignment::Right => metrics.width.saturating_sub(self.config.bar_padding) as i32,
        };

        if matches!(alignment, Alignment::Right) {
            items.reverse();
        }
        for (position, item) in items.iter().enumerate() {
            let width = item.width();
            let (left, right) = match alignment {
                Alignment::Right => (cursor.saturating_sub(width), cursor),
                Alignment::Left | Alignment::Center => (cursor, cursor.saturating_add(width)),
            };
            self.draw_prepared_item(pixmap, item, metrics, alignment, left, right);
            let spacing = next_spacing_for_position(&items, position, module_spacing, item_spacing);
            cursor = match alignment {
                Alignment::Right => left.saturating_sub(spacing),
                Alignment::Left | Alignment::Center => right.saturating_add(spacing),
            };
        }
    }

    fn prepare_items(
        &mut self,
        widgets: &mut [Box<dyn RenderWidget>],
        metrics: &RenderMetrics,
    ) -> Vec<PreparedItem> {
        let mut items = Vec::with_capacity(widgets.len());
        for (index, widget) in widgets.iter_mut().enumerate() {
            let content = widget.content();
            let buttons = match content {
                RenderContent::Text(text) => vec![RenderButton {
                    text: Some(text),
                    icon: None,
                    padding: None,
                    max_width: None,
                    bold: None,
                    color: None,
                    background: None,
                    indicator: None,
                }],
                RenderContent::Buttons(buttons) => buttons,
            };
            let content_spacing = widget.content_spacing().map(|spacing| spacing as i32);
            for (button_index, mut button) in buttons.into_iter().enumerate() {
                let button_padding = button.padding.unwrap_or(self.config.item_padding) as i32;
                let button_padding = button.max_width.map_or(button_padding, |max_width| {
                    button_padding.min(max_width as i32 / 2)
                });
                let text_width = if let Some(text) = button.text.take() {
                    let (text, width) = if let Some(max_width) = button.max_width {
                        let available = max_width as i32 - button_padding * 2;
                        self.fit_text(
                            &text,
                            available,
                            metrics,
                            button.bold.unwrap_or(self.config.bold),
                        )
                    } else {
                        let width = self.measure_text(
                            &text,
                            metrics,
                            button.bold.unwrap_or(self.config.bold),
                        );
                        (text, width)
                    };
                    button.text = Some(text);
                    width
                } else {
                    0
                };
                items.push(PreparedItem {
                    widget_index: index,
                    button_index,
                    button,
                    text_width,
                    padding: button_padding,
                    content_spacing,
                });
            }
        }
        items
    }

    fn draw_prepared_item(
        &mut self,
        pixmap: &mut Pixmap,
        item: &PreparedItem,
        metrics: &RenderMetrics,
        alignment: Alignment,
        left: i32,
        right: i32,
    ) {
        let button = &item.button;
        let bold = button.bold.unwrap_or(self.config.bold);
        let button_color = button
            .color
            .map(|[r, g, b, a]| TextColor::rgba(r, g, b, a))
            .unwrap_or(metrics.text_color);
        self.draw_button_background(pixmap, left, right, metrics.height, button.background);
        self.draw_button_indicator(pixmap, left, right, metrics.height, button.indicator);
        self.current_hitboxes.push(Hitbox::new(
            left,
            right,
            metrics.width,
            alignment,
            item.widget_index,
            item.button_index,
        ));
        if let Some(text) = button.text.as_deref() {
            let text_right = match alignment {
                Alignment::Right => right.saturating_sub(item.padding),
                Alignment::Left | Alignment::Center => left
                    .saturating_add(item.padding)
                    .saturating_add(item.text_width),
            };
            self.draw_text(pixmap, text, text_right, metrics, button_color, bold);
        }
        if let Some(icon) = button.icon.as_ref() {
            let icon_x = match alignment {
                Alignment::Right => right
                    .saturating_sub(item.padding)
                    .saturating_sub(icon.width as i32),
                Alignment::Left | Alignment::Center => left.saturating_add(item.padding),
            };
            self.draw_icon(pixmap, icon, icon_x);
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

    fn fit_text(
        &mut self,
        text: &str,
        max_width: i32,
        metrics: &RenderMetrics,
        bold: bool,
    ) -> (String, i32) {
        let full_width = self.measure_text(text, metrics, bold);
        if full_width <= max_width {
            return (text.to_string(), full_width);
        }
        let ellipsis_width = self.measure_text("…", metrics, bold);
        if ellipsis_width > max_width {
            return (String::new(), 0);
        }

        let boundaries: Vec<usize> = std::iter::once(0)
            .chain(
                text.grapheme_indices(true)
                    .map(|(index, part)| index + part.len()),
            )
            .collect();
        let mut low = 0;
        let mut high = boundaries.len() - 1;
        let mut width = ellipsis_width;
        while low < high {
            let middle = (low + high).div_ceil(2);
            let candidate = format!("{}…", &text[..boundaries[middle]]);
            let candidate_width = self.measure_text(&candidate, metrics, bold);
            if candidate_width <= max_width {
                low = middle;
                width = candidate_width;
            } else {
                high = middle - 1;
            }
        }
        (format!("{}…", &text[..boundaries[low]]), width)
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
        let x = right.saturating_sub(text_width);

        let text_y = ((metrics.height as f32 - line_height) / 2.0).max(0.0) as i32;
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color,
            |glyph_x, glyph_y, _w, _h, color| {
                let px = x.saturating_add(glyph_x);
                let py = text_y.saturating_add(glyph_y);
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
                let px = x.saturating_add(ix as i32);
                let py = y.saturating_add(iy as i32);
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

fn native_argb8888([red, green, blue, alpha]: [u8; 4]) -> [u8; 4] {
    u32::from_be_bytes([alpha, red, green, blue]).to_ne_bytes()
}

fn next_spacing(
    current_index: usize,
    next_index: usize,
    content_spacing: Option<i32>,
    module_spacing: i32,
    item_spacing: i32,
) -> i32 {
    if current_index == next_index {
        content_spacing.unwrap_or(item_spacing)
    } else {
        module_spacing
    }
}

fn next_spacing_for_position(
    items: &[PreparedItem],
    position: usize,
    module_spacing: i32,
    item_spacing: i32,
) -> i32 {
    items.get(position + 1).map_or(0, |next| {
        next_spacing(
            items[position].widget_index,
            next.widget_index,
            items[position].content_spacing,
            module_spacing,
            item_spacing,
        )
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

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::{
        BarRenderer, MouseButton, RenderButton, RenderContent, RenderMetrics, RenderWidget,
        UnicodeSegmentation,
    };
    use crate::config::StyleConfig;

    struct MonitorWidget {
        output: Option<String>,
        clicks: Rc<Cell<usize>>,
    }

    impl RenderWidget for MonitorWidget {
        fn set_monitor_name(&mut self, output: Option<&str>) {
            self.output = output.map(str::to_owned);
        }

        fn content(&mut self) -> RenderContent {
            if self.output.as_deref() == Some("A") {
                RenderContent::Text("A".to_string())
            } else {
                RenderContent::Buttons(Vec::new())
            }
        }

        fn on_click(&mut self, _: MouseButton, _: usize) {
            self.clicks.set(self.clicks.get() + 1);
        }
    }

    struct LimitedWidget;

    impl RenderWidget for LimitedWidget {
        fn content(&mut self) -> RenderContent {
            RenderContent::Buttons(vec![RenderButton {
                text: Some("A very long window title that exceeds the limit".to_string()),
                icon: None,
                padding: Some(10),
                max_width: Some(80),
                bold: None,
                color: None,
                background: None,
                indicator: None,
            }])
        }
    }

    struct FixedButtons;

    impl RenderWidget for FixedButtons {
        fn content(&mut self) -> RenderContent {
            RenderContent::Buttons(
                (0..2)
                    .map(|_| RenderButton {
                        text: None,
                        icon: None,
                        padding: Some(5),
                        max_width: None,
                        bold: None,
                        color: None,
                        background: None,
                        indicator: None,
                    })
                    .collect(),
            )
        }

        fn content_spacing(&self) -> Option<u32> {
            Some(3)
        }
    }

    #[test]
    fn positions_buttons_in_each_alignment() {
        let mut renderer = BarRenderer::new(&StyleConfig::default()).unwrap();
        let mut left: Vec<Box<dyn RenderWidget>> = vec![Box::new(FixedButtons)];
        let mut center: Vec<Box<dyn RenderWidget>> = vec![Box::new(FixedButtons)];
        let mut right: Vec<Box<dyn RenderWidget>> = vec![Box::new(FixedButtons)];

        renderer.render(100, 30, None, &mut left, &mut center, &mut right);
        let hitboxes = &renderer.hitboxes[&None];
        let bounds: Vec<_> = hitboxes
            .iter()
            .map(|hitbox| (hitbox.left, hitbox.right))
            .collect();
        assert_eq!(
            bounds,
            [(6, 16), (19, 29), (38, 48), (51, 61), (84, 94), (71, 81)]
        );
    }

    #[test]
    fn large_item_padding_does_not_overflow_layout() {
        let config = StyleConfig {
            item_padding: i32::MAX as u32,
            ..StyleConfig::default()
        };
        let mut renderer = BarRenderer::new(&config).unwrap();
        let widget = || {
            Box::new(MonitorWidget {
                output: Some("A".to_string()),
                clicks: Rc::new(Cell::new(0)),
            }) as Box<dyn RenderWidget>
        };
        let mut left = vec![widget()];
        let mut center = vec![widget()];
        let mut right = vec![widget()];

        let pixels = renderer.render(100, 30, None, &mut left, &mut center, &mut right);
        assert_eq!(pixels.len(), 100 * 30 * 4);
    }

    #[test]
    fn limits_button_width_and_preserves_graphemes() {
        let config = StyleConfig::default();
        let mut renderer = BarRenderer::new(&config).unwrap();
        let metrics = RenderMetrics {
            width: 800,
            height: 30,
            font_size: config.font_size,
            text_color: renderer.text_color,
        };
        let title = "e\u{301} abcdefghijklmnopqrstuvwxyz";
        let budget = renderer.measure_text("e\u{301} ab…", &metrics, false);
        let (fitted, width) = renderer.fit_text(title, budget, &metrics, false);
        assert!(fitted.ends_with('…'));
        assert!(width <= budget);
        let prefix = fitted.strip_suffix('…').unwrap();
        assert!(title.starts_with(prefix));
        assert!(
            prefix.is_empty()
                || title
                    .grapheme_indices(true)
                    .any(|(index, part)| index + part.len() == prefix.len())
        );

        let mut left: Vec<Box<dyn RenderWidget>> = vec![Box::new(LimitedWidget)];
        renderer.render(800, 30, None, &mut left, &mut [], &mut []);
        let hitbox = &renderer.hitboxes[&None][0];
        assert!(hitbox.right - hitbox.left <= 80);
    }

    #[test]
    fn keeps_hitboxes_for_each_output() {
        let clicks = Rc::new(Cell::new(0));
        let mut left: Vec<Box<dyn RenderWidget>> = vec![Box::new(MonitorWidget {
            output: None,
            clicks: Rc::clone(&clicks),
        })];
        let mut center = Vec::new();
        let mut right = Vec::new();
        let config = StyleConfig::default();
        let mut renderer = BarRenderer::new(&config).unwrap();

        left[0].set_monitor_name(Some("A"));
        renderer.render(300, 30, Some("A"), &mut left, &mut center, &mut right);
        left[0].set_monitor_name(Some("B"));
        renderer.render(300, 30, Some("B"), &mut left, &mut center, &mut right);

        renderer.handle_click(
            10.0,
            MouseButton::Left,
            Some("A"),
            &mut left,
            &mut center,
            &mut right,
        );
        renderer.handle_click(
            10.0,
            MouseButton::Left,
            Some("B"),
            &mut left,
            &mut center,
            &mut right,
        );
        assert_eq!(clicks.get(), 1);
    }
}
