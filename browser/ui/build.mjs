import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({ scope: 'browser', keyframePrefixes: ['br-ui-'], lint: { strict: true } })
