//! Files pasted into a Console terminal. The page sends the file; this
//! writes it to a new folder of its own under the cache folder of the user
//! the worker runs as, under its own name made safe, and answers the file's
//! path, which the page then pastes like text: Claude Code and Codex read a
//! pasted path as an attachment. Nothing is typed here.
//!
//! Uploads stay a week, not a moment: Codex reads a file only when the
//! message it is attached to is sent. All of them together stay under
//! [`BUDGET`]; past it the oldest go first.
//!
//! The folders are checked one level at a time below the user's cache folder
//! and never followed through a symbolic link, and a file is written to a
//! temporary file first, so a kill or a full disk never leaves a cut-short
//! file under an upload's name.

use std::ffi::OsStr;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The largest file accepted, decoded: its base64 still fits a 16 MiB
/// browser frame for a paste from the Console page.
pub const MAX_BYTES: usize = 12 * 1024 * 1024;

/// How long an upload is kept.
pub const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What the uploads may add up to; past it the oldest go.
pub const BUDGET: u64 = 256 * 1024 * 1024;

/// An upload folder a cut-short write left without its file goes after this.
const TEMP_KEEP: Duration = Duration::from_secs(60 * 60);

/// Temporary files are named this and 32 hex digits.
const TEMP_PREFIX: &str = ".upload-";

/// The folders under the user's cache folder, made and checked one by one.
const LEVELS: [&str; 3] = ["iii", "tmux", "uploads"];

/// The longest file name kept, in bytes (all ASCII once made safe).
const MAX_NAME: usize = 100;

/// An extension is at most this long, letters and digits only.
const MAX_EXTENSION: usize = 16;

const OCTET_STREAM: &str = "application/octet-stream";

const LINKED: &str = "The folder for uploaded files is a symbolic link, so no file is kept there.";
const NOT_A_FOLDER: &str =
    "Something that isn't a folder is where uploaded files go, so no file is kept there.";
const NOT_OURS: &str =
    "The folder for uploaded files belongs to another user, so no file is kept there.";
const OPEN_CACHE: &str =
    "Any user can change this user's cache folder, so no uploaded file is kept there.";
const NOT_SAVED: &str = "The file could not be saved.";

/// The images agents recognize, told by their first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageKind {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    /// The extension a name gets when it has none of [`Self::extensions`].
    pub fn extension(self) -> &'static str {
        self.extensions()[0]
    }

    /// Extensions that already say what it is (compared ignoring case).
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Png => &["png"],
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Gif => &["gif"],
            Self::Webp => &["webp"],
        }
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

/// The decoded file, or the one sentence that refuses it. `data` is
/// base64; its length is checked before it is decoded.
pub fn decode(data: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let too_big = || "The file is larger than 12 MiB.".to_string();
    if data.len() > MAX_BYTES.div_ceil(3) * 4 {
        return Err(too_big());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|_| "The file is not valid base64.".to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err(too_big());
    }
    Ok(bytes)
}

/// `name` split into a stem and an extension when it has one: letters and
/// digits after the last dot, and something before it.
fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, extension))
            if !stem.is_empty()
                && !extension.is_empty()
                && extension.len() <= MAX_EXTENSION
                && extension.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            (stem, Some(extension))
        }
        _ => (name, None),
    }
}

/// A file name made safe to paste as one shell token: the base name only
/// (nothing up to the last `/` or `\`), every character outside
/// `[A-Za-z0-9._-]` turned into `-`, runs of `-` and `.` collapsed, no
/// leading dot or dash, at most 100 bytes with its extension kept, and
/// `file` when nothing is left.
pub fn sanitize(name: &str) -> String {
    let safe = safe_chars(name);
    let (stem, extension) = split_extension(&safe);
    limited(stem, extension)
}

/// [`sanitize`] before the length limit.
fn safe_chars(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut safe = String::with_capacity(base.len());
    for c in base.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            c
        } else {
            '-'
        };
        match (safe.chars().last(), c) {
            (Some('-'), '-') | (Some('.'), '.') => {}
            // `photo (1).png` is `photo-1.png`, not `photo-1-.png`.
            (Some('-'), '.') => {
                safe.pop();
                if !safe.ends_with('.') {
                    safe.push('.');
                }
            }
            (Some('.'), '-') => {}
            _ => safe.push(c),
        }
    }
    safe.trim_matches(['.', '-']).to_string()
}

/// `stem.extension` cut to [`MAX_NAME`] bytes, the extension kept whole;
/// `file` for an empty stem.
fn limited(stem: &str, extension: Option<&str>) -> String {
    let room = match extension {
        Some(extension) => MAX_NAME - 1 - extension.len(),
        None => MAX_NAME,
    };
    // ASCII by now, so any byte is a character boundary.
    let stem = stem[..stem.len().min(room)].trim_end_matches(['.', '-']);
    let stem = if stem.is_empty() { "file" } else { stem };
    match extension {
        Some(extension) => format!("{stem}.{extension}"),
        None => stem.to_string(),
    }
}

/// The name an upload is kept under, and what its bytes are when they are a
/// known image: then the extension follows the content (`photo.HEIC` that
/// is really a JPEG is `photo.jpg`), so agents read it as an image.
pub fn file_name(name: &str, bytes: &[u8]) -> (String, Option<ImageKind>) {
    let safe = safe_chars(name);
    let (stem, extension) = split_extension(&safe);
    let Some(kind) = ImageKind::sniff(bytes) else {
        return (limited(stem, extension), None);
    };
    let named = extension.is_some_and(|extension| {
        kind.extensions()
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension))
    });
    let extension = if named {
        extension
    } else {
        Some(kind.extension())
    };
    (limited(stem, extension), Some(kind))
}

/// The type the answer names: what the bytes are when known, else the
/// caller's (a plain `type/subtype`), else `application/octet-stream`.
pub fn answer_mime(kind: Option<ImageKind>, given: Option<&str>) -> String {
    if let Some(kind) = kind {
        return kind.mime().to_string();
    }
    let token = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
    };
    match given.map(str::trim) {
        Some(mime)
            if mime.len() <= 127
                && mime
                    .split_once('/')
                    .is_some_and(|(kind, sub)| token(kind) && token(sub)) =>
        {
            mime.to_ascii_lowercase()
        }
        _ => OCTET_STREAM.to_string(),
    }
}

/// The cache folder of the user the worker runs as, which uploads go under
/// (`iii/tmux/uploads`): `$XDG_CACHE_HOME` (when absolute) or
/// `$HOME/.cache` on Linux, `$HOME/Library/Caches` on macOS. A relative or
/// empty value would mean the worker's current folder, so it counts as unset.
pub fn cache_folder(
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
    cache.ok_or_else(|| "This computer's user has no home folder to keep the file in.".to_string())
}

/// [`cache_folder`] for this process.
pub fn this_cache_folder() -> Result<PathBuf, String> {
    cache_folder(
        cfg!(target_os = "macos"),
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("XDG_CACHE_HOME").as_deref(),
    )
}

/// The one sentence for a file system error; what it was goes to the log.
/// `std::io::Error` names no path, so neither does the sentence.
fn io_sentence(error: &std::io::Error) -> String {
    tracing::warn!(%error, "uploaded file or its folder");
    match error.kind() {
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => {
            "There is no space left to save the file.".to_string()
        }
        ErrorKind::PermissionDenied => {
            "This user can't write to the folder for uploaded files.".to_string()
        }
        ErrorKind::ReadOnlyFilesystem => {
            "The folder for uploaded files is on a disk that can't be written to.".to_string()
        }
        _ => NOT_SAVED.to_string(),
    }
}

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// One folder, 0700, never through a link; one made just now by someone
/// else is checked like any other.
fn make_folder(dir: &Path) -> Result<(), String> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(io_sentence(&error)),
    }
}

/// `cache/iii/tmux/uploads`, checked one level at a time: each is a real
/// folder (never a symbolic link) of this user that no other user can
/// write to; one of ours that is wider is narrowed (the last to 0700, the
/// others lose group and other write, as other iii programs share
/// `cache/iii`). With `create`, a missing level is made with 0700, one at a
/// time, so nothing is followed through a link; without it, a missing one
/// is `None`. The cache folder itself is the user's (perhaps a link to
/// another disk): it only has to be a folder that not every user can change,
/// unless sticky like `/tmp`.
pub fn checked_folder(cache: &Path, create: bool) -> Result<Option<PathBuf>, String> {
    let meta = match std::fs::metadata(cache) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound && create => {
            make_folder(cache)?;
            std::fs::metadata(cache).map_err(|error| io_sentence(&error))?
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_sentence(&error)),
    };
    if !meta.is_dir() {
        return Err(NOT_A_FOLDER.to_string());
    }
    if meta.permissions().mode() & 0o1002 == 0o002 {
        return Err(OPEN_CACHE.to_string());
    }
    let mut dir = cache.to_path_buf();
    for (index, level) in LEVELS.iter().enumerate() {
        dir.push(level);
        let last = index + 1 == LEVELS.len();
        let meta = match dir.symlink_metadata() {
            Ok(meta) => meta,
            Err(error) if error.kind() == ErrorKind::NotFound && create => {
                make_folder(&dir)?;
                dir.symlink_metadata()
                    .map_err(|error| io_sentence(&error))?
            }
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_sentence(&error)),
        };
        if meta.file_type().is_symlink() {
            return Err(LINKED.to_string());
        }
        if !meta.is_dir() {
            return Err(NOT_A_FOLDER.to_string());
        }
        if meta.uid() != uid() {
            return Err(NOT_OURS.to_string());
        }
        let mode = meta.permissions().mode() & 0o7777;
        let wanted = if last { 0o700 } else { mode & !0o022 };
        if mode != wanted {
            // Not a link (checked above), in a folder no one else can change.
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(wanted))
                .map_err(|error| io_sentence(&error))?;
        }
    }
    Ok(Some(dir))
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

/// An upload's own folder: `<UTC yyyymmdd-hhmmss>-<random>`.
pub fn folder_name(at: SystemTime, random: &str) -> String {
    let seconds = at
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default();
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let time = seconds.rem_euclid(86_400);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}-{random}",
        time / 3600,
        time % 3600 / 60,
        time % 60,
    )
}

/// A name [`folder_name`] gives, and so a folder a sweep may remove.
pub fn is_ours(name: &str) -> bool {
    let digits =
        |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    let mut parts = name.split('-');
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
}

/// One of our upload folders as a sweep sees it.
struct Upload {
    path: PathBuf,
    modified: SystemTime,
    /// What the regular files in it add up to.
    size: u64,
    /// It holds a file that isn't a temporary one: the write finished.
    complete: bool,
}

/// Our upload folders in `dir`: real folders of this user with a name
/// [`is_ours`]; a symlink or anything else is never ours.
fn uploads(dir: &Path) -> Vec<Upload> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            if !entry.file_name().to_str().is_some_and(is_ours) {
                return None;
            }
            // `DirEntry::metadata` doesn't follow a symlink.
            let meta = entry
                .metadata()
                .ok()
                .filter(|meta| meta.is_dir() && meta.uid() == uid())?;
            let mut size = 0;
            let mut complete = false;
            for file in std::fs::read_dir(entry.path()).ok()?.flatten() {
                let Ok(file_meta) = file.metadata() else {
                    continue;
                };
                if file_meta.is_file() {
                    size += file_meta.len();
                    let temp = file
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with(TEMP_PREFIX));
                    complete |= !temp;
                }
            }
            Some(Upload {
                path: entry.path(),
                modified: meta.modified().ok()?,
                size,
                complete,
            })
        })
        .collect()
}

/// `remove_dir_all` doesn't follow symbolic links inside the folder.
fn remove(upload: &Upload) -> bool {
    std::fs::remove_dir_all(&upload.path).is_ok()
}

/// Removes our uploads in `dir` older than `older_than`, and the folders a
/// cut-short write left without their file after an hour; never anything
/// else there.
pub fn sweep(dir: &Path, older_than: Duration) -> usize {
    uploads(dir)
        .iter()
        .filter(|upload| {
            let age = upload.modified.elapsed().unwrap_or_default();
            let limit = if upload.complete {
                older_than
            } else {
                TEMP_KEEP
            };
            age > limit && remove(upload)
        })
        .count()
}

/// Removes our oldest uploads in `dir`, never `keep`, while they add up to
/// more than `budget` bytes. Only ours count: someone else's files there
/// can't make ours go.
pub fn trim(dir: &Path, budget: u64, keep: Option<&Path>) -> usize {
    let mut all = uploads(dir);
    let mut total: u64 = all.iter().map(|upload| upload.size).sum();
    all.sort_by(|a, b| (a.modified, &a.path).cmp(&(b.modified, &b.path)));
    let mut removed = 0;
    for upload in &all {
        if total <= budget {
            break;
        }
        if Some(upload.path.as_path()) == keep {
            continue;
        }
        if remove(upload) {
            total -= upload.size;
            removed += 1;
        }
    }
    removed
}

/// `bytes` as `name` in a new folder of its own in `dir`, written whole to
/// a temporary file there and then linked under its name; only this user
/// can read it, no one can run it. The upload's folder and file.
fn write(dir: &Path, name: &str, bytes: &[u8]) -> Result<(PathBuf, PathBuf), String> {
    for _ in 0..3 {
        let random = uuid::Uuid::new_v4().simple().to_string();
        let folder = dir.join(folder_name(SystemTime::now(), &random[..8]));
        match std::fs::DirBuilder::new().mode(0o700).create(&folder) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_sentence(&error)),
        }
        let path = folder.join(name);
        let temp = format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4().simple());
        let written =
            crate::install::write_new_file_with(&path, &temp, 0o600, |file| file.write_all(bytes));
        return match written {
            Ok(true) => Ok((folder, path)),
            Ok(false) => {
                let _ = std::fs::remove_dir_all(&folder);
                Err(NOT_SAVED.to_string())
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&folder);
                Err(io_sentence(&error))
            }
        };
    }
    Err(NOT_SAVED.to_string())
}

/// An upload as [`save`] kept it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: PathBuf,
    pub bytes: usize,
    pub mime: String,
}

/// Decodes the file, then keeps it as `cache/iii/tmux/uploads/<folder>/<name>`:
/// old uploads go first, and the oldest beyond [`BUDGET`] after. Blocking
/// (base64 of up to 16 MiB, then the disk); every refusal is one sentence.
pub fn save(cache: &Path, name: &str, mime: Option<&str>, data: &str) -> Result<Saved, String> {
    let bytes = decode(data)?;
    let (name, kind) = file_name(name, &bytes);
    let dir = checked_folder(cache, true)?.ok_or_else(|| NOT_SAVED.to_string())?;
    sweep(&dir, KEEP_FOR);
    let (folder, path) = write(&dir, &name, &bytes)?;
    trim(&dir, BUDGET, Some(&folder));
    Ok(Saved {
        path,
        bytes: bytes.len(),
        mime: answer_mime(kind, mime),
    })
}

/// The sweep when the worker starts: only when the folder is there and
/// passes [`checked_folder`]. How many uploads it removed.
pub fn sweep_at_start(cache: &Path) -> Result<usize, String> {
    Ok(match checked_folder(cache, false)? {
        Some(dir) => sweep(&dir, KEEP_FOR) + trim(&dir, BUDGET, None),
        None => 0,
    })
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
    fn images_are_told_by_their_first_bytes() {
        for (bytes, kind) in [
            (PNG, ImageKind::Png),
            (JPEG, ImageKind::Jpeg),
            (GIF, ImageKind::Gif),
            (WEBP, ImageKind::Webp),
        ] {
            assert_eq!(ImageKind::sniff(bytes), Some(kind));
        }
        assert_eq!(ImageKind::sniff(b"GIF88a"), None);
        assert_eq!(ImageKind::sniff(b"RIFF\0\0\0\0WAVE"), None);
        assert_eq!(ImageKind::sniff(b"%PDF-1.7"), None);
        assert_eq!(ImageKind::sniff(b""), None);
    }

    #[test]
    fn any_file_up_to_twelve_mib() {
        assert_eq!(decode(&b64(b"%PDF-1.7 hello")).unwrap(), b"%PDF-1.7 hello");
        assert_eq!(decode("").unwrap(), b"");
        assert_eq!(
            decode("not base64!").unwrap_err(),
            "The file is not valid base64."
        );
        let mut exact = vec![7u8; MAX_BYTES];
        assert_eq!(decode(&b64(&exact)).unwrap().len(), MAX_BYTES);
        exact.push(0);
        assert_eq!(
            decode(&b64(&exact)).unwrap_err(),
            "The file is larger than 12 MiB."
        );
        // Refused by its length, before decoding.
        let long = "A".repeat(MAX_BYTES / 3 * 4 + 8);
        assert_eq!(
            decode(&long).unwrap_err(),
            "The file is larger than 12 MiB."
        );
    }

    #[test]
    fn names_are_one_safe_token() {
        for (name, safe) in [
            ("report.pdf", "report.pdf"),
            ("My Report (final).PDF", "My-Report-final.PDF"),
            ("photo (1).png", "photo-1.png"),
            ("ação ✓.txt", "a-o.txt"),
            ("../x", "x"),
            ("../../etc/passwd", "passwd"),
            ("C:\\Users\\u\\notes.md", "notes.md"),
            ("dir/", "file"),
            ("..", "file"),
            (".hidden", "hidden"),
            ("...", "file"),
            ("", "file"),
            ("   ", "file"),
            ("-rf", "rf"),
            ("a..b", "a.b"),
            ("tar.gz.", "tar.gz"),
            ("archive.tar.gz", "archive.tar.gz"),
            ("emoji 😀 name.json", "emoji-name.json"),
        ] {
            assert_eq!(sanitize(name), safe, "{name:?}");
        }
        // 300 bytes: cut to 100 with the extension kept.
        let long = format!("{}.markdown", "n".repeat(291));
        let safe = sanitize(&long);
        assert_eq!(safe.len(), 100);
        assert!(safe.ends_with("nnn.markdown"), "{safe}");
        let no_extension = "x".repeat(300);
        assert_eq!(sanitize(&no_extension), "x".repeat(100));
        // Something that isn't an extension stays part of the stem.
        let odd = format!("{}.not an extension", "y".repeat(120));
        assert_eq!(sanitize(&odd).len(), 100);
        assert!(sanitize(&odd).starts_with("yyy"));
    }

    #[test]
    fn an_image_is_named_by_what_it_is() {
        assert_eq!(
            file_name("photo.HEIC", JPEG),
            ("photo.jpg".to_string(), Some(ImageKind::Jpeg))
        );
        assert_eq!(
            file_name("Photo.JPEG", JPEG),
            ("Photo.JPEG".to_string(), Some(ImageKind::Jpeg))
        );
        assert_eq!(
            file_name("Screenshot 2026-11-10 at 10.15.02", PNG),
            (
                "Screenshot-2026-11-10-at-10.15.png".to_string(),
                Some(ImageKind::Png)
            )
        );
        assert_eq!(
            file_name("clip", WEBP),
            ("clip.webp".to_string(), Some(ImageKind::Webp))
        );
        assert_eq!(
            file_name("not-really.png", b"plain text"),
            ("not-really.png".to_string(), None)
        );
        let (long, _) = file_name(&format!("{}.heic", "p".repeat(300)), GIF);
        assert_eq!(long.len(), 100);
        assert!(long.ends_with(".gif"));
        assert_eq!(
            answer_mime(Some(ImageKind::Gif), Some("image/heic")),
            "image/gif"
        );
        assert_eq!(
            answer_mime(None, Some(" Application/PDF ")),
            "application/pdf"
        );
        assert_eq!(
            answer_mime(None, Some("text/plain; charset=utf-8")),
            "application/octet-stream"
        );
        assert_eq!(
            answer_mime(None, Some("nonsense")),
            "application/octet-stream"
        );
        assert_eq!(answer_mime(None, None), "application/octet-stream");
    }

    #[test]
    fn the_folder_is_the_users_cache() {
        let os = |value: &str| Some(OsStr::new(value).to_owned());
        let at = |macos, home: Option<&str>, xdg: Option<&str>| {
            cache_folder(
                macos,
                home.and_then(os).as_deref(),
                xdg.and_then(os).as_deref(),
            )
        };
        assert_eq!(
            at(false, Some("/home/u"), None).unwrap(),
            PathBuf::from("/home/u/.cache")
        );
        assert_eq!(
            at(false, Some("/home/u"), Some("/var/cache/u")).unwrap(),
            PathBuf::from("/var/cache/u")
        );
        assert_eq!(
            at(false, Some("/home/u"), Some("cache")).unwrap(),
            PathBuf::from("/home/u/.cache"),
            "a relative XDG_CACHE_HOME is ignored"
        );
        assert_eq!(at(false, None, Some("/x")).unwrap(), PathBuf::from("/x"));
        assert_eq!(
            at(true, Some("/Users/u"), Some("/x")).unwrap(),
            PathBuf::from("/Users/u/Library/Caches")
        );
        for (macos, home) in [
            (false, None),
            (false, Some("")),
            (true, Some("u")),
            (true, None),
        ] {
            assert_eq!(
                at(macos, home, None).unwrap_err(),
                "This computer's user has no home folder to keep the file in."
            );
        }
    }

    #[test]
    fn folders_carry_the_utc_time_and_nothing_else() {
        let at = UNIX_EPOCH + Duration::from_secs(1_794_241_008);
        assert_eq!(folder_name(at, "0a1b2c3d"), "20261109-161648-0a1b2c3d");
        assert_eq!(
            folder_name(UNIX_EPOCH, "ffffffff"),
            "19700101-000000-ffffffff"
        );
        // A leap day.
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400 + 86_399);
        assert_eq!(folder_name(leap, "00000000"), "20000229-235959-00000000");
        assert!(is_ours("20261110-161648-0a1b2c3d"));
        for other in [
            "20261110-161648-0A1B2C3D",
            "20261110-161648-0a1b2c3",
            "2026111-161648-0a1b2c3d",
            "20261110-161648-0a1b2c3d-x",
            "20261110-161648-0a1b2c3d.png",
            "notes",
        ] {
            assert!(!is_ours(other), "{other}");
        }
    }

    fn temp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("iii-tmux-upload-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn age(path: &Path, by: Duration) {
        let file = std::fs::File::open(path).unwrap();
        file.set_modified(SystemTime::now() - by).unwrap();
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// An upload folder of ours in `dir`, `size` bytes, `older` than now.
    fn upload(dir: &Path, folder: &str, size: usize, older: Duration) -> PathBuf {
        let path = dir.join(folder);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("f.bin"), vec![0u8; size]).unwrap();
        age(&path, older);
        path
    }

    #[test]
    fn an_upload_is_this_users_alone_under_its_own_name() {
        let cache = temp_dir();
        let dir = cache.join("iii/tmux/uploads");
        let saved = save(&cache, "My notes.md", Some("text/markdown"), &b64(b"# hi")).unwrap();
        assert_eq!((saved.bytes, saved.mime.as_str()), (4, "text/markdown"));
        assert_eq!(saved.path.file_name().unwrap(), "My-notes.md");
        let folder = saved.path.parent().unwrap();
        assert_eq!(folder.parent(), Some(dir.as_path()));
        assert!(is_ours(folder.file_name().unwrap().to_str().unwrap()));
        assert_eq!(std::fs::read(&saved.path).unwrap(), b"# hi");
        assert_eq!(
            (mode(&saved.path), mode(folder), mode(&dir)),
            (0o600, 0o700, 0o700)
        );
        assert_eq!(mode(&cache.join("iii")), 0o700);
        assert_eq!(
            names(folder),
            vec!["My-notes.md"],
            "no temporary file is left"
        );
        // The same name again: a folder of its own, nothing overwritten.
        let again = save(&cache, "My notes.md", None, &b64(b"# two")).unwrap();
        assert_ne!(again.path, saved.path);
        assert_eq!(again.mime, "application/octet-stream");
        assert_eq!(std::fs::read(&saved.path).unwrap(), b"# hi");
        // Folders another program opened up: ours is closed again, a shared
        // one only loses group and other write.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(cache.join("iii"), std::fs::Permissions::from_mode(0o775))
            .unwrap();
        let photo = save(&cache, "IMG_0001.HEIC", Some("image/heic"), &b64(JPEG)).unwrap();
        assert_eq!(photo.path.file_name().unwrap(), "IMG_0001.jpg");
        assert_eq!(photo.mime, "image/jpeg");
        assert_eq!((mode(&dir), mode(&cache.join("iii"))), (0o700, 0o755));
        // Nothing to decode, nothing written.
        assert_eq!(
            save(&cache, "x", None, "%%%").unwrap_err(),
            "The file is not valid base64."
        );
        assert_eq!(names(&dir).len(), 3);
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn no_folder_on_the_way_is_followed_through_a_link() {
        let cache = temp_dir();
        let elsewhere = temp_dir();
        std::fs::create_dir_all(cache.join("iii/tmux")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, cache.join("iii/tmux/uploads")).unwrap();
        let linked = "The folder for uploaded files is a symbolic link, so no file is kept there.";
        assert_eq!(save(&cache, "a.txt", None, &b64(b"a")).unwrap_err(), linked);
        assert_eq!(sweep_at_start(&cache).unwrap_err(), linked);
        // One level up too.
        std::fs::remove_file(cache.join("iii/tmux/uploads")).unwrap();
        std::fs::remove_dir(cache.join("iii/tmux")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, cache.join("iii/tmux")).unwrap();
        assert_eq!(save(&cache, "a.txt", None, &b64(b"a")).unwrap_err(), linked);
        assert!(names(&elsewhere).is_empty(), "nothing was written there");
        // A file where the folder goes.
        std::fs::remove_file(cache.join("iii/tmux")).unwrap();
        std::fs::write(cache.join("iii/tmux"), b"").unwrap();
        assert_eq!(
            save(&cache, "a.txt", None, &b64(b"a")).unwrap_err(),
            "Something that isn't a folder is where uploaded files go, so no file is kept there."
        );
        // A cache folder any user can change, unless sticky like /tmp.
        let open = temp_dir();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            save(&open, "a.txt", None, &b64(b"a")).unwrap_err(),
            "Any user can change this user's cache folder, so no uploaded file is kept there."
        );
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o1777)).unwrap();
        save(&open, "a.txt", None, &b64(b"a")).unwrap();
        for dir in [cache, elsewhere, open] {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn the_start_sweep_needs_the_folder_and_makes_none() {
        let cache = temp_dir();
        assert_eq!(sweep_at_start(&cache).unwrap(), 0);
        assert!(names(&cache).is_empty(), "no folder made at start");
        let dir = cache.join("iii/tmux/uploads");
        std::fs::create_dir_all(&dir).unwrap();
        let week = KEEP_FOR + Duration::from_secs(60);
        let old = upload(&dir, "20261001-120000-0a1b2c3d", 4, week);
        let new = upload(&dir, "20261110-120000-0a1b2c3d", 4, Duration::ZERO);
        assert_eq!(sweep_at_start(&cache).unwrap(), 1);
        assert!(!old.exists() && new.exists());
        assert_eq!(mode(&dir), 0o700, "checked like an upload's");
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn only_week_old_uploads_of_ours_are_swept() {
        let dir = temp_dir();
        let week = KEEP_FOR + Duration::from_secs(60);
        let old = upload(&dir, "20261001-120000-0a1b2c3d", 4, week);
        let new = upload(&dir, "20261110-120000-0a1b2c3d", 4, Duration::ZERO);
        let foreign = upload(&dir, "keep-me", 4, week);
        // A symlinked "upload" pointing at an old folder of someone's.
        let target = upload(&dir, "20261001-120000-00000000.target", 4, week);
        let link = dir.join("20261001-120000-11111111");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        // A write cut short leaves a folder with only a temporary file: it
        // goes after an hour.
        let stale = dir.join("20261110-110000-22222222");
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(
            stale.join(format!("{TEMP_PREFIX}{}", "a".repeat(32))),
            b"half",
        )
        .unwrap();
        age(&stale, TEMP_KEEP + Duration::from_secs(60));
        let fresh = dir.join("20261110-115900-33333333");
        std::fs::create_dir(&fresh).unwrap();
        std::fs::write(
            fresh.join(format!("{TEMP_PREFIX}{}", "b".repeat(32))),
            b"half",
        )
        .unwrap();
        assert_eq!(sweep(&dir, KEEP_FOR), 2);
        assert!(!old.exists() && !stale.exists());
        assert!(new.exists() && foreign.exists() && target.exists() && fresh.exists());
        assert!(link.symlink_metadata().is_ok(), "a symlink is left alone");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_oldest_of_ours_go_past_the_budget() {
        let dir = temp_dir();
        let at = |seconds| Duration::from_secs(seconds);
        let oldest = upload(&dir, "20261101-000000-00000001", 40, at(300));
        let older = upload(&dir, "20261102-000000-00000002", 40, at(200));
        let newest = upload(&dir, "20261103-000000-00000003", 40, at(100));
        let foreign = upload(&dir, "big-foreign", 1000, at(400));
        let keep = upload(&dir, "20261104-000000-00000004", 40, at(500));
        // 160 bytes of ours, a 100-byte budget: the two oldest but `keep` go.
        assert_eq!(trim(&dir, 100, Some(&keep)), 2);
        assert!(!oldest.exists() && !older.exists());
        assert!(newest.exists() && keep.exists() && foreign.exists());
        assert_eq!(trim(&dir, 100, Some(&keep)), 0, "under the budget now");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
