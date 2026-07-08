// Smoke-test mock: registers `router::chat`, closes the caller's stream
// channel (EOF for the harness frame loop), then fails — so a harness turn
// step ends fast and its spans export with their trace tags. Not part of
// the implementation; lives here only to borrow this package's iii-sdk.
import { registerWorker } from 'iii-sdk'

const ENGINE = process.env.III_URL ?? 'ws://127.0.0.1:49234'
const worker = registerWorker(ENGINE)

worker.registerFunction(
  'router::chat',
  async (input: { writer_ref?: { channel_id: string; access_key: string } }) => {
    const ref = input?.writer_ref
    if (ref?.channel_id) {
      const url = `${ENGINE.replace(/\/$/, '')}/channels/${ref.channel_id}?key=${encodeURIComponent(ref.access_key)}&dir=write`
      await new Promise<void>((resolve) => {
        const ws = new WebSocket(url)
        ws.onopen = () => {
          ws.close(1000, 'stream_complete')
          resolve()
        }
        ws.onerror = () => resolve()
        setTimeout(resolve, 2000)
      })
    }
    throw new Error('mock router: no providers configured (smoke test)')
  },
)

console.log('mock router::chat registered')
setInterval(() => {}, 1 << 30)
