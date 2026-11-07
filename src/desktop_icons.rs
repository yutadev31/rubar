use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

use image::imageops::FilterType;
use tiny_skia::{Pixmap, Transform};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8888 pixels in row-major order.
    pub pixels: Vec<u8>,
}

/// How PNG images are sized. SVG images are always rendered at the target size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterSize {
    Original,
    #[allow(unused)]
    Target,
}

/// Loads desktop icons from configured themes and XDG data directories.
pub struct Loader {
    roots: Vec<PathBuf>,
    themes: Vec<String>,
}

impl Loader {
    /// Use XDG icon roots with the given themes in priority order.
    pub fn new(themes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::with_roots(xdg_icon_roots(), themes)
    }

    /// Use explicit icon roots, useful for a controlled icon source.
    pub fn with_roots(
        roots: Vec<PathBuf>,
        themes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            roots,
            themes: themes.into_iter().map(Into::into).collect(),
        }
    }

    /// Load an absolute icon path or look up an icon name in the themes.
    ///
    /// Returns `None` for an empty name, zero target size, or if no readable,
    /// supported icon can be decoded. Theme lookup tries inherited themes and fallbacks.
    #[must_use]
    pub fn load(&self, name: &str, target: u32, raster_size: RasterSize) -> Option<Icon> {
        if target == 0 || name.is_empty() {
            return None;
        }

        let path = Path::new(name);

        if path.is_absolute() {
            return load_file(path, target, raster_size);
        }

        if path.components().count() != 1 {
            return None;
        }

        let name = name
            .strip_suffix(".png")
            .or_else(|| name.strip_suffix(".svg"))
            .unwrap_or(name);

        let mut visited = HashSet::new();

        for theme in &self.themes {
            if let Some(icon) = self.load_from_theme(theme, name, target, raster_size, &mut visited)
            {
                return Some(icon);
            }
        }

        if let Some(icon) = self.load_from_theme("hicolor", name, target, raster_size, &mut visited)
        {
            return Some(icon);
        }

        for root in &self.roots {
            for file in candidate_files(root, name) {
                if let Some(icon) = load_file(&file, target, raster_size) {
                    return Some(icon);
                }
            }
        }

        for base in data_dirs() {
            for file in candidate_files(&base.join("pixmaps"), name) {
                if let Some(icon) = load_file(&file, target, raster_size) {
                    return Some(icon);
                }
            }
        }

        None
    }

    fn load_from_theme(
        &self,
        theme: &str,
        name: &str,
        target: u32,
        raster_size: RasterSize,
        visited: &mut HashSet<String>,
    ) -> Option<Icon> {
        if !visited.insert(theme.to_string()) {
            return None;
        }

        let mut parents = Vec::new();

        for root in &self.roots {
            let theme_dir = root.join(theme);
            let (mut directories, inherits) = theme_index(&theme_dir);

            parents.extend(inherits);

            if directories.is_empty() {
                directories = conventional_directories(target);
            }

            directories.sort_by_key(|directory| directory_distance(directory, target));

            for directory in directories {
                for file in candidate_files(&theme_dir.join(directory), name) {
                    if let Some(icon) = load_file(&file, target, raster_size) {
                        return Some(icon);
                    }
                }
            }
        }

        for parent in parents {
            if let Some(icon) = self.load_from_theme(&parent, name, target, raster_size, visited) {
                return Some(icon);
            }
        }

        None
    }
}

fn xdg_icon_roots() -> Vec<PathBuf> {
    data_dirs()
        .into_iter()
        .map(|dir| dir.join("icons"))
        .collect()
}

fn data_dirs() -> Vec<PathBuf> {
    let user = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let mut dirs = user.into_iter().collect::<Vec<_>>();
    let system = env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(env::split_paths(&system).filter(|path| path.is_absolute()));
    dirs
}

fn theme_index(theme_dir: &Path) -> (Vec<String>, Vec<String>) {
    let Ok(contents) = fs::read_to_string(theme_dir.join("index.theme")) else {
        return (Vec::new(), Vec::new());
    };
    let mut in_theme = false;
    let mut directories = Vec::new();
    let mut parents = Vec::new();

    for line in contents.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            in_theme = line == "[Icon Theme]";
        } else if in_theme {
            if let Some(value) = line.strip_prefix("Directories=") {
                directories.extend(parse_theme_list(value));
            } else if let Some(value) = line.strip_prefix("Inherits=") {
                parents.extend(parse_theme_list(value));
            }
        }
    }

    (directories, parents)
}

fn parse_theme_list(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
}

fn conventional_directories(target: u32) -> Vec<String> {
    let mut directories = Vec::new();

    for size in [
        format!("{target}x{target}"),
        "16x16".into(),
        "22x22".into(),
        "24x24".into(),
        "32x32".into(),
        "scalable".into(),
    ] {
        for context in ["apps", "status", "actions", "devices", "places"] {
            directories.push(format!("{size}/{context}"));
        }
    }

    directories
}

fn directory_distance(directory: &str, target: u32) -> u32 {
    if directory.starts_with("scalable/") {
        return 1;
    }

    let Some((size, _)) = directory.split_once('x') else {
        return u32::MAX;
    };
    size.parse::<u32>()
        .map_or(u32::MAX, |size| size.abs_diff(target))
}

fn candidate_files(dir: &Path, name: &str) -> [PathBuf; 2] {
    [
        dir.join(format!("{name}.png")),
        dir.join(format!("{name}.svg")),
    ]
}

/// Decode a PNG or SVG file into premultiplied RGBA pixels.
///
/// Returns `None` for zero target size, an unsupported extension, or unreadable
/// or invalid image data. SVGs use the target size; PNGs follow `raster_size`.
#[must_use]
// SVG transforms use f32; rounding large pixel dimensions is inherent to resvg.
#[allow(clippy::cast_precision_loss)]
pub fn load_file(path: &Path, target: u32, raster_size: RasterSize) -> Option<Icon> {
    if target == 0 {
        return None;
    }

    match path.extension()?.to_str()? {
        "svg" => {
            let data = fs::read(path).ok()?;
            let tree =
                resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
            let mut pixmap = Pixmap::new(target, target)?;
            let size = tree.size();
            let scale = (target as f32 / size.width()).min(target as f32 / size.height());

            resvg::render(
                &tree,
                Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            Some(Icon {
                width: target,
                height: target,
                pixels: pixmap.take(),
            })
        }
        "png" => {
            let image = image::open(path).ok()?.into_rgba8();

            let image = match raster_size {
                RasterSize::Original => image,
                RasterSize::Target => {
                    image::imageops::resize(&image, target, target, FilterType::Lanczos3)
                }
            };

            let (width, height) = image.dimensions();
            let mut pixels = image.into_raw();

            for pixel in pixels.as_chunks_mut::<4>().0 {
                let alpha = u16::from(pixel[3]);

                for channel in &mut pixel[..3] {
                    *channel = u8::try_from((u16::from(*channel) * alpha) / 255).ok()?;
                }
            }

            Some(Icon {
                width,
                height,
                pixels,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_theme_and_premultiplied_png() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let child = root.join("child");
        let parent = root.join("parent");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(parent.join("32x32/apps")).unwrap();
        fs::write(child.join("index.theme"), "[Icon Theme]\nInherits=parent\n").unwrap();
        fs::write(
            parent.join("index.theme"),
            "[Icon Theme]\nDirectories=32x32/apps\n",
        )
        .unwrap();
        let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([200, 100, 40, 128]));

        image.save(parent.join("32x32/apps/example.png")).unwrap();
        let loader = Loader::with_roots(vec![root.to_path_buf()], ["child"]);
        let icon = loader.load("example", 32, RasterSize::Original).unwrap();
        assert_eq!((icon.width, icon.height), (1, 1));
        assert_eq!(icon.pixels, [100, 50, 20, 128]);
    }

    #[test]
    fn decodes_svg_at_target_size() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("icon.svg");
        fs::write(&path, r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000" fill-opacity="0.5"/></svg>"##).unwrap();
        let icon = load_file(&path, 16, RasterSize::Original).unwrap();
        assert_eq!(icon.pixels[..4], [128, 0, 0, 128]);
    }

    #[test]
    fn finds_hicolor_icon_without_index_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hicolor/scalable/apps/example.svg");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##).unwrap();
        let loader = Loader::with_roots(vec![directory.path().to_path_buf()], ["hicolor"]);
        let icon = loader.load("example", 16, RasterSize::Original).unwrap();
        assert_eq!(icon.pixels[..4], [255, 0, 0, 255]);
    }
}
