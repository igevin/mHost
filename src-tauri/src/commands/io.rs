use mhost_core::MhostError;

const MAX_FILE_READ_SIZE: usize = 1_048_576; // 1MB

#[tauri::command]
pub fn read_file_text(path: String) -> Result<String, MhostError> {
    let metadata = std::fs::metadata(&path)?;
    if metadata.len() > MAX_FILE_READ_SIZE as u64 {
        return Err(MhostError::InvalidInput(format!(
            "File too large (max {} bytes)",
            MAX_FILE_READ_SIZE
        )));
    }
    std::fs::read_to_string(&path).map_err(Into::into)
}

#[tauri::command]
pub fn write_file_text(path: String, content: String) -> Result<(), MhostError> {
    std::fs::write(&path, &content).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_file_text() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "hello world").unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        let result = read_file_text(path).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_read_file_text_empty() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        let result = read_file_text(path).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_read_file_text_not_found() {
        let result = read_file_text("/nonexistent/path/file.txt".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_read_file_too_large() {
        let mut tmp = NamedTempFile::new().unwrap();
        // Write 1MB + 1 byte
        let large_content = vec![0u8; MAX_FILE_READ_SIZE + 1];
        tmp.write_all(&large_content).unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        let result = read_file_text(path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("File too large"),
            "expected 'File too large' in error message, got: {}",
            msg
        );
    }

    #[test]
    fn test_read_file_at_limit() {
        let mut tmp = NamedTempFile::new().unwrap();
        // Write exactly 1MB
        let content = vec![0u8; MAX_FILE_READ_SIZE];
        tmp.write_all(&content).unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        let result = read_file_text(path);
        assert!(result.is_ok(), "1MB file should be readable");
    }

    #[test]
    fn test_write_file_text() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        write_file_text(path.clone(), "test content".to_string()).unwrap();

        let read_back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read_back, "test content");
    }

    #[test]
    fn test_write_file_text_empty() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        write_file_text(path.clone(), "".to_string()).unwrap();

        let read_back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read_back, "");
    }

    #[test]
    fn test_write_and_read_roundtrip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        let original = "line1\nline2\nline3";
        write_file_text(path.clone(), original.to_string()).unwrap();
        let read_back = read_file_text(path).unwrap();
        assert_eq!(read_back, original);
    }
}
