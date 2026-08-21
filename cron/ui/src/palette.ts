/**
 * The cron worker in the command palette, before its page is even open.
 *
 * A schedules source answers with every schedule by label, each row opening
 * the page on that schedule's detail. Registered from setup, so it exists
 * only while the worker is connected; older consoles without `host.palette`
 * / `host.commands` simply get nothing.
 */

import type { Host } from '@iii-dev/console-ui'
import { listAllSchedules } from './lib/api'

const ROWS = 30

export function registerCronPalette(host: Host): void {
  host.palette?.registerSource({
    id: 'cron-schedules',
    title: 'Schedules',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const needle = query.trim().toLowerCase()
      const tasks = await listAllSchedules(host)
      if (signal.aborted) return []
      return tasks
        .filter((task) => (task.label ?? '').toLowerCase().includes(needle))
        .slice(0, ROWS)
        .map((task) => ({
          id: task.subscriptionId,
          title: task.label ?? 'Untitled schedule',
          detail: task.expression,
          keywords: [task.expression],
          run: () =>
            host.panels?.open({
              pageId: 'cron',
              context: {
                action: 'schedule',
                subscriptionId: task.subscriptionId,
              },
            }),
        }))
    },
  })

  host.commands?.register('cron', [
    {
      id: 'open',
      title: 'Open schedules',
      detail: 'Natural-language and manual cron schedules, and system bindings',
      keywords: ['cron', 'schedule', 'tasks'],
      run: () => host.panels?.open({ pageId: 'cron', context: {} }),
    },
    {
      id: 'new-schedule',
      title: 'New schedule…',
      detail: 'Open the schedule composer',
      keywords: ['cron', 'create', 'add'],
      run: () =>
        host.panels?.open({ pageId: 'cron', context: { action: 'new' } }),
    },
  ])
}
