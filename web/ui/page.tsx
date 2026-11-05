import type { Host } from '@iii-dev/console-ui'
import { createWebImageRenderer } from './src/function-call'

export default function setup(host: Host) {
  host.functionCalls.register(createWebImageRenderer())
}
