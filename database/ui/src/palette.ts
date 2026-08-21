/**
 * The database worker in the command palette, before its page is even open.
 *
 * A tables source answers with every configured database's tables and views
 * by name, each row opening the page with that table selected. Registered
 * from setup, so it exists only while the worker is connected; older
 * consoles without `host.palette` / `host.commands` simply get nothing.
 */

import type { Host } from '@iii-dev/console-ui'
import { listDbs, listTables } from './page/db-data'

const ROWS = 30

export function registerDatabasePalette(host: Host): void {
  host.palette?.registerSource({
    id: 'database-tables',
    title: 'Database tables',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const needle = query.trim().toLowerCase()
      const dbs = await listDbs(host)
      if (signal.aborted) return []
      const out: { db: string; table: string; kind: 'table' | 'view' }[] = []
      for (const db of dbs) {
        if (signal.aborted) return []
        const tables = await listTables(host, db.name, db.driver).catch(
          () => [],
        )
        for (const table of tables) {
          if (table.name.toLowerCase().includes(needle)) {
            out.push({ db: db.name, table: table.name, kind: table.kind })
          }
        }
      }
      if (signal.aborted) return []
      return out.slice(0, ROWS).map(({ db, table, kind }) => ({
        id: `${db}.${table}`,
        title: table,
        detail: dbs.length > 1 ? db : kind === 'view' ? 'view' : undefined,
        keywords: [db, table],
        run: () =>
          host.panels?.open({
            pageId: 'database',
            context: { db, table },
          }),
      }))
    },
  })

  host.commands?.register('database', [
    {
      id: 'open',
      title: 'Open the database browser',
      detail: 'Schema, table rows, and an ad-hoc SQL panel',
      keywords: ['sql', 'tables', 'schema', 'rows'],
      run: () => host.panels?.open({ pageId: 'database', context: {} }),
    },
  ])
}
