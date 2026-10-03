use std::{error::Error, fs, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub clock: ClockConfig,
    pub battery: BatteryConfig,
    pub modules: ModulesConfig,
    pub volume: VolumeConfig,
    pub workspace: WorkspaceConfig,
    pub style: StyleConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ClockConfig {
    pub show_seconds: bool,
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
    pub out_format: String,
    pub out_muted_format: String,
    pub in_format: String,
    pub in_muted_format: String,
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
    pub active_background_color: String,
    pub all_monitors: bool,
    /// Numeric workspaces that should always be shown by i3/Sway.
    pub persistent_workspaces: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct StyleConfig {
    pub colors: ColorsConfig,
    pub height: u32,
    pub font_size: f32,
    pub font_family: String,
    pub bold: bool,
    pub padding: u32,
    pub spacing: u32,
    pub button_padding: u32,
}

#[derive(Debug, Deserialize)]
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
            padding: 8,
            spacing: 8,
            button_padding: 8,
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
            out_format: "󰕾 {volume}%".to_string(),
            out_muted_format: "󰖁".to_string(),
            in_format: "󰍬 {volume}%".to_string(),
            in_muted_format: "󰍭".to_string(),
        }
    }
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            provider: "sysfs".to_string(),
            format: "󰁹 {percent}%".to_string(),
            refresh_seconds: 30,
        }
    }
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            format: "{workspaces}".to_string(),
            padding: 12,
            spacing: 8,
            active_color: "#7aa2f7".to_string(),
            active_background_color: "#414868".to_string(),
            // A bar now exists on every output, so the workspace module should
            // expose the complete Hyprland workspace state by default. Users
            // who prefer the focused monitor only can still set this to false.
            all_monitors: true,
            persistent_workspaces: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn Error>> {
        let Some(path) = config_path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(toml::from_str(&fs::read_to_string(path)?)?)
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("rubar").join("config.toml"))
}
