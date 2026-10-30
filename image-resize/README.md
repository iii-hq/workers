# image-resize

A high-performance image resizing worker built for production workloads.

## Installation

```bash
iii call compose::add worker=image-resize
```

## Configuration

The worker reads its settings from the YAML file passed with `--config`
(default `./config.yaml`):

```yaml
height: 200
quality:
  jpeg: 85
  webp: 80
strategy: scale-to-fit
width: 200
```

### Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `width` | number | 200 | Target width in pixels |
| `height` | number | 200 | Target height in pixels |
| `strategy` | string | `scale-to-fit` | Resize strategy: `scale-to-fit`, `crop`, `fill`, `contain` |
| `quality.jpeg` | number | 85 | JPEG output quality (1-100) |
| `quality.webp` | number | 80 | WebP output quality (1-100) |

### Strategies

- **scale-to-fit** — Resize to fit within the given dimensions, preserving aspect ratio
- **crop** — Crop to exact dimensions from center
- **fill** — Fill the area, stretching if necessary
- **contain** — Contain within dimensions, adding padding if needed

## Functions

| Function | Input | Returns |
|---|---|---|
| `image_resize::resize` | `{ input_channel, output_channel, metadata? }`: the image bytes arrive on `input_channel` (a readable `StreamChannelRef`); `metadata` overrides `format`, `output_format`, `strategy`, `target_width`, `target_height` and `quality` | `{ format, width, height, strategy }`; the thumbnail is written to `output_channel` as a metadata text frame followed by the image bytes |

## Usage

```typescript
import { createChannel } from 'iii-sdk/helpers'

const input = await createChannel(iii)
const output = await createChannel(iii)
input.writer.stream.end(imageBuffer)

const result = iii.call({
  function_id: 'image_resize::resize',
  payload: {
    input_channel: input.readerRef,
    output_channel: output.writerRef,
    metadata: { format: 'jpeg', target_width: 400, target_height: 300, strategy: 'crop-to-fit' },
  },
  timeoutMs: 30_000,
})
// read the thumbnail from output.reader, then await result
```

[`example/`](example) serves `GET /health`, `POST /thumbnail` and `GET /image`
through the http worker with this call.

## Performance

Benchmarks on a standard worker instance (2 vCPU, 512MB):

| Format | Avg Latency | Throughput |
|--------|-------------|------------|
| JPEG | 12ms | ~83 ops/s |
| WebP | 18ms | ~55 ops/s |
| PNG | 24ms | ~41 ops/s |

## License

Apache 2.0 — see [LICENSE](https://github.com/iii-hq/workers/blob/main/LICENSE).
