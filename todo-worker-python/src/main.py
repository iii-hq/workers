from __future__ import annotations

import os
import signal
import threading

from iii import InitOptions, register_worker

from .handlers import create_handlers
from .hooks import use_api
from .store import TodoStore


def main() -> None:
    engine_ws_url = os.environ.get("III_URL", "ws://localhost:49134")

    iii = register_worker(
        address=engine_ws_url,
        options=InitOptions(
            worker_name="todo-worker-python",
            otel={"enabled": True, "service_name": "todo-worker-python"},
        ),
    )

    print(f"Todo worker started (engine: {engine_ws_url})")

    store = TodoStore()
    handlers = create_handlers(store)

    use_api(
        iii,
        {"api_path": "/todos", "http_method": "POST", "description": "Create a new todo"},
        handlers["create_todo"],
    )

    use_api(
        iii,
        {"api_path": "/todos", "http_method": "GET", "description": "List all todos"},
        handlers["list_todos"],
    )

    use_api(
        iii,
        {"api_path": "/todos/:id", "http_method": "GET", "description": "Get a todo by ID"},
        handlers["get_todo"],
    )

    use_api(
        iii,
        {"api_path": "/todos/:id", "http_method": "PUT", "description": "Update a todo"},
        handlers["update_todo"],
    )

    use_api(
        iii,
        {"api_path": "/todos/:id", "http_method": "DELETE", "description": "Delete a todo"},
        handlers["delete_todo"],
    )

    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    signal.signal(signal.SIGINT, lambda *_: stop.set())
    stop.wait()
    iii.shutdown()


if __name__ == "__main__":
    main()
