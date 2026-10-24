use std::fs;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub output: OutputConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    pub path_format: Option<String>,
    pub clipboard: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            path_format: None,
            clipboard: true,
        }
    }
}

impl Config {
    pub fn load() -> crate::Result<Self> {
        let Some(config_dir) = dirs::config_dir() else {
            return Ok(Self::default());
        };
        let path = config_dir.join("shot").join("config.toml");
        match fs::read_to_string(path) {
            Ok(contents) => {
                let mut config: Self = toml::from_str(&contents)?;
                if config.output.path_format.as_deref() == Some("") {
                    eprintln!(
                        "Warning: output.path_format is empty; using the default output path"
                    );
                    config.output.path_format = None;
                }
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }
}
