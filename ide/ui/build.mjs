import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

// The asset scope is `ide` (page id, configuration id and data-iii-ui alike); only the shell:: function ids kept their name.
await buildWorkerUi({
  scope: 'ide',
  keyframePrefixes: ['shui-'],
  // styles.css @imports @xterm/xterm's own sheet, whose `.xterm*` rules are
  // unscoped vendor CSS the terminal needs as-is.
  allowUnscopedSelectors: ['.xterm'],
  lint: { strict: true },
})
