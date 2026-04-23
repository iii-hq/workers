import { useApi } from './hooks.js'
import { createHandlers } from './handlers.js'
import { TodoStore } from './store.js'

const store = new TodoStore()
const handlers = createHandlers(store)

useApi({ api_path: '/todos', http_method: 'POST', description: 'Create a new todo' }, handlers.createTodo)

useApi({ api_path: '/todos', http_method: 'GET', description: 'List all todos' }, handlers.listTodos)

useApi({ api_path: '/todos/:id', http_method: 'GET', description: 'Get a todo by ID' }, handlers.getTodo)

useApi({ api_path: '/todos/:id', http_method: 'PUT', description: 'Update a todo' }, handlers.updateTodo)

useApi({ api_path: '/todos/:id', http_method: 'DELETE', description: 'Delete a todo' }, handlers.deleteTodo)
