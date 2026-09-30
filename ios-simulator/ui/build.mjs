import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({ scope: 'ios-simulator', keyframePrefixes: ['ios-ui-'], lint: { strict: true } })
