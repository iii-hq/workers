import { describe, expect, it, vi } from 'vitest'
import { paneConfigurationHash } from './pane-configuration'

describe('pane configuration routing', () => {
  it('opens the sole live dynamically named instance', async () => {
    await expect(
      paneConfigurationHash('browser', async () => [
        {
          id: 'browser-team-a',
          name: 'Browser',
          description: '',
          schema: {},
          metadata: { ui_form: 'browser' },
        },
      ]),
    ).resolves.toBe('#/configuration/workers/browser-team-a')
  })

  it('opens the worker list for a missing family', async () => {
    await expect(
      paneConfigurationHash('browser', async () => []),
    ).resolves.toBe('#/configuration/workers')
  })

  it('opens the worker list for an ambiguous family', async () => {
    await expect(
      paneConfigurationHash('browser', async () => [
        {
          id: 'browser-a',
          name: 'Browser',
          description: '',
          schema: {},
          metadata: { ui_form: 'browser' },
        },
        {
          id: 'browser-b',
          name: 'Browser',
          description: '',
          schema: {},
          metadata: { ui_form: 'browser' },
        },
      ]),
    ).resolves.toBe('#/configuration/workers')
  })

  it('falls back to the worker list when discovery fails', async () => {
    await expect(
      paneConfigurationHash('browser', async () => {
        throw new Error('offline')
      }),
    ).resolves.toBe('#/configuration/workers')
  })

  it('does not need to call an exact-id-specific API', async () => {
    const load = vi.fn(async () => [
      {
        id: 'browser',
        name: 'Browser',
        description: '',
        schema: {},
      },
    ])

    await expect(paneConfigurationHash('browser', load)).resolves.toBe(
      '#/configuration/workers/browser',
    )
    expect(load).toHaveBeenCalledOnce()
  })
})
