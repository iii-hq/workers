import { describe, expect, it, vi } from 'vitest'
import { activate, type ConsoleExtensionHost } from './extension'

describe('approval-gate console extension', () => {
  it('registers every supported console slot and disposes them', () => {
    const contributions: Array<{ id: string; slot: string; mount: unknown }> =
      []
    const disposed: string[] = []
    const host: ConsoleExtensionHost = {
      apiVersion: 1,
      browserId: 'test-browser',
      extension: { id: 'approval-gate', workerVersion: 'test' },
      registerSlot(contribution) {
        contributions.push(contribution)
        return () => disposed.push(contribution.id)
      },
      trigger: vi.fn(),
      on: vi.fn(),
      registerTrigger: vi.fn(),
    }

    const extension = activate(host)

    expect(contributions.map((contribution) => contribution.slot)).toEqual([
      'chat.composer.controls',
      'chat.banner',
      'function-call.pending-actions',
      'settings.sections',
      'chat.workspace-access',
    ])
    expect(
      contributions.every(
        (contribution) => typeof contribution.mount === 'function',
      ),
    ).toBe(true)

    extension.dispose()
    expect(disposed.sort()).toEqual(
      contributions.map((contribution) => contribution.id).sort(),
    )
  })

  it('rejects an unsupported host API', () => {
    expect(() =>
      activate({
        apiVersion: 2,
        browserId: 'test-browser',
        extension: { id: 'approval-gate', workerVersion: 'test' },
        registerSlot: vi.fn(),
        trigger: vi.fn(),
        on: vi.fn(),
        registerTrigger: vi.fn(),
      }),
    ).toThrow('requires console extension API v1')
  })
})
