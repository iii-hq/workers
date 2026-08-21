/**
 * The computer worker in the command palette, before its page is even open.
 *
 * A sessions source answers any query with live desktop sessions read from
 * `computer::sessions::list` (the same read useSessionsLive bootstraps
 * from), each row opening that session in the computer page. Registered
 * from setup, so it exists only while the worker is connected; older
 * consoles without host.palette / host.commands simply get nothing.
 */

import type { Host } from '@iii-dev/console-ui'
import { listSessions } from '../lib/computer'

const SESSION_ROWS = 30

export function registerComputerPalette(host: Host): void {
  host.palette?.registerSource({
    id: 'computer-sessions',
    title: 'Computer sessions',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const sessions = await listSessions(host.iii)
      if (signal.aborted) return []
      const needle = query.toLowerCase()
      return sessions
        .filter(
          (session) =>
            session.session_id.toLowerCase().includes(needle) ||
            session.endpoint.toLowerCase().includes(needle) ||
            session.os.toLowerCase().includes(needle),
        )
        .slice(0, SESSION_ROWS)
        .map((session) => ({
          id: session.session_id,
          title: session.session_id,
          detail: `${session.endpoint} · ${session.os}`,
          keywords: [session.os],
          run: () =>
            host.panels?.open({
              pageId: 'computer',
              context: { sessionId: session.session_id },
            }),
        }))
    },
  })

  host.commands?.register('computer', [
    {
      id: 'open',
      title: 'Open the computer',
      detail: 'Live desktops you can watch and drive',
      keywords: ['desktop', 'sessions', 'screencast'],
      run: () => host.panels?.open({ pageId: 'computer', context: {} }),
    },
  ])
}
