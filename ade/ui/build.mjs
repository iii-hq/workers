import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

// Asset paths keep console/…; the content function uses the ade:: namespace.
await buildWorkerUi({
  scope: 'console',
  entryPoints: ['config-form.tsx', 'catalog-page.tsx', 'workspace-proposal.tsx', 'styles.css'],
  lint: { strict: true },
})
