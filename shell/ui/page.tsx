/**
 * Entry for the shell worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served
 * over the `console:script` trigger (see src/ui.rs). The stylesheet is
 * its own asset: ./styles.css ships over `console:style` as
 * shell/styles.css.
 *
 * `setup(host)` composes the worker's two console contributions:
 *
 * - src/page/             — the shell explorer (#/ext/shell): file tree +
 *                           git + search sidebar beside the shared Monaco
 *                           editor / FileDiff pane
 * - src/function-trigger/ — how shell::* function triggers render in
 *                           chat/traces (moved out of the console SPA)
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host, PageRenderProps } from '@iii-dev/console-ui'
import { createShellTriggerRenderer } from './src/function-trigger'
import { createAgentRunRenderer } from './src/function-trigger/AgentRunView'
import { createFileChangesRenderer } from './src/function-trigger/FileChangesView'
import { ShellExplorerPage } from './src/page'
import { ShellTurnSummary } from './src/page/ShellTurnSummary'

export default function setup(host: Host) {
  host.pages.register({
    id: 'shell',
    title: 'shell',
    render: (props: PageRenderProps) => <ShellExplorerPage host={host} {...props} />,
  })

  // File mutations own a prominent chat artifact; register them before the
  // general shell renderer so their non-null card and display metadata win.
  // Agent runs are terminal-shaped work, so the terminal owner claims them —
  // one control here instead of the same button in every agent worker.
  host.functionTriggers.register(createAgentRunRenderer(host))
  host.functionTriggers.register(createFileChangesRenderer(host))
  host.functionTriggers.register(createShellTriggerRenderer())

  host.chat?.registerTurnSummary?.({
    id: 'shell-last-turn',
    render: ShellTurnSummary,
  })
}
