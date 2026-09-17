import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({ scope: 'github', keyframePrefixes: ['gh-ui-'], lint: { strict: true } })
