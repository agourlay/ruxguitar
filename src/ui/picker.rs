use std::path::PathBuf;

#[derive(Debug, Clone, thiserror::Error)]
pub enum PickerError {
    #[error("dialog window closed without selecting a file")]
    DialogClosed,
    #[error("IO error: {0}")]
    IoError(String),
}

pub async fn open_file() -> Result<(Vec<u8>, String), PickerError> {
    let picked_file = rfd::AsyncFileDialog::new()
        .add_filter("Guitar Pro files", &["gp5"]) // only gp5 for now in parser
        .set_title("Pick a GP file")
        .pick_file()
        .await
        .ok_or(PickerError::DialogClosed)?;
    let file_name = picked_file.file_name();
    log::info!("Loading file: {:?}", file_name);
    let content = load_file(picked_file).await?;
    Ok((content, file_name))
}

async fn load_file(path: impl Into<PathBuf>) -> Result<Vec<u8>, PickerError> {
    let path = path.into();
    tokio::fs::read(&path)
        .await
        .map_err(|error| PickerError::IoError(error.to_string()))
}
