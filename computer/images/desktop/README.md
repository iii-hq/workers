# iii-desktop image

A minimal Linux desktop for sandboxed computer-use sessions. The `computer`
worker boots this inside an [`iii-sandbox`](https://workers.iii.dev/workers/iii-sandbox)
microVM and drives it entirely through iii primitives (`sandbox::exec` +
`sandbox::fs`), with no executor and no socket into the guest.

## Why a sandboxed desktop

Driving a real host desktop means fighting multi-monitor layout, HiDPI/Retina
point-vs-pixel scaling, and OS permission prompts. A sandbox sidesteps all of
it: one fixed-resolution virtual display means coordinates are 1:1 with the
screenshot, and there is nothing to grant. It is also reproducible and needs no
physical display — though it still needs a host that can run iii-sandbox
(macOS Apple Silicon via libkrun, or Linux with `/dev/kvm`).

## How it works

iii-sandbox boots the image filesystem as a libkrun microVM rootfs but does
**not** run its ENTRYPOINT/CMD (PID 1 is the engine supervisor). So the worker
starts the display itself on `sessions::start`:

1. `sandbox::create { image, network, idle_timeout_secs }` (network follows the
   worker's `sandbox_network` config, on by default)
2. `sandbox::exec` runs `setsid Xvfb :0 -screen 0 <W>x<H>x24 &` + `openbox`,
   then waits for `xdpyinfo` (detached with `setsid` so it survives between
   exec calls).
3. Per action: `import -window root jpg:- | base64` for a screenshot, `xdotool`
   for pointer/keyboard, all under `DISPLAY=:0`.

## Build and register

```bash
docker build -t ghcr.io/<you>/iii-desktop:latest computer/images/desktop
docker push  ghcr.io/<you>/iii-desktop:latest
```

Register it with the sandbox worker in the engine `config.yaml`:

```yaml
- name: iii-sandbox
  config:
    auto_install: true
    image_allowlist:
      - desktop
    custom_images:
      desktop: ghcr.io/<you>/iii-desktop:latest
```

Then start a session:

```json
{ "trigger": "computer::sessions::start", "payload": { "image": "desktop" } }
```

Or set `sandbox_image: desktop` in the `computer` worker config so a bare
`sessions::start` uses it by default. Resolution comes from the worker config
(`sandbox_width` / `sandbox_height`, default 1280x800).

## Extending

Add your own apps to the image (a browser, an editor, whatever the task needs).
Nothing else changes: the worker launches and drives them via `xdotool`.
