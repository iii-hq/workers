import type { Host } from '@iii-dev/console-ui'
import { createLayaConfigForm } from './src/configuration'

export default function setup(host: Host) {
  host.configForms.register('judge-laya', createLayaConfigForm(host.iii))
}
