import type { ConfigFormProps, Host } from '@iii-dev/console-ui'
import { DeclarativeWorkerConfigurationForm } from './form'
import { validateWorkerConfigurationManifest, workerConfigurationManifest, workerConfigurationSpecs } from './manifest'
import { ConfigurationHostContext } from './model-catalog'
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

/**
 * The form `host.configForms.register` mounts for `configurationId`. `host` is
 * the registering script's own handle (it goes stale with that script, which
 * the loader disposes on hot reload), for fields that call the engine.
 */
export function configurationForm(configurationId: string, host: Host) {
  return function RegisteredWorkerConfigurationForm(props: ConfigFormProps) {
    return (
      <ConfigurationHostContext.Provider value={host}>
        <WorkerConfigurationForm configurationId={configurationId} {...props} />
      </ConfigurationHostContext.Provider>
    )
  }
}

// Fail immediately in development and in the focused manifest test if an ID
// or declarative field snapshot drifts. This checks the hand-authored specs;
// JSON Schema remains validation/options input and never generates controls.
validateWorkerConfigurationManifest()

void workerConfigurationManifest
