import { registerWorker } from 'iii-sdk'

const engineWsUrl = process.env.III_URL ?? 'ws://localhost:49134'

export const iii = registerWorker(engineWsUrl, {
  workerName: 'conductor',
  otel: {
    enabled: true,
    serviceName: 'conductor',
  },
})

console.info('Conductor worker started', { engineWsUrl })
