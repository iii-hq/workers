//! Images pasted into a Console terminal. The page sends the image; this
//! writes it to a new file in the cache folder of the user the worker runs
//! as and answers the file's path, which the page then pastes like text:
//! Claude Code and Codex read a pasted image path as an attachment. Nothing
//! is typed here.
//!
//! Files stay a week, not a moment: Codex reads the file only when the
//! message it is attached to is sent.

use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The largest image accepted, decoded: its base64 still fits a 16 MiB
/// browser frame for a paste from the Console page.
pub const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;

/// How long a pasted image is kept.
pub const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Webp,
}

const KINDS: [ImageKind; 4] = [
    ImageKind::Png,
    ImageKind::Jpeg,
    ImageKind::Gif,
    ImageKind::Webp,
];

impl ImageKind {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
        }
    }

    pub fn from_mime(mime: &str) -> Option<Self> {
        let mime = mime.trim();
        KINDS
            .into_iter()
            .find(|kind| kind.mime().eq_ignore_ascii_case(mime))
    }

    /// What the file's first bytes say it is.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.starts_with(b"\xff\xd8\xff") {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else {
            None
        }
    }
}

/// The decoded image and its kind, or the one sentence that refuses it.
/// `data` is base64; its length is checked before it is decoded.
pub fn decode(mime: &str, data: &str) -> Result<(ImageKind, Vec<u8>), String> {
    use base64::Engine;
    let kind = ImageKind::from_mime(mime)
        .ok_or_else(|| "Only PNG, JPEG, GIF and WebP images can be pasted.".to_string())?;
    let too_big = || "The image is larger than 12 MiB.".to_string();
    if data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err(too_big());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|_| "The image is not valid base64.".to_string())?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(too_big());
    }
    if ImageKind::sniff(&bytes) != Some(kind) {
        return Err(format!(
            "The image's data is not {}, as its type says.",
            kind.mime()
        ));
    }
    Ok((kind, bytes))
}

/// The folder pasted images go to: `$XDG_CACHE_HOME/iii/tmux/paste` (when
/// absolute) or `$HOME/.cache/iii/tmux/paste` on Linux,
/// `$HOME/Library/Caches/iii/tmux/paste` on macOS. A relative or empty value
/// would mean the worker's current folder, so it counts as unset.
pub fn folder(
    macos: bool,
    home: Option<&OsStr>,
    xdg_cache_home: Option<&OsStr>,
) -> Result<PathBuf, String> {
    fn absolute(value: Option<&OsStr>) -> Option<&Path> {
        value.map(Path::new).filter(|path| path.is_absolute())
    }
    let cache = if macos {
        absolute(home).map(|home| home.join("Library/Caches"))
    } else {
        absolute(xdg_cache_home)
            .map(Path::to_path_buf)
            .or_else(|| absolute(home).map(|home| home.join(".cache")))
    };
    cache
        .map(|cache| cache.join("iii/tmux/paste"))
        .ok_or_else(|| "This computer's user has no home folder to keep the image in.".to_string())
}

/// [`folder`] for this process.
pub fn this_folder() -> Result<PathBuf, String> {
    folder(
        cfg!(target_os = "macos"),
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("XDG_CACHE_HOME").as_deref(),
    )
}

/// `(year, month, day)` of a day counted from 1970-01-01 (proleptic
/// Gregorian; Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `<UTC yyyymmdd-hhmmss>-<random>.<extension>`: no spaces, nothing the
/// caller chose.
pub fn file_name(at: SystemTime, random: &str, kind: ImageKind) -> String {
    let seconds = at
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default();
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let time = seconds.rem_euclid(86_400);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}-{random}.{}",
        time / 3600,
        time % 3600 / 60,
        time % 60,
        kind.extension()
    )
}

/// A name [`file_name`] gives, and so a file a sweep may remove.
pub fn is_ours(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let digits =
        |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    let mut parts = stem.split('-');
    let (Some(date), Some(time), Some(random), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    digits(date, 8)
        && digits(time, 6)
        && random.len() == 8
        && random
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && KINDS.iter().any(|kind| kind.extension() == extension)
}

/// Removes the pasted images in `dir` older than `older_than`: regular files
/// with a name [`is_ours`], never a symlink or anything else there.
pub fn sweep(dir: &Path, older_than: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let ours = entry.file_name().to_str().is_some_and(is_ours);
        // `DirEntry::metadata` doesn't follow a symlink.
        let old = entry.metadata().is_ok_and(|meta| {
            meta.is_file()
                && meta
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > older_than)
        });
        if ours && old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// `dir` made (only this user can open it), then a sweep, then `bytes` in a
/// new file there that only this user can read. Returns the file's path.
pub fn store(dir: &Path, kind: ImageKind, bytes: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let meta = dir.symlink_metadata()?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    if !meta.is_dir() || meta.uid() != uid {
        return Err(std::io::Error::other(format!(
            "{} is not a folder of this user",
            dir.display()
        )));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    sweep(dir, KEEP_FOR);
    let random = uuid::Uuid::new_v4().simple().to_string();
    let path = dir.join(file_name(SystemTime::now(), &random[..8], kind));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    if let Err(error) = written {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    const JPEG: &[u8] = b"\xff\xd8\xff\xe0\0\x10JFIF";
    const GIF: &[u8] = b"GIF89a\x01\0\x01\0";
    const WEBP: &[u8] = b"RIFF\x24\0\0\0WEBPVP8 ";

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn the_four_kinds_are_told_by_their_first_bytes() {
        for (bytes, kind) in [
            (PNG, ImageKind::Png),
            (JPEG, ImageKind::Jpeg),
            (GIF, ImageKind::Gif),
            (WEBP, ImageKind::Webp),
        ] {
            assert_eq!(ImageKind::sniff(bytes), Some(kind));
            assert_eq!(ImageKind::from_mime(kind.mime()), Some(kind));
            assert_eq!(
                decode(kind.mime(), &b64(bytes)).unwrap(),
                (kind, bytes.to_vec())
            );
        }
        assert_eq!(ImageKind::from_mime(" Image/PNG "), Some(ImageKind::Png));
        assert_eq!(ImageKind::sniff(b"GIF88a"), None);
        assert_eq!(ImageKind::sniff(b"RIFF\0\0\0\0WAVE"), None);
        assert_eq!(ImageKind::sniff(b""), None);
    }

    #[test]
    fn anything_else_is_refused_in_one_sentence() {
        assert_eq!(
            decode("image/svg+xml", &b64(b"<svg/>")).unwrap_err(),
            "Only PNG, JPEG, GIF and WebP images can be pasted."
        );
        assert_eq!(
            decode("image/jpg", &b64(JPEG)).unwrap_err(),
            "Only PNG, JPEG, GIF and WebP images can be pasted."
        );
        assert_eq!(
            decode("image/png", &b64(JPEG)).unwrap_err(),
            "The image's data is not image/png, as its type says."
        );
        assert_eq!(
            decode("image/png", "").unwrap_err(),
            "The image's data is not image/png, as its type says."
        );
        assert_eq!(
            decode("image/png", "not base64!").unwrap_err(),
            "The image is not valid base64."
        );
    }

    #[test]
    fn twelve_mib_is_the_limit() {
        let mut exact = PNG.to_vec();
        exact.resize(MAX_IMAGE_BYTES, 0);
        assert_eq!(
            decode("image/png", &b64(&exact)).unwrap().1.len(),
            MAX_IMAGE_BYTES
        );
        exact.push(0);
        assert_eq!(
            decode("image/png", &b64(&exact)).unwrap_err(),
            "The image is larger than 12 MiB."
        );
        // Refused by its length, before decoding.
        let long = "A".repeat(MAX_IMAGE_BYTES / 3 * 4 + 8);
        assert_eq!(
            decode("image/png", &long).unwrap_err(),
            "The image is larger than 12 MiB."
        );
    }

    #[test]
    fn the_folder_is_the_users_cache() {
        let os = |value: &str| Some(OsStr::new(value).to_owned());
        let at = |macos, home: Option<&str>, xdg: Option<&str>| {
            folder(
                macos,
                home.and_then(os).as_deref(),
                xdg.and_then(os).as_deref(),
            )
        };
        assert_eq!(
            at(false, Some("/home/u"), None).unwrap(),
            PathBuf::from("/home/u/.cache/iii/tmux/paste")
        );
        assert_eq!(
            at(false, Some("/home/u"), Some("/var/cache/u")).unwrap(),
            PathBuf::from("/var/cache/u/iii/tmux/paste")
        );
        assert_eq!(
            at(false, Some("/home/u"), Some("cache")).unwrap(),
            PathBuf::from("/home/u/.cache/iii/tmux/paste"),
            "a relative XDG_CACHE_HOME is ignored"
        );
        assert_eq!(
            at(false, None, Some("/x")).unwrap(),
            PathBuf::from("/x/iii/tmux/paste")
        );
        assert_eq!(
            at(true, Some("/Users/u"), Some("/x")).unwrap(),
            PathBuf::from("/Users/u/Library/Caches/iii/tmux/paste")
        );
        for (macos, home) in [
            (false, None),
            (false, Some("")),
            (true, Some("u")),
            (true, None),
        ] {
            assert_eq!(
                at(macos, home, None).unwrap_err(),
                "This computer's user has no home folder to keep the image in."
            );
        }
    }

    #[test]
    fn names_carry_the_utc_time_and_nothing_else() {
        let at = UNIX_EPOCH + Duration::from_secs(1_794_241_008);
        assert_eq!(
            file_name(at, "0a1b2c3d", ImageKind::Jpeg),
            "20261109-161648-0a1b2c3d.jpg"
        );
        assert_eq!(
            file_name(UNIX_EPOCH, "ffffffff", ImageKind::Webp),
            "19700101-000000-ffffffff.webp"
        );
        // A leap day.
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400 + 86_399);
        assert_eq!(
            file_name(leap, "00000000", ImageKind::Png),
            "20000229-235959-00000000.png"
        );
        assert!(is_ours("20261110-161648-0a1b2c3d.jpg"));
        assert!(is_ours("20261110-161648-0a1b2c3d.webp"));
        for other in [
            "20261110-161648-0a1b2c3d.jpeg",
            "20261110-161648-0A1B2C3D.png",
            "20261110-161648-0a1b2c3.png",
            "2026111-161648-0a1b2c3d.png",
            "20261110-161648-0a1b2c3d-x.png",
            "notes.txt",
            "20261110-161648-0a1b2c3d",
        ] {
            assert!(!is_ours(other), "{other}");
        }
    }

    fn temp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("iii-tmux-paste-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn age(path: &Path, by: Duration) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - by).unwrap();
    }

    #[test]
    fn a_stored_image_is_this_users_alone() {
        let root = temp_dir();
        let dir = root.join("iii/tmux/paste");
        let path = store(&dir, ImageKind::Png, PNG).unwrap();
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert!(is_ours(path.file_name().unwrap().to_str().unwrap()));
        assert_eq!(std::fs::read(&path).unwrap(), PNG);
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir), 0o700);
        // A folder another program opened up is closed again.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let second = store(&dir, ImageKind::Gif, GIF).unwrap();
        assert_ne!(second, path);
        assert_eq!(mode(&dir), 0o700);
        // Not a folder: refused.
        let link = root.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(store(&link, ImageKind::Png, PNG).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn only_week_old_images_of_ours_are_swept() {
        let dir = temp_dir();
        let old = dir.join("20261001-120000-0a1b2c3d.png");
        let new = dir.join("20261110-120000-0a1b2c3d.png");
        let foreign = dir.join("keep-me.png");
        for path in [&old, &new, &foreign] {
            std::fs::write(path, PNG).unwrap();
        }
        let week = KEEP_FOR + Duration::from_secs(60);
        age(&old, week);
        age(&foreign, week);
        let target = dir.join("20261001-120000-00000000.png.target");
        std::fs::write(&target, PNG).unwrap();
        age(&target, week);
        let link = dir.join("20261001-120000-11111111.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(sweep(&dir, KEEP_FOR), 1);
        assert!(!old.exists());
        assert!(new.exists() && foreign.exists() && target.exists());
        assert!(link.symlink_metadata().is_ok(), "a symlink is left alone");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
