use std::{error::Error, fs, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub clock: ClockConfig,
    pub volume: VolumeConfig,
    pub style: StyleConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ClockConfig {
    pub show_seconds: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct VolumeConfig {
    pub provider: String,
    pub format: String,
    pub refresh_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct StyleConfig {
    pub colors: ColorsConfig,
    pub height: u32,
    pub font_size: f32,
    pub padding: u32,
    pub spacing: u32,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ColorsConfig {
    pub bg: String,
    pub text: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            clock: ClockConfig::default(),
            volume: VolumeConfig::default(),
            style: StyleConfig::default(),
        }
    }
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            colors: ColorsConfig::default(),
            height: 28,
            font_size: 14.0,
            padding: 8,
            spacing: 8,
        }
    }
}

impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            bg: "#1f232b".to_string(),
            text: "#ffffff".to_string(),
        }
    }
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            show_seconds: false,
        }
    }
}

impl Default for VolumeConfig {
    fn default() -> Self {
        Self {
            provider: "pulseaudio".to_string(),
            format: "VOL {volume}%".to_string(),
            refresh_seconds: 1,
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
