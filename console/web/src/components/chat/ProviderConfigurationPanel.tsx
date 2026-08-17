import { useCallback, useEffect, useMemo, useState } from 'react'
import { Skeleton } from '@/components/ui/Skeleton'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { useExtProviderConfigForm } from '@/lib/ui-slots'
import { cn } from '@/lib/utils'
import type {
  JsonSchema,
  JsonValue,
} from '@/pages/Configuration/tabs/WorkersTab/api'
import { isDirty } from '@/pages/Configuration/tabs/WorkersTab/dirty'
import { parseSetError } from '@/pages/Configuration/tabs/WorkersTab/errors'
import {
  useConfigurationSchema,
  useConfigurationValue,
  useSetConfiguration,
} from '@/pages/Configuration/tabs/WorkersTab/hooks'
import {
  SaveBar,
  type SaveStatus,
} from '@/pages/Configuration/tabs/WorkersTab/SaveBar'
import { isObjectSchema } from '@/pages/Configuration/tabs/WorkersTab/schema-form/guard'
import { validateConfig } from '@/pages/Configuration/tabs/WorkersTab/schema-form/validate'
import { providerForModel } from './model-picker-presentation'
import { ProviderSettingsForm } from './ProviderSettingsForm'

const ROUTER_CONFIGURATION_ID = 'llm-router'

type JsonObject = { [key: string]: JsonValue }

interface ProviderConfigurationPanelProps {
  providerId: string
  onDirtyChange?: (dirty: boolean) => void
  className?: string
}

function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value
    : {}
}

function nestedObject(value: unknown, key: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {}
  const next = (value as Record<string, unknown>)[key]
  return next && typeof next === 'object' && !Array.isArray(next)
    ? (next as Record<string, unknown>)
    : {}
}

function providerSchemaOf(
  schema: JsonSchema | null,
  providerId: string,
): JsonSchema | null {
  const properties = nestedObject(schema, 'properties')
  const providers = nestedObject(properties.providers, 'properties')
  const provider = providers[providerId]
  return isObjectSchema(provider) ? provider : null
}

function providerSliceOf(value: JsonValue | undefined, providerId: string) {
  const providers = asObject(asObject(value).providers)
  return asObject(providers[providerId])
}

/**
 * Focused editor for one provider slice inside the authoritative llm-router
 * configuration. Provider workers may replace the form body, while this host
 * keeps validation/save lifecycle and merges only the provider slice back into
 * the complete router value.
 */
export function ProviderConfigurationPanel({
  providerId,
  onDirtyChange,
  className,
}: ProviderConfigurationPanelProps) {
  const ctx = useConversationsCtxOptional()
  const providerFormOverride = useExtProviderConfigForm(providerId)
  const provider = ctx?.presentProviders.find(
    (entry) => entry.id === providerId,
  )
  const modelCount =
    ctx?.modelOptions.filter(
      (option) => providerForModel(option.id) === providerId,
    ).length ?? 0
  const schemaQuery = useConfigurationSchema(ROUTER_CONFIGURATION_ID)
  const valueQuery = useConfigurationValue(ROUTER_CONFIGURATION_ID)
  const setMutation = useSetConfiguration(ROUTER_CONFIGURATION_ID)
  const [draft, setDraft] = useState<JsonValue | undefined>(undefined)
  const [baseline, setBaseline] = useState<JsonValue | undefined>(undefined)
  const [status, setStatus] = useState<SaveStatus>({ kind: 'idle' })
  const [serverErrors, setServerErrors] = useState<Map<string, string>>(
    new Map(),
  )

  const providerSchema = useMemo(
    () => providerSchemaOf(schemaQuery.data?.schema ?? null, providerId),
    [schemaQuery.data?.schema, providerId],
  )
  const loadedSlice = useMemo(
    () => providerSliceOf(valueQuery.data, providerId),
    [valueQuery.data, providerId],
  )

  useEffect(() => {
    if (valueQuery.data === undefined) return
    setBaseline(loadedSlice)
    setDraft(loadedSlice)
    setStatus((current) =>
      current.kind === 'saving' ? current : { kind: 'idle' },
    )
  }, [valueQuery.data, loadedSlice])

  const dirty =
    baseline !== undefined && draft !== undefined && isDirty(baseline, draft)
  useEffect(() => {
    onDirtyChange?.(dirty)
  }, [dirty, onDirtyChange])
  useEffect(
    () => () => {
      onDirtyChange?.(false)
    },
    [onDirtyChange],
  )

  const clientErrors = useMemo(
    () =>
      draft === undefined || !providerSchema
        ? new Map<string, string>()
        : validateConfig(draft, providerSchema),
    [draft, providerSchema],
  )
  const errors = useMemo(() => {
    const merged = new Map(serverErrors)
    for (const [pointer, message] of clientErrors) {
      merged.set(pointer, message)
    }
    return merged
  }, [clientErrors, serverErrors])

  const handleChange = useCallback((next: JsonValue) => {
    setDraft(next)
    setServerErrors(new Map())
    setStatus((current) =>
      current.kind === 'error' || current.kind === 'saved'
        ? { kind: 'idle' }
        : current,
    )
  }, [])

  const handleReset = useCallback(() => {
    setDraft(baseline)
    setServerErrors(new Map())
    setStatus({ kind: 'idle' })
  }, [baseline])

  const handleSave = useCallback(() => {
    if (draft === undefined || clientErrors.size > 0) return
    const root = asObject(valueQuery.data)
    const providers = asObject(root.providers)
    setStatus({ kind: 'saving' })
    setServerErrors(new Map())
    setMutation.mutate(
      {
        id: ROUTER_CONFIGURATION_ID,
        value: {
          ...root,
          providers: { ...providers, [providerId]: draft },
        },
      },
      {
        onSuccess: (response) => {
          const savedSlice = providerSliceOf(response.new_value, providerId)
          setBaseline(savedSlice)
          setDraft(savedSlice)
          setStatus({ kind: 'saved', savedAtMs: Date.now() })
          void ctx?.refreshModels()
        },
        onError: (error) => {
          const parsed = parseSetError(error)
          setStatus({ kind: 'error', message: parsed.message })
          setServerErrors(new Map([['', parsed.message]]))
        },
      },
    )
  }, [clientErrors.size, ctx, draft, providerId, setMutation, valueQuery.data])

  const loading =
    schemaQuery.isLoading || valueQuery.isLoading || draft === undefined
  const loadError = schemaQuery.error ?? valueQuery.error

  return (
    <div
      className={cn(
        'configuration-surface workers-tab flex min-h-0 flex-1 flex-col',
        className,
      )}
    >
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        {loading ? (
          <div className="space-y-4 py-2">
            <Skeleton className="h-5 w-36" />
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-10 w-full" />
          </div>
        ) : loadError ? (
          <div className="rounded-lg bg-alert-muted px-3 py-4 font-sans text-base text-alert sm:text-sm">
            {loadError instanceof Error
              ? loadError.message
              : 'Failed to load provider configuration.'}
          </div>
        ) : providerSchema && draft !== undefined ? (
          providerFormOverride ? (
            <providerFormOverride.component
              providerId={providerId}
              schema={providerSchema}
              value={draft}
              onChange={handleChange}
              errors={errors}
              configured={provider?.configured}
              available={provider?.available}
              modelCount={modelCount}
            />
          ) : (
            <ProviderSettingsForm
              schema={providerSchema}
              value={draft}
              onChange={handleChange}
              errors={errors}
              credentialEnvVar={provider?.credential_env_var}
              configured={provider?.configured}
            />
          )
        ) : (
          <div className="rounded-lg bg-surface px-3 py-4 font-sans text-base text-ink-faint sm:text-sm">
            This provider does not expose editable configuration.
          </div>
        )}
      </div>
      <SaveBar
        dirty={dirty}
        status={status}
        onSave={handleSave}
        onReset={handleReset}
        saveDisabled={clientErrors.size > 0}
      />
    </div>
  )
}
