/**
 * The state worker in the command palette, before its page is even open.
 *
 * A keys source answers with every scope's keys by name, each row opening
 * the page with that scope/key selected. Registered from setup, so it
 * exists only while the worker is connected; older consoles without
 * `host.palette` / `host.commands` simply get nothing.
 */

import type { Host } from '@iii-dev/console-ui'

const ROWS = 30

export function registerStatePalette(host: Host): void {
  host.palette?.registerSource({
    id: 'state-keys',
    title: 'State keys',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const needle = query.trim().toLowerCase()
      const { groups } = await host.iii.trigger<{ groups: string[] }>(
        'state::list_groups',
        {},
      )
      if (signal.aborted) return []
      const out: { scope: string; key: string }[] = []
      for (const scope of groups) {
        if (signal.aborted) return []
        const { keys } = await host.iii
          .trigger<{ keys: string[] }>('state::list_keys', { scope })
          .catch(() => ({ keys: [] as string[] }))
        for (const key of keys) {
          if (key.toLowerCase().includes(needle)) out.push({ scope, key })
        }
      }
      if (signal.aborted) return []
      return out.slice(0, ROWS).map(({ scope, key }) => ({
        id: `${scope} ${key}`,
        title: key,
        detail: scope,
        keywords: [scope, key],
        run: () =>
          host.panels?.open({
            pageId: 'state-manager',
            context: { scope, key },
          }),
      }))
    },
  })

  host.commands?.register('state-manager', [
    {
      id: 'open',
      title: 'Open state',
      detail: 'Scoped key–value store, live',
      keywords: ['kv', 'state', 'store'],
      run: () => host.panels?.open({ pageId: 'state-manager', context: {} }),
    },
  ])
}
