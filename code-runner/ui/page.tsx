/**
 * Entry for the code-runner worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external into dist/page.js) and served over the
 * `console:script` trigger (see src/ui.rs). The stylesheet is its own asset:
 * ./styles.css ships over `console:style` as code-runner/styles.css — the
 * console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * The worker's only console contribution is how its function triggers render:
 *
 * - src/function-trigger-message/ — the per-op cards (run, register_function,
 *   teardown)
 * - src/lib/shared.tsx            — the frame those cards share
 *
 * Registrations go through `host` so the loader disposes them on hot reload /
 * worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { createCodeRunnerRenderers } from './src/function-trigger-message'

export default function setup(host: Host) {
  const removers = createCodeRunnerRenderers(host).map((renderer) => host.functionTriggers.register(renderer))
  // The loader already disposes every registration; returning the removers
  // makes an early teardown (hot reload mid-session) explicit and ordered.
  return () => {
    for (const remove of removers) remove()
  }
}
