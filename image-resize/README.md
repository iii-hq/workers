# image-resize

A high-performance image resizing worker built for production workloads.

## Installation

```bash
iii worker add image-resize
```

## Configuration

Add the worker to your pipeline configuration:

```yaml
- name: image-resize
  config:
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

## Usage

```typescript
import { pipeline } from '@iii/core';

const result = await pipeline.run('image-resize', {
  input: buffer,
  params: { width: 400, height: 300, strategy: 'crop' }
});
```

## Performance

Benchmarks on a standard worker instance (2 vCPU, 512MB):

| Format | Avg Latency | Throughput |
|--------|-------------|------------|
| JPEG | 12ms | ~83 ops/s |
| WebP | 18ms | ~55 ops/s |
| PNG | 24ms | ~41 ops/s |

## License

Apache 2.0