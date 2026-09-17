import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({ scope: 'computer', keyframePrefixes: ['cp-ui-'], lint: { strict: true } })
