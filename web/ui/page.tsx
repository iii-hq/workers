import type { Host } from '@iii-dev/console-ui'
import { createWebImageRenderer } from './src/function-trigger'

export default function setup(host: Host) {
  host.functionTriggers.register(createWebImageRenderer())
}
