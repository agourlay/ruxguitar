use crate::ui::application::RuxApplication;
use crate::RuxError::ConfigError;
use clap::Parser;
use std::io;
use std::path::PathBuf;

mod audio;
mod parser;
mod ui;

fn main() {
    let result = main_result();
    std::process::exit(match result {
        Ok(()) => 0,
        Err(err) => {
            // use Display instead of Debug for user friendly error messages
            log::error!("{}", err);
            1
        }
    });
}

pub fn main_result() -> Result<(), RuxError> {
    // setup logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("ruxguitar=info"))
        .init();

    // args
    let mut args = CliArgs::parse();
    let file = args.file.take().map(PathBuf::from);
    let sound_font_file = args.sound_font_file.take().map(PathBuf::from);

    // check if sound font file exists
    if let Some(sound_font_file) = &sound_font_file {
        if !sound_font_file.exists() {
            let err = ConfigError(format!("Sound font file not found {:?}", sound_font_file));
            return Err(err);
        }
        log::info!("Starting with custom sound font file {:?}", sound_font_file);
    }

    let args = ApplicationArgs {
        file,
        sound_font_bank: sound_font_file,
        no_antialiasing: args.no_antialiasing,
    };

    // go!
    RuxApplication::start(args)?;
    Ok(())
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct CliArgs {
    /// Optional path to a gp5 file. If not passed, a file must be selected with
    /// the file selector.
    #[arg(long)]
    file: Option<String>,
    /// Optional path to a sound font file.
    #[arg(long)]
    sound_font_file: Option<String>,
    /// Disable antialiasing.
    #[arg(long, default_value_t = false)]
    no_antialiasing: bool,
}

#[derive(Debug, Clone)]
pub struct ApplicationArgs {
    file: Option<PathBuf>,
    sound_font_bank: Option<PathBuf>,
    no_antialiasing: bool,
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
        RuxError::IcedError(error)
    }
}

impl From<io::Error> for RuxError {
    fn from(error: io::Error) -> Self {
        RuxError::OtherError(error.to_string())
    }
}
