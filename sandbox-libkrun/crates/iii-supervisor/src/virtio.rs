// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Guest-side lookup of the named virtio-console ports the host wires in.

use std::path::PathBuf;

/// Walk `/sys/class/virtio-ports/*/name` and return the `/dev/<dev>`
/// path for the entry whose name matches `target`. Returns `None` when
/// sysfs isn't mounted, no entries exist, or no name matches.
///
/// Virtio-console port numbering depends on controller count and whether
/// the implicit console is enabled — hardcoding `/dev/vport0p1` breaks
/// when libkrun wires the implicit console first. The sysfs name is
/// stable by construction (we pick it on the host), so lookup by name
/// is the right primitive.
pub fn find_virtio_port_by_name(target: &str) -> Option<PathBuf> {
    let sysfs = std::path::Path::new("/sys/class/virtio-ports");
    let entries = std::fs::read_dir(sysfs).ok()?;
    for entry in entries.flatten() {
        let dev_name = entry.file_name();
        let dev_name_str = dev_name.to_string_lossy();
        let name_file = entry.path().join("name");
        if let Ok(contents) = std::fs::read_to_string(&name_file)
            && contents.trim() == target
        {
            return Some(PathBuf::from("/dev").join(dev_name_str.as_ref()));
        }
    }
    None
}
