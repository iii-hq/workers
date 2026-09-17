import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

// The asset path is console/…: the ade worker kept its console:: ids on rename.
await buildWorkerUi({
  scope: 'console',
  entryPoints: ['config-form.tsx', 'catalog-page.tsx', 'workspace-proposal.tsx', 'styles.css'],
})
