import type { Server, ToolAnnotations } from './types'

/** Which functions count as a read: the table of the spec's "Agent safety
 * defaults". The Writes chip, the Test confirmation and the rule writer all
 * read it from here, so they cannot disagree. */

export type AccessClass = 'read' | 'write' | 'destructive'
export type AccessSource = 'server_read' | 'server_write' | 'name' | 'definition'

export const SOURCE_TEXT: Record<AccessSource, string> = {
  server_read: 'Marked read-only by the server',
  server_write: 'Marked as writing by the server',
  name: 'Guessed from the name',
  definition: 'Reads by definition',
}

export interface FunctionAccess {
  functionId: string
  kind: 'tool' | 'instructions' | 'resource' | 'prompt'
  name: string
  cls: AccessClass
  source: AccessSource
}

const READ_PREFIXES = ['get_', 'list_', 'search_', 'read_', 'find_', 'describe_']

/** Any annotations object, an empty one included, is the server's word; the
 * name is a guess only when the server sent none. */
export function classifyTool(
  name: string,
  annotations: ToolAnnotations | null,
): { cls: AccessClass; source: AccessSource } {
  if (annotations) {
    if (annotations.readOnlyHint === true) return { cls: 'read', source: 'server_read' }
    return { cls: annotations.destructiveHint === false ? 'write' : 'destructive', source: 'server_write' }
  }
  const read = name === 'whoami' || READ_PREFIXES.some((prefix) => name.startsWith(prefix))
  return { cls: read ? 'read' : 'destructive', source: 'name' }
}

export function serverAccess(server: Server): FunctionAccess[] {
  const read = (functionId: string, kind: FunctionAccess['kind'], name: string): FunctionAccess => ({
    functionId,
    kind,
    name,
    cls: 'read',
    source: 'definition',
  })
  return [
    ...server.tools.map(
      (tool): FunctionAccess => ({
        functionId: tool.function_id,
        kind: 'tool',
        name: tool.name,
        ...classifyTool(tool.name, tool.annotations),
      }),
    ),
    ...(server.has_instructions ? [read(`mcp::${server.name}::instructions`, 'instructions', 'instructions')] : []),
    ...server.resource_functions.map((fn) => read(fn.function_id, 'resource', `resources::${fn.name}`)),
    ...server.prompts.map((prompt) => read(prompt.function_id, 'prompt', prompt.name)),
  ]
}

/** The ids that get an allow rule, sorted. */
export function readIds(server: Server): string[] {
  return serverAccess(server)
    .filter((fn) => fn.cls === 'read')
    .map((fn) => fn.functionId)
    .sort()
}
