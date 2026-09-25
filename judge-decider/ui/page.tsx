import type { Host } from '@iii-dev/console-ui'
import { createDeciderConfigForm } from './src/configuration'

export default function setup(host: Host) {
  host.configForms.register('judge-decider', createDeciderConfigForm(host.iii))
}
