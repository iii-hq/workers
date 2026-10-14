import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({
  scope: 'tmux',
  // xterm.ts → dist/xterm.js, the console:module the terminal imports lazily.
  entryPoints: ['page.tsx', 'styles.css', 'xterm.ts'],
  // styles.css @imports @xterm/xterm's own sheet, whose `.xterm*` rules are
  // unscoped vendor CSS the terminal needs as-is.
  allowUnscopedSelectors: ['.xterm'],
  lint: { strict: true },
})
