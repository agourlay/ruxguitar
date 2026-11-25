use crate::RuxError::ConfigError;
use crate::ui::application::RuxApplication;
use clap::Parser;
use config::Config;
use std::io;
use std::path::PathBuf;

mod audio;
mod config;
mod parser;
mod ui;

fn main() {
    let result = main_result();
    std::process::exit(match result {
        Ok(()) => 0,
        Err(err) => {
            // use Display instead of Debug for user friendly error messages
            log::error!("{err}");
            1
        }
    });
}

pub fn main_result() -> Result<(), RuxError> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("ruxguitar=info"))
        .init();

    let mut args = CliArgs::parse();
    let sound_font_file = args.sound_font_file.take();
    let tab_file_path = args.tab_file_path.take();

    if let Some(sound_font_file) = &sound_font_file {
        if !sound_font_file.exists() {
            return Err(ConfigError(format!(
                "Sound font file not found {sound_font_file:?}"
            )));
        }
        log::info!("Starting with custom sound font file {sound_font_file:?}");
    }

    if let Some(tab_file_path) = &tab_file_path {
        if !tab_file_path.exists() {
            return Err(ConfigError(format!("Tab file not found {tab_file_path:?}")));
        }
        log::info!("Starting with tab file {tab_file_path:?}");
    }

    let args = ApplicationArgs {
        sound_font_bank: sound_font_file,
        tab_file_path,
        track: args.track.map(usize::from),
        no_antialiasing: args.no_antialiasing,
        theme: args.theme,
        local_config: Config::read_config()?,
    };

    RuxApplication::start(args)?;
    Ok(())
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct CliArgs {
    /// Optional path to a sound font file.
    #[arg(long)]
    sound_font_file: Option<PathBuf>,
    /// Optional path to tab file to by-pass the file picker.
    #[arg(long)]
    tab_file_path: Option<PathBuf>,
    /// Track to open, numbered from 1 as in the track overview.
    #[arg(long, requires = "tab_file_path", value_parser = clap::value_parser!(u16).range(1..))]
    track: Option<u16>,
    /// Disable antialiasing.
    #[arg(long, default_value_t = false)]
    no_antialiasing: bool,
    /// Force the color theme.
    #[arg(long, value_enum)]
    theme: Option<ThemeChoice>,
}

/// Color theme requested on the command line.
#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub enum ThemeChoice {
    Light,
    Dark,
}

#[derive(Debug, Clone)]
pub struct ApplicationArgs {
    sound_font_bank: Option<PathBuf>,
    tab_file_path: Option<PathBuf>,
    /// The track to open the tab file on, numbered from 1.
    track: Option<usize>,
    no_antialiasing: bool,
    theme: Option<ThemeChoice>,
    local_config: Config,
}

#[derive(Debug, thiserror::Error)]
pub enum RuxError {
    #[error("iced error: {0}")]
    IcedError(iced::Error),
    #[error("configuration error: {0}")]
    ConfigError(String),
    #[error("parsing error: {0}")]
    ParsingError(String),
    #[error("other error: {0}")]
    OtherError(String),
}

impl From<iced::Error> for RuxError {
    fn from(error: iced::Error) -> Self {
        Self::IcedError(error)
    }
}

impl From<io::Error> for RuxError {
    fn from(error: io::Error) -> Self {
        Self::OtherError(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<CliArgs, clap::Error> {
        CliArgs::try_parse_from(std::iter::once("ruxguitar").chain(args.iter().copied()))
    }

    #[test]
    fn a_track_opens_with_a_tab_file() {
        let args = parse(&["--tab-file-path", "song.gp5", "--track", "3"]).unwrap();
        assert_eq!(args.track, Some(3));
    }

    #[test]
    fn a_track_needs_a_tab_file() {
        assert!(parse(&["--track", "3"]).is_err());
    }

    #[test]
    fn tracks_are_numbered_from_one() {
        assert!(parse(&["--tab-file-path", "song.gp5", "--track", "0"]).is_err());
        assert!(parse(&["--tab-file-path", "song.gp5", "--track", "bass"]).is_err());
    }
}
