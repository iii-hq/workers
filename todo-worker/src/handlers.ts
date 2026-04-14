import type { ApiRequest, ApiResponse, Logger } from 'iii-sdk'
import type { TodoStore } from './store.js'

// biome-ignore lint/suspicious/noExplicitAny: body can be Todo, Todo[], or error object
type HandlerResponse = ApiResponse<number, any>

export function createHandlers(store: TodoStore) {
  async function createTodo(req: ApiRequest, logger: Logger): Promise<HandlerResponse> {
    const { title } = req.body as { title?: string }

    if (!title) {
      return {
        status_code: 400,
        body: { error: 'Missing "title" field' },
        headers: { 'Content-Type': 'application/json' },
      }
    }

    const todo = store.create(title)
    logger.info('Todo created', { id: todo.id, title })

    return {
      status_code: 201,
      body: todo,
      headers: { 'Content-Type': 'application/json' },
    }
  }

  async function listTodos(_req: ApiRequest, logger: Logger): Promise<HandlerResponse> {
    const todos = store.list()
    logger.info('Listing todos', { count: todos.length })

    return {
      status_code: 200,
      body: todos,
      headers: { 'Content-Type': 'application/json' },
    }
  }

  async function getTodo(req: ApiRequest, logger: Logger): Promise<HandlerResponse> {
    const { id } = req.path_params
    const todo = store.get(id)

    if (!todo) {
      logger.info('Todo not found', { id })
      return {
        status_code: 404,
        body: { error: 'Todo not found' },
        headers: { 'Content-Type': 'application/json' },
      }
    }

    return {
      status_code: 200,
      body: todo,
      headers: { 'Content-Type': 'application/json' },
    }
  }

  async function updateTodo(req: ApiRequest, logger: Logger): Promise<HandlerResponse> {
    const { id } = req.path_params
    const { title, completed } = req.body as { title?: string; completed?: boolean }

    if (title === undefined && completed === undefined) {
      return {
        status_code: 400,
        body: { error: 'Provide "title" and/or "completed"' },
        headers: { 'Content-Type': 'application/json' },
      }
    }

    const todo = store.update(id, { title, completed })

    if (!todo) {
      logger.info('Todo not found for update', { id })
      return {
        status_code: 404,
        body: { error: 'Todo not found' },
        headers: { 'Content-Type': 'application/json' },
      }
    }

    logger.info('Todo updated', { id, title: todo.title, completed: todo.completed })

    return {
      status_code: 200,
      body: todo,
      headers: { 'Content-Type': 'application/json' },
    }
  }

  async function deleteTodo(req: ApiRequest, logger: Logger): Promise<HandlerResponse> {
    const { id } = req.path_params
    const removed = store.remove(id)

    if (!removed) {
      logger.info('Todo not found for deletion', { id })
      return {
        status_code: 404,
        body: { error: 'Todo not found' },
        headers: { 'Content-Type': 'application/json' },
      }
    }

    logger.info('Todo deleted', { id })

    return {
      status_code: 204,
    }
  }

  return { createTodo, listTodos, getTodo, updateTodo, deleteTodo }
}
