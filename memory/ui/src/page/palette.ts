/**
 * The memory worker in the command palette, before its page is even open.
 *
 * Two live sources answer a query: banks by name, and memories by their
 * text/tags across every bank (there is no cross-bank search function, so
 * this fetches each bank's page and filters client-side — bounded by
 * MEMORIES_PER_BANK and the shared row cap). Both open the page selecting
 * the matched row. Registered from setup, so they exist only while the
 * memory worker is connected; older consoles without `host.palette` /
 * `host.commands` simply get nothing.
 */

import type { Host, PaletteSourceRow } from '@iii-dev/console-ui'
import { listBanks, listMemories } from './memory-data'

const ROWS = 30
const MEMORIES_PER_BANK = 100

export function registerMemoryPalette(host: Host): void {
  host.palette?.registerSource({
    id: 'memory-banks',
    title: 'Memory banks',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const banks = await listBanks(host)
      if (signal.aborted) return []
      const needle = query.toLowerCase()
      return banks
        .filter(
          (bank) =>
            bank.name.toLowerCase().includes(needle) ||
            bank.description.toLowerCase().includes(needle),
        )
        .slice(0, ROWS)
        .map((bank) => ({
          id: bank.name,
          title: bank.name,
          detail:
            bank.description ||
            `${bank.memories} memories · ${bank.rules} rules`,
          run: () =>
            host.panels?.open({
              pageId: 'memory',
              context: { type: 'bank', bank: bank.name },
            }),
        }))
    },
  })

  host.palette?.registerSource({
    id: 'memories',
    title: 'Memories',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const banks = await listBanks(host)
      if (signal.aborted) return []
      const needle = query.toLowerCase()
      const rows: PaletteSourceRow[] = []
      for (const bank of banks) {
        if (rows.length >= ROWS) break
        const { memories } = await listMemories(
          host,
          bank.name,
          false,
          0,
          MEMORIES_PER_BANK,
        )
        if (signal.aborted) return []
        for (const memory of memories) {
          if (rows.length >= ROWS) break
          const haystack =
            `${memory.text} ${memory.tags.join(' ')}`.toLowerCase()
          if (!haystack.includes(needle)) continue
          rows.push({
            id: `${bank.name}:${memory.id}`,
            title:
              memory.text.length > 88
                ? `${memory.text.slice(0, 88)}…`
                : memory.text,
            detail: bank.name,
            keywords: memory.tags,
            run: () =>
              host.panels?.open({
                pageId: 'memory',
                context: { type: 'memory', bank: bank.name, id: memory.id },
              }),
          })
        }
      }
      return rows
    },
  })

  host.commands?.register('memory', [
    {
      id: 'open',
      title: 'Open memory',
      detail: 'Banks of rules and memories, injected per turn',
      keywords: ['banks', 'rules', 'recall'],
      run: () => host.panels?.open({ pageId: 'memory', context: {} }),
    },
  ])
}
