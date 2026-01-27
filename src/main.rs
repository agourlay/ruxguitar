use crate::ui::application::RuxApplication;
use crate::AppError::ConfigError;
use clap::Parser;
use config::Config;
use ruxguitar::RuxError as LibRuxError;
use std::io;
use std::path::PathBuf;

mod config;
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

pub fn main_result() -> Result<(), AppError> {
    // setup logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("ruxguitar=info"))
        .init();

    // args
    let mut args = CliArgs::parse();
    let sound_font_file = args.sound_font_file.take().map(PathBuf::from);
    let tab_file_path = args.tab_file_path.take().map(PathBuf::from);

    // check if sound font file exists
    if let Some(sound_font_file) = &sound_font_file {
        if !sound_font_file.exists() {
            let err = ConfigError(format!("Sound font file not found {sound_font_file:?}"));
            return Err(err);
        }
        log::info!("Starting with custom sound font file {sound_font_file:?}");
    }

    // check if tab file exists
    if let Some(tab_file_path) = &tab_file_path {
        if !tab_file_path.exists() {
            let err = ConfigError(format!("Tab file not found {tab_file_path:?}"));
            return Err(err);
        }
        log::info!("Starting with tab file {tab_file_path:?}");
    }

    // read local config
    let local_config = Config::read_config()?;

    // bundle application args
    let args = ApplicationArgs {
        sound_font_bank: sound_font_file,
        tab_file_path,
        no_antialiasing: args.no_antialiasing,
        local_config,
    };

    // go!
    RuxApplication::start(args)?;
    Ok(())
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct CliArgs {
    /// Optional path to a sound font file.
    #[arg(long)]
    sound_font_file: Option<String>,
    /// Optional path to tab file to by-pass the file picker.
    #[arg(long)]
    tab_file_path: Option<String>,
    /// Disable antialiasing.
    #[arg(long, default_value_t = false)]
    no_antialiasing: bool,
}

#[derive(Debug, Clone)]
pub struct ApplicationArgs {
    sound_font_bank: Option<PathBuf>,
    tab_file_path: Option<PathBuf>,
    no_antialiasing: bool,
    local_config: Config,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("iced error: {0}")]
    IcedError(iced::Error),
    #[error("configuration error: {0}")]
    ConfigError(String),
    #[error("parsing error: {0}")]
    ParsingError(String),
    #[error("other error: {0}")]
    OtherError(String),
}

impl From<LibRuxError> for AppError {
    fn from(error: LibRuxError) -> Self {
        match error {
            LibRuxError::ParsingError(s) => Self::ParsingError(s),
            LibRuxError::ConfigError(s) => Self::ConfigError(s),
            LibRuxError::AudioError(s) => Self::OtherError(s),
            LibRuxError::IoError(s) => Self::OtherError(s),
        }
    }
}

impl From<iced::Error> for AppError {
    fn from(error: iced::Error) -> Self {
        Self::IcedError(error)
    }
}

impl From<io::Error> for AppError {
    fn from(error: io::Error) -> Self {
        Self::OtherError(error.to_string())
    }
}
