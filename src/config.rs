use std::{error::Error, fs, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub clock: ClockConfig,
}

#[derive(Debug, Deserialize)]
pub struct ClockConfig {
    pub show_seconds: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            clock: ClockConfig::default(),
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
    dirs::config_dir().map(|base| base.join("unibar").join("config.toml"))
}
