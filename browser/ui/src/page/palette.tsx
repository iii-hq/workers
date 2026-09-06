/**
 * The browser worker in the command palette, before its page is even open.
 *
 * A sessions source answers any query with live Chromium sessions read the
 * same way the rail does (`browser::sessions::list`, `useBrowserSessionsLive`'s
 * backing function), each row opening that session in the browser page.
 * Registered from setup, so it exists only while the browser worker is
 * connected; older consoles without host.palette / host.commands simply get
 * nothing.
 */

import type { Host } from '@iii-dev/console-ui'
import { listBrowserSessions } from '../lib/browser'
import { formatMtime } from '../lib/format'
import { listAnnotationSets } from './annotations-store'

const SESSION_ROWS = 30

export function registerBrowserPalette(host: Host): void {
  host.palette?.registerSource({
    id: 'sessions',
    title: 'Browser sessions',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const sessions = await listBrowserSessions(host.iii)
      if (signal.aborted) return []
      const needle = query.toLowerCase()
      return sessions
        .filter(
          (session) =>
            session.title?.toLowerCase().includes(needle) ||
            session.url.toLowerCase().includes(needle) ||
            session.session_id.toLowerCase().includes(needle),
        )
        .slice(0, SESSION_ROWS)
        .map((session) => ({
          id: session.session_id,
          title: session.title?.trim() || session.url,
          detail: `${session.incognito ? 'incognito · ' : ''}${session.active === false ? 'asleep · ' : ''}${session.url}`,
          keywords: [session.session_id],
          run: () =>
            host.panels?.open({
              pageId: 'browser',
              context: { type: 'session', sessionId: session.session_id },
            }),
        }))
    },
  })

  host.palette?.registerSource({
    id: 'annotation-sets',
    title: 'Saved annotations',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const sets = await listAnnotationSets(host.iii, signal)
      if (signal.aborted) return []
      const needle = query.toLowerCase()
      return sets
        .filter((set) => set.subject.toLowerCase().includes(needle))
        .slice(0, 20)
        .map((set) => ({
          id: set.key,
          title: `Annotations on ${set.subject}`,
          detail: `${set.count} ${set.count === 1 ? 'mark' : 'marks'} · ${formatMtime(Math.floor(set.capturedAt / 1000))}`,
          keywords: ['annotations', 'saved'],
          run: () =>
            host.panels?.open({
              pageId: 'browser',
              context: { type: 'saved-set', key: set.key },
            }),
        }))
    },
  })

  host.commands?.register('browser', [
    {
      id: 'open',
      title: 'Open the browser',
      detail: 'Tabs you can watch and drive, shared with agents',
      keywords: ['chromium', 'sessions', 'tabs', 'screencast'],
      run: () => host.panels?.open({ pageId: 'browser', context: {} }),
    },
    {
      id: 'new-tab',
      title: 'New browser tab',
      keywords: ['browser', 'tab', 'open'],
      run: () => host.panels?.open({ pageId: 'browser', context: { type: 'new-tab' } }),
    },
    {
      id: 'new-incognito-tab',
      title: 'New incognito tab',
      detail: 'A private tab: nothing saved, closes when idle',
      keywords: ['browser', 'tab', 'private', 'incognito'],
      run: () => host.panels?.open({ pageId: 'browser', context: { type: 'new-tab', incognito: true } }),
    },
  ])
}
