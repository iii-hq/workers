/**
 * The directory worker in the command palette, before its page is even
 * open. One combined source answers a query across all three collections
 * (skills, prompts, system prompts) — the worker has no unified search
 * function, so this fetches each collection's list and filters
 * client-side, the same as the in-page filter does. Rows open the page on
 * that collection with the entry selected (see the page's panelContext
 * handler). Registered from setup, so it exists only while the
 * iii-directory worker is connected; older consoles without
 * `host.palette` / `host.commands` simply get nothing.
 */

import type { Host, PaletteSourceRow } from '@iii-dev/console-ui'

const ROWS = 30

export type DirectoryCollection =
  | 'skills'
  | 'prompts'
  | 'system-prompts'
  | 'agents'

const COLLECTIONS: {
  id: DirectoryCollection
  label: string
  listFn: string
}[] = [
  { id: 'skills', label: 'Skills', listFn: 'directory::skills::list' },
  { id: 'prompts', label: 'Prompts', listFn: 'directory::prompts::list' },
  {
    id: 'system-prompts',
    label: 'System prompts',
    listFn: 'directory::system-prompts::list',
  },
  { id: 'agents', label: 'Agents', listFn: 'directory::agents::list' },
]

interface Row {
  key: string
  title: string
  description: string
}

async function listCollection(
  host: Host,
  collection: (typeof COLLECTIONS)[number],
): Promise<Row[]> {
  const payload =
    collection.id === 'skills' ? { include_description: true } : {}
  const out = await host.iii.trigger<Record<string, unknown>>(
    collection.listFn,
    payload,
  )
  // Skills answer `{ skills }`; prompts and system prompts both answer
  // `{ prompts }` (same shape, per PromptRow); agents answer `{ agents }`.
  const items = (out.skills ??
    out.prompts ??
    out.agents ??
    []) as Record<string, unknown>[]
  return items
    .map((item) => ({
      key: String(item.id ?? item.name ?? ''),
      title: String(item.title || item.name || item.id || ''),
      description: String(item.description ?? ''),
    }))
    .filter((row) => row.key !== '')
}

export function registerDirectoryPalette(host: Host): void {
  host.palette?.registerSource({
    id: 'directory-entries',
    title: 'Directory',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const needle = query.toLowerCase()
      const lists = await Promise.all(
        COLLECTIONS.map((collection) => listCollection(host, collection)),
      )
      if (signal.aborted) return []
      const rows: PaletteSourceRow[] = []
      for (let i = 0; i < COLLECTIONS.length; i++) {
        const collection = COLLECTIONS[i]
        for (const row of lists[i]) {
          if (rows.length >= ROWS) break
          const haystack =
            `${row.title} ${row.key} ${row.description}`.toLowerCase()
          if (!haystack.includes(needle)) continue
          rows.push({
            id: `${collection.id}:${row.key}`,
            title: row.title || row.key,
            detail: collection.label,
            keywords: [row.key],
            run: () =>
              host.panels?.open({
                pageId: 'directory',
                context: { collection: collection.id, key: row.key },
              }),
          })
        }
      }
      return rows
    },
  })

  host.commands?.register('directory', [
    {
      id: 'open',
      title: 'Open Directory',
      detail: 'Filesystem-backed skills, prompts, system prompts and agents',
      keywords: ['skills', 'prompts', 'system prompts', 'agents'],
      run: () => host.panels?.open({ pageId: 'directory', context: {} }),
    },
  ])
}
