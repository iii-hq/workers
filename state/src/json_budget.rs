//! Count encoded JSON bytes without allocating the encoded representation.
use serde::Serialize;
use std::io::{self, Write};

pub(crate) fn size_within(
    value: &impl Serialize,
    limit: usize,
) -> Result<Option<usize>, serde_json::Error> {
    struct Counter {
        used: usize,
        limit: usize,
        exceeded: bool,
    }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.used) {
                self.exceeded = true;
                return Err(io::Error::other("JSON byte budget exceeded"));
            }
            self.used += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter {
        used: 0,
        limit,
        exceeded: false,
    };
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => Ok(Some(counter.used)),
        Err(_) if counter.exceeded => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_encoded_size_with_unicode_and_escapes() {
        let value =
            serde_json::json!({"s":"中文\n\\\"","n":18446744073709551615u64,"v":[true,null,1.25]});
        let len = serde_json::to_vec(&value).unwrap().len();
        assert_eq!(size_within(&value, len).unwrap(), Some(len));
        assert_eq!(size_within(&value, len - 1).unwrap(), None);
    }
    #[test]
    fn serialization_errors_are_not_reported_as_empty_values() {
        struct Invalid;
        impl Serialize for Invalid {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("invalid test value"))
            }
        }
        assert!(size_within(&Invalid, 1024).is_err());
    }
}
