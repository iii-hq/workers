/**
 * The worker this bundle runs as. The console serves it at `ui/<worker>/page.js`, and a
 * namespaced board (`KANBAN_NAMESPACE=todo-local` on the worker) is `kanban-todo-local`: every
 * function id, trigger type, page id and mention name carries that name in place of `kanban`.
 * The check scripts run from a file URL and get the plain name.
 */
export const WORKER = /\/ui\/(kanban[^/]*)\/page\.js/.exec(import.meta.url)?.[1] ?? 'kanban'

/** `todo-local` for `kanban-todo-local`; empty for the plain worker. */
export const NAMESPACE = WORKER.startsWith('kanban-') ? WORKER.slice('kanban-'.length) : ''
