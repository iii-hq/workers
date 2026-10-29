/**
 * Entry for the console worker's engine-catalogue pages — compiled by
 * esbuild (react + @iii-dev/console-ui external) into dist/catalog-page.js
 * and shipped as a `script` asset of `console::ui::asset` (see src/ui.rs).
 * The stylesheet is its own `style` asset: console/styles.css.
 *
 * Two contributions, both reading engine-level data no single worker owns:
 *
 * - src/catalog/FunctionsPage — every registered function, its schemas, an
 *                               invoke panel, its observers, and its live
 *                               call feed (page `functions`)
 * - src/catalog/ObserversPage — every observer, grouped by the function it
 *                               observes, each with its family's real fire
 *                               path (page `observers`)
 *
 * Both observe engine sources (`engine::on-functions-changed`,
 * `engine::on-workers-changed`, `engine::on-observers-changed`,
 * `observability::on-span`) rather than polling timers, so they are live
 * without polling. Per-worker drill-down lives on the console's own Workers
 * page, which expands a row into that worker's functions and observers.
 *
 * They ship as injected UI rather than console pages so the console SPA
 * keeps no per-view code: this bundle can be rebuilt, hot-reloaded, and
 * toggled off without touching the host.
 */

import type { Host, PageRenderProps } from '@iii-dev/console-ui'
import { FunctionsPage } from './src/catalog/FunctionsPage'
import { ObserversPage } from './src/catalog/ObserversPage'

export default function setup(host: Host) {
  host.pages.register({
    id: 'functions',
    title: 'functions',
    render: ({ panelSide, onRequestClose, commands }: PageRenderProps) => (
      <FunctionsPage
        host={host}
        side={panelSide}
        onRequestClose={onRequestClose}
        commands={commands}
      />
    ),
  })

  host.pages.register({
    id: 'observers',
    title: 'observers',
    render: ({ panelSide, onRequestClose, commands }: PageRenderProps) => (
      <ObserversPage
        host={host}
        side={panelSide}
        onRequestClose={onRequestClose}
        commands={commands}
      />
    ),
  })

  host.commands?.register('functions', [
    {
      id: 'open',
      title: 'Open functions',
      detail: 'Every registered function, grouped by worker',
      keywords: ['catalog', 'invoke', 'call'],
      run: () => host.panels?.open({ pageId: 'functions', context: {} }),
    },
  ])

  host.commands?.register('observers', [
    {
      id: 'open',
      title: 'Open observers',
      detail: 'Observers and their sources',
      keywords: ['catalog', 'sources', 'events'],
      run: () => host.panels?.open({ pageId: 'observers', context: {} }),
    },
  ])
}
