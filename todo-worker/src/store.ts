import { randomUUID } from 'node:crypto'

export interface Todo {
  id: string
  title: string
  completed: boolean
  created_at: string
}

export class TodoStore {
  private todos = new Map<string, Todo>()

  create(title: string): Todo {
    const todo: Todo = {
      id: randomUUID(),
      title,
      completed: false,
      created_at: new Date().toISOString(),
    }
    this.todos.set(todo.id, todo)
    return todo
  }

  list(): Todo[] {
    return Array.from(this.todos.values())
  }

  get(id: string): Todo | null {
    return this.todos.get(id) ?? null
  }

  update(id: string, data: { title?: string; completed?: boolean }): Todo | null {
    const todo = this.todos.get(id)
    if (!todo) return null

    if (data.title !== undefined) todo.title = data.title
    if (data.completed !== undefined) todo.completed = data.completed

    return todo
  }

  remove(id: string): boolean {
    return this.todos.delete(id)
  }
}
