from __future__ import annotations

from typing import Any

from iii import ApiRequest, ApiResponse, Logger

from .store import TodoStore

JSON_HEADERS = {"Content-Type": "application/json"}


def create_handlers(store: TodoStore) -> dict[str, Any]:
    async def create_todo(req: ApiRequest[Any], logger: Logger) -> ApiResponse[Any]:
        title = req.body.get("title") if req.body else None

        if not title:
            return ApiResponse(statusCode=400, body={"error": 'Missing "title" field'}, headers=JSON_HEADERS)

        todo = store.create(title)
        logger.info("Todo created", {"id": todo["id"], "title": title})

        return ApiResponse(statusCode=201, body=todo, headers=JSON_HEADERS)

    async def list_todos(req: ApiRequest[Any], logger: Logger) -> ApiResponse[Any]:
        todos = store.list()
        logger.info("Listing todos", {"count": len(todos)})

        return ApiResponse(statusCode=200, body=todos, headers=JSON_HEADERS)

    async def get_todo(req: ApiRequest[Any], logger: Logger) -> ApiResponse[Any]:
        todo_id = req.path_params.get("id", "")
        todo = store.get(todo_id)

        if not todo:
            logger.info("Todo not found", {"id": todo_id})
            return ApiResponse(statusCode=404, body={"error": "Todo not found"}, headers=JSON_HEADERS)

        return ApiResponse(statusCode=200, body=todo, headers=JSON_HEADERS)

    async def update_todo(req: ApiRequest[Any], logger: Logger) -> ApiResponse[Any]:
        todo_id = req.path_params.get("id", "")
        body = req.body or {}
        title = body.get("title")
        completed = body.get("completed")

        if title is None and completed is None:
            return ApiResponse(
                statusCode=400,
                body={"error": 'Provide "title" and/or "completed"'},
                headers=JSON_HEADERS,
            )

        todo = store.update(todo_id, body)

        if not todo:
            logger.info("Todo not found for update", {"id": todo_id})
            return ApiResponse(statusCode=404, body={"error": "Todo not found"}, headers=JSON_HEADERS)

        logger.info("Todo updated", {"id": todo_id, "title": todo["title"], "completed": todo["completed"]})

        return ApiResponse(statusCode=200, body=todo, headers=JSON_HEADERS)

    async def delete_todo(req: ApiRequest[Any], logger: Logger) -> ApiResponse[Any]:
        todo_id = req.path_params.get("id", "")
        removed = store.remove(todo_id)

        if not removed:
            logger.info("Todo not found for deletion", {"id": todo_id})
            return ApiResponse(statusCode=404, body={"error": "Todo not found"}, headers=JSON_HEADERS)

        logger.info("Todo deleted", {"id": todo_id})

        return ApiResponse(statusCode=204)

    return {
        "create_todo": create_todo,
        "list_todos": list_todos,
        "get_todo": get_todo,
        "update_todo": update_todo,
        "delete_todo": delete_todo,
    }
