# Image Resize Demo

Demonstrates the full III module lifecycle with a Node.js HTTP API:
**install module -> start engine -> register HTTP endpoint -> process images via REST**

The Node.js worker delegates image processing to the **image-resize** Rust binary
via the WebSocket two-frame protocol — no Node.js image libraries needed.

## Prerequisites

Build the binaries from the project root:

```bash
cargo build --release -p iii-cli -p iii
cargo build --release --manifest-path ../workers/image-resize/Cargo.toml
```

Requires [Bun](https://bun.sh) runtime and pnpm.

## Quick Start

```bash
# From project root — install workspace deps
pnpm install --filter @iii-hq/image-resize-demo

# Run the demo
cd examples/image-resize-demo
./run-demo.sh
```

This will:
1. Install image-resize via the local registry
2. Start the III engine (RestApiModule on :3111, StreamModule on :3112, external image-resize)
3. Start the Node.js worker (registers HTTP endpoints)
4. Send test images to `POST /thumbnail` and display results
5. Save thumbnails to `output/`

## API Endpoints

### GET /health

Returns service status.

### POST /thumbnail

Generate a thumbnail from a base64-encoded image. The request is processed by
the image-resize Rust binary via WebSocket relay.

**Request:**
```json
{
  "image": "<base64-encoded image>",
  "width": 200,
  "height": 200,
  "strategy": "scale-to-fit",
  "format": "jpeg"
}
```

**Response:**
```json
{
  "thumbnail": "<base64-encoded thumbnail>",
  "format": "jpeg",
  "width": 200,
  "height": 160,
  "size": 1505
}
```

### GET /image

Resize a remote image by URL — similar to Next.js `/_next/image`. The image is
fetched server-side, resized via the image-resize binary, and returned as JSON.

**Query parameters:**

| Param | Required | Default | Description |
|-------|----------|---------|-------------|
| `url` | Yes | — | URL-encoded image URL (http/https only) |
| `w` | No | 200 | Target width (1-4096) |
| `h` | No | 200 | Target height (1-4096) |
| `format` | No | jpeg | Output format: jpeg, png, webp |
| `strategy` | No | scale-to-fit | Resize strategy: scale-to-fit, crop-to-fit |

**Example:**
```bash
# Save resized image to file
curl -o resized.webp "http://localhost:3111/image?url=https%3A%2F%2Fexample.com%2Fphoto.jpg&w=300&h=200&format=webp"
```

**Response:** Binary image data with appropriate `Content-Type` header (`image/jpeg`,
`image/png`, or `image/webp`). Can be used directly in `<img>` tags:

```html
<img src="http://localhost:3111/image?url=https://example.com/photo.jpg&w=300&h=200&format=webp" />
```

## Architecture

```
                         III Engine (:49134)
                        ┌──────────────────────────────┐
[HTTP Client] ──POST──> │ RestApiModule (:3111)         │
                        │       │                       │
                        │  [Node Worker]                │
                        │       │ WebSocket relay        │
                        │       ▼                       │
                        │ [image-resize binary]    │
                        │   (external module)           │
                        │       │                       │
[HTTP Client] <──JSON── │  thumbnail response           │
                        └──────────────────────────────┘
```

## Web UI

A browser-based interface is included at `web/index.html`. With the engine and worker
running, open the file directly in your browser:

```bash
open web/index.html       # macOS
xdg-open web/index.html   # Linux
```

The UI lets you drag-and-drop an image, configure thumbnail options (width, height,
strategy, format), and see a side-by-side comparison of the original vs generated
thumbnail. It calls the same `POST /thumbnail` endpoint on `http://localhost:3111`.

## Manual Testing

With the engine and worker running:

```bash
# Health check
curl http://localhost:3111/health

# Generate thumbnail from file
curl -X POST http://localhost:3111/thumbnail \
  -H "Content-Type: application/json" \
  -d "{\"image\": \"$(base64 -i images/sample.jpg)\", \"width\": 200, \"height\": 200}"

# Resize from URL (returns binary image)
curl -o resized.webp "http://localhost:3111/image?url=https%3A%2F%2Fexample.com%2Fphoto.jpg&w=300&h=200&format=webp"
```

## Cleanup

```bash
./cleanup.sh
```
