use std::path::PathBuf;

#[derive(Debug, Clone, thiserror::Error)]
pub enum FilePickerError {
    #[error("dialog window closed without selecting a file")]
    DialogClosed,
    #[error("IO error: {0}")]
    IoError(String),
}

/// Opens a file dialog and returns the content of the picked file.
pub async fn open_file_dialog() -> Result<(Vec<u8>, String), FilePickerError> {
    let picked_file = rfd::AsyncFileDialog::new()
        .add_filter("Guitar Pro files", &["gp5"]) // only gp5 for now in parser
        .set_title("Pick a GP file")
        .pick_file()
        .await
        .ok_or(FilePickerError::DialogClosed)?;
    load_file(picked_file).await
}

/// Loads the content of a file at the given path.
///
/// Return the content of the file and its name.
pub async fn load_file(path: impl Into<PathBuf>) -> Result<(Vec<u8>, String), FilePickerError> {
    let path = path.into();
    let file_extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if file_extension != "gp5" {
        return Err(FilePickerError::IoError(format!(
            "Unsupported file extension: {}",
            file_extension
        )));
    }
    let file_name = path
        .file_name()
        .and_then(|f| f.to_str())
        .map(|f| f.to_string())
        .unwrap_or_default();
    log::info!("Loading file: {:?}", file_name);
    tokio::fs::read(&path)
        .await
        .map_err(|error| FilePickerError::IoError(error.to_string()))
        .map(|content| (content, file_name))
}
