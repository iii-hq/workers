/**
 * The storage worker in the command palette, before its page is even open.
 *
 * An objects source scans every configured bucket's keys, each row opening
 * the page at that object's bucket/folder. Registered from setup, so it
 * exists only while the worker is connected; older consoles without
 * `host.palette` / `host.commands` simply get nothing.
 */

import type { Host } from '@iii-dev/console-ui'
import { leafName, parentPrefix } from './widgets'

const ROWS = 30
/** Objects scanned per bucket — a flat, undelimited listing, so this is a
    best-effort search over the first page of each bucket, not every key. */
const SCAN_LIMIT = 200

interface BucketSummary {
  name: string
  provider: string
}

interface StorageObject {
  key: string
}

interface Listing {
  objects: StorageObject[]
  common_prefixes: string[]
  next_cursor?: string | null
}

export function registerStoragePalette(host: Host): void {
  host.palette?.registerSource({
    id: 'storage-objects',
    title: 'Storage objects',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const needle = query.trim().toLowerCase()
      const { buckets } = await host.iii.trigger<{ buckets: BucketSummary[] }>(
        'storage::listBuckets',
        {},
      )
      if (signal.aborted) return []
      const out: { bucket: string; key: string }[] = []
      for (const bucket of buckets) {
        if (signal.aborted) return []
        const listing = await host.iii
          .trigger<Listing>('storage::listObjects', {
            bucket: bucket.name,
            prefix: '',
            limit: SCAN_LIMIT,
          })
          .catch((): Listing => ({ objects: [], common_prefixes: [] }))
        for (const object of listing.objects) {
          if (object.key.toLowerCase().includes(needle)) {
            out.push({ bucket: bucket.name, key: object.key })
          }
        }
      }
      if (signal.aborted) return []
      return out.slice(0, ROWS).map(({ bucket, key }) => ({
        id: `${bucket}/${key}`,
        title: leafName(key),
        detail: `${bucket} · ${parentPrefix(key) || '/'}`,
        keywords: [bucket, key],
        run: () =>
          host.panels?.open({
            pageId: 'storage-manager',
            context: { bucket, prefix: parentPrefix(key), objectKey: key },
          }),
      }))
    },
  })

  host.commands?.register('storage-manager', [
    {
      id: 'open',
      title: 'Open storage',
      detail: 'Buckets, folders, and objects',
      keywords: ['bucket', 'object', 'file', 'upload'],
      run: () => host.panels?.open({ pageId: 'storage-manager', context: {} }),
    },
  ])
}
