"""Image resize demo worker — Python edition.

Mirrors the TypeScript example: registers HTTP endpoints that send images
to the image_resize::resize Rust worker via SDK channels and return thumbnails.
"""

import asyncio
import base64
import json
import logging
import os
from typing import Any

from iii import ApiRequest, ApiResponse, Logger, register_worker

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("image-resize-demo-python")

engine_url = os.environ.get("III_URL", "ws://localhost:49134")
iii = register_worker(engine_url, {"otel": {"enabled": True, "service_name": "image-resize-demo-python"}})

# Serialize access to the resizer (same as the TS example)
_lock = asyncio.Lock()


def detect_format(data: bytes) -> str:
    """Detect image format from magic bytes."""
    if data[:2] == b"\xff\xd8":
        return "jpeg"
    if data[:4] == b"\x89PNG":
        return "png"
    if len(data) > 11 and data[8:12] == b"WEBP":
        return "webp"
    return "jpeg"


async def process_image(
    image_bytes: bytes,
    *,
    fmt: str,
    output_format: str,
    width: int,
    height: int,
    strategy: str,
) -> tuple[bytes, dict[str, Any]]:
    """Send image to image_resize::resize via channels, return thumbnail bytes + metadata."""
    async with _lock:
        channel_input = iii.create_channel()
        channel_output = iii.create_channel()

        # Write image to input channel
        await channel_input.writer.stream.write(image_bytes)
        channel_input.writer.close()

        # Trigger the resize function
        trigger_task = asyncio.ensure_future(
            iii.trigger({
                "function_id": "image_resize::resize",
                "payload": {
                    "input_channel": channel_input.reader_ref,
                    "output_channel": channel_output.writer_ref,
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
                "timeoutMs": 30000,
            })
        )

        # Read metadata (text message) and thumbnail (binary) from output channel
        metadata_future: asyncio.Future[dict] = asyncio.get_event_loop().create_future()

        def on_message(msg: str) -> None:
            if not metadata_future.done():
                metadata_future.set_result(json.loads(msg))

        channel_output.reader.on_message(on_message)

        chunks: list[bytes] = []
        async for chunk in channel_output.reader.stream:
            chunks.append(chunk)

        metadata = await metadata_future
        await trigger_task

        channel_output.reader.stream.destroy()
        channel_input.writer.close()

        return b"".join(chunks), metadata


# -- Health check -------------------------------------------------------

logger_health = Logger(None, "api::get::/health")


def _register_health() -> None:
    function_id = "api::get::/health"

    async def handler(_req: Any) -> ApiResponse:
        return ApiResponse(
            statusCode=200,
            body={"status": "ok", "service": "image-resize-demo-python"},
            headers={"Content-Type": "application/json"},
        )

    iii.register_function({"id": function_id}, handler)
    iii.register_trigger({
        "type": "http",
        "function_id": function_id,
        "config": {
            "api_path": "/health",
            "http_method": "GET",
            "description": "Health check",
        },
    })


# -- Thumbnail endpoint -------------------------------------------------

logger_thumb = Logger(None, "api::post::/thumbnail")


def _register_thumbnail() -> None:
    function_id = "api::post::/thumbnail"

    async def handler(req: Any) -> ApiResponse:
        body = req if isinstance(req, dict) else (req.body if hasattr(req, "body") else {})
        if isinstance(body, ApiRequest):
            body = body.body or {}

        image_b64 = body.get("image")
        if not image_b64:
            return ApiResponse(
                statusCode=400,
                body={"error": 'Missing "image" field (base64-encoded image data)'},
                headers={"Content-Type": "application/json"},
            )

        width = body.get("width", 200)
        height = body.get("height", 200)
        strategy = body.get("strategy", "scale-to-fit")
        output_format = body.get("outputFormat", "jpeg")

        image_bytes = base64.b64decode(image_b64)
        input_format = body.get("format") or detect_format(image_bytes)

        logger_thumb.info("Processing thumbnail request", {
            "inputFormat": input_format,
            "outputFormat": output_format,
            "width": width,
            "height": height,
            "strategy": strategy,
        })

        try:
            thumbnail, metadata = await process_image(
                image_bytes,
                fmt=input_format,
                output_format=output_format,
                width=width,
                height=height,
                strategy=strategy,
            )

            logger_thumb.info("Thumbnail generated", {
                "format": metadata.get("format"),
                "width": metadata.get("width"),
                "height": metadata.get("height"),
                "size": len(thumbnail),
            })

            return ApiResponse(
                statusCode=200,
                body={
                    "thumbnail": base64.b64encode(thumbnail).decode(),
                    "format": metadata.get("format"),
                    "width": metadata.get("width"),
                    "height": metadata.get("height"),
                    "size": len(thumbnail),
                },
                headers={"Content-Type": "application/json"},
            )
        except Exception as e:
            logger_thumb.error("Thumbnail generation failed", {"error": str(e)})
            return ApiResponse(
                statusCode=500,
                body={"error": f"Thumbnail generation failed: {e}"},
                headers={"Content-Type": "application/json"},
            )

    iii.register_function(
        {"id": function_id, "metadata": {"tags": ["image", "thumbnail"]}},
        handler,
    )
    iii.register_trigger({
        "type": "http",
        "function_id": function_id,
        "config": {
            "api_path": "/thumbnail",
            "http_method": "POST",
            "description": "Generate a thumbnail from a base64-encoded image via the image-resize module",
            "metadata": {"tags": ["image", "thumbnail"]},
        },
    })


# -- Register all endpoints and start -----------------------------------

_register_health()
_register_thumbnail()

log.info("Image resize demo (Python) started -- registering endpoints...")
