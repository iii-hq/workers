import type { ISdk } from 'iii-sdk'
import { describe, expect, it, vi } from 'vitest'
import { mergeRun } from '../src/dispatch.js'
import type { RunState } from '../src/types.js'

function fakeSdk(): ISdk {
  return {
    trigger: vi.fn().mockResolvedValue(undefined),
    registerFunction: vi.fn(),
    registerTrigger: vi.fn(),
    registerService: vi.fn(),
    listFunctions: vi.fn(),
    listTriggers: vi.fn(),
  } as unknown as ISdk
}

describe('mergeRun', () => {
  it('returns no winner when every agent failed', async () => {
    const sdk = fakeSdk()
    const run: RunState = {
      id: 'r1',
      task: 't',
      cwd: '/x',
      startedAt: 1,
      agents: [
        { agent: { kind: 'claude' }, status: 'failed' },
        { agent: { kind: 'codex' }, status: 'failed' },
      ],
    }
    const r = await mergeRun(sdk, run)
    expect(r.ok).toBe(false)
    expect(r.losers).toEqual([0, 1])
  })

  it('picks first finished agent with non-empty diff and passing gates', async () => {
    const sdk = fakeSdk()
    const run: RunState = {
      id: 'r2',
      task: 't',
      cwd: '/x',
      startedAt: 1,
      agents: [
        { agent: { kind: 'claude' }, status: 'finished', diff: '', gateResults: { tests: { ok: true } } },
        {
          agent: { kind: 'codex' },
          status: 'finished',
          diff: 'diff --git a b\n',
          gateResults: { tests: { ok: true } },
        },
        {
          agent: { kind: 'gemini' },
          status: 'finished',
          diff: 'diff --git c d\n',
          gateResults: { tests: { ok: true } },
        },
      ],
    }
    const r = await mergeRun(sdk, run)
    expect(r.ok).toBe(true)
    expect(r.winner?.index).toBe(1)
    expect(r.losers).toEqual([0, 2])
  })

  it('skips agents whose gates failed', async () => {
    const sdk = fakeSdk()
    const run: RunState = {
      id: 'r3',
      task: 't',
      cwd: '/x',
      startedAt: 1,
      agents: [
        {
          agent: { kind: 'claude' },
          status: 'finished',
          diff: 'diff --git a b\n',
          gateResults: { tests: { ok: false, reason: 'unit failed' } },
        },
        {
          agent: { kind: 'codex' },
          status: 'finished',
          diff: 'diff --git e f\n',
          gateResults: { tests: { ok: true } },
        },
      ],
    }
    const r = await mergeRun(sdk, run)
    expect(r.ok).toBe(true)
    expect(r.winner?.index).toBe(1)
  })
})
