use std::{
    fs::{create_dir_all, File},
    io::{BufReader, Write},
    path::PathBuf,
};

use home::home_dir;
use serde::{Deserialize, Serialize};

use crate::RuxError;

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    tabs_folder: Option<PathBuf>,
}

impl Config {
    // folder placed in $HOME directory
    const FOLDER: &'static str = ".ruxguitar";

    pub fn get_tabs_folder(&self) -> Option<PathBuf> {
        self.tabs_folder.clone()
    }

    pub fn set_tabs_folder(&mut self, new_tabs_folder: Option<PathBuf>) -> Result<(), RuxError> {
        if self.tabs_folder == new_tabs_folder {
            // no op
            Ok(())
        } else {
            self.tabs_folder = new_tabs_folder;
            self.save_config()
        }
    }

    fn get_base_path() -> Result<PathBuf, RuxError> {
        let home = home_dir()
            .ok_or_else(|| RuxError::ConfigError("Could not find home directory".to_string()))?;
        let path = home.join(Self::FOLDER);
        Ok(path)
    }

    fn get_path() -> Result<PathBuf, RuxError> {
        let base = Self::get_base_path()?;
        Ok(base.join("config.json"))
    }

    /// Creates config if it does not exist
    pub fn read_config() -> Result<Self, RuxError> {
        let base_path = Self::get_base_path()?;
        if !base_path.exists() {
            create_dir_all(base_path)?;
        }
        let config_path = Self::get_path()?;
        if !config_path.exists() {
            // create empty config
            Config::default().save_config()?;
        }
        let file = File::open(config_path)?;
        let reader = BufReader::new(file);
        let config: Config = serde_json::from_reader(reader).map_err(|err| {
            RuxError::ConfigError(format!("Could not read local configuration {err:}"))
        })?;
        Ok(config)
    }

    /// Assumes the config folder exists
    pub fn save_config(&self) -> Result<(), RuxError> {
        let config_path = Self::get_path()?;
        let json = serde_json::to_string_pretty(self).map_err(|err| {
            RuxError::ConfigError(format!("Could not save local configuration {err:}"))
        })?;
        let mut file = File::create(config_path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }
}
