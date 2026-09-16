import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({ scope: 'database', keyframePrefixes: ['db-ui-', 'db-page-'] })
