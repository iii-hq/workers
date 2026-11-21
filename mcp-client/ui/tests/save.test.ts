import { describe, expect, it } from 'vitest'
import type { Api } from '../src/api'
import {
  approveEntry,
  effectSentences,
  needsConfirm,
  removeEntry,
  type SaveDeps,
  saveEntry,
  secretIssues,
  setDisabled,
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
    const before = { entry: { ...SPEC.linear, url: 'https://old.example/mcp' }, fingerprint: 'fp0' }
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
    await saveEntry(f.deps, 'search', SPEC.search, { entry: { ...SPEC.search, headers: {} }, fingerprint: 'fp0' })
    expect(f.calls).toEqual(['validate', 'confirm', 'approve:search:fp1', 'write:search'])
  })

  it('approves without a confirm when nothing runs and nothing is sent', async () => {
    const f = fake({ validation: QUIET })
    await saveEntry(f.deps, 'linear-auto', SPEC['linear-auto'], null)
    expect(f.calls).toEqual(['validate', 'blockCount', 'approve:linear-auto:fp1', 'write:linear-auto'])
  })

  it('writes without approving when the fingerprint did not change', async () => {
    const f = fake()
    await saveEntry(f.deps, 'linear', { ...SPEC.linear, timeout: 30 }, { entry: SPEC.linear, fingerprint: 'fp1' })
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
  const linear = srv({ name: 'linear', tools })

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

describe('secretIssues', () => {
  it('keeps the errors that sit at a secret position', () => {
    const tails = [
      'auth/token',
      'auth/client_secret',
      'headers/X-API-Key',
      'env/GITHUB_TOKEN',
      'headers/X-Workspace',
      'url',
    ]
    const errors = tails.map((tail) => ({ path: `/mcpServers/a/${tail}`, message: 'm' }))
    expect(secretIssues(errors)).toEqual(errors.slice(0, 4))
  })
})
