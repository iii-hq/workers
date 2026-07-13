# Worker-owned console extensions

Workers can contribute frontend behavior at runtime without being compiled
into the console and without adding fields to `iii.worker.yaml`.

## Discovery contract

An extension-owning worker registers an internal typed function whose id ends
with `::console-extension` and carries this engine metadata:

```json
{
  "internal": true,
  "capability": "iii.console-extension",
  "api_version": 1
}
```

The console reads internal function discovery, verifies this metadata through
`engine::functions::info`, then calls the capability and validates this
response:

```jsonc
{
  "id": "approval-gate",
  "api_version": 1,
  "worker_version": "1.0.7",
  "asset_function": "approval::console-extension::asset",
  "entry": {
    "path": "extension.js",
    "media_type": "text/javascript",
    "etag": "fnv1a64-..."
  },
  "styles": [
    {
      "path": "extension.css",
      "media_type": "text/css",
      "etag": "fnv1a64-..."
    }
  ],
  "slots": ["chat.composer.controls"]
}
```

The asset function accepts `{ "path": "<manifest path>" }` and returns
`{ path, media_type, encoding: "base64", content, etag }`. Workers must use an
explicit allowlist; the asset function is not a filesystem server.

## Host API

The entry module exports `activate(host)`. API v1 provides:

- `registerSlot({ id, slot, order?, mount })`
- `trigger(functionId, payload)`
- `on(functionId, handler)`
- `registerTrigger(input)`
- `browserId`
- the active extension id and worker version

`mount(element, context)` may return a cleanup function or `{ dispose() }`.
The host also dispatches `iii:console-extension-context` on the mount element
when its React-owned context changes.

Every registration made through the host is tracked. When the worker leaves,
the console removes its styles, disposes the activated module, unregisters
slots and trigger handlers, and renders slot fallbacks without reloading the
page.

## Security boundary

An installed native worker is already trusted code. Asset etags detect stale
or corrupted transport; they are not signatures. The console only imports
functions with the versioned internal capability metadata above, and rejects
unsupported manifest API versions. Extension functions must never be exposed
to an in-run model.

## Reference implementation

- Worker capability: `approval-gate/src/functions/console_extension.rs`
- Worker UI source: `approval-gate/web/src/`
- Worker UI bundle: `approval-gate/web/dist/` (generated and embedded at build time)
- Console loader and slot host: `console/web/src/extensions/`
