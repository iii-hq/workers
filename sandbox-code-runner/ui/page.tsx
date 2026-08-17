/**
 * Entry for the sandbox-code-runner worker's injected console UI — compiled
 * by esbuild (react + @iii-dev/console-ui external) into dist/page.js and
 * served over the `console:script` trigger (see src/ui.rs). The stylesheet
 * is its own asset: ./styles.css ships over `console:style` as
 * sandbox-code-runner/styles.css — the console mounts and link-swaps it,
 * styles-before-scripts on boot.
 *
 * The worker's console contributions:
 *
 * - src/function-trigger-message/ — the per-op cards (run, register_function,
 *   teardown)
 * - src/sandbox-family/           — the sandbox::* daemon family cards
 *   (formerly first-party in the console)
 * - src/page/                     — the sandbox fleet page (#/ext/sandbox),
 *   the worker config form, and the session chip
 * - src/lib/shared.tsx            — the frame the op cards share
 *
 * Registrations go through `host` so the loader disposes them on hot reload /
 * worker disconnect. The chat slot is feature-detected: older consoles
 * without session chips still load everything else.
 */

import type { Host } from '@iii-dev/console-ui'
import { createSandboxCodeRunnerRenderers } from './src/function-trigger-message'
import { SandboxConfigForm, SandboxPage, createSandboxSessionChip } from './src/page'
import { createSandboxFamilyRenderer } from './src/sandbox-family'

export default function setup(host: Host) {
  const removers = [
    ...createSandboxCodeRunnerRenderers(host).map((renderer) =>
      host.functionTriggers.register(renderer),
    ),
    host.functionTriggers.register(createSandboxFamilyRenderer(host)),
    host.pages.register({
      id: 'sandbox',
      title: 'sandbox',
      render: (props) => <SandboxPage host={host} {...props} />,
    }),
    host.configForms.register('sandbox-code-runner', SandboxConfigForm),
    host.chat?.registerSessionChip(createSandboxSessionChip(host)),
  ].filter((remove) => remove !== undefined)

  // The loader already disposes every registration; returning the removers
  // makes an early teardown (hot reload mid-session) explicit and ordered.
  return () => {
    for (const remove of removers) remove()
  }
}
