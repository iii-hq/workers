import type { Host } from '@iii-dev/console-ui'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { BROWSER_SESSION_UPDATED_TRIGGER } from '../lib/browser'
import { useBrowserSessionsLive } from './useBrowserSessionsLive'

const live = vi.hoisted(() => ({
  options: [] as unknown[],
}))

vi.mock('@iii-dev/console-ui/hooks', () => ({
  useWorkerLive: (options: unknown) => {
    live.options.push(options)
    return {
      data: [{ session_id: 'b1', title: 'Inbox' }],
      loading: false,
      error: null,
      live: true,
      refresh: () => {},
    }
  },
}))

function Probe({ host }: { host: Host }) {
  const { sessions, live: isLive } = useBrowserSessionsLive(host)
  return (
    <span>
      {sessions.map((s) => s.title).join(',')}:{String(isLive)}
    </span>
  )
}

afterEach(() => {
  live.options.length = 0
  vi.restoreAllMocks()
})

describe('useBrowserSessionsLive', () => {
  it('re-reads on the lifecycle triggers, session-updated included, with no timer', () => {
    const setInterval = vi.spyOn(globalThis, 'setInterval')
    const setTimeout = vi.spyOn(globalThis, 'setTimeout')
    const host = { iii: {} } as unknown as Host
    const html = renderToStaticMarkup(<Probe host={host} />)
    expect(html).toContain('Inbox:true')
    const options = live.options[0] as { triggers: readonly unknown[]; pollMs?: number }
    // Titles a page sets after it loaded arrive as session-updated.
    expect(options.triggers).toContain(BROWSER_SESSION_UPDATED_TRIGGER)
    expect(options.pollMs).toBeUndefined()
    expect(setInterval).not.toHaveBeenCalled()
    expect(setTimeout).not.toHaveBeenCalled()
  })
})
