use std::{error::Error, fs, path::PathBuf};

use serde::Deserialize;

use crate::color::parse_rgba;
use crate::provider::{memory, tray};

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub clock: ClockConfig,
    pub battery: BatteryConfig,
    pub memory: MemoryConfig,
    pub modules: ModulesConfig,
    pub volume: VolumeConfig,
    pub workspace: WorkspaceConfig,
    pub window: WindowConfig,
    pub tray: TrayConfig,
    pub style: StyleConfig,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct TrayConfig {
    pub provider: String,
    pub icon_size: u32,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            icon_size: 16,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClockConfig {
    pub format: Option<String>,
    pub timezone: Option<chrono_tz::Tz>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModulesConfig {
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct VolumeConfig {
    pub provider: String,
    pub scroll_step: u8,
    pub format_out: String,
    pub format_out_muted: String,
    pub format_in: String,
    pub format_in_muted: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct BatteryConfig {
    pub provider: String,
    pub format: String,
    pub refresh_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct MemoryConfig {
    pub provider: String,
    pub format: String,
    pub refresh_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct WorkspaceConfig {
    pub format: String,
    pub padding: u32,
    pub spacing: u32,
    pub active_color: String,
    pub urgent_color: String,
    pub active_indicator_height: u32,
    pub active_indicator_position: String,
    pub all_monitors: bool,
    /// Numeric workspaces that should always be shown by i3/Sway.
    pub persistent_workspaces: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowConfig {
    pub format: String,
    pub padding: u32,
    /// Maximum width of the window item, including padding, in pixels.
    pub max_width: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct StyleConfig {
    pub colors: ColorsConfig,
    pub height: u32,
    pub font_size: f32,
    pub font_family: String,
    pub bold: bool,
    pub bar_padding: u32,
    pub item_padding: u32,
    pub module_spacing: u32,
    pub item_spacing: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct ColorsConfig {
    pub bg: String,
    pub text: String,
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            colors: ColorsConfig::default(),
            height: 30,
            font_size: 14.0,
            font_family: String::new(),
            bold: false,
            bar_padding: 6,
            item_padding: 6,
            module_spacing: 12,
            item_spacing: 4,
        }
    }
}

impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            bg: "#24283b".to_string(),
            text: "#c0caf5".to_string(),
        }
    }
}

impl Default for VolumeConfig {
    fn default() -> Self {
        Self {
            provider: "pulseaudio".to_string(),
            scroll_step: 1,
            format_out: "{volume}% 󰕾".to_string(),
            format_out_muted: "󰖁".to_string(),
            format_in: "{volume}% 󰍬".to_string(),
            format_in_muted: "󰍭".to_string(),
        }
    }
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            format: "{percent}% 󰁹".to_string(),
            refresh_seconds: 30,
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            format: "{used}/{total} GiB 󰍛".to_string(),
            refresh_seconds: 5,
        }
    }
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            format: "{workspaces}".to_string(),
            padding: 10,
            spacing: 4,
            active_color: "#7aa2f7".to_string(),
            urgent_color: "#f7768e".to_string(),
            active_indicator_height: 2,
            active_indicator_position: "bottom".to_string(),
            // A bar now exists on every output, so the workspace module should
            // expose the complete Hyprland workspace state by default. Users
            // who prefer the focused monitor only can still set this to false.
            all_monitors: true,
            persistent_workspaces: Vec::new(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            format: "{title}".to_string(),
            padding: 10,
            max_width: None,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn Error>> {
        let Some(path) = config_path() else {
            tracing::debug!("configuration directory unavailable; using defaults");
            let config = Self::default();
            config.validate()?;
            return Ok(config);
        };
        if !path.exists() {
            tracing::debug!(path = %path.display(), "configuration file not found; using defaults");
            let config = Self::default();
            config.validate()?;
            return Ok(config);
        }
        tracing::debug!(path = %path.display(), "loading configuration");
        let config = Self::parse(&fs::read_to_string(&path)?)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        tracing::debug!(path = %path.display(), "configuration loaded");
        Ok(config)
    }

    pub fn parse(contents: &str) -> Result<Self, Box<dyn Error>> {
        let mut config: Self = toml::from_str(contents)?;
        config.ignore_unknown_modules();
        config.validate()?;
        Ok(config)
    }

    fn ignore_unknown_modules(&mut self) {
        for (group, names) in [
            ("left", &mut self.modules.left),
            ("center", &mut self.modules.center),
            ("right", &mut self.modules.right),
        ] {
            names.retain(|name| {
                let known = matches!(
                    name.as_str(),
                    "clock" | "battery" | "memory" | "volume" | "workspace" | "window" | "tray"
                );
                if !known {
                    tracing::error!("modules.{group}: unknown module `{name}`; ignoring");
                }
                known
            });
        }
    }

    fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("style.colors.bg", &self.style.colors.bg),
            ("style.colors.text", &self.style.colors.text),
            ("workspace.active_color", &self.workspace.active_color),
            ("workspace.urgent_color", &self.workspace.urgent_color),
        ] {
            parse_rgba(value).map_err(|error| format!("{name}: {error}"))?;
        }
        if !(1..=4096).contains(&self.style.height) {
            return Err("style.height must be between 1 and 4096".to_string());
        }
        if !self.style.font_size.is_finite() || self.style.font_size <= 0.0 {
            return Err("style.font_size must be a finite positive number".to_string());
        }
        if self.tray.icon_size == 0 {
            return Err("tray.icon_size must be greater than zero".to_string());
        }
        tray::validate_name(&self.tray.provider)
            .map_err(|error| format!("tray.provider: {error}"))?;
        if self.battery.refresh_seconds == 0 {
            return Err("battery.refresh_seconds must be greater than zero".to_string());
        }
        if self.memory.refresh_seconds == 0 {
            return Err("memory.refresh_seconds must be greater than zero".to_string());
        }
        memory::validate_name(&self.memory.provider)
            .map_err(|error| format!("memory.provider: {error}"))?;
        if self.volume.scroll_step == 0 || self.volume.scroll_step > 100 {
            return Err("volume.scroll_step must be between 1 and 100".to_string());
        }
        if let Some(max_width) = self.window.max_width
            && !(1..=i32::MAX as u32).contains(&max_width)
        {
            return Err("window.max_width must be between 1 and 2147483647".to_string());
        }
        for (name, value) in [
            ("style.bar_padding", self.style.bar_padding),
            ("style.item_padding", self.style.item_padding),
            ("style.module_spacing", self.style.module_spacing),
            ("style.item_spacing", self.style.item_spacing),
            ("workspace.padding", self.workspace.padding),
            ("workspace.spacing", self.workspace.spacing),
            ("window.padding", self.window.padding),
        ] {
            if value > i32::MAX as u32 {
                return Err(format!("{name} exceeds the supported range"));
            }
        }
        if !matches!(
            self.workspace.active_indicator_position.as_str(),
            "top" | "bottom"
        ) {
            return Err(
                "workspace.active_indicator_position must be `top` or `bottom`".to_string(),
            );
        }
        Ok(())
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("rubar").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn rejects_invalid_values_before_rendering() {
        let mut config = Config::default();
        config.style.colors.bg = "#not-a-color".to_string();
        assert!(config.validate().unwrap_err().contains("style.colors.bg"));

        config.style.colors.bg = "#123456".to_string();
        config.style.font_size = f32::NAN;
        assert!(config.validate().unwrap_err().contains("style.font_size"));
    }

    #[test]
    fn parses_clock_timezone_and_rejects_unknown_zones() {
        let config = Config::parse("[clock]\ntimezone = 'Asia/Tokyo'\n").unwrap();
        assert_eq!(config.clock.timezone, Some(chrono_tz::Asia::Tokyo));
        assert!(Config::default().clock.timezone.is_none());

        let error = Config::parse("[clock]\ntimezone = 'Unknown/Zone'\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("timezone"));
    }

    #[test]
    fn ignores_unknown_modules_and_keeps_known_modules_in_order() {
        let config = Config::parse(
            "[modules]\nleft = ['clock', 'missing', 'battery']\ncenter = ['unknown']\nright = ['tray', 'clock']\n",
        )
        .unwrap();

        assert_eq!(config.modules.left, ["clock", "battery"]);
        assert!(config.modules.center.is_empty());
        assert_eq!(config.modules.right, ["tray", "clock"]);
    }

    #[test]
    fn validates_scroll_step() {
        let mut config = Config::default();
        config.volume.scroll_step = 0;
        assert!(
            config
                .validate()
                .unwrap_err()
                .contains("volume.scroll_step")
        );

        config.volume.scroll_step = 101;
        assert!(
            config
                .validate()
                .unwrap_err()
                .contains("volume.scroll_step")
        );
    }

    #[test]
    fn accepts_memory_module_and_validates_refresh_interval() {
        let config = Config::parse(
            "[modules]\nright = ['memory']\n[memory]\nformat = '{percent}%'\nrefresh_seconds = 2\n",
        )
        .unwrap();
        assert_eq!(config.memory.format, "{percent}%");
        assert_eq!(config.memory.provider, "auto");

        let mut config = config;
        config.memory.refresh_seconds = 0;
        assert!(
            config
                .validate()
                .unwrap_err()
                .contains("memory.refresh_seconds")
        );
    }

    #[test]
    fn validates_memory_provider_name() {
        assert!(Config::parse("[memory]\nprovider = 'auto'\n").is_ok());
        #[cfg(target_os = "linux")]
        assert!(Config::parse("[memory]\nprovider = 'linux'\n").is_ok());

        let error = Config::parse("[memory]\nprovider = 'unknown'\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("memory.provider"));
        assert!(error.contains("unknown"));
    }

    #[test]
    fn validates_tray_provider_name() {
        assert_eq!(Config::default().tray.provider, "auto");
        assert!(Config::parse("[tray]\nprovider = 'sni'\n").is_ok());

        let error = Config::parse("[tray]\nprovider = 'unknown'\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("tray.provider"));
        assert!(error.contains("unknown"));
    }

    #[test]
    fn rejects_old_style_keys() {
        let error = toml::from_str::<Config>("[style]\npadding = 8")
            .unwrap_err()
            .to_string();
        assert!(error.contains("padding"));
    }

    #[test]
    fn validates_window_max_width() {
        let mut config = Config::default();
        config.window.max_width = Some(0);
        assert!(config.validate().unwrap_err().contains("window.max_width"));
    }
}
