"""Image resize demo worker — Python port of the Node.js example.

Connects to the iii engine, registers a health check and a thumbnail
endpoint that delegates to the ``image_resize::resize`` function via
streaming channels.
"""

from __future__ import annotations

import asyncio
import base64
import json
import logging
import os
import signal
import sys
from typing import Any

from iii import InitOptions, register_worker

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(name)s %(levelname)s %(message)s")
log = logging.getLogger("image-resize-demo-python")

# ── Connect to the iii engine ─────────────────────────────

engine_url = os.environ.get("III_URL", "ws://localhost:49134")

iii = register_worker(
    engine_url,
    InitOptions(
        otel={"enabled": True, "serviceName": "image-resize-demo-python"},
    ),
)


# ── Helpers ───────────────────────────────────────────────

def detect_format(data: bytes) -> str:
    """Detect image format from magic bytes."""
    if len(data) >= 2 and data[0] == 0xFF and data[1] == 0xD8:
        return "jpeg"
    if len(data) >= 4 and data[0:4] == b"\x89PNG":
        return "png"
    if len(data) >= 12 and data[8:12] == b"WEBP":
        return "webp"
    return "jpeg"  # fallback


# Serialize access to the resizer to avoid concurrent invocation issues.
# Mirrors the Node.js example's serialization pattern.
_lock = asyncio.Lock()


async def process_image(
    image_bytes: bytes,
    *,
    fmt: str,
    output_format: str,
    width: int,
    height: int,
    strategy: str,
) -> dict[str, Any]:
    """Send an image to ``image_resize::resize`` via channels and return the thumbnail.

    1. Create two channels (input for image data, output for thumbnail data)
    2. Trigger the resize function with channel refs + metadata
    3. Write image bytes to input channel
    4. Read thumbnail metadata (text frame) + bytes (binary) from output channel
    5. Return result
    """
    # Create channels
    input_channel = iii.create_channel()
    output_channel = iii.create_channel()

    # Write image bytes to the input channel, then close it.
    await input_channel.writer.write(image_bytes)
    await input_channel.writer.close_async()

    # Trigger the resize function
    trigger_result = iii.trigger({
        "function_id": "image_resize::resize",
        "payload": {
            "input_channel": input_channel.reader_ref.model_dump(),
            "output_channel": output_channel.writer_ref.model_dump(),
            "metadata": {
                "format": fmt,
                "output_format": output_format,
                "width": 0,
                "height": 0,
                "target_width": width,
                "target_height": height,
                "strategy": strategy,
            },
        },
        "timeout_ms": 30_000,
    })

    # Read metadata (text message) and thumbnail bytes from the output channel
    metadata: dict[str, Any] = {}

    def on_metadata(msg: str) -> None:
        nonlocal metadata
        metadata = json.loads(msg)

    output_channel.reader.on_message(on_metadata)
    thumbnail_bytes = await output_channel.reader.read_all()

    # Explicitly close all channel endpoints to free engine resources
    await output_channel.reader.close_async()
    input_channel.writer.close()

    return {"thumbnail": thumbnail_bytes, "metadata": metadata}


# ── Health check ──────────────────────────────────────────

async def health_handler(data: Any) -> dict[str, Any]:
    """Return a simple health status."""
    return {
        "status_code": 200,
        "body": {"status": "ok", "service": "image-resize-demo-python"},
        "headers": {"Content-Type": "application/json"},
    }


iii.register_function(
    {"id": "api::get::/health", "description": "Health check"},
    health_handler,
)

iii.register_trigger({
    "type": "http",
    "function_id": "api::get::/health",
    "config": {
        "api_path": "/health",
        "http_method": "GET",
        "description": "Health check",
    },
})


# ── Thumbnail endpoint ────────────────────────────────────

async def thumbnail_handler(data: Any) -> dict[str, Any]:
    """Generate a thumbnail from a base64-encoded image via the image-resize module."""
    body = data if isinstance(data, dict) else {}
    image_b64: str | None = body.get("image")
    width: int = body.get("width", 200)
    height: int = body.get("height", 200)
    strategy: str = body.get("strategy", "scale-to-fit")
    explicit_format: str | None = body.get("format")
    output_format: str = body.get("outputFormat", "jpeg")

    if not image_b64:
        return {
            "status_code": 400,
            "body": {"error": 'Missing "image" field (base64-encoded image data)'},
            "headers": {"Content-Type": "application/json"},
        }

    image_bytes = base64.b64decode(image_b64)
    input_format = explicit_format or detect_format(image_bytes)

    log.info(
        "Processing thumbnail request: format=%s output=%s %dx%d strategy=%s",
        input_format, output_format, width, height, strategy,
    )

    try:
        async with _lock:
            result = await process_image(
                image_bytes,
                fmt=input_format,
                output_format=output_format,
                width=width,
                height=height,
                strategy=strategy,
            )

        thumbnail = result["thumbnail"]
        metadata = result["metadata"]

        log.info(
            "Thumbnail generated: format=%s %sx%s size=%d",
            metadata.get("format"), metadata.get("width"), metadata.get("height"), len(thumbnail),
        )

        return {
            "status_code": 200,
            "body": {
                "thumbnail": base64.b64encode(thumbnail).decode("ascii"),
                "format": metadata.get("format"),
                "width": metadata.get("width"),
                "height": metadata.get("height"),
                "size": len(thumbnail),
            },
            "headers": {"Content-Type": "application/json"},
        }
    except Exception as exc:
        log.error("Thumbnail generation failed: %s", exc)
        return {
            "status_code": 500,
            "body": {"error": f"Thumbnail generation failed: {exc}"},
            "headers": {"Content-Type": "application/json"},
        }


iii.register_function(
    {
        "id": "api::post::/thumbnail",
        "description": "Generate a thumbnail from a base64-encoded image via the image-resize module",
        "metadata": {"tags": ["image", "thumbnail"]},
    },
    thumbnail_handler,
)

iii.register_trigger({
    "type": "http",
    "function_id": "api::post::/thumbnail",
    "config": {
        "api_path": "/thumbnail",
        "http_method": "POST",
        "description": "Generate a thumbnail from a base64-encoded image via the image-resize module",
        "metadata": {"tags": ["image", "thumbnail"]},
    },
})


# ── Keep the process alive ────────────────────────────────

print("Image resize demo worker (Python) started — registering endpoints...")

if sys.platform != "win32":
    signal.pause()
else:
    # signal.pause() is not available on Windows
    try:
        while True:
            asyncio.get_event_loop().run_until_complete(asyncio.sleep(3600))
    except KeyboardInterrupt:
        pass
