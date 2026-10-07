import { describe, expect, it } from 'vitest'
import { parseMentionProviders, parseMentionView } from './runtime'

function row(functionId: string, mention: unknown, extra: object = {}) {
  return {
    function_id: functionId,
    metadata: { internal: true, mention },
    ...extra,
  }
}

describe('parseMentionProviders', () => {
  it('reads descriptors out of function metadata', () => {
    const { providers, conflicts } = parseMentionProviders({
      functions: [
        { function_id: 'kanban::ticket::get', metadata: {} },
        row(
          'kanban::mention::get',
          {
            v: 1,
            name: 'kanban',
            label: 'Tickets',
            icon: 'ticket',
            color: 'blue',
            search: 'kanban::mention::search',
            details: { function_id: 'kanban::ticket::get' },
          },
          { worker_name: 'kanban' },
        ),
      ],
    })
    expect(conflicts).toEqual([])
    expect(providers).toEqual([
      {
        v: 1,
        name: 'kanban',
        label: 'Tickets',
        description: undefined,
        icon: 'ticket',
        color: 'blue',
        search: 'kanban::mention::search',
        details: { function_id: 'kanban::ticket::get', id_field: 'id' },
        getFunctionId: 'kanban::mention::get',
        workerName: 'kanban',
      },
    ])
  })

  it('skips invalid descriptors', () => {
    const { providers } = parseMentionProviders({
      functions: [
        row('a::get', { name: 'fn', label: 'x', search: 'a::search' }),
        row('b::get', { name: 'Bad', label: 'x', search: 'b::search' }),
        row('c::get', { name: 'c', label: '', search: 'c::search' }),
        row('d::get', { name: 'd', label: 'x' }),
        'junk',
      ],
    })
    expect(providers).toEqual([])
    expect(parseMentionProviders(null).providers).toEqual([])
  })

  it('resolves a name claimed twice to the first function id, and says so', () => {
    const { providers, conflicts } = parseMentionProviders({
      functions: [
        row('z::get', { name: 'dup', label: 'Z', search: 'z::search' }),
        row('a::get', { name: 'dup', label: 'A', search: 'a::search' }),
      ],
    })
    expect(providers.map((p) => p.getFunctionId)).toEqual(['a::get'])
    expect(conflicts).toHaveLength(1)
  })
})

describe('parseMentionView', () => {
  it('keeps a complete view and its open target', () => {
    const view = parseMentionView({
      id: 'u1',
      label: 'Fix login',
      hint: 'KAN-1',
      fields: [
        { label: 'Status', value: 'todo', tone: 'info' },
        { label: 'bad' },
      ],
      open: { page: 'kanban-ticket', context: { id: 'KAN-1' } },
      data: { key: 'KAN-1' },
    })
    expect(view).toMatchObject({
      id: 'u1',
      label: 'Fix login',
      hint: 'KAN-1',
      fields: [{ label: 'Status', value: 'todo', tone: 'info' }],
      open: { page: 'kanban-ticket', context: { id: 'KAN-1' } },
      data: { key: 'KAN-1' },
    })
  })

  it('reads session and url targets, refusing non-http urls', () => {
    expect(
      parseMentionView({ id: 'a', label: 'a', open: { session: 's_1' } })?.open,
    ).toEqual({
      session: 's_1',
    })
    expect(
      parseMentionView({ id: 'a', label: 'a', open: { url: 'https://x.dev' } })
        ?.open,
    ).toEqual({ url: 'https://x.dev' })
    expect(
      parseMentionView({
        id: 'a',
        label: 'a',
        open: { url: 'javascript:alert(1)' },
      })?.open,
    ).toBeUndefined()
  })

  it('reads null (unknown id) and unusable shapes as no view', () => {
    expect(parseMentionView(null)).toBeNull()
    expect(parseMentionView({ id: 'a' })).toBeNull()
  })
})
