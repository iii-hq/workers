# ios-simulator

iOS Simulators on the iii bus. Create and boot simulators on a Mac, watch
them live in the console as an iPhone you can touch — tap, swipe, pinch,
hardware buttons, typing — and save screenshots and recordings. Agents drive
the same simulators through `ios-simulator::*` functions. One Mac can serve
several consumers: each tenant gets its own private set of simulators and its
own media, and never sees another tenant's.

Simulators run headless (no Simulator.app window). The live view and input
use the same CoreSimulator/SimulatorKit frameworks Simulator.app uses, through
a small helper the worker compiles with the host's Xcode on first use.

## Install

```bash
iii call compose::add worker=ios-simulator
```

Requires macOS with Xcode and at least one iOS runtime installed
(Xcode › Settings › Components). Open the **Simulators** page in the console.

## Quickstart

```bash
# What can I create?
iii call ios-simulator::runtimes

# Create and boot an iPhone (tenant is optional; omitted means "default")
iii call ios-simulator::devices::create --json \
  '{"name":"Checkout","device_type":"com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro"}'
# -> { "tenant": "default", "device": { "udid": "5FEF…", "state": "Shutdown", … } }
iii call ios-simulator::devices::boot udid=5FEF…
iii call ios-simulator::open udid=5FEF…      # or boot and show it live over the console in one call

# Look, then act in the screenshot's pixel space
iii call ios-simulator::screenshot udid=5FEF…
iii call ios-simulator::gesture --json '{"udid":"5FEF…","kind":"tap","x":600,"y":1400}'
iii call ios-simulator::gesture --json \
  '{"udid":"5FEF…","kind":"swipe","x":603,"y":2615,"to_x":603,"to_y":1300}'   # swipe up from the bottom edge: home
iii call ios-simulator::type --json '{"udid":"5FEF…","text":"hello"}'
iii call ios-simulator::button udid=5FEF… button=lock

# Record
iii call ios-simulator::recording::start udid=5FEF…
iii call ios-simulator::recording::stop udid=5FEF…
# -> { "ok": true, "media": { "name": "5FEF…/recording-1790535293088.mov", "bytes": 58084, … }, "duration_ms": 6492 }
```

Gestures are `tap`, `double_tap`, `long_press`, `swipe` and `pinch`
(`from_distance`/`to_distance` in pixels); buttons are `home`, `lock` (the side
button), `action`, `volume_up` and `volume_down`. Screenshots are PNG,
recordings H.264 QuickTime movies; list them with `media::list` and read them
with `media::read` (8 MiB per call, page with `offset`). A screenshot's result
carries a JPEG preview of at most 96 KB base64 (the harness caps a result at
256 KB and counts the image twice) and says how to scale from it to touch
coordinates.

In the console page: drag to touch, **Option-drag** to pinch (a second finger
mirrored around the center, like Simulator.app), **Option-Shift-drag** for two
fingers together, trackpad pinch and scroll work as themselves, two fingers on
a touchscreen are two fingers, the side buttons on the bezel are pressable,
and while the screen is focused your keyboard types into the simulator
(Shift+Esc leaves).

When an agent boots a simulator, opens it (`ios-simulator::open`) or drives it
(gesture, button, typing, apps, URLs, screenshot, recording), and when Xcode or
a terminal boots one, a live preview of it floats in a corner of the console,
like the browser worker's: drag it, pinch or double-click to resize, tap to fan
several out, expand to open the Simulators page. Hiding a card keeps it hidden
while the agent keeps working, until two quiet minutes pass or
`ios-simulator::open` asks for it again. Nothing pops for a simulator the
Simulators page already shows in that window, or for a boot started from the
page.

## Configuration

Stored in the `configuration` worker under `ios-simulator`; edit it from the
page's settings action. Every field applies to the next operation.

| Field | Default | Meaning |
|---|---|---|
| `data_dir` | `data/ios-simulator` | Tenant device sets and media: `tenants/<tenant>/{devices,media}` |
| `developer_dir` | `""` | Xcode to use; empty means `xcode-select -p` |
| `share_system_devices` | `true` | The `default` tenant manages this Mac's own Xcode simulators |
| `require_tenant` | `false` | Refuse calls without `tenant` |
| `max_booted` | `3` | Booted simulators at once, whole Mac |
| `max_booted_per_tenant` | `2` | Booted simulators at once, per tenant |
| `max_devices_per_tenant` | `10` | Simulators a tenant may own |
| `stream_fps` / `stream_max_dimension` / `stream_quality` | `30` / `1000` / `70` | Live view: frame rate cap, longest edge, JPEG quality |
| `max_recording_seconds` | `600` | Recordings stop by themselves after this |

## Multiple consumers on one Mac

One worker can serve several clients or projects without them seeing or
disturbing each other:

- **Isolation.** A tenant's simulators live in its own CoreSimulator device
  set, so every simctl and CoreSimulator call runs *inside* that set: another
  tenant's UDID is simply "unknown device". Media is confined to the tenant's
  folder, and media names are validated so they cannot escape it.
- **Concurrency.** Lifecycle calls on one simulator (boot, shutdown, erase,
  delete) are serialized per simulator; boots and creates are admitted under
  one lock so the Mac-wide and per-tenant caps hold under concurrent calls;
  each simulator's input flows through one ordered channel. Tenants never
  share a simulator.
- **Identity is not the worker's call.** The tenant arrives in the payload.
  On a shared deployment, put [`rbac-proxy`](https://github.com/iii-hq/workers/tree/main/rbac-proxy)
  in front of the engine: your auth function returns the tenant in the session
  `context`, and your middleware function overwrites `payload.tenant` with
  `context.tenant` before invoking any `ios-simulator::*` call, so a client
  cannot choose someone else's. Set
  `require_tenant: true` and `share_system_devices: false` so nothing falls
  through to the Mac's own simulators. Keep `ios-simulator::tenants::list`
  operator-only and have the observer-registration hook
  (`on_observer_registration_function_id`) stamp the tenant on observers of
  the `ios-simulator::on-*` functions: `args.tenant` on
  `ios-simulator::on-frame-event`, an `input.tenant` `$eq` filter on the
  other three.

## Observable functions

Each event is a call of a private function whose handler returns the event:
only this worker calls them, anyone may observe them. Calls are
fire-and-forget, and nothing is built for a function nobody observes.

| Function | Args | Delivery | Event |
|---|---|---|---|
| `ios-simulator::on-device-changed` | none; filter on `input.tenant`, `input.udid` | iii's default | `{ tenant, udid, name, state, previous_state, change, preview, timestamp }` whenever a simulator is added, removed, booted or shut down, whoever did it (this worker, Xcode, a terminal). `preview: false` marks a boot started from a surface that already shows the simulator. |
| `ios-simulator::on-media-changed` | none; filter on `input.tenant`, `input.udid` | iii's default | `{ tenant, udid, name, kind, change, timestamp }` when a screenshot or recording is saved or deleted. |
| `ios-simulator::on-frame-event` | `{ tenant?, udid? }` | by key: an observer gets the frames of the tenant and simulator it names (every tenant or simulator for one it omits); a frame is built only when some observer wants it | `{ tenant, udid, data, width, height, device_width, device_height, seq }`, one JPEG frame of the live view, sent while someone renews `ios-simulator::watch` (every few seconds; the lease lasts 15s). |
| `ios-simulator::on-activity` | none; filter on `input.tenant`, `input.udid` | iii's default | `{ tenant, udid, function, timestamp }` after a caller drove a simulator (gesture, button, type, key, apps, open-url, screenshot, recording::start, open); live previews surface it. |

```ts
// every device change of one tenant
iii.observe(
  { function_id: 'ios-simulator::on-device-changed', filters: { 'input.tenant': { $eq: 'acme' } }, deliver: 'output' },
  'my-worker::on-simulator',
)

// the live view of one simulator; one frame in flight, so a slow viewer drops frames
iii.observe(
  { function_id: 'ios-simulator::on-frame-event', args: { tenant: 'acme', udid: '5FEF…' }, deliver: 'output' },
  'my-worker::on-frame',
  { max_in_flight: 1 },
)
```

`ios-simulator::on-frame-event` rejects unknown args (`INVALID_ARGS`).
