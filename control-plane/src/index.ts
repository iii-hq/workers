import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { registerWorker } from 'iii-sdk'
import { registerControlPlane } from './worker.js'

const iii = registerWorker(process.env.III_URL ?? process.env.III_ENGINE_URL, {
  workerName: 'control-plane',
  workerDescription: 'Project control plane and read-only summary scheduler.',
})
const here = dirname(fileURLToPath(import.meta.url))
const controlPlane = await registerControlPlane(iii, [
  here,
  join(here, '..', 'ui', 'dist'),
  join(here, '..', '..', 'ui', 'dist'),
])
console.log('control-plane worker connected')
const stop = async () => {
  await controlPlane.dispose()
  await iii.shutdown()
}
process.once('SIGINT', () => {
  void stop()
})
process.once('SIGTERM', () => {
  void stop()
})
