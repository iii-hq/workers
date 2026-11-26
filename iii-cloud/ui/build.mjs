import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({ scope: 'iii-cloud', keyframePrefixes: ['iii-cloud-'], lint: { strict: true } })
