use std::{error::Error, fs, path::PathBuf};

use serde::Deserialize;

use crate::color::parse_rgba;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub clock: ClockConfig,
    pub battery: BatteryConfig,
    pub modules: ModulesConfig,
    pub volume: VolumeConfig,
    pub workspace: WorkspaceConfig,
    pub window: WindowConfig,
    pub tray: TrayConfig,
    pub style: StyleConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct TrayConfig {
    pub icon_size: u32,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self { icon_size: 16 }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ClockConfig {
    pub format: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ModulesConfig {
    pub left: Vec<String>,
    pub center: Vec<String>,
    pub right: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct VolumeConfig {
    pub provider: String,
    pub format_out: String,
    pub format_out_muted: String,
    pub format_in: String,
    pub format_in_muted: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct BatteryConfig {
    pub provider: String,
    pub format: String,
    pub refresh_seconds: u64,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub format: String,
    pub padding: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct StyleConfig {
    pub colors: ColorsConfig,
    pub height: u32,
    pub font_size: f32,
    pub font_family: String,
    pub bold: bool,
    pub padding: u32,
    pub spacing: u32,
    pub button_spacing: u32,
}

#[derive(Clone, Debug, Deserialize)]
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
            padding: 6,
            spacing: 12,
            button_spacing: 4,
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
        let config: Self = toml::from_str(&fs::read_to_string(&path)?)?;
        config
            .validate()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        tracing::debug!(path = %path.display(), "configuration loaded");
        Ok(config)
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
        if self.battery.refresh_seconds == 0 {
            return Err("battery.refresh_seconds must be greater than zero".to_string());
        }
        for (name, value) in [
            ("style.padding", self.style.padding),
            ("style.spacing", self.style.spacing),
            ("style.button_spacing", self.style.button_spacing),
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
        for (group, names) in [
            ("left", &self.modules.left),
            ("center", &self.modules.center),
            ("right", &self.modules.right),
        ] {
            for name in names {
                if !matches!(
                    name.as_str(),
                    "clock" | "battery" | "volume" | "workspace" | "window" | "tray"
                ) {
                    return Err(format!("modules.{group}: unknown module `{name}`"));
                }
            }
        }
        Ok(())
    }
}

fn config_path() -> Option<PathBuf> {
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

        config.style.font_size = 14.0;
        config.modules.left.push("missing".to_string());
        assert!(config.validate().unwrap_err().contains("modules.left"));
    }
}
