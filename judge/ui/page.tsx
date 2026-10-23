import type { Host, WorkerConfigurationPanelProps } from '@iii-dev/console-ui'
import type { ComponentType } from 'react'
import { createJudgeConfigForm } from './src/configuration'
import { createJudgeSessionControl } from './src/session'

export default function setup(host: Host) {
  host.configForms.register('judge', createJudgeConfigForm(host.iii))
  // The Console's inline settings editor, on Consoles that have it: a judge
  // is configured right in the picker. Older ones open it in Settings.
  const configurationPanel = host.components?.WorkerConfigurationPanel as
    | ComponentType<WorkerConfigurationPanelProps>
    | undefined
  // Per-session provider beside the model picker; older consoles lack the slot.
  host.chat?.registerComposerControl?.({
    id: 'judge-provider',
    render: createJudgeSessionControl(host.iii, configurationPanel),
  })
}
