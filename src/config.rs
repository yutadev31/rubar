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
    pub format: String,
    pub muted_format: String,
    pub microphone_format: String,
    pub microphone_muted_format: String,
    pub refresh_seconds: u64,
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
    pub button_padding: u32,
    pub active_color: String,
    pub active_background_color: String,
    pub refresh_seconds: u64,
    pub all_monitors: bool,
    /// Inclusive numeric range that should always be shown by i3/Sway.
    pub workspace_range: Option<[i64; 2]>,
    /// Additional workspace names to always show by i3/Sway.
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
    pub button_vertical_padding: u32,
    pub button_spacing: u32,
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
            spacing: 16,
            button_padding: 0,
            button_vertical_padding: 0,
            button_spacing: 8,
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
            format: "VOL {volume}%".to_string(),
            muted_format: "VOL [M]".to_string(),
            microphone_format: " MIC {volume}%".to_string(),
            microphone_muted_format: " MIC [M]".to_string(),
            refresh_seconds: 1,
        }
    }
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            provider: "sysfs".to_string(),
            format: "BAT {percent}%".to_string(),
            refresh_seconds: 30,
        }
    }
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            format: "{workspaces}".to_string(),
            button_padding: 12,
            active_color: "#7aa2f7".to_string(),
            active_background_color: "#414868".to_string(),
            refresh_seconds: 1,
            // A bar now exists on every output, so the workspace module should
            // expose the complete Hyprland workspace state by default. Users
            // who prefer the focused monitor only can still set this to false.
            all_monitors: true,
            workspace_range: None,
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
