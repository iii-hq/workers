import type { Host } from '@iii-dev/console-ui'
import { vi } from 'vitest'
import type { Status } from '../types'

export type Call = { id: string; payload: Record<string, unknown> | undefined }

/** A fake host: `iii.call` answers `status` from the fixture (or a function, read on every call) and records every call. */
export function fakeHost(
  status: Status | (() => Status),
  answers: Record<string, unknown | ((p: Record<string, unknown>) => unknown)> = {},
) {
  const calls: Call[] = []
  const handlers = new Map<string, (e: unknown) => void>()
  const observed: { source: unknown; target: string }[] = []
  const compose = vi.fn()
  const open = vi.fn()
  const host = {
    iii: {
      browserId: 'console-test',
      call: vi.fn(async (id: string, payload?: Record<string, unknown>) => {
        calls.push({ id, payload })
        if (id === 'iii-cloud::status') return typeof status === 'function' ? status() : status
        const a = answers[id]
        if (a instanceof Error) throw a
        return typeof a === 'function' ? a(payload ?? {}) : (a ?? {})
      }),
      on: vi.fn((id: string, h: (e: unknown) => void) => {
        handlers.set(`${id}::console-test`, h)
        return () => handlers.delete(`${id}::console-test`)
      }),
      observe: vi.fn((source: unknown, target: string) => {
        observed.push({ source, target })
        return () => undefined
      }),
    },
    components: {} as Record<string, unknown>,
    pages: { register: vi.fn() },
    functionCalls: { register: vi.fn() },
    configForms: { register: vi.fn() },
    commands: { register: vi.fn() },
    panels: { open },
    chat: { compose },
  } as unknown as Host
  return { host, calls, handlers, observed, compose, open }
}
