use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantHint {
    pub v: u8,
    pub dir: String,
    pub path: String,
    pub code: String,
}

impl GrantHint {
    pub fn new(code: &str, raw_path: &str, resolved_path: &Path) -> Self {
        Self {
            v: 1,
            dir: nearest_existing_dir(resolved_path).display().to_string(),
            path: raw_path.to_string(),
            code: code.to_string(),
        }
    }

    pub fn suffix(&self) -> String {
        let json = serde_json::to_string(self).expect("GrantHint serializes");
        format!(" grant_hint={json}")
    }
}

pub fn hint_suffix(code: &str, raw_path: &str, resolved_path: &Path) -> String {
    GrantHint::new(code, raw_path, resolved_path).suffix()
}

fn nearest_existing_dir(path: &Path) -> PathBuf {
    if path.is_dir() {
        return path.to_path_buf();
    }
    for ancestor in path.ancestors().skip(1) {
        if ancestor.is_dir() {
            return ancestor.to_path_buf();
        }
    }
    path.parent().unwrap_or(path).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_existing_dir_uses_parent_for_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("dir/file.txt");
        std::fs::create_dir(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "x").unwrap();

        assert_eq!(nearest_existing_dir(&file), file.parent().unwrap());
    }

    #[test]
    fn nearest_existing_dir_uses_existing_ancestor_for_missing_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("dir");
        std::fs::create_dir(&dir).unwrap();
        let missing = dir.join("missing/file.txt");

        assert_eq!(nearest_existing_dir(&missing), dir);
    }
}
