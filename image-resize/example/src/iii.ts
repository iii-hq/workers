import { registerWorker } from 'iii-sdk'

const engineWsUrl = process.env.III_URL ?? 'ws://localhost:49134'

export const iii = registerWorker(engineWsUrl, {
  otel: {
    enabled: true,
    serviceName: 'image-resize-demo',
  },
})

console.info('III worker started', { myCustomVar: process.env.MY_CUSTOM_VAR })
