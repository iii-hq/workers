import { describe, expect, it } from 'vitest'
import type { Api } from '../src/api'
import { mapAsJson, readMapEdit } from '../src/config-model'
import {
  approveEntry,
  asksGate,
  effectSentences,
  needsConfirm,
  removeEntry,
  type SaveDeps,
  saveBefore,
  saveEntry,
  saveMap,
  setDisabled,
  shareEntry,
} from '../src/save'
import type { Issue, ServerValidation } from '../src/types'
import { server as srv } from './fixtures/servers'
import spec from './fixtures/spec-config.json'

const SPEC = spec as Record<string, Record<string, unknown>>

const VALID: ServerValidation = {
  ok: true,
  errors: [],
  warnings: [],
  fingerprint: 'fp1',
  effects: { runs: null, sends: [{ ref: 'secret://LINEAR_API_KEY', host: 'mcp.linear.app' }] },
}
const QUIET: ServerValidation = { ...VALID, effects: { runs: null, sends: [] } }
const SEND = 'mcp-client will send LINEAR_API_KEY to mcp.linear.app'
const BLOCK =
  'The approval gate allows 41 functions under this name. Those rules will be removed; allow reads again once the server is connected.'
const NO_BLOCK = { ok: false, reason: 'boom', count: 0 } // a removeBlock that fails

type Body = Parameters<SaveDeps['confirm']>[0]

/** Deps whose every call pushes one token to `calls`. */
function fake(
  opts: {
    validation?: ServerValidation
    confirm?: boolean
    blockCount?: number
    blockCountError?: Error
    removeBlock?: { ok: boolean; reason?: string; count: number }
    missing?: string
  } = {},
) {
  const calls: string[] = []
  const confirms: Body[] = []
  const written: Record<string, unknown> = {}
  const api = {
    validate: async (mcpServers: Record<string, unknown>) => {
      calls.push('validate')
      return {
        servers: Object.fromEntries(Object.keys(mcpServers).map((name) => [name, opts.validation ?? VALID])),
      }
    },
    approve: async (server: string, fingerprint?: string) => {
      calls.push(`approve:${server}:${fingerprint}`)
      return { approved: true }
    },
    writeEntry: async (name: string, entry: unknown) => {
      calls.push(`write:${name}`)
      written[name] = entry
    },
    readConfig: async () => {
      calls.push('readConfig')
      return { id: 'mcp-client', mcpServers: SPEC }
    },
    secretConsumers: async (reference: string) => {
      calls.push(`read:${reference}`)
      if (reference === opts.missing) throw new Error('`GONE` is not in the vault.')
      return ['database']
    },
    grantSecret: async (reference: string, consumers: string[]) => {
      calls.push(`share:${reference}:${consumers.join(',')}`)
    },
    reconnect: async (server: string) => {
      calls.push(`reconnect:${server}`)
      return { state: 'connecting' }
    },
  } as unknown as Api
  const deps: SaveDeps = {
    api,
    confirm: async (body) => {
      calls.push('confirm')
      confirms.push(body)
      return opts.confirm ?? true
    },
    blockCount: async () => {
      calls.push('blockCount')
      if (opts.blockCountError) throw opts.blockCountError
      return opts.blockCount ?? 0
    },
    removeBlock: async () => {
      calls.push('removeBlock')
      return opts.removeBlock ?? { ok: true, count: 41 }
    },
  }
  return { deps, calls, confirms, written }
}

describe('effectSentences', () => {
  it('says what approving allows, in plain words', () => {
    expect(
      effectSentences({
        runs: 'npx -y @playwright/mcp@0.0.41',
        sends: [
          { ref: 'secret://PLAYWRIGHT_GITHUB_TOKEN', host: 'process' },
          { ref: 'env://HTTPS_PROXY', host: 'process' },
        ],
      }),
    ).toEqual([
      'mcp-client will run `npx -y @playwright/mcp@0.0.41`',
      'mcp-client will pass PLAYWRIGHT_GITHUB_TOKEN to the process',
      'mcp-client will pass HTTPS_PROXY to the process',
    ])
    expect(effectSentences(VALID.effects)).toEqual([SEND])
    expect(
      effectSentences({
        runs: null,
        sends: [
          { ref: 'secret://SEARCH_URL', host: 'secret://SEARCH_URL' },
          { ref: 'secret://SEARCH_X_API_KEY', host: 'secret://SEARCH_URL' },
        ],
      }),
    ).toEqual([
      'mcp-client will connect to the address stored in SEARCH_URL',
      'mcp-client will send SEARCH_X_API_KEY to the address stored in SEARCH_URL',
    ])
    expect(effectSentences(QUIET.effects)).toEqual([])
  })
})

describe('needsConfirm', () => {
  it('asks for a new or changed entry that runs or sends something', () => {
    expect(needsConfirm(null, VALID)).toEqual(true)
    expect(needsConfirm({ fingerprint: 'fp0' }, VALID)).toEqual(true)
    expect(needsConfirm({ fingerprint: 'fp1' }, VALID)).toEqual(false)
    expect(needsConfirm(null, QUIET)).toEqual(false)
    expect(needsConfirm(null, { ...QUIET, effects: { runs: 'npx x', sends: [] } })).toEqual(true)
  })
})

describe('asksGate', () => {
  it('asks the approval gate for an add, or for a changed connection the fingerprint does not already cover', () => {
    const before = { entry: { type: 'http', url: 'https://a.example.com/mcp' }, fingerprint: 'f1', approved: true }
    expect(asksGate(null, 'f2', before.entry)).toBe(true)
    expect(asksGate(before, 'f2', { ...before.entry, url: 'https://b.example.com/mcp' })).toBe(true)
    expect(asksGate(before, 'f2', { ...before.entry, headers: { 'X-Workspace': 'acme' } })).toBe(false)
    expect(asksGate(before, 'f1', { ...before.entry, url: 'https://b.example.com/mcp' })).toBe(false)
  })

  it('asks for any change to an entry changed outside the page: its stored entry is not what was approved', () => {
    const before = { entry: { type: 'http', url: 'https://a.example.com/mcp' }, fingerprint: 'f1', approved: false }
    expect(asksGate(before, 'f2', { ...before.entry, headers: { 'X-Workspace': 'acme' } })).toBe(true)
    expect(asksGate(before, 'f1', before.entry)).toBe(false)
  })
})

describe('saveEntry', () => {
  it('validates, confirms, approves the validated fingerprint, then writes', async () => {
    const f = fake()
    expect(await saveEntry(f.deps, 'linear', SPEC.linear, null)).toEqual({ saved: true, errors: [] })
    expect(f.calls).toEqual(['validate', 'blockCount', 'confirm', 'approve:linear:fp1', 'write:linear'])
    expect(f.confirms).toEqual([{ title: 'Save linear?', lines: [SEND] }])
    expect(f.written.linear).toEqual(SPEC.linear)
  })

  it('removes the block first when a new entry takes a name that has one', async () => {
    const f = fake({ blockCount: 41 })
    await saveEntry(f.deps, 'linear', SPEC.linear, null)
    expect(f.calls).toEqual(['validate', 'blockCount', 'confirm', 'removeBlock', 'approve:linear:fp1', 'write:linear'])
    expect(f.confirms[0].lines).toEqual([SEND, BLOCK])
  })

  it('removes the block when url, command or args changed', async () => {
    const f = fake({ blockCount: 41 })
    const before = { entry: { ...SPEC.linear, url: 'https://old.example/mcp' }, fingerprint: 'fp0', approved: true }
    await saveEntry(f.deps, 'linear', SPEC.linear, before)
    expect(f.calls).toEqual(['validate', 'blockCount', 'confirm', 'removeBlock', 'approve:linear:fp1', 'write:linear'])
  })

  it('asks about the block alone when nothing runs and nothing is sent', async () => {
    const f = fake({ validation: QUIET, blockCount: 41 })
    await saveEntry(f.deps, 'linear-auto', SPEC['linear-auto'], null)
    expect(f.confirms).toEqual([{ title: 'Save linear-auto?', lines: [BLOCK] }])
    expect(f.calls).toEqual([
      'validate',
      'blockCount',
      'confirm',
      'removeBlock',
      'approve:linear-auto:fp1',
      'write:linear-auto',
    ])
  })

  it('leaves the block alone when only headers changed', async () => {
    const f = fake({ blockCount: 41 })
    await saveEntry(f.deps, 'search', SPEC.search, {
      entry: { ...SPEC.search, headers: {} },
      fingerprint: 'fp0',
      approved: true,
    })
    expect(f.calls).toEqual(['validate', 'confirm', 'approve:search:fp1', 'write:search'])
  })

  it('removes the block when only headers changed on an entry changed outside the page', async () => {
    const f = fake({ blockCount: 41 })
    const before = { entry: { ...SPEC.search, headers: {} }, fingerprint: 'fp0', approved: false }
    await saveEntry(f.deps, 'search', SPEC.search, before)
    expect(f.calls).toEqual(['validate', 'blockCount', 'confirm', 'removeBlock', 'approve:search:fp1', 'write:search'])
    expect(f.confirms[0].lines).toContain(BLOCK)
  })

  it('approves without a confirm when nothing runs and nothing is sent', async () => {
    const f = fake({ validation: QUIET })
    await saveEntry(f.deps, 'linear-auto', SPEC['linear-auto'], null)
    expect(f.calls).toEqual(['validate', 'blockCount', 'approve:linear-auto:fp1', 'write:linear-auto'])
  })

  it('writes without approving when the fingerprint did not change', async () => {
    const f = fake()
    await saveEntry(
      f.deps,
      'linear',
      { ...SPEC.linear, timeout: 30 },
      { entry: SPEC.linear, fingerprint: 'fp1', approved: true },
    )
    expect(f.calls).toEqual(['validate', 'write:linear'])
  })

  it('stops on a validation error', async () => {
    const errors: Issue[] = [{ path: '/mcpServers/linear/url', message: 'url must be https' }]
    const f = fake({ validation: { ...VALID, ok: false, errors } })
    expect(await saveEntry(f.deps, 'linear', SPEC.linear, null)).toEqual({ saved: false, errors })
    expect(f.calls).toEqual(['validate'])
  })

  it('stops when the person declines', async () => {
    const f = fake({ confirm: false, blockCount: 41 })
    expect(await saveEntry(f.deps, 'linear', SPEC.linear, null)).toEqual({ saved: false, errors: [] })
    expect(f.calls).toEqual(['validate', 'blockCount', 'confirm'])
  })

  it('writes nothing when the block cannot be removed', async () => {
    const f = fake({
      blockCount: 41,
      removeBlock: { ok: false, reason: 'The approval-gate entry has no rules array.', count: 0 },
    })
    expect(await saveEntry(f.deps, 'linear', SPEC.linear, null)).toEqual({
      saved: false,
      errors: [],
      message:
        'Nothing was saved. The approval-gate rules could not be removed: The approval-gate entry has no rules array.',
    })
    expect(f.calls).toEqual(['validate', 'blockCount', 'confirm', 'removeBlock'])
  })

  it('writes nothing when the gate cannot be read', async () => {
    const f = fake({ blockCountError: new Error('timeout') })
    expect(await saveEntry(f.deps, 'linear', SPEC.linear, null)).toEqual({
      saved: false,
      errors: [],
      message: 'Nothing was saved. The approval-gate rules could not be checked: timeout',
    })
    expect(f.calls).toEqual(['validate', 'blockCount'])
  })
})

describe('approveEntry', () => {
  const unapproved = srv({
    name: 'linear',
    state: 'unapproved',
    approved: false,
    fingerprint: 'fp-loaded',
    target: 'https://mcp.linear.app/mcp',
  })

  it('approves what the person confirmed, after removing the block', async () => {
    const f = fake({ blockCount: 41 })
    expect(await approveEntry(f.deps, unapproved)).toEqual(true)
    expect(f.calls).toEqual(['readConfig', 'validate', 'blockCount', 'confirm', 'removeBlock', 'approve:linear:fp1'])
    expect(f.confirms).toEqual([{ title: 'Approve linear?', lines: [SEND, BLOCK] }])
  })

  it('always asks, and names the target when there is nothing else to say', async () => {
    const f = fake({ validation: QUIET, confirm: false })
    expect(await approveEntry(f.deps, unapproved)).toEqual(false)
    expect(f.confirms[0].lines).toEqual(['mcp-client will connect to https://mcp.linear.app/mcp'])
    expect(f.calls).toEqual(['readConfig', 'validate', 'blockCount', 'confirm'])
  })

  it('approves nothing when the block cannot be removed', async () => {
    const f = fake({ blockCount: 41, removeBlock: NO_BLOCK })
    await expect(approveEntry(f.deps, unapproved)).rejects.toThrow(
      'Nothing was approved. The approval-gate rules could not be removed: boom',
    )
    expect(f.calls).not.toContain('approve:linear:fp1')
  })
})

describe('removeEntry', () => {
  const tools = Array.from({ length: 69 }, (_, i) => ({
    name: `list_${i}`,
    function_id: `mcp::linear::list_${i}`,
    registered: true,
    annotations: null,
  }))
  const linear = srv({ name: 'linear', tools, auth: 'oauth' })

  it('removes the block, then the entry', async () => {
    const f = fake({ blockCount: 41 })
    expect(await removeEntry(f.deps, linear)).toEqual(true)
    expect(f.calls).toEqual(['blockCount', 'confirm', 'removeBlock', 'write:linear'])
    expect(f.confirms).toEqual([
      {
        title: 'Remove linear?',
        tone: 'danger',
        lines: ['Its 69 functions go away and its sign-in is cleared.', 'Its 41 approval-gate rules are removed too.'],
      },
    ])
    expect(f.written.linear).toEqual(null)
  })

  it('says only what the server has: no sign-in for an API key, no functions when none are listed', async () => {
    const f = fake()
    await removeEntry(f.deps, srv({ name: 'k', tools }))
    await removeEntry(f.deps, srv({ name: 'e', tools: [], auth: 'none' }))
    expect(f.confirms.map((c) => c.lines)).toEqual([['Its 69 functions go away.'], []])
  })

  it('says nothing about rules when there are none, and keeps the entry when declined', async () => {
    const f = fake({ confirm: false })
    expect(await removeEntry(f.deps, linear)).toEqual(false)
    expect(f.confirms[0].lines).toEqual(['Its 69 functions go away and its sign-in is cleared.'])
    expect(f.calls).toEqual(['blockCount', 'confirm'])
  })

  it('keeps the entry when the block cannot be removed', async () => {
    const f = fake({ blockCount: 41, removeBlock: NO_BLOCK })
    await expect(removeEntry(f.deps, linear)).rejects.toThrow(
      'Nothing was removed. The approval-gate rules could not be removed: boom',
    )
    expect(f.calls).not.toContain('write:linear')
  })
})

describe('setDisabled', () => {
  it('writes the flag and asks for no approval', async () => {
    const f = fake()
    await setDisabled(f.deps.api, 'playwright', true)
    expect(f.calls).toEqual(['readConfig', 'write:playwright'])
    expect(f.written.playwright).toEqual({ ...SPEC.playwright, disabled: true })
    await setDisabled(f.deps.api, 'playwright', false)
    expect(f.written.playwright).not.toHaveProperty('disabled')
  })
})

describe('saveBefore', () => {
  const config = { a: { url: 'A' }, b: { url: 'B' } }
  const a = srv({ name: 'a', fingerprint: 'fa' })
  const b = srv({ name: 'b', fingerprint: 'fb', approved: false })

  it('takes the entry, fingerprint and approval of the server being saved, never of another one', () => {
    expect(saveBefore(false, 'a', config, [b, a])).toEqual({ entry: { url: 'A' }, fingerprint: 'fa', approved: true })
    expect(saveBefore(false, 'b', config, [b, a])).toEqual({ entry: { url: 'B' }, fingerprint: 'fb', approved: false })
  })

  it('has nothing before an add, a name the worker does not list, or one not in the config', () => {
    expect(saveBefore(true, 'a', config, [a])).toEqual(null)
    expect(saveBefore(false, 'a', config, [b])).toEqual(null)
    expect(saveBefore(false, 'c', config, [srv({ name: 'c' })])).toEqual(null)
    expect(saveBefore(false, 'a', null, undefined)).toEqual(null)
  })
})

describe('saveMap', () => {
  const shown = { a: { url: 'A' }, b: { url: 'B' } }
  const listed = [srv({ name: 'a', fingerprint: 'fp1' }), srv({ name: 'b' })]

  it('goes through removed and changed names in name order; a new name has nothing before it', async () => {
    const f = fake()
    const edit = { candidate: { a: { url: 'A2' }, c: { url: 'C' } }, changed: ['a', 'c'], removed: ['b'] }
    expect(await saveMap(f.deps, shown, edit, listed)).toEqual({ saved: true, issues: [] })
    expect(f.calls).toEqual([
      'validate',
      // a: the listed fingerprint is the validated one, so no approve
      'validate',
      'write:a',
      // b: removed
      'blockCount',
      'confirm',
      'write:b',
      // c: new, so the gate is read and the person asked
      'validate',
      'blockCount',
      'confirm',
      'approve:c:fp1',
      'write:c',
    ])
  })

  it('stops at the first entry the person declines', async () => {
    const f = fake({ confirm: false })
    const edit = { candidate: { ...shown, c: { url: 'C' }, d: { url: 'D' } }, changed: ['c', 'd'], removed: [] }
    expect(await saveMap(f.deps, shown, edit, listed)).toEqual({ saved: false, issues: [] })
    expect(f.calls).toEqual(['validate', 'validate', 'blockCount', 'confirm'])
  })

  it('notes a removed name the worker does not list, and removes nothing for it', async () => {
    const f = fake()
    const edit = { candidate: shown, changed: [], removed: ['x'] }
    expect(await saveMap(f.deps, { ...shown, x: {} }, edit, listed)).toEqual({
      saved: true,
      issues: [{ path: '', message: 'x: not removed, the worker does not list it' }],
    })
    expect(f.calls).toEqual(['validate'])
  })

  it('stops before any write when the map does not validate', async () => {
    const issue = { path: '/mcpServers/c/url', message: 'must be https' }
    const f = fake({ validation: { ...VALID, ok: false, errors: [issue] } })
    const edit = { candidate: { ...shown, c: { url: 'C' } }, changed: ['c'], removed: [] }
    const result = await saveMap(f.deps, shown, edit, listed)
    expect(result.saved).toEqual(false)
    expect(result.issues).toContainEqual(issue)
    expect(f.calls).toEqual(['validate'])
  })

  it('never writes an entry left as shown, its hidden value and all', async () => {
    const stored = {
      linear: { type: 'http', url: 'https://mcp.linear.app/mcp', auth: { type: 'bearer', token: 'LITERAL' } },
      b: { url: 'B' },
    }
    const edit = readMapEdit(stored, mapAsJson(stored).replace('"B"', '"B2"'))
    if ('issues' in edit) throw new Error(`refused: ${JSON.stringify(edit.issues)}`)
    const f = fake()
    await saveMap(f.deps, stored, edit, [
      srv({ name: 'linear', fingerprint: 'fp1' }),
      srv({ name: 'b', fingerprint: 'fp1' }),
    ])
    expect(f.written).toEqual({ b: { url: 'B2' } })
  })
})

describe('shareEntry', () => {
  const refs = ['env://GH_TOKEN', 'secret://LINEAR_API_KEY']

  it('asks once, reads every reference, then shares each and reconnects', async () => {
    const f = fake()
    expect(await shareEntry(f.deps, 'linear', refs)).toEqual(true)
    expect(f.confirms).toEqual([{ title: 'Share with mcp-client?', lines: refs }])
    expect(f.calls).toEqual([
      'confirm',
      'read:env://GH_TOKEN',
      'read:secret://LINEAR_API_KEY',
      'share:env://GH_TOKEN:database',
      'share:secret://LINEAR_API_KEY:database',
      'reconnect:linear',
    ])
  })

  it('a missing second reference writes nothing', async () => {
    const f = fake({ missing: 'secret://LINEAR_API_KEY' })
    await expect(shareEntry(f.deps, 'linear', refs)).rejects.toThrow('`GONE` is not in the vault.')
    expect(f.calls).toEqual(['confirm', 'read:env://GH_TOKEN', 'read:secret://LINEAR_API_KEY'])
  })

  it('shares nothing when the person declines', async () => {
    const f = fake({ confirm: false })
    expect(await shareEntry(f.deps, 'linear', refs)).toEqual(false)
    expect(f.calls).toEqual(['confirm'])
  })
})
