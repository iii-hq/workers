import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({ scope: 'mesh', outdir: '../assets', lint: { strict: true } })
