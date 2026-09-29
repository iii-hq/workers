import type { Host } from '@iii-dev/console-ui'
import { createJevConfigForm } from './src/configuration'

export default function setup(host: Host) {
  host.configForms.register('judge-typesafe', createJevConfigForm(host.iii))
}
