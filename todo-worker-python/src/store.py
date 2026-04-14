from __future__ import annotations

import uuid
from datetime import datetime, timezone
from typing import Any, TypedDict


class Todo(TypedDict):
    id: str
    title: str
    completed: bool
    created_at: str


class TodoStore:
    def __init__(self) -> None:
        self._todos: dict[str, Todo] = {}

    def create(self, title: str) -> Todo:
        todo: Todo = {
            "id": str(uuid.uuid4()),
            "title": title,
            "completed": False,
            "created_at": datetime.now(timezone.utc).isoformat(),
        }
        self._todos[todo["id"]] = todo
        return todo

    def list(self) -> list[Todo]:
        return list(self._todos.values())

    def get(self, todo_id: str) -> Todo | None:
        return self._todos.get(todo_id)

    def update(self, todo_id: str, data: dict[str, Any]) -> Todo | None:
        todo = self._todos.get(todo_id)
        if todo is None:
            return None

        if "title" in data:
            todo["title"] = data["title"]
        if "completed" in data:
            todo["completed"] = data["completed"]

        return todo

    def remove(self, todo_id: str) -> bool:
        return self._todos.pop(todo_id, None) is not None
