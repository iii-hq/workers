// Shared by build.rs and tests/firmware_check.rs through include!, so it stays
// self-contained (sha2 is the only dependency).

/// Validates an embedded firmware file: the magic must match its extension
/// (so a Git LFS pointer or an HTML error page fails), and its sha256 must
/// equal its `SHA256SUMS` line (`<hex>  <file name>`, as `shasum -a 256` prints).
pub fn check_firmware(file_name: &str, bytes: &[u8], sha256sums: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    if file_name.ends_with(".so") {
        if !bytes.starts_with(b"\x7fELF") {
            return Err(format!("{file_name}: not an ELF shared object"));
        }
    } else if file_name.ends_with(".dylib") {
        if !bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]) {
            return Err(format!("{file_name}: not a 64-bit Mach-O dylib"));
        }
    } else {
        return Err(format!("{file_name}: unsupported firmware extension"));
    }

    let expected = sha256sums
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim().trim_start_matches('*') == file_name)
        .map(|(sum, _)| sum)
        .ok_or_else(|| format!("{file_name}: no entry in SHA256SUMS"))?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "{file_name}: sha256 {actual} does not match SHA256SUMS ({expected})"
        ));
    }
    Ok(())
}
