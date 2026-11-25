import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { registerWorker } from 'iii-sdk'
import { resolveEngineUrl } from './engine-url.js'
import { registerControlPlane } from './worker.js'

const engineUrl = resolveEngineUrl(process.env.III_URL ?? process.env.III_ENGINE_URL)
const iii = registerWorker(engineUrl, {
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
