import type { ConfigFormProps } from '@iii-dev/console-ui'
import { DeclarativeWorkerConfigurationForm } from './form'
import { validateWorkerConfigurationManifest, workerConfigurationManifest, workerConfigurationSpecs } from './manifest'
import { normalizeWorkerConfiguration } from './normalization'

export {
  declaredFields,
  validateWorkerConfigurationManifest,
  workerConfigurationIds,
  workerConfigurationManifest,
  workerConfigurationSpecs,
} from './manifest'
export {
  normalizeTelegramBotConfiguration,
  normalizeWorkerConfiguration,
} from './normalization'
export type {
  FormFieldSpec,
  FormSectionSpec,
  WorkerConfigurationSpec,
} from './types'

export function WorkerConfigurationForm({ configurationId, ...props }: ConfigFormProps & { configurationId: string }) {
  const spec = workerConfigurationSpecs.get(configurationId)
  if (!spec) return null
  const value = normalizeWorkerConfiguration(configurationId, props.value)
  return (
    <DeclarativeWorkerConfigurationForm
      spec={spec}
      {...props}
      value={value}
      onChange={(next) => props.onChange(normalizeWorkerConfiguration(configurationId, next))}
    />
  )
}

export function configurationForm(configurationId: string) {
  return function RegisteredWorkerConfigurationForm(props: ConfigFormProps) {
    return <WorkerConfigurationForm configurationId={configurationId} {...props} />
  }
}

// Fail immediately in development and in the focused manifest test if an ID
// or declarative field snapshot drifts. This checks the hand-authored specs;
// JSON Schema remains validation/options input and never generates controls.
validateWorkerConfigurationManifest()

void workerConfigurationManifest
