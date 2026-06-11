use std::path::PathBuf;

/// File extensions supported by the parser; used by both the dialog filter
/// and the validation of files loaded directly (CLI argument, drag and drop).
const SUPPORTED_EXTENSIONS: [&str; 5] = ["gp5", "gp4", "gp3", "gpx", "gp"];

#[derive(Debug, Clone, thiserror::Error)]
pub enum FilePickerError {
    #[error("dialog window closed without selecting a file")]
    DialogClosed,
    #[error("IO error: {0}")]
    IoError(String),
}

/// Opens a file dialog and returns the content of the picked file.
pub async fn open_file_dialog(
    picker_folder: Option<PathBuf>,
) -> Result<(Vec<u8>, Option<PathBuf>, String), FilePickerError> {
    let mut picker = rfd::AsyncFileDialog::new()
        .add_filter("Guitar Pro files", &SUPPORTED_EXTENSIONS)
        .set_title("Select a Guitar Pro file");

    if let Some(folder) = picker_folder {
        picker = picker.set_directory(folder);
    }

    let picked_file = picker
        .pick_file()
        .await
        .ok_or(FilePickerError::DialogClosed)?;
    load_file(picked_file).await
}

/// Loads the content of a file at the given path.
///
/// Return the content of the file and its name.
pub async fn load_file(
    path: impl Into<PathBuf>,
) -> Result<(Vec<u8>, Option<PathBuf>, String), FilePickerError> {
    let path = path.into();
    let file_extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    if !SUPPORTED_EXTENSIONS.contains(&file_extension.as_str()) {
        return Err(FilePickerError::IoError(format!(
            "Unsupported file extension: {file_extension}"
        )));
    }
    let file_name = path
        .file_name()
        .and_then(|f| f.to_str())
        .map(ToString::to_string)
        .unwrap_or_default();
    let parent_folder = path.parent().and_then(|parent| {
        // make sure relative path from CLI is returned as absolute path
        let absolute_path = std::fs::canonicalize(parent);
        absolute_path.ok()
    });
    log::info!("Loading file: {file_name:?}");
    tokio::fs::read(&path)
        .await
        .map_err(|error| FilePickerError::IoError(error.to_string()))
        .map(|content| (content, parent_folder, file_name))
}
