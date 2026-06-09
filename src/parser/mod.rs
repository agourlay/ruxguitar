pub mod gp345;
pub mod gp67;
pub mod model;
mod parse;
pub mod song_parser_tests;

// Top-level parsing entry point (dispatches by container format).
pub use parse::parse_gp_data;

// The GP3/4/5 binary parser lives in `gp345`; re-export `song_parser` at the
// parser root since it is the model re-export hub used across the audio, UI
// and gp67 modules.
pub use gp345::song_parser;
