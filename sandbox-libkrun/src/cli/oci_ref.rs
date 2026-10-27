// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! OCI image-ref helpers for the rootfs cache: [`canonical_cache_key`] strips
//! a tagged ref's `@sha256:...` suffix before hashing so pinned and unpinned
//! variants of the same tag can share one cache dir, and [`pinned_digest`]
//! returns the pin the cache checks before sharing it.

/// Return the portion of `image_ref` that should be used as the cache-dir
/// hash input. Strips a trailing `@sha256:...` when the ref names a tag, so
/// `foo:latest` and `foo:latest@sha256:abc` resolve to the same cache dir,
/// and a pinned ref doesn't re-pull an image the cache already holds (as
/// long as that image is the pinned one: see `rootfs_cache::canonical_path`).
/// A digest-only ref says nothing about any tag, so it keeps its digest and
/// never shares `foo:latest`'s dir.
///
/// Examples:
/// - `foo:latest` -> `foo:latest`
/// - `foo:latest@sha256:abc` -> `foo:latest`
/// - `foo@sha256:abc` -> `foo@sha256:abc`
/// - `docker.io/foo/bar:tag@sha256:deadbeef...` -> `docker.io/foo/bar:tag`
pub(crate) fn canonical_cache_key(image_ref: &str) -> &str {
    match image_ref.split_once("@sha256:") {
        // A tag is a `:` in the last path segment; a registry port is not.
        Some((name, _)) if name.rsplit('/').next().is_some_and(|n| n.contains(':')) => name,
        _ => image_ref,
    }
}

/// The `sha256:...` digest `image_ref` is pinned to, if any.
pub(crate) fn pinned_digest(image_ref: &str) -> Option<&str> {
    image_ref.find("@sha256:").map(|at| &image_ref[at + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_digest_from_tagged_ref() {
        assert_eq!(
            canonical_cache_key("docker.io/foo/bar:latest@sha256:abc"),
            "docker.io/foo/bar:latest"
        );
    }

    #[test]
    fn keeps_digest_on_a_digest_only_ref() {
        // Stripping it made `foo@sha256:abc` share and fill `foo:latest`'s dir.
        assert_eq!(canonical_cache_key("foo@sha256:abc"), "foo@sha256:abc");
        assert_eq!(
            canonical_cache_key("localhost:5000/foo@sha256:abc"),
            "localhost:5000/foo@sha256:abc"
        );
        assert_eq!(
            canonical_cache_key("localhost:5000/foo:1@sha256:abc"),
            "localhost:5000/foo:1"
        );
    }

    #[test]
    fn unpinned_ref_passes_through() {
        assert_eq!(canonical_cache_key("foo:latest"), "foo:latest");
        assert_eq!(canonical_cache_key("foo"), "foo");
        assert_eq!(
            canonical_cache_key("registry.example.com/path/foo:v1"),
            "registry.example.com/path/foo:v1"
        );
    }

    #[test]
    fn pinned_and_unpinned_canonicalize_equal() {
        // This is the load-bearing invariant: two refs that describe the
        // same image must hash to the same cache dir, so the engine at
        // start time reuses the rootfs we just pulled.
        let unpinned = "docker.io/andersonofl/todo-worker:latest";
        let pinned = "docker.io/andersonofl/todo-worker:latest@sha256:aabbcc";
        assert_eq!(canonical_cache_key(unpinned), canonical_cache_key(pinned));
    }

    #[test]
    fn pinned_digest_is_the_part_after_the_at() {
        assert_eq!(
            pinned_digest("docker.io/foo/bar:latest@sha256:abc"),
            Some("sha256:abc")
        );
        assert_eq!(pinned_digest("foo@sha256:abc"), Some("sha256:abc"));
        assert_eq!(pinned_digest("foo:latest"), None);
    }

    #[test]
    fn empty_ref_passes_through() {
        // Degenerate input: empty string. Don't panic, just return it.
        assert_eq!(canonical_cache_key(""), "");
    }

    #[test]
    fn ref_with_only_at_sha256_marker_passes_through() {
        // Pathological but deterministic: no tag, so nothing is stripped.
        assert_eq!(canonical_cache_key("@sha256:abc"), "@sha256:abc");
    }
}
