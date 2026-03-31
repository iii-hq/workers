# TODOS

## Rootfs image build pipeline
**Priority:** P1 | **Effort:** M (human) → S (CC) | **Blocked by:** Phase 0 (SIGILL test)

CI/CD workflow to build and publish per-language Debian-slim rootfs images (`iii-guest-node`, `iii-guest-python`, `iii-guest-rust`). Phase 1 prototype uses manually-built rootfs. Before shipping to users, need reproducible builds, version pinning, and hosted downloads. Dockerfile per language + GitHub Actions workflow + hosting (GitHub Releases or Container Registry). SIGILL test in Phase 0 determines if Rust rootfs is viable on Apple Silicon.

**Context:** Design doc defers this: "Rootfs image build pipeline is out of scope for this design doc." Required before Phase 1 can ship to external users.

## macOS code signing for Hypervisor entitlement
**Priority:** P1 | **Effort:** S (human) → S (CC) | **Blocked by:** Apple Developer account ($99/year)

Add code signing step to CI/CD release workflow for macOS builds. Both `iii` and `iii-vm-helper` binaries must be codesigned with `com.apple.security.hypervisor` entitlement to use Apple's Hypervisor.framework. Without this, libkrun fails for all macOS users. Ad-hoc signing works for local dev builds; proper signing required before public distribution. Also enables Gatekeeper/notarization.

**Context:** libkrun.dylib itself also needs signing. The release archive must include signed binaries.
