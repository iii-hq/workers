from __future__ import annotations

import logging
from typing import Any, Awaitable, Callable

from iii import IIIClient, ObserverHandle
from iii_helpers.http import HttpRequest, HttpResponse

#: The http worker's request function: each observer serves one route.
HTTP_ON_REQUEST_FN = "http::on-request"


def use_api(
    iii: IIIClient,
    config: dict[str, Any],
    handler: Callable[[HttpRequest[Any], logging.Logger], Awaitable[HttpResponse[Any]]],
) -> ObserverHandle:
    """Register `handler` as a function and serve `http_method api_path` with
    it by observing `http::on-request` with the route as args."""
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
        description=config.get("description"),
        request_format=config.get("request_format"),
        response_format=config.get("response_format"),
    )
    # Serve the route: the http worker calls `function_id` with each matching
    # request and answers with its return value. The observer stays pending
    # while the http worker is absent.
    metadata = config.get("metadata")
    return iii.observe(
        {"function_id": HTTP_ON_REQUEST_FN, "args": {"method": http_method, "path": api_path}},
        function_id,
        {"metadata": metadata} if metadata is not None else None,
    )
