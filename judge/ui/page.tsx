import type { Host } from '@iii-dev/console-ui'
import { createJudgeConfigForm } from './src/configuration'

export default function setup(host: Host) {
  host.configForms.register('judge', createJudgeConfigForm(host.iii))
}
