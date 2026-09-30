import { RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { resolveConfigurationFamily } from '@/lib/configuration-family'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { useExtProviderConfigForm } from '@/lib/ui-slots'
import { cn } from '@/lib/utils'
import { isObjectSchema } from '@/pages/Configuration/lib/schema/guard'
import { validateConfig } from '@/pages/Configuration/lib/schema/validate'
import type {
  JsonSchema,
  JsonValue,
} from '@/pages/Configuration/tabs/WorkersTab/api'
import { isDirty, jsonEqual } from '@/pages/Configuration/tabs/WorkersTab/dirty'
import { parseSetError } from '@/pages/Configuration/tabs/WorkersTab/errors'
import {
  useConfigurationSchema,
  useConfigurationsList,
  useConfigurationValue,
  useSetConfiguration,
} from '@/pages/Configuration/tabs/WorkersTab/hooks'
import {
  SaveBar,
  type SaveStatus,
} from '@/pages/Configuration/tabs/WorkersTab/SaveBar'
import { providerForModel } from './model-picker-presentation'
import { ProviderSettingsForm } from './ProviderSettingsForm'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

const ROUTER_CONFIGURATION_FAMILY = 'llm-router'

type JsonObject = { [key: string]: JsonValue }

type ProviderSliceResult =
  | { kind: 'ready'; value: JsonValue }
  | { kind: 'invalid'; message: string }

/**
 * Return the provider slice that a form may edit without coercing an opaque
 * root or sibling block. A null root/provider is the engine's unconfigured
 * value and starts as an empty object only in the local draft; no write is
 * performed until the operator explicitly saves.
 */
export function providerSliceFromValue(
  value: JsonValue | undefined,
  providerId: string,
): ProviderSliceResult {
  if (value === undefined) {
    return { kind: 'invalid', message: 'configuration value is still loading' }
  }
  if (value === null) return { kind: 'ready', value: {} }
  if (!isJsonObject(value)) {
    return {
      kind: 'invalid',
      message: 'the router configuration value is not an editable object',
    }
  }

  const providers = value.providers
  if (providers === undefined) return { kind: 'ready', value: {} }
  if (providers === null || !isJsonObject(providers)) {
    return {
      kind: 'invalid',
      message: 'the router providers block is not an editable object',
    }
  }

  const providerValue = providers[providerId]
  return providerValue === undefined || providerValue === null
    ? { kind: 'ready', value: {} }
    : { kind: 'ready', value: providerValue }
}

/**
 * Merge one provider draft into the complete raw router value. Unknown root
 * fields, sibling providers, credentials and `${ENV:default}` strings remain
 * untouched. Returning null refuses to overwrite an incompatible raw shape.
 */
export function mergeProviderConfigurationValue(
  value: JsonValue | undefined,
  providerId: string,
  providerValue: JsonValue,
): JsonValue | null {
  if (value === null) {
    return { providers: { [providerId]: providerValue } }
  }
  if (!isJsonObject(value)) return null

  const providers = value.providers
  if (providers === undefined) {
    return { ...value, providers: { [providerId]: providerValue } }
  }
  if (!isJsonObject(providers)) return null

  return {
    ...value,
    providers: { ...providers, [providerId]: providerValue },
  }
}

function isJsonObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
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

function errorText(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) return error.message
  if (typeof error === 'string' && error) return error
  return fallback
}

function retryButton(onRetry: () => void) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      onClick={onRetry}
      className="shrink-0"
    >
      <RefreshCw className="size-4" aria-hidden />
      Retry
    </Button>
  )
}

function loadErrorPanel(headline: string, detail: string, onRetry: () => void) {
  return (
    <StatusPanel
      role="alert"
      variant="alert"
      headline={headline}
      detail={detail}
      action={retryButton(onRetry)}
    />
  )
}

function resolutionMessage(
  resolution: ReturnType<typeof resolveConfigurationFamily>,
): string {
  if (resolution.kind === 'ambiguous') {
    return `Multiple router configurations match this provider form: ${resolution.ids.join(', ')}. Open Settings → Workers to choose or remove the duplicate router configuration, then retry.`
  }
  return 'No live router configuration advertises this provider form. Start or configure the router, then retry.'
}

function relativeProviderPointer(
  pointer: string | undefined,
  providerId: string,
): string {
  if (!pointer) return ''
  const escapedProviderId = providerId.replace(/~/g, '~0').replace(/\//g, '~1')
  const prefix = `/providers/${escapedProviderId}`
  if (pointer === prefix) return ''
  if (pointer.startsWith(`${prefix}/`)) return pointer.slice(prefix.length)
  return ''
}

interface ProviderConfigurationPanelProps {
  providerId: string
  onDirtyChange?: (dirty: boolean) => void
  className?: string
}

/**
 * Focused editor for one provider slice inside the authoritative router
 * configuration. The host resolves the live router id from the registry,
 * then keeps validation/save lifecycle and raw-value merging authoritative.
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

  // Resolution is a prerequisite, not a fallback. No schema/value request is
  // enabled until this exact live id is known.
  const configurationsQuery = useConfigurationsList()
  const resolution = useMemo(
    () =>
      configurationsQuery.data
        ? resolveConfigurationFamily(
            ROUTER_CONFIGURATION_FAMILY,
            configurationsQuery.data,
          )
        : null,
    [configurationsQuery.data],
  )
  const effectiveConfigurationId =
    resolution?.kind === 'resolved' ? resolution.id : null
  const schemaQuery = useConfigurationSchema(effectiveConfigurationId)
  const valueQuery = useConfigurationValue(effectiveConfigurationId)
  const setMutation = useSetConfiguration(effectiveConfigurationId)

  const editorIdentity = effectiveConfigurationId
    ? `${effectiveConfigurationId}\u0000${providerId}`
    : null
  const identityRef = useRef<string | null>(editorIdentity)
  const generationRef = useRef(0)
  const seededIdentityRef = useRef<string | null>(null)
  const hydratedDataUpdatedAtRef = useRef<number | null>(null)
  const draftsByIdentityRef = useRef(
    new Map<string, { draft: JsonValue; baseline: JsonValue | undefined }>(),
  )
  const pendingRestoreRef = useRef<{
    identity: string
    draft: JsonValue
    baseline: JsonValue | undefined
  } | null>(null)
  const activeEditorIdentityRef = useRef<string | null>(editorIdentity)
  const dataUpdatedAtRef = useRef(valueQuery.dataUpdatedAt)
  dataUpdatedAtRef.current = valueQuery.dataUpdatedAt
  const [draft, setDraft] = useState<JsonValue | undefined>(undefined)
  const [baseline, setBaseline] = useState<JsonValue | undefined>(undefined)
  const [status, setStatus] = useState<SaveStatus>({ kind: 'idle' })
  const [serverErrors, setServerErrors] = useState<Map<string, string>>(
    new Map(),
  )
  const draftRef = useRef<JsonValue | undefined>(draft)
  draftRef.current = draft
  const baselineRef = useRef<JsonValue | undefined>(baseline)
  baselineRef.current = baseline

  const providerValue = useMemo(
    () => providerSliceFromValue(valueQuery.data, providerId),
    [valueQuery.data, providerId],
  )

  // A configuration/provider switch is a new editor, even when React keeps
  // the panel mounted for the picker page transition. Dirty drafts are kept
  // only under their complete identity; a destination never inherits another
  // configuration/provider's draft or secrets.
  useEffect(() => {
    generationRef.current += 1
    const previousIdentity = activeEditorIdentityRef.current
    if (
      previousIdentity !== null &&
      previousIdentity !== editorIdentity &&
      draftRef.current !== undefined &&
      baselineRef.current !== undefined &&
      isDirty(baselineRef.current, draftRef.current)
    ) {
      draftsByIdentityRef.current.set(previousIdentity, {
        draft: draftRef.current,
        baseline: baselineRef.current,
      })
    }

    activeEditorIdentityRef.current = editorIdentity
    identityRef.current = editorIdentity
    seededIdentityRef.current = null
    hydratedDataUpdatedAtRef.current = null
    const preserved = editorIdentity
      ? draftsByIdentityRef.current.get(editorIdentity)
      : undefined
    if (preserved && editorIdentity) {
      // Consume the parked entry immediately. It is restored only after the
      // current server value has been read and its baseline is revalidated.
      draftsByIdentityRef.current.delete(editorIdentity)
      pendingRestoreRef.current = { identity: editorIdentity, ...preserved }
    } else {
      pendingRestoreRef.current = null
    }
    setDraft(undefined)
    setBaseline(undefined)
    setServerErrors(new Map())
    setStatus({ kind: 'idle' })
  }, [editorIdentity])

  const hydrated =
    editorIdentity !== null && seededIdentityRef.current === editorIdentity
  const dirty =
    hydrated &&
    baseline !== undefined &&
    draft !== undefined &&
    isDirty(baseline, draft)

  // Hydrate once per identity. A background refetch updates a clean editor,
  // but never overwrites a dirty draft. A parked draft is restored only when
  // its baseline still matches the current server value.
  useEffect(() => {
    if (
      editorIdentity === null ||
      effectiveConfigurationId === null ||
      valueQuery.data === undefined ||
      (pendingRestoreRef.current !== null && valueQuery.isFetching) ||
      providerValue.kind !== 'ready'
    ) {
      return
    }
    if (seededIdentityRef.current !== editorIdentity) {
      seededIdentityRef.current = editorIdentity
      hydratedDataUpdatedAtRef.current = dataUpdatedAtRef.current
      const pendingRestore = pendingRestoreRef.current
      pendingRestoreRef.current = null
      if (
        pendingRestore?.identity === editorIdentity &&
        jsonEqual(pendingRestore.baseline, providerValue.value)
      ) {
        setBaseline(providerValue.value)
        setDraft(pendingRestore.draft)
      } else {
        setBaseline(providerValue.value)
        setDraft(providerValue.value)
        if (pendingRestore?.identity === editorIdentity) {
          setStatus({
            kind: 'error',
            message:
              'Unsaved changes were discarded because the server value changed.',
          })
        }
      }
      setServerErrors(new Map())
      return
    }
    if (
      !dirty &&
      hydratedDataUpdatedAtRef.current !== valueQuery.dataUpdatedAt
    ) {
      hydratedDataUpdatedAtRef.current = dataUpdatedAtRef.current
      setBaseline(providerValue.value)
      setDraft(providerValue.value)
    }
  }, [
    dirty,
    editorIdentity,
    effectiveConfigurationId,
    providerValue,
    valueQuery.data,
    valueQuery.dataUpdatedAt,
    valueQuery.isFetching,
  ])

  useEffect(() => {
    onDirtyChange?.(dirty)
  }, [dirty, onDirtyChange])
  useEffect(
    () => () => {
      onDirtyChange?.(false)
    },
    [onDirtyChange],
  )

  const providerSchema = useMemo(
    () => providerSchemaOf(schemaQuery.data?.schema ?? null, providerId),
    [schemaQuery.data?.schema, providerId],
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
    for (const [pointer, message] of clientErrors) merged.set(pointer, message)
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
    if (!hydrated) return
    if (editorIdentity) draftsByIdentityRef.current.delete(editorIdentity)
    pendingRestoreRef.current = null
    setDraft(baseline)
    setServerErrors(new Map())
    setStatus({ kind: 'idle' })
  }, [baseline, hydrated, editorIdentity])

  const schemaMatchesId = schemaQuery.data?.id === effectiveConfigurationId
  const schemaMissing = schemaMatchesId && schemaQuery.data?.schema === null
  const canSave =
    resolution?.kind === 'resolved' &&
    !configurationsQuery.error &&
    !schemaQuery.error &&
    !valueQuery.error &&
    schemaMatchesId &&
    !schemaMissing &&
    hydrated &&
    dirty &&
    draft !== undefined &&
    valueQuery.data !== undefined &&
    clientErrors.size === 0 &&
    status.kind !== 'saving'

  const handleSave = useCallback(() => {
    if (
      !canSave ||
      editorIdentity === null ||
      effectiveConfigurationId === null
    ) {
      return
    }
    const mergedValue = mergeProviderConfigurationValue(
      valueQuery.data,
      providerId,
      draft,
    )
    if (mergedValue === null) {
      setStatus({
        kind: 'error',
        message: 'The raw router value cannot safely preserve its shape.',
      })
      return
    }

    const saveIdentity = editorIdentity
    const saveId = effectiveConfigurationId
    const saveGeneration = generationRef.current
    const savedDraft = draft
    setStatus({ kind: 'saving' })
    setServerErrors(new Map())
    setMutation.mutate(
      { id: saveId, value: mergedValue },
      {
        onSuccess: (response) => {
          if (
            identityRef.current !== saveIdentity ||
            generationRef.current !== saveGeneration
          )
            return
          const savedSlice = providerSliceFromValue(
            response.new_value,
            providerId,
          )
          if (savedSlice.kind !== 'ready') {
            setStatus({ kind: 'error', message: savedSlice.message })
            return
          }
          const draftChangedDuringSave = isDirty(savedDraft, draftRef.current)
          draftsByIdentityRef.current.delete(saveIdentity)
          pendingRestoreRef.current = null
          setBaseline(savedSlice.value)
          if (draftChangedDuringSave) {
            setStatus({ kind: 'idle' })
          } else {
            setDraft(savedSlice.value)
            setStatus({ kind: 'saved', savedAtMs: Date.now() })
          }
          void ctx?.refreshModels()
        },
        onError: (error) => {
          if (
            identityRef.current !== saveIdentity ||
            generationRef.current !== saveGeneration
          )
            return
          const parsed = parseSetError(error)
          setStatus({ kind: 'error', message: parsed.message })
          setServerErrors(
            new Map([
              [
                relativeProviderPointer(parsed.pointer, providerId),
                parsed.message,
              ],
            ]),
          )
        },
      },
    )
  }, [
    canSave,
    ctx,
    draft,
    editorIdentity,
    effectiveConfigurationId,
    providerId,
    setMutation,
    valueQuery.data,
  ])

  const retryResolution = useCallback(() => {
    void configurationsQuery.refetch()
  }, [configurationsQuery.refetch])
  const retryReads = useCallback(() => {
    // Retry is read-only. The next resolved identity must hydrate from its own
    // schema/value; never copy this editor's draft, baseline, or secrets into it.
    void configurationsQuery.refetch()
    void schemaQuery.refetch()
    void valueQuery.refetch()
  }, [configurationsQuery.refetch, schemaQuery.refetch, valueQuery.refetch])

  const loading =
    schemaQuery.isLoading ||
    valueQuery.isLoading ||
    !schemaMatchesId ||
    valueQuery.data === undefined ||
    (providerValue.kind === 'ready' && !hydrated)
  const loadError = schemaQuery.error ?? valueQuery.error

  return (
    <div
      className={cn(
        'configuration-surface flex min-h-0 flex-1 flex-col',
        className,
      )}
    >
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        {configurationsQuery.error ? (
          loadErrorPanel(
            'Could not resolve router configuration',
            errorText(
              configurationsQuery.error,
              'The configuration registry could not be read.',
            ),
            retryResolution,
          )
        ) : configurationsQuery.isLoading || !resolution ? (
          <div
            className="space-y-4 py-2"
            role="status"
            aria-label="Loading provider configuration"
          >
            <Skeleton className="h-5 w-36" />
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-10 w-full" />
          </div>
        ) : resolution.kind !== 'resolved' ? (
          loadErrorPanel(
            'Router configuration unavailable',
            resolutionMessage(resolution),
            retryResolution,
          )
        ) : loadError ? (
          loadErrorPanel(
            'Could not load provider configuration',
            errorText(
              loadError,
              'The router schema or value could not be read.',
            ),
            retryReads,
          )
        ) : loading ? (
          <div
            className="space-y-4 py-2"
            role="status"
            aria-label="Loading provider configuration"
          >
            <Skeleton className="h-5 w-36" />
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-10 w-full" />
          </div>
        ) : schemaMissing ? (
          <StatusPanel
            role="alert"
            variant="alert"
            headline="Provider configuration schema unavailable"
            detail="The router registered this configuration without a JSON schema. No fields or defaults were materialized."
            action={retryButton(retryReads)}
          />
        ) : providerValue.kind === 'invalid' ? (
          <StatusPanel
            role="alert"
            variant="alert"
            headline="Provider configuration is not editable"
            detail={providerValue.message}
          />
        ) : draft !== undefined && providerFormOverride ? (
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
        ) : draft !== undefined && providerSchema && isJsonObject(draft) ? (
          <ProviderSettingsForm
            schema={providerSchema}
            value={draft}
            onChange={handleChange}
            errors={errors}
            credentialEnvVar={provider?.credential_env_var}
            configured={provider?.configured}
          />
        ) : (
          <div className="rounded-lg bg-surface px-3 py-4 font-sans text-base text-ink-faint sm:text-sm">
            {draft !== undefined && !isJsonObject(draft)
              ? 'This provider value is opaque; its provider-owned form must handle the value explicitly.'
              : 'This provider does not expose editable configuration.'}
          </div>
        )}
      </div>
      <SaveBar
        dirty={dirty}
        status={status}
        onSave={handleSave}
        onReset={handleReset}
        saveDisabled={!canSave}
      />
    </div>
  )
}
