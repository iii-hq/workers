import { QueryClient } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'
import * as config from '@/lib/console-config'
import {
  CONSOLE_CONFIG_QUERY_KEY,
  consoleConfigWriter,
} from './console-config-writer'

describe('consoleConfigWriter', () => {
  it('hands every hook of one query client the same writer', () => {
    const qc = new QueryClient()
    expect(consoleConfigWriter(qc)).toBe(consoleConfigWriter(qc))
    expect(consoleConfigWriter(new QueryClient())).not.toBe(
      consoleConfigWriter(qc),
    )
  })

  it('serializes two writers that used to race, so neither loses the other', async () => {
    let remote: Record<string, unknown> = { workspace: { columns: 2 } }
    const reads: number[] = []
    vi.spyOn(config, 'fetchConsoleConfigValue').mockImplementation(async () => {
      reads.push(Date.now())
      return remote
    })
    vi.spyOn(config, 'setConsoleConfigValue').mockImplementation(
      async (value) => {
        remote = value as Record<string, unknown>
      },
    )
    const qc = new QueryClient()
    const writer = consoleConfigWriter(qc)
    // The workspace splits a pane while the traces page records a view.
    writer.enqueue((value) => ({ ...value, workspace: { columns: 3 } }))
    writer.enqueue((value) => ({ ...value, traces: { activeView: 'v1' } }))
    await writer.whenIdle()
    expect(remote).toEqual({
      workspace: { columns: 3 },
      traces: { activeView: 'v1' },
    })
    expect(qc.getQueryData(CONSOLE_CONFIG_QUERY_KEY)).toEqual(remote)
    vi.restoreAllMocks()
  })
})
