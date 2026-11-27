# image-resize

Downscales JPEG, PNG and WebP images to fit or fill a box. A caller streams
the image in over an iii stream and gets the thumbnail back on another one,
as JPEG, PNG or lossy WebP. The worker applies the EXIF orientation, never
enlarges an image, and returns the output's dimensions together with the
source's, so a caller can map a point on the thumbnail back to the original.

## Install

```bash
iii call compose::add worker=image-resize
```

## Quickstart

```typescript
import { createReadStream, createWriteStream } from 'node:fs'
import { InputStream, OutputStream, registerWorker } from 'iii-sdk'

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134')

// Fit a screenshot into a 1568 px box as WebP. The call resolves once the
// file holds the whole image.
const result = await iii.call({
  function_id: 'image_resize::resize',
  payload: {
    input_channel: InputStream.from(createReadStream('screenshot.png')),
    output_channel: OutputStream.to(createWriteStream('screenshot.webp')),
    metadata: { output_format: 'webp', target_width: 1568, target_height: 1568 },
  },
  timeoutMs: 30_000,
})
// → { format: 'webp', width: 1568, height: 1049, source_width: 3116, source_height: 2084, strategy: 'scale-to-fit', quality: 80 }
```

Every `metadata` key is optional:

| Key | Values | Default |
|---|---|---|
| `output_format` | `jpeg`, `png`, `webp` | the input's format |
| `target_width`, `target_height` | the box in pixels; the image is never enlarged | the configured box (200 x 200 out of the box) |
| `strategy` | `scale-to-fit` fits the image inside the box; `crop-to-fit` centre-crops it to the box's aspect ratio, then shrinks it to the box | the configured strategy (`scale-to-fit`) |
| `quality` | 1-100; ignored for `png` | the configured quality (jpeg 85, webp 80) |
| `max_pixels` | the largest output, width times height; a larger one shrinks further, keeping its aspect ratio | the configured limit (none out of the box) |
| `max_bytes` | the largest output in bytes; `jpeg` and `webp` lower their quality, then shrink; `png` shrinks | the configured limit (none out of the box) |

Any other key, in `metadata` or beside it, is rejected with `INVALID_ARGS`,
including `format`, `width` and `height` (deprecated in 0.3.0, now removed).
A limit in `metadata` replaces the configured one, larger or smaller; `null`
or leaving it out uses the configured one, so a call cannot remove a
configured limit. For `jpeg` and `webp` output the result also carries
`quality`, the quality used, which `max_bytes` can lower.

The input format is detected from the bytes. `output_channel` also carries the
result as one JSON text frame before the image bytes; byte readers and sinks
such as `OutputStream.to` never see it.

## Configuration

The worker owns one entry in the engine's `configuration` worker: id
`image-resize`, or the name compose assigns through `III_CONFIG_NAME`. It
holds the defaults for calls that leave an option out of `metadata`; per-call
`metadata` always wins. Every key is optional:

```yaml
width: 200              # default box width, px
height: 200             # default box height, px
strategy: scale-to-fit  # or crop-to-fit
quality:
  jpeg: 85              # 1-100
  webp: 80              # 1-100
max_pixels: null        # largest output, width x height; null or absent: no limit
max_bytes: null         # largest output in bytes; null or absent: no limit
```

Under compose, set `config_override` on the worker's container in
`worker-compose.yaml`:

```yaml
containers:
  image-resize:
    # ...the lines compose::add wrote stay as they are
    config_override:
      width: 1568
      height: 1568
      quality: { webp: 75 }
```

On a bare engine the built-in defaults are stored on first boot in the engine
project's `./config/<id>.yaml`; edit that file, use the Console's global
Settings modal, or call `configuration::set`. The worker ships the modal's
form for this entry: the box, strategy, quality and limits are fields, and an
empty field falls back to the built-in default, which for a limit is none.
`--config <file>` seeds the entry once, only while nothing is stored; a file
that cannot be read or is invalid is ignored with a warning.

Every key hot-reloads: a change applies to the calls that start after it.
Each call takes one snapshot, so a call in flight keeps the values it started
with, and nothing needs a restart. A value that does not match the schema (an
unknown key, a 0 side or limit, quality outside 1-100) is rejected: on a live
change the previous configuration stays in use, and at start the worker does
not start. A change moves the default output of every caller that relies on
the defaults.

## Console page

The worker ships a page in the ADE: open `Image resize` from the command
palette, pick an image, resize it, compare it with the original and download
it. The page calls `image_resize::resize` with streams, so it takes images up
to 64 MiB. It needs an ADE that relays stream sockets
(`/ws/ws/channels/{id}`); with an older ADE the streams never open, and the
page shows an error that tells you to update the ade worker.

## Limits and behaviour

- Input up to 64 MiB and 50 megapixels.
- One image per CPU core at a time; further calls queue.
- On SIGTERM or Ctrl-C the worker unregisters `image_resize::resize`, so the
  engine routes it no new call, and waits up to 8 s for the calls in flight
  to finish before it exits; it exits once no call has run for 250 ms, so
  their results go out. Calls still running at 8 s are cut off, and the exit
  does not wait for a resize they started. A second signal does not shorten
  the wait. iii compose waits 10 s by default before its SIGKILL.
- WebP output is at most 16383 px per side, JPEG 65535.
- EXIF orientation is applied; other metadata is dropped.
- An embedded RGB colour profile, such as Display P3 or Adobe RGB, is
  converted to sRGB with the relative colorimetric intent, and the output
  carries no profile. Gray, CMYK, unreadable or over-4-MiB profiles, and
  ones that list more than 64 tags, are ignored and the pixels kept as they
  are.
- WebP output is lossy; use `png` for pixel-exact output such as line art.
- JPEG output flattens transparency onto white.
- Images with transparency are resized with premultiplied alpha, so the
  colour hidden under fully transparent pixels does not bleed into the edges.
- The output is always re-encoded, even when the image already fits, so it
  can be larger than the input.
- `max_pixels` applies after the box and the image is resized once, straight
  to the final size; no side goes below 1 px.
- An output over `max_bytes` is encoded again: `jpeg` and `webp` first at
  quality 70, 55 and 40 (those below the quality asked for), then smaller,
  by the ratio of the limit to the size, until it fits. A call encodes at
  most 8 times; if the output still does not fit, the call fails with
  `INVALID_ARGS` naming `max_bytes`.
- An animated WebP is resized from its first frame; the output is a still
  image.
- A JPEG must end with its end-of-image marker: one that stops after its
  first scan begins fails with `INVALID_ARGS`, even when only the marker is
  missing. Bytes after the marker are ignored.
- A bad image, `metadata` or request key fails with code `INVALID_ARGS` and
  aborts `output_channel`. A missing or malformed `input_channel` or
  `output_channel` ref fails before the handler runs, as `invocation_failed`.

## Migration notes

- Options sent as a text frame on `input_channel` are rejected; pass them in
  `metadata`.
- `format`, `width` and `height` in `metadata` were deprecated in 0.3.0 and
  are now removed: like any unknown `metadata` key they are rejected with
  `INVALID_ARGS` naming the key. Drop them: the input format is detected from
  the bytes and the source size is in the result.
- `output_format: 'jpg'` is rejected; use `jpeg`.
- Images are never enlarged, so crop-to-fit can return less than the box.
- WebP output is lossy and honours `quality`.
- A bad image or bad `metadata` returns `INVALID_ARGS` instead of
  `invocation_failed` and aborts the output.
- The result gains `source_width` and `source_height`, and for `jpeg` and
  `webp` output `quality`.
- PNG output is no longer forced to 8-bit RGBA: gray stays gray, 16-bit stays
  16-bit, and alpha is written only when the source has it (a tRNS chunk
  counts). Palette and 1/2/4-bit PNGs come out as 8-bit gray, RGB or RGBA.
- `quality` outside 1-100 is an error.
- A truncated JPEG, one that ends before its end-of-image marker, is rejected
  with `INVALID_ARGS`; it used to decode with the missing rows grey.
- Images with an embedded RGB colour profile are converted to sRGB. The
  profile used to be dropped and the values kept, so Display P3 or Adobe RGB
  photos came out with wrong colours.
- A stop lets the calls in flight finish, for up to 8 s, instead of cutting
  them off at once.
- WebP input is decoded by libwebp instead of image-webp. A frame whose size
  disagrees with its canvas fails with `INVALID_ARGS` before any pixels are
  decoded, an animation frame whose header disagrees with its bitstream is
  sized by the bitstream instead of failing the call, and lossy pixels can
  differ slightly.
- Configuration moved to the `configuration` worker: `./config.yaml` is no
  longer read and no longer shipped, `--config` only seeds an empty entry,
  compose's `config_override` now takes effect, and the worker needs the
  engine's configuration worker to start.

## License

Apache 2.0 — see [LICENSE](https://github.com/iii-hq/workers/blob/main/LICENSE).
Release binaries statically link libwebp 1.6.0
([BSD-3-Clause](https://chromium.googlesource.com/webm/libwebp/+/refs/tags/v1.6.0/COPYING),
with Google's
[additional patent grant](https://chromium.googlesource.com/webm/libwebp/+/refs/tags/v1.6.0/PATENTS)).
