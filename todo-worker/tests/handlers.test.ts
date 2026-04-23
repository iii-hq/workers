import { describe, expect, it } from 'vitest'
import { createHandlers } from '../src/handlers.js'
import { TodoStore } from '../src/store.js'

const noopLogger = {
  info: () => {},
  warn: () => {},
  error: () => {},
  debug: () => {},
} as unknown as Parameters<ReturnType<typeof createHandlers>['createTodo']>[1]

const make = () => createHandlers(new TodoStore())

const req = (overrides: Record<string, unknown> = {}) =>
  ({
    body: undefined,
    path_params: {},
    query_params: {},
    headers: {},
    ...overrides,
  }) as Parameters<ReturnType<typeof createHandlers>['createTodo']>[0]

describe('todo-worker handlers', () => {
  it('creates a todo and lists it back', async () => {
    const h = make()

    const created = await h.createTodo(req({ body: { title: 'Buy milk' } }), noopLogger)
    expect(created.status_code).toBe(201)
    const todo = created.body as { id: string; title: string; completed: boolean }
    expect(todo.title).toBe('Buy milk')
    expect(todo.completed).toBe(false)
    expect(todo.id).toMatch(/^[0-9a-f-]{36}$/)

    const listed = await h.listTodos(req(), noopLogger)
    expect(listed.status_code).toBe(200)
    expect((listed.body as Array<{ id: string }>).map((t) => t.id)).toEqual([todo.id])
  })

  it('rejects create without a title', async () => {
    const h = make()
    const res = await h.createTodo(req({ body: {} }), noopLogger)
    expect(res.status_code).toBe(400)
    expect(res.body).toEqual({ error: 'Missing "title" field' })
  })

  it('returns 404 for a missing todo', async () => {
    const h = make()
    const res = await h.getTodo(req({ path_params: { id: 'does-not-exist' } }), noopLogger)
    expect(res.status_code).toBe(404)
  })

  it('updates and deletes a todo', async () => {
    const h = make()
    const created = await h.createTodo(req({ body: { title: 'Walk dog' } }), noopLogger)
    const id = (created.body as { id: string }).id

    const updated = await h.updateTodo(req({ path_params: { id }, body: { completed: true } }), noopLogger)
    expect(updated.status_code).toBe(200)
    expect((updated.body as { completed: boolean }).completed).toBe(true)

    const removed = await h.deleteTodo(req({ path_params: { id } }), noopLogger)
    expect(removed.status_code).toBe(204)

    const after = await h.getTodo(req({ path_params: { id } }), noopLogger)
    expect(after.status_code).toBe(404)
  })

  it('rejects update with no fields', async () => {
    const h = make()
    const created = await h.createTodo(req({ body: { title: 'x' } }), noopLogger)
    const id = (created.body as { id: string }).id
    const res = await h.updateTodo(req({ path_params: { id }, body: {} }), noopLogger)
    expect(res.status_code).toBe(400)
  })
})
