import { describe, expect, it } from 'vitest'
import { classifyTool, readIds, SOURCE_TEXT, serverAccess } from '../src/access'
import { docs, server } from './fixtures/servers'

describe('classifyTool, one case per row of 5.6', () => {
  it('trusts annotations over the name', () => {
    expect(classifyTool('save_issue', { readOnlyHint: true })).toEqual({ cls: 'read', source: 'server_read' })
    expect(classifyTool('list_issues', { readOnlyHint: false, destructiveHint: false })).toEqual({
      cls: 'write',
      source: 'server_write',
    })
    for (const a of [{ destructiveHint: true }, {}, { title: 'List' }]) {
      expect(classifyTool('list_issues', a)).toEqual({ cls: 'destructive', source: 'server_write' })
    }
  })

  it('falls back to the name only without annotations', () => {
    for (const name of [
      'get_issue',
      'list_issues',
      'search_documentation',
      'read_file',
      'find_user',
      'describe_table',
      'whoami',
    ]) {
      expect(classifyTool(name, null), name).toEqual({ cls: 'read', source: 'name' })
    }
    for (const name of ['save_issue', 'delete_comment', 'getter', 'whoami_v2', 'LIST_ISSUES', 'relist_issues']) {
      expect(classifyTool(name, null), name).toEqual({ cls: 'destructive', source: 'name' })
    }
  })
})

describe('serverAccess', () => {
  it('serverAccess covers instructions, resources and prompts as reads by definition', () => {
    const access = serverAccess(docs())
    expect(access.map((a) => [a.kind, a.name, a.cls, a.source])).toEqual([
      ['tool', 'search', 'destructive', 'name'],
      ['instructions', 'instructions', 'read', 'definition'],
      ['resource', 'resources::list', 'read', 'definition'],
      ['resource', 'resources::read', 'read', 'definition'],
      ['resource', 'resources::templates', 'read', 'definition'],
      ['prompt', 'code_review', 'read', 'definition'],
    ])
    expect(access[1].functionId).toBe('mcp::docs::instructions')
    expect(readIds(docs())).toEqual([
      'mcp::docs::instructions',
      'mcp::docs::prompts::code_review',
      'mcp::docs::resources::list',
      'mcp::docs::resources::read',
      'mcp::docs::resources::templates',
    ])
    expect(readIds(server())).toEqual(['mcp::linear::list_issues'])
  })
})

describe('SOURCE_TEXT', () => {
  it('words the source', () => {
    expect(SOURCE_TEXT).toEqual({
      server_read: 'Marked read-only by the server',
      server_write: 'Marked as writing by the server',
      name: 'Guessed from the name',
      definition: 'Reads by definition',
    })
  })
})
