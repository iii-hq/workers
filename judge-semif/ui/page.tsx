import type { Host } from '@iii-dev/console-ui'
import { createSemifConfigForm } from './src/configuration'

export default function setup(host: Host) {
  host.configForms.register('judge-semif', createSemifConfigForm(host.iii))
}
