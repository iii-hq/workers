/**
 * Entry for the workflow worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served as a
 * `script` asset through `ade::ui::asset` (see src/ui.rs). The stylesheet is
 * its own `style` asset: ./styles.css ships as workflow/styles.css.
 *
 * `setup(host)` composes the worker's console contributions:
 *
 * - src/function-call-message/ — the `workflow::*` chat cards (start, status,
 *   node-result, stop).
 * - src/context-section/       — the Workflow section of the chat's Context
 *   panel. The slot is newer than some consoles, so it is feature-detected:
 *   an older console simply gets no section.
 * - src/message-block/         — a node session's `<workflow_input>` as a
 *   card (the chat's message-block slot), feature-detected the same way:
 *   an older console shows the raw tag.
 *
 * Registrations go through `host` so the loader disposes them on hot reload /
 * worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { registerWorkflowSection } from './src/context-section'
import { registerWorkflowRenderer } from './src/function-call-message'
import { registerWorkflowInputBlock } from './src/message-block'

export default function setup(host: Host) {
  const removers = [
    registerWorkflowRenderer(host),
    registerWorkflowSection(host),
    registerWorkflowInputBlock(host),
  ].filter((remove) => remove !== undefined)

  // The loader already disposes every registration; returning the removers
  // makes an early teardown (hot reload mid-session) explicit and ordered.
  return () => {
    for (const remove of removers) remove()
  }
}
