import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

// The asset path is shell/…: the ide worker kept its shell:: ids on rename.
await buildWorkerUi({
  scope: 'shell',
  keyframePrefixes: ['shui-'],
  // styles.css @imports @xterm/xterm's own sheet, whose `.xterm*` rules are
  // unscoped vendor CSS the terminal needs as-is.
  allowUnscopedSelectors: ['.xterm'],
  lint: { strict: true },
})
