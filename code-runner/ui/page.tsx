/**
 * Entry for the code-runner worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external into dist/page.js) and served through an
 * observer of `console::ui::asset` with `kind: 'script'` (see src/ui.rs). The
 * stylesheet is its own asset: ./styles.css ships with `kind: 'style'` as
 * code-runner/styles.css — the
 * console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * The worker's console contributions, one module each:
 *
 * - src/function-call-message/ — the per-op cards (run, register_function,
 *   teardown)
 * - src/configuration/           — deliberate form for the `code-runner`
 *   configuration entry in global Settings
 * - src/lib/shared.tsx            — the frame the cards share, over the
 *   package's terminal atoms
 *
 * Registrations go through `host` so the loader disposes them on hot reload /
 * worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { CodeRunnerConfigForm } from './src/configuration'
import { createCodeRunnerRenderers } from './src/function-call-message'

export default function setup(host: Host) {
  const removers = createCodeRunnerRenderers(host).map((renderer) => host.functionCalls.register(renderer))

  removers.push(host.configForms.register('code-runner', CodeRunnerConfigForm))

  // The loader already disposes every registration; returning the removers
  // makes an early teardown (hot reload mid-session) explicit and ordered.
  return () => {
    for (const remove of removers) remove()
  }
}
