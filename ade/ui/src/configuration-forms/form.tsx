import { type ConfigFormProps, SettingsList, SettingsSection } from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useRef } from 'react'
import { FieldRenderer } from './fields'
import type { WorkerConfigurationSpec } from './types'
import { legacyConfigurationValue, migrateLegacyConfiguration } from './value'

export function DeclarativeWorkerConfigurationForm({
  spec,
  leading,
  ...props
}: ConfigFormProps & {
  spec: WorkerConfigurationSpec
  /** A section ahead of the spec's own, for settings that are not part of the entry. */
  leading?: ReactNode
}) {
  const rootRef = useRef<HTMLDivElement | null>(null)
  const legacyWrapper = spec.legacyWrapper
  const value = legacyWrapper ? legacyConfigurationValue(props.value, legacyWrapper) : props.value
  const ownedTopLevelFields = spec.expectedFields.map((path) => path.split('.')[0])
  const onChange = legacyWrapper
    ? (nextValue: ConfigFormProps['value']) =>
        props.onChange(migrateLegacyConfiguration(props.value, legacyWrapper, nextValue, ownedTopLevelFields))
    : props.onChange

  useEffect(() => {
    const focusPath = props.focusField
    if (!focusPath?.length || !rootRef.current) return
    const exact = rootRef.current.querySelector<HTMLElement>(`[data-path="${CSS.escape(focusPath.join('.'))}"]`)
    const topLevel = rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusPath[0])}"]`)
    const target = exact ?? topLevel
    target?.scrollIntoView({ block: 'center' })
    target?.querySelector<HTMLElement>('input,button,[tabindex]')?.focus()
  }, [props.focusField])

  return (
    // No title of its own: the settings page header already names the worker.
    <div ref={rootRef} className="console-worker-config" data-configuration-id={spec.id}>
      {leading}
      {spec.sections.map((section) => (
        <SettingsSection key={section.title} title={section.title} description={section.description}>
          <SettingsList>
            {section.fields.map((field, index) => (
              <FieldRenderer
                key={`${field.kind}-${field.path.join('.')}-${index}`}
                field={field}
                root={value}
                onChange={onChange}
                errors={props.errors}
              />
            ))}
          </SettingsList>
        </SettingsSection>
      ))}
    </div>
  )
}
