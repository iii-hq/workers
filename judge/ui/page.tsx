import type { Host } from '@iii-dev/console-ui'
import { createJudgeConfigForm } from './src/configuration'
import { createJudgeSessionControl } from './src/session'

export default function setup(host: Host) {
  host.configForms.register('judge', createJudgeConfigForm(host.iii))
  // Per-session provider beside the model picker; older consoles lack the slot.
  host.chat?.registerComposerControl?.({ id: 'judge-provider', render: createJudgeSessionControl(host.iii) })
}
