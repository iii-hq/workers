use crate::error::InitError;

/// Default RLIMIT_NOFILE value (soft and hard).
pub const DEFAULT_NOFILE: u64 = 65536;

/// Raise RLIMIT_NOFILE to the value specified in `III_INIT_NOFILE` env var,
/// else to the host's `--nofile-limit` (from `KRUN_RLIMITS`), else to
/// `DEFAULT_NOFILE` (65536).
pub fn raise_nofile() -> Result<(), InitError> {
    raise_nofile_from(
        std::env::var("III_INIT_NOFILE").ok(),
        std::env::var("KRUN_RLIMITS").ok(),
    )
}

fn raise_nofile_from(
    init_nofile: Option<String>,
    krun_rlimits: Option<String>,
) -> Result<(), InitError> {
    if let Some(val) = init_nofile {
        let limit = val.parse::<u64>().map_err(|e| InitError::ParseNofile {
            value: val,
            source: e,
        })?;
        return set_nofile(limit, limit);
    }
    if let Some((soft, hard)) = krun_rlimits.as_deref().and_then(krun_nofile) {
        // A host value the guest refuses (e.g. above `fs.nr_open`) must not
        // stop the boot.
        match set_nofile(soft, hard) {
            Ok(()) => return Ok(()),
            Err(e) => eprintln!(
                "iii-init: warning: host nofile limit {soft}:{hard} not applied ({e}); \
                 using {DEFAULT_NOFILE}"
            ),
        }
    }
    set_nofile(DEFAULT_NOFILE, DEFAULT_NOFILE)
}

fn set_nofile(soft: u64, hard: u64) -> Result<(), InitError> {
    let rlim = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) };
    if ret != 0 {
        return Err(InitError::Rlimit(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// The NOFILE `(soft, hard)` in libkrun's `KRUN_RLIMITS`
/// (`RLIMIT_NOFILE:soft:hard,...`). Only libkrun's stock init applies that
/// variable, and iii-init runs in its place.
fn krun_nofile(rlimits: &str) -> Option<(u64, u64)> {
    rlimits.split(',').find_map(|entry| {
        let mut parts = entry.split(':');
        if parts.next()?.trim_start_matches("RLIMIT_") != "NOFILE" {
            return None;
        }
        Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn krun_rlimits_nofile_is_parsed() {
        assert_eq!(krun_nofile("RLIMIT_NOFILE:1024:4096"), Some((1024, 4096)));
        assert_eq!(krun_nofile("RLIMIT_NPROC:1:2,NOFILE:3:4"), Some((3, 4)));
        assert_eq!(krun_nofile("RLIMIT_NPROC:1:2"), None);
        assert_eq!(krun_nofile("RLIMIT_NOFILE:x:4096"), None);
    }

    /// A host limit the guest refuses (soft above hard here) falls back to
    /// the default instead of failing the boot.
    #[test]
    fn a_refused_host_nofile_limit_falls_back_to_the_default() {
        let res = raise_nofile_from(None, Some("RLIMIT_NOFILE:2:1".into()));
        assert!(
            !matches!(res, Err(InitError::Rlimit(ref e)) if e.raw_os_error() == Some(libc::EINVAL)),
            "{res:?}"
        );
    }

    #[test]
    fn test_default_nofile_value() {
        assert_eq!(DEFAULT_NOFILE, 65536);
    }

    #[test]
    fn test_raise_nofile_succeeds_with_default() {
        // This should succeed on any system -- 65536 is a reasonable limit.
        // If the test environment restricts setrlimit, this will still pass
        // because we test with the default value which is typically allowed.
        let result = raise_nofile();
        // On most systems this succeeds; on restrictive systems it may fail
        // with EPERM. Either outcome is acceptable for a unit test.
        match result {
            Ok(()) => {} // expected on most systems
            Err(InitError::Rlimit(ref e)) if e.raw_os_error() == Some(libc::EPERM) => {} // restricted env
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn test_parse_nofile_invalid_value() {
        // Directly test the parsing logic that raise_nofile uses internally.
        let result = "not_a_number".parse::<u64>();
        assert!(result.is_err());

        // Verify our error type captures the parse failure correctly.
        let parse_err = result.unwrap_err();
        let init_err = InitError::ParseNofile {
            value: "not_a_number".to_string(),
            source: parse_err,
        };
        let msg = format!("{init_err}");
        assert!(msg.contains("not_a_number"));
        assert!(msg.contains("III_INIT_NOFILE"));
    }

    #[test]
    fn test_parse_nofile_valid_value() {
        let result = "32768".parse::<u64>();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 32768);
    }
}
