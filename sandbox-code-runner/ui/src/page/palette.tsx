/**
 * The sandbox worker in the command palette, before its page is even open.
 *
 * A sandboxes source answers any query with the live fleet — the same
 * `sandbox::list` read useFleet's store bootstraps from — each row opening
 * the sandbox page with that sandbox selected. Registered from setup, so it
 * exists only while the worker is connected; older consoles without
 * host.palette / host.commands simply get nothing.
 */

import type { Host } from '@iii-dev/console-ui'
import { LIST_FN, parseSandboxList } from './store'

const SANDBOX_ROWS = 30

export function registerSandboxPalette(host: Host): void {
  host.palette?.registerSource({
    id: 'sandboxes',
    title: 'Sandboxes',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const sandboxes = parseSandboxList(await host.iii.trigger(LIST_FN, {}))
      if (signal.aborted) return []
      const needle = query.toLowerCase()
      return sandboxes
        .filter(
          (sandbox) =>
            (sandbox.name?.toLowerCase().includes(needle) ?? false) ||
            sandbox.sandbox_id.toLowerCase().includes(needle) ||
            sandbox.image.toLowerCase().includes(needle),
        )
        .slice(0, SANDBOX_ROWS)
        .map((sandbox) => ({
          id: sandbox.sandbox_id,
          title: sandbox.name || sandbox.sandbox_id,
          detail: sandbox.image,
          keywords: [sandbox.sandbox_id],
          run: () =>
            host.panels?.open({
              pageId: 'sandbox',
              context: { sandboxId: sandbox.sandbox_id },
            }),
        }))
    },
  })

  host.commands?.register('sandbox', [
    {
      id: 'open',
      title: 'Open the sandbox fleet',
      detail: 'MicroVM fleet, exec console, files',
      keywords: ['microvm', 'fleet', 'exec'],
      run: () => host.panels?.open({ pageId: 'sandbox', context: {} }),
    },
  ])
}
