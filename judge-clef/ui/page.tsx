import type { Host } from '@iii-dev/console-ui'
import { createClefConfigForm } from './src/configuration'

export default function setup(host: Host) {
  host.configForms.register('judge-clef', createClefConfigForm(host.iii))
}
