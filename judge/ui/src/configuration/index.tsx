import {
  Button,
  Chip,
  type ConfigFormProps,
  type ExtensionIii,
  Select,
  type SelectOption,
  SettingsField,
  SettingsList,
  SettingsSection,
  StatusPanel,
  Switch,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'

/** What the hub uses when the entry stores no provider. */
export const BUILT_IN_PROVIDER = 'typesafe'
const PROVIDER_FUNCTION = /^judge-([a-z0-9-]{1,64})::(evaluate|configuration-id)$/

export interface RegisteredProvider {
  provider: string
  worker: string
  namespace: string
}
interface FunctionsList {
  functions?: { function_id: string; namespace?: string; worker_name?: string }[]
}
type Engine = Pick<ExtensionIii, 'trigger'>

/** Every `judge-<provider>` worker, one row per provider (by its `evaluate` or
 * its configuration id). A local provider loads its model on first use. */
export async function listProviders(iii: Engine): Promise<RegisteredProvider[]> {
  // Providers register their functions as internal (callers use the hub), so
  // they only show up when internal registrations are included.
  const reply = await iii.trigger<FunctionsList>(
    'engine::functions::list',
    { include_internal: true },
    { timeoutMs: 10_000 },
  )
  const seen = new Map<string, RegisteredProvider>()
  for (const fn of reply?.functions ?? []) {
    const match = PROVIDER_FUNCTION.exec(fn.function_id)
    if (!match) continue
    const [, provider] = match
    if (!seen.has(provider)) seen.set(provider, { provider, worker: fn.worker_name ?? `judge-${provider}`, namespace: fn.namespace ?? '' })
  }
  return [...seen.values()].sort((a, b) => a.provider.localeCompare(b.provider))
}

/** Bind the form to the console's engine client once, at registration. */
export function createJudgeConfigForm(iii: Engine) {
  return function JudgeConfigForm(props: ConfigFormProps) {
    return <JudgeRoutingForm {...props} iii={iii} />
  }
}

export function JudgeRoutingForm({ iii, ...props }: ConfigFormProps & { iii: Engine }) {
  const rootRef = useRef<HTMLDivElement>(null)
  // null = the first listing has not answered yet.
  const [providers, setProviders] = useState<RegisteredProvider[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const refresh = useCallback(() => {
    setListError(null)
    listProviders(iii)
      .then(setProviders)
      .catch((error: unknown) => {
        setListError(error instanceof Error ? error.message : String(error))
        setProviders((current) => current ?? [])
      })
  }, [iii])
  useEffect(() => {
    refresh()
  }, [refresh])

  const focusField = props.focusField?.[0]
  useEffect(() => {
    if (focusField !== 'provider') return
    const trigger = rootRef.current?.querySelector<HTMLElement>('#judge-cfg-provider')
    trigger?.scrollIntoView?.({ block: 'center' })
    trigger?.focus()
  }, [focusField])

  if (props.value !== null && (typeof props.value !== 'object' || Array.isArray(props.value))) {
    return (
      <StatusPanel
        variant="info"
        headline="Judge configuration is supplied as a single value"
        detail="Edit this value in the configuration source. It is preserved until you replace it with an object."
      />
    )
  }
  const value = props.value ?? {}
  const stored = typeof value.provider === 'string' ? value.provider : undefined
  const effective = stored ?? BUILT_IN_PROVIDER
  const registered = providers?.find((entry) => entry.provider === effective)
  const preloadAll = value.preload_all === true
  const setPreloadAll = (on: boolean) => {
    const draft = { ...value }
    if (on) draft.preload_all = true
    else delete draft.preload_all
    props.onChange(draft)
  }
  const setProvider = (next: string | undefined) => {
    const draft = { ...value }
    if (next === undefined) delete draft.provider
    else draft.provider = next
    props.onChange(draft)
  }

  const options: SelectOption[] = (providers ?? []).map((entry) => ({
    value: entry.provider,
    label: entry.provider,
    description: [entry.worker, entry.namespace].filter(Boolean).join(' · '),
  }))
  if (stored && !options.some((option) => option.value === stored)) {
    options.push({ value: stored, label: stored, description: 'Not registered on the engine' })
  }
  const unassociatedErrors = [...(props.errors?.entries() ?? [])].filter(
    ([pointer]) => pointer !== '/provider' && pointer !== '/preload_all',
  )

  return (
    <div className="judge-ui-form" ref={rootRef}>
      <SettingsSection
        title="Routing"
        description="judge::evaluate, judge::models::list and judge::cancel go to the provider selected here unless a request names its own. New calls pick up a change immediately."
        action={
          <Button type="button" variant="ghost" size="sm" onClick={refresh} disabled={providers === null}>
            Refresh
          </Button>
        }
      >
        <SettingsList>
          <SettingsField
            id="judge-cfg-provider"
            field="provider"
            label="Default provider"
            description="Workers registered as judge-<provider>. Credentials and models are configured on the provider itself, not here."
            error={props.errors?.get('/provider')}
            meta={
              providers === null ? (
                <Chip tone="neutral">Checking registrations…</Chip>
              ) : registered ? (
                <Chip tone="success">
                  {registered.worker}
                  {registered.namespace ? ` · ${registered.namespace}` : ''}
                </Chip>
              ) : (
                <Chip tone="warning">judge-{effective} not registered</Chip>
              )
            }
            renderControl={(controlProps) => (
              <Select
                {...controlProps}
                value={stored}
                options={options}
                placeholder={`Built-in default (${BUILT_IN_PROVIDER})`}
                allowEmpty
                emptyLabel={`Built-in default (${BUILT_IN_PROVIDER})`}
                onClear={() => setProvider(undefined)}
                onChange={setProvider}
                aria-label="Default provider"
                aria-busy={providers === null}
              />
            )}
          />
          <SettingsField
            id="judge-cfg-preload_all"
            field="preload_all"
            label="Keep every local provider loaded"
            description="Local providers (judge-decider, judge-semif, judge-laya) keep their model loaded while they are the default; any other one loads its model on the first request that names it and releases it after 10 idle minutes, so that first request may time out while it loads. On, every local provider keeps its model loaded, so no request waits; each one holds its memory (on a GPU, about 6 GB for decider or SemIf and 1.5 GB for laya). Hosted providers are unaffected."
            error={props.errors?.get('/preload_all')}
            renderControl={(controlProps) => (
              <Switch
                {...controlProps}
                checked={preloadAll}
                aria-label="Keep every local provider loaded"
                onChange={(event) => setPreloadAll(event.currentTarget.checked)}
              />
            )}
          />
        </SettingsList>
      </SettingsSection>
      {listError ? (
        <StatusPanel
          variant="warn"
          headline="Could not list judge providers"
          detail={`${listError}. The stored value stays selectable; retry once the engine answers.`}
          action={
            <Button type="button" variant="ghost" size="sm" onClick={refresh}>
              Retry
            </Button>
          }
        />
      ) : providers && !registered ? (
        <StatusPanel
          variant="warn"
          headline={`judge-${effective} is not running`}
          detail="Calls answer provider_unavailable until that worker registers. Start it, then refresh, or pick a registered provider."
        />
      ) : null}
      {unassociatedErrors.length > 0 ? (
        <StatusPanel
          variant="alert"
          headline="Review the configuration errors"
          detail={unassociatedErrors.map(([pointer, message]) => (
            <div key={pointer}>
              {pointer ? `${pointer}: ` : ''}
              {message}
            </div>
          ))}
        />
      ) : null}
    </div>
  )
}
