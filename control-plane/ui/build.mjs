import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({ scope: 'control-plane', lint: { strict: true } })
