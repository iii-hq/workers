import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'
import { tailwindPlugin } from './tailwind.mjs'

// The page's markup uses Tailwind utilities, compiled and scoped by tailwind.mjs (see there); the shared lint's
// tailwind-in-worker rule assumes a worker has no Tailwind build of its own, so it is off for this one.
await buildWorkerUi({
  scope: 'kanban',
  plugins: [tailwindPlugin],
  lint: { strict: true, disable: ['tailwind-in-worker'] },
})
