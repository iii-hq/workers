# Motia Workers

Worker modules for the [III engine](https://github.com/motia).

## Modules

### image-resize

A Rust-based image resize worker that connects to the III engine via WebSocket and processes images through stream channels.

**Supported formats:** JPEG, PNG, WebP (including cross-format conversion)

**Resize strategies:**

| Strategy | Behavior |
|---|---|
| `scale-to-fit` | Scales the image to fit within the target dimensions, preserving aspect ratio (default) |
| `crop-to-fit` | Scales and center-crops to fill the exact target dimensions |

**Features:**
- EXIF orientation auto-correction
- Configurable quality per format (JPEG, WebP)
- Per-request parameter overrides (dimensions, quality, strategy, output format)
- Module manifest output (`--manifest`)

#### Prerequisites

- Rust 1.70+
- A running III engine instance

#### Build

```bash
cd image-resize
cargo build --release
```

#### Usage

```bash
# Run with defaults (connects to ws://127.0.0.1:49134)
./target/release/image-resize

# Custom config and engine URL
./target/release/image-resize --config ./config.yaml --url ws://host:port

# Print module manifest
./target/release/image-resize --manifest
```

#### Configuration

Create a `config.yaml` file:

```yaml
width: 200        # default target width
height: 200       # default target height
strategy: scale-to-fit  # or crop-to-fit
quality:
  jpeg: 85
  webp: 80
```

All fields are optional and fall back to the defaults shown above.

#### Tests

```bash
cd image-resize
cargo test
```
