/**
 * Entry for the queue worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: styles.css ships over `console:style` as queue/styles.css.
 *
 * One contribution: the Queues page (#/ext/queues) — topics, stats,
 * publish, and the dead-letter queue with redrive/discard. It ships FROM
 * the queue worker because queues are queue-worker data: the page appears
 * when the worker connects and leaves with it.
 */

import type { Host, PageRenderProps } from '@iii-dev/console-ui'
import { QueuesPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'queues',
    title: 'queues',
    render: ({ panelSide, onRequestClose }: PageRenderProps) => (
      <QueuesPage
        host={host}
        side={panelSide}
        onRequestClose={onRequestClose}
      />
    ),
  })
}
