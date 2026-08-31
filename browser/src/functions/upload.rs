//! `browser::upload` — attach local bytes to one page file input.

use std::io::Write;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_FILES: usize = 8;
pub const MAX_FILE_BYTES: usize = 25 * 1024 * 1024;
const MAX_ENCODED_BYTES: usize = MAX_FILE_BYTES.div_ceil(3) * 4;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UploadFile {
    /// File name exposed to the page. Path components are rejected.
    pub name: String,
    /// File bytes, base64.
    pub data: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UploadInput {
    pub session_id: String,
    /// CSS selector that must match exactly one input[type=file].
    pub selector: String,
    /// Up to eight files, each at most 25 MB decoded.
    pub files: Vec<UploadFile>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UploadOutput {
    pub ok: bool,
    pub attached: usize,
    pub file_names: Vec<String>,
}

pub fn validate_files(files: &[UploadFile]) -> Result<(), String> {
    if files.len() > MAX_FILES {
        return Err(format!(
            "upload has {} files; at most {MAX_FILES} are allowed",
            files.len()
        ));
    }
    for file in files {
        if file.name.is_empty()
            || file.name == "."
            || file.name.contains(['/', '\\'])
            || file.name.contains("..")
        {
            return Err(format!(
                "upload file name '{}' must not contain path parts",
                file.name
            ));
        }
        if file.data.len() > MAX_ENCODED_BYTES {
            return Err(format!(
                "upload file '{}' is over the {MAX_FILE_BYTES} byte decoded cap",
                file.name
            ));
        }
    }
    Ok(())
}

pub fn stage_files(dir: &Path, files: Vec<UploadFile>) -> Result<Vec<PathBuf>, String> {
    validate_files(&files)?;
    let mut paths = Vec::with_capacity(files.len());
    for file in files {
        let bytes = STANDARD
            .decode(&file.data)
            .map_err(|e| format!("upload file '{}' has invalid base64: {e}", file.name))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(format!(
                "upload file '{}' is {} bytes, over the {MAX_FILE_BYTES} byte cap",
                file.name,
                bytes.len()
            ));
        }
        let path = dir.join(&file.name);
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("stage upload file '{}' failed: {e}", file.name))?;
        output
            .write_all(&bytes)
            .map_err(|e| format!("stage upload file '{}' failed: {e}", file.name))?;
        paths.push(path);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, bytes: &[u8]) -> UploadFile {
        UploadFile {
            name: name.to_string(),
            data: STANDARD.encode(bytes),
        }
    }

    #[test]
    fn rejects_path_parts_and_too_many_files() {
        for name in ["../secret", "a/b", "a\\b", "report..pdf", ".", ""] {
            assert!(validate_files(&[file(name, b"x")]).is_err(), "{name}");
        }
        let files: Vec<_> = (0..=MAX_FILES)
            .map(|index| file(&format!("{index}.txt"), b"x"))
            .collect();
        assert!(validate_files(&files).is_err());
    }

    #[test]
    fn stages_decoded_bytes_with_the_requested_name() {
        let dir = std::env::temp_dir().join(format!(
            "iii-browser-upload-test-{}-{}",
            std::process::id(),
            crate::session::now_ms()
        ));
        std::fs::create_dir(&dir).unwrap();
        let paths = stage_files(&dir, vec![file("report.txt", b"hello")]).unwrap();
        assert_eq!(paths, vec![dir.join("report.txt")]);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"hello");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_invalid_base64() {
        let input = UploadFile {
            name: "bad.txt".to_string(),
            data: "not base64".to_string(),
        };
        let error = stage_files(Path::new("/unused"), vec![input]).unwrap_err();
        assert!(error.contains("invalid base64"), "{error}");
    }
}
