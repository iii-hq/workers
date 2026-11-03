//! Temporary fixture ownership; never adopts or removes an existing directory.
use std::path::{Path, PathBuf};

pub struct TempDir {
    path: PathBuf,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("sentinel-{label}-{}", uuid::Uuid::now_v7()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .expect("create exclusive fixture directory");
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::symlink_metadata(&path).expect("stat owned fixture directory");
            (metadata.dev(), metadata.ino())
        };
        Self {
            path,
            #[cfg(unix)]
            identity,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            eprintln!("fixture cleanup refused a replaced root");
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (metadata.dev(), metadata.ino()) != self.identity {
                eprintln!("fixture cleanup refused a different root inode");
                return;
            }
        }
        // remove_dir_all does not follow the synthetic symlinks inside this
        // exclusively created root. Cleanup errors must not mask a test panic.
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            eprintln!("fixture cleanup failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_fixture_is_removed_when_scope_ends() {
        let path;
        {
            let root = TempDir::new("cleanup");
            path = root.path().to_path_buf();
            std::fs::write(path.join("synthetic.rs"), "synthetic fixture").unwrap();
        }
        assert!(!path.exists());
    }

    #[test]
    fn cleanup_preserves_the_original_test_panic() {
        let mut path = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let root = TempDir::new("panic-cleanup");
            path = Some(root.path().to_path_buf());
            std::fs::write(root.path().join("synthetic.rs"), "synthetic fixture").unwrap();
            panic!("original fixture test failure");
        }));
        let error = result.expect_err("the original panic must not be swallowed");
        assert_eq!(
            error.downcast_ref::<&str>(),
            Some(&"original fixture test failure")
        );
        assert!(!path.unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_does_not_follow_symlinks_into_another_root() {
        let outside = TempDir::new("outside");
        let target = outside.path().join("keep.rs");
        std::fs::write(&target, "keep outside fixture").unwrap();
        let root = TempDir::new("symlink-cleanup");
        std::os::unix::fs::symlink(&target, root.path().join("linked.rs")).unwrap();
        let path = root.path().to_path_buf();
        drop(root);
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_to_string(target).unwrap(),
            "keep outside fixture"
        );
    }
}
