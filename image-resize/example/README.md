# Image Resize Demo

A Node.js HTTP API over the **image-resize** worker: three routes served by the
**http** worker, each resizing through `image_resize::resize` over SDK
channels, with no Node.js image libraries.

## Prerequisites

- the `iii` CLI (0.26 or later)
- Rust (the http and image-resize workers build from this repository)
- Node.js 20.6 or later (for `node --import`) and npm

## Quick Start

```bash
cd image-resize/example
iii compose --up
```

[`worker-compose.yaml`](worker-compose.yaml) starts:

1. the iii engine on `ws://127.0.0.1:49134`
2. the http worker (`../../http`), serving HTTP on `127.0.0.1:3111`
3. the image-resize worker (`..`); its defaults come from its configuration
   entry (see [Configuration](../README.md#configuration)): add a
   `config_override:` to its container in `worker-compose.yaml` to change them
4. this Node.js worker (`npm install`, then `npm run dev`), which registers the
   routes

Stop everything with Ctrl+C.

The stack binds `127.0.0.1`, but the http worker allows every CORS origin (the
web UI is opened from `file://`), so while the stack runs any web page open in
your browser can call `GET /image` and read what it fetches, including
localhost and LAN addresses. Stop the stack when you are done and never expose
it.

## API Endpoints

Each endpoint is a function (`api::get::/health`, `api::post::/thumbnail`,
`api::get::/image`) that observes `http::on-request` with its route as args,
so the http worker routes matching requests to it:

```typescript
iii.observe({ function_id: 'http::on-request', args: { method: 'GET', path: '/image' } }, 'api::get::/image')
```

### GET /health

Returns service status.

### POST /thumbnail

Generate a thumbnail from a base64-encoded image. The image-resize worker
detects the input format (JPEG, PNG or WebP) from the bytes and resizes it over
SDK channels. `width` and `height` (default 200) are the box, `strategy` is
`scale-to-fit` (default) or `crop-to-fit`, and `outputFormat` is `jpeg`
(default), `png` or `webp`.

**Request:**
```json
{
  "image": "<base64-encoded image>",
  "width": 50,
  "height": 50,
  "strategy": "scale-to-fit",
  "outputFormat": "jpeg"
}
```

**Response** (for `images/sample.jpg`, 100x80):
```json
{
  "thumbnail": "<base64-encoded thumbnail>",
  "format": "jpeg",
  "width": 50,
  "height": 40,
  "size": 691
}
```

The image is never enlarged. With `scale-to-fit`, an image that already fits
the box comes back at its own size. With `crop-to-fit`, it comes back as the
largest centred crop with the box's aspect ratio, unscaled (the 100x80 sample
in a 200x200 box gives 80x80). A bad image or option returns 400.

### GET /image

Resize a remote image by URL — similar to Next.js `/_next/image`. The image is
fetched server-side, resized by the image-resize worker, and returned as binary
image data. The input format is detected from the bytes.

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
                     iii engine (ws://127.0.0.1:49134)
                    ┌───────────────────────────────────────────┐
[HTTP client] ────> │ http worker (:3111)                       │
                    │   │ http::on-request observer of the route │
                    │   ▼                                        │
                    │ image-resize-demo (Node.js)                │
                    │   │ image_resize::resize over channels     │
                    │   ▼                                        │
                    │ image-resize worker                        │
                    └───────────────────────────────────────────┘
[HTTP client] <──── thumbnail response
```

## Web UI

A browser-based interface is included at `web/index.html`. With the stack
running, open the file directly in your browser:

```bash
open web/index.html       # macOS
xdg-open web/index.html   # Linux
```

The UI lets you drag-and-drop an image, configure thumbnail options (width, height,
strategy, format), and see a side-by-side comparison of the original vs generated
thumbnail. It calls the same `POST /thumbnail` endpoint on `http://localhost:3111`.

## Manual Testing

With the stack running, from this directory (`image-resize/example`):

```bash
# Health check
curl http://localhost:3111/health

# Generate thumbnail from file
curl -X POST http://localhost:3111/thumbnail \
  -H "Content-Type: application/json" \
  -d "{\"image\": \"$(base64 < images/sample.jpg | tr -d '\n')\", \"width\": 50, \"height\": 50}"

# Resize from URL (returns binary image)
curl -o resized.webp "http://localhost:3111/image?url=https%3A%2F%2Fexample.com%2Fphoto.jpg&w=300&h=200&format=webp"
```
