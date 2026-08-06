/**
 * Entry for the harness worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as harness/styles.css.
 *
 * `setup(host)` composes the worker's two console contributions:
 *
 * - src/context-chip/             — the `context` session chip (live context
 *   window usage). The `chat.registerSessionChip` slot is newer than the
 *   published Host type, so it is feature-detected: an older console simply
 *   gets no chip.
 * - src/function-trigger-message/ — how `harness::metrics` calls render.
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { ComponentType } from 'react'
import type { Host } from '@iii-dev/console-ui'
import { createContextChip, type SessionChipProps } from './src/context-chip'
import { createMetricsRenderer } from './src/function-trigger-message'

type SessionChipHost = Host & {
  chat?: {
    registerSessionChip?: (chip: {
      id: string
      render: ComponentType<SessionChipProps>
    }) => () => void
  }
}

export default function setup(host: Host) {
  const chat = (host as SessionChipHost).chat
  chat?.registerSessionChip?.({ id: 'context', render: createContextChip(host) })

  host.functionTriggers.register(createMetricsRenderer())
}
