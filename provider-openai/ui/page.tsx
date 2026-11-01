/**
 * Entry for the provider-openai injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * a `script` asset of `console::ui::asset` (see src/ui.rs); ../styles.css
 * ships as provider-openai/styles.css, a `style` asset.
 *
 * One contribution: how `provider::openai::image::generate` / `image::read`
 * render in chat. `generate` returns only the saved path (no base64 in the
 * transcript), so the card fetches its preview from the worker itself.
 */

import type { Host } from '@iii-dev/console-ui'
import { createImageRenderer } from './src/image-renderer'

export default function setup(host: Host) {
  host.functionCalls.register(createImageRenderer(host))
}
