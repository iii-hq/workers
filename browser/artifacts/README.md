# Pinned browser artifacts

`manifest.json` pins the Tier-1 Chromium builds that `security_mode: compat`
certifies against: the x86_64 Chrome-for-Testing 148 build and the aarch64
Playwright chromium build 1223. Each archive is recorded with its url, size
and SHA-256, and `scripts/fetch_chromium_artifacts.sh` fetches and verifies
them:

```sh
./scripts/fetch_chromium_artifacts.sh fetch x86_64-unknown-linux-gnu
./scripts/fetch_chromium_artifacts.sh verify x86_64-unknown-linux-gnu
```

`CERTIFIED_CHROME_VERSIONS` in `src/scrapling/raw_browser.rs` must list the
same versions; a build without the pinned artifacts returns a capability
error instead of degrading to an uncertified browser.

The curl-impersonate archives are pinned separately in
`vendor/curl_impersonate_sys/artifacts.manifest`.

This file used to also record the frozen Python oracle (the standalone
`scrapling` worker's source, its pinned interpreter and packages, and the
capture host). That worker is gone and the Python differentials with it; the
goldens under `tests/golden/` stay as frozen regression fixtures.
