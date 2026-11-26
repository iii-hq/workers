/**
 * Entry for the image-resize worker's injected console UI: compiled by
 * esbuild (react and @iii-dev/console-ui external) into dist/page.js and
 * served as a `script` asset (see src/ui.rs); ./styles.css ships as its own
 * `style` asset. Registrations go through `host` so the loader disposes them
 * on hot reload and worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { ImageResizePage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'image-resize',
    title: 'Image resize',
    configurationId: 'image-resize',
    render: (props) => <ImageResizePage host={host} {...props} />,
  })

  host.commands?.register('image-resize', [
    {
      id: 'open',
      title: 'Open Image resize',
      detail: 'Resize an image and compare it with the original',
      keywords: ['thumbnail', 'jpeg', 'png', 'webp', 'crop'],
      run: () => host.panels?.open({ pageId: 'image-resize', context: {} }),
    },
  ])
}
