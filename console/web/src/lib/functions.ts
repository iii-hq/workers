export interface FunctionEntry {
  id: string
  description: string
}

/** Mock-only catalog for playground / examples `@` autocomplete. */
export const STATIC_FUNCTIONS: FunctionEntry[] = [
  { id: 'engine::echo', description: 'echo a string back' },
  { id: 'engine::list', description: 'list workers' },
  { id: 'engine::info', description: 'inspect a worker' },
  { id: 'functions::list', description: 'list every function' },
  { id: 'functions::info', description: 'inspect a function' },
  { id: 'trigger::run', description: 'fire a trigger' },
  { id: 'directory::index', description: 'index a worker skill bundle' },
  { id: 'directory::resolve', description: 'resolve an iii:// link' },
  {
    id: 'context-compaction::compact_session',
    description: 'compact a session now',
  },
  {
    id: 'context-compaction::prune_tool_outputs',
    description: 'prune old tool outputs',
  },
  {
    id: 'session-tree::compactions',
    description: 'list compaction history for a session',
  },
]

export function fuzzyFilter(
  entries: FunctionEntry[],
  query: string,
  limit = 8,
): FunctionEntry[] {
  const q = query.trim().toLowerCase()
  if (!q) return entries.slice(0, limit)
  return entries
    .filter(
      (f) =>
        f.id.toLowerCase().includes(q) ||
        f.description.toLowerCase().includes(q),
    )
    .slice(0, limit)
}

export function getFunctionEntry(id: string): FunctionEntry | undefined {
  return STATIC_FUNCTIONS.find((f) => f.id === id)
}
