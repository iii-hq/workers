/**
 * Entry for the database worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as database/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers one contribution: the function-trigger renderer
 * in src/function-trigger-message/ — how every database::* call renders in
 * chat and traces (SQL, request chips, result tables). No page and no
 * config form yet; those slots can be added here later.
 *
 * Registration goes through `host` so the loader disposes it on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { createDatabaseTriggerRenderer } from './src/function-trigger-message'

export default function setup(host: Host) {
  host.functionTriggers.register(createDatabaseTriggerRenderer(host))
}
