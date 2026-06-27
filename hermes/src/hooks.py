from __future__ import annotations

import logging
from typing import Any, Awaitable, Callable

from iii import IIIClient
from iii_helpers.http import HttpRequest, HttpResponse


def use_api(
    iii: IIIClient,
    config: dict[str, Any],
    handler: Callable[[HttpRequest[Any], logging.Logger], Awaitable[HttpResponse[Any]]],
) -> None:
    api_path = config["api_path"]
    http_method = config["http_method"]
    # Prefer an explicit, ::-namespaced function_id (callers always pass one).
    # The fallback normalizes api_path into the same :: style instead of
    # producing a dot-namespaced id with a leading slash from the path.
    function_id = config.get("function_id") or "api::{}::{}".format(
        http_method.lower(), api_path.strip("/").replace("/", "::")
    )
    logger = logging.getLogger(function_id)

    async def wrapped(data: HttpRequest[Any] | dict[str, Any]) -> dict[str, Any]:
        req = HttpRequest(**data) if isinstance(data, dict) else data
        result = await handler(req, logger)
        return result.model_dump(by_alias=True)

    iii.register_function(
        function_id,
        wrapped,
        request_format=config.get("request_format"),
        response_format=config.get("response_format"),
    )
    iii.register_trigger(
        {
            "type": "http",
            "function_id": function_id,
            "config": {
                "api_path": api_path,
                "http_method": http_method,
                "description": config.get("description"),
                "metadata": config.get("metadata"),
            },
        }
    )
