// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! `adapter::info` advertises `worker.build` only where a build can boot: the
//! overlay rootfs mode, which needs an embedded iii-init. Its own binary, so
//! setting `III_ROOTFS_MODE` races no other test.

use sandbox_libkrun::sandbox_daemon::adapter_info;
use sandbox_libkrun::wire::FEATURE_BUILD;

fn advertises_build() -> bool {
    adapter_info().features.iter().any(|f| f == FEATURE_BUILD)
}

#[test]
fn worker_build_is_advertised_in_the_overlay_mode_only() {
    // SAFETY: the only test in this binary.
    unsafe { std::env::remove_var("III_ROOTFS_MODE") };
    assert_eq!(
        advertises_build(),
        iii_filesystem::init::has_init(),
        "overlay is the default whenever iii-init is embedded"
    );
    for legacy in ["legacy", "off", "0"] {
        unsafe { std::env::set_var("III_ROOTFS_MODE", legacy) };
        assert!(!advertises_build(), "III_ROOTFS_MODE={legacy}");
    }
    unsafe { std::env::remove_var("III_ROOTFS_MODE") };
}
