import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { registerWorker } from 'iii-sdk'
import { registerTower } from './worker.js'

const iii = registerWorker(process.env.III_URL ?? process.env.III_ENGINE_URL, {
  workerName: 'tower',
  workerDescription: 'Project control tower and read-only summary scheduler.',
})
const here = dirname(fileURLToPath(import.meta.url))
const tower = await registerTower(iii, [
  here,
  join(here, '..', 'ui', 'dist'),
  join(here, '..', '..', 'ui', 'dist'),
])
console.log('tower worker connected')
const stop = async () => {
  await tower.dispose()
  await iii.shutdown()
}
process.once('SIGINT', () => {
  void stop()
})
process.once('SIGTERM', () => {
  void stop()
})
