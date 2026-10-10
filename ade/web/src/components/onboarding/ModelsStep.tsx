import { LoaderCircle } from 'lucide-react'
import type * as React from 'react'
import { useEffect, useId, useMemo, useState } from 'react'
import { DeviceSignIn } from '@/components/chat/DeviceSignIn'
import { Skeleton } from '@/components/ui/Skeleton'
import { KEY_PROVIDERS, SUBSCRIPTION_PROVIDERS } from '@/lib/onboarding/catalog'
import {
  connectPlan,
  type KeyInput,
  type ProviderChoice,
  type ProviderSelection,
  providerChoices,
  type RegistryProviderRow,
  registryChoices,
  type SubscriptionChoice,
} from '@/lib/onboarding/plan'
import { cn } from '@/lib/utils'
import { fetchRegistryProviders } from '@/lib/workers-registry'
import { Button, Checkbox } from './controls'
import { ProviderMark } from './ProviderMark'
import {
  CommandLine,
  defaultKeyInput,
  EngineLog,
  KeyField,
  keyInputReady,
  Rows,
  Section,
  StatusChip,
  StepHeader,
  StepLayout,
  Terminal,
} from './parts'
import { ScanButton } from './ScanButton'
import type { OnboardingController } from './use-onboarding'

interface Draft {
  /** Wanted connected: a connected provider unchecked is removed. */
  selected: boolean
  key?: KeyInput
}

/**
 * llm-router resolves `secret://` and `env://` references alike, so a
 * provider key can stay encrypted or live in this project's `.env`.
 */
const ROUTER_KEY_STORES = ['vault', 'env'] as const

/**
 * The one step that gets a model connected. Every provider has one place,
 * grouped by how it is paid for — a subscription a signed-in coding agent
 * already has, or an API key — and keeps that place whatever its state:
 * connected ones are simply checked, and unchecking one removes it. What
 * this machine already has starts checked, so most of the time there is
 * nothing to type.
 */
export function ModelsStep({
  onboarding,
  onBack,
  onNext,
}: {
  onboarding: OnboardingController
  onBack: () => void
  onNext: () => void
}) {
  const { snapshot, scanning, refresh, activity, running, run } = onboarding
  const [registry, setRegistry] = useState<RegistryProviderRow[]>([])
  const [drafts, setDrafts] = useState<ReadonlyMap<string, Draft>>(new Map())

  useEffect(() => {
    const controller = new AbortController()
    void fetchRegistryProviders({ signal: controller.signal })
      .then(setRegistry)
      .catch(() => undefined)
    return () => controller.abort()
  }, [])

  const choices = useMemo(
    () =>
      providerChoices({
        tools: snapshot.tools ?? [],
        providers: snapshot.providers ?? [],
        detections: snapshot.detections ?? [],
      }),
    [snapshot.tools, snapshot.providers, snapshot.detections],
  )
  const extra = useMemo(
    () => registryChoices(registry, snapshot.installed, choices),
    [registry, snapshot.installed, choices],
  )
  const subscriptions = useMemo(
    () =>
      catalogOrder(choices.filter((choice) => choice.kind === 'subscription')),
    [choices],
  )
  const keyed = useMemo(
    () => catalogOrder(choices.filter((choice) => choice.kind === 'key')),
    [choices],
  )
  // Providers that sign in from here (GitHub Copilot), and any other running
  // provider the router already serves models from.
  const devices = useMemo(
    () => choices.filter((choice) => choice.kind === 'device'),
    [choices],
  )
  const connectedOthers = useMemo(
    () => choices.filter((choice) => choice.kind === 'registry'),
    [choices],
  )
  const all = [
    ...subscriptions,
    ...keyed,
    ...devices,
    ...connectedOthers,
    ...extra,
  ]

  // Connected choices start checked, and recommended ones until the first
  // provider is connected; the user's own clicks win after.
  const nothingConnected = !choices.some((choice) => choice.ready)
  const draftFor = (choice: ProviderChoice): Draft =>
    resolveDraft(drafts.get(choice.providerId), {
      selected: choice.ready || preselected(choice, nothingConnected),
      key:
        choice.kind === 'key' ? defaultKeyInput(choice.detection) : undefined,
    })
  const update = (choice: ProviderChoice, next: Partial<Draft>) =>
    setDrafts((current) =>
      new Map(current).set(choice.providerId, { ...draftFor(choice), ...next }),
    )

  const firstScan = snapshot.tools === null
  const keysFound = keyed.some(
    (choice) =>
      choice.kind === 'key' &&
      choice.detection !== null &&
      (choice.detection.stored || choice.detection.sources.length > 0),
  )
  const selections: ProviderSelection[] = all
    .filter((choice) => !choice.ready && draftFor(choice).selected)
    .map((choice) => ({ choice, key: draftFor(choice).key }))
  const removals = all.filter(
    (choice) => choice.ready && !draftFor(choice).selected,
  )
  const incomplete = selections.some(
    ({ choice, key }) => choice.kind === 'key' && !keyInputReady(key),
  )
  const plan = connectPlan(
    selections,
    snapshot.installed,
    snapshot.envFile,
    removals,
  )
  const log = activity.filter((entry) => entry.group === 'models')
  const busy = running !== null
  const anyReady = all.some((choice) => choice.ready)
  const changes = selections.length + removals.length
  const needsKey = selections.filter(({ choice }) => choice.kind === 'key') as {
    choice: Extract<ProviderChoice, { kind: 'key' }>
    key?: KeyInput
  }[]

  const apply = async () => {
    if (await run('models', plan)) setDrafts(draftsAfterConnect(drafts))
  }

  const renderTile = (choice: ProviderChoice) => (
    <ProviderTile
      key={choice.providerId}
      choice={choice}
      selected={draftFor(choice).selected}
      disabled={busy || !usable(choice)}
      onToggle={(selected) => update(choice, { selected })}
    />
  )

  return (
    <StepLayout
      footer={
        <>
          <Button variant="outline" onClick={onBack} disabled={busy}>
            Back
          </Button>
          <span className="flex items-center gap-2">
            {changes === 0 && !anyReady ? (
              <Button variant="ghost" onClick={onNext} disabled={busy}>
                Skip for now
              </Button>
            ) : null}
            {changes === 0 && anyReady ? (
              <Button onClick={onNext} disabled={busy}>
                Continue
              </Button>
            ) : (
              <Button
                onClick={() => void apply()}
                disabled={busy || changes === 0 || incomplete}
              >
                {running === 'models' ? (
                  <LoaderCircle
                    className="animate-spin motion-reduce:animate-none"
                    aria-hidden
                  />
                ) : null}
                {running === 'models'
                  ? 'Connecting…'
                  : connectLabel(selections.length, removals.length)}
              </Button>
            )}
          </span>
        </>
      }
    >
      <StepHeader
        title="Connect a model"
        lead="Bring a subscription you already use, or connect with an API key."
        action={
          <ScanButton
            scanning={scanning}
            disabled={busy}
            onScan={() => void refresh()}
          />
        }
      />

      {snapshot.toolsError ? (
        <p
          role="alert"
          className="rounded-lg border border-alert/30 bg-alert-muted px-3 py-2 font-sans text-[13px] text-ink"
        >
          Unable to scan this machine: {snapshot.toolsError}
        </p>
      ) : null}

      {firstScan ? (
        <Section title="Looking at this machine">
          <Rows>
            {[0, 1].map((key) => (
              <div key={key} className="flex h-11 items-center gap-3 px-3.5">
                <Skeleton className="size-4 rounded-sm" />
                <Skeleton className="h-3.5 w-40 max-w-[50%]" />
              </div>
            ))}
          </Rows>
        </Section>
      ) : (
        <>
          <Section title="Subscriptions" hint="uses your plan, no API key">
            <Rows>
              {subscriptions.map((choice) =>
                choice.kind === 'subscription' ? (
                  <SubscriptionRow
                    key={choice.providerId}
                    choice={choice}
                    selected={draftFor(choice).selected}
                    disabled={busy || !usable(choice)}
                    onToggle={(selected) => update(choice, { selected })}
                    scanning={scanning}
                    onScan={() => void refresh()}
                  />
                ) : null,
              )}
            </Rows>
          </Section>

          {devices.length > 0 ? (
            <Section
              title="Sign in from here"
              hint="uses your plan, no API key"
            >
              <Rows>
                {devices.map((choice) =>
                  choice.kind === 'device' ? (
                    <DeviceRow
                      key={choice.providerId}
                      choice={choice}
                      selected={draftFor(choice).selected}
                      disabled={busy}
                      onToggle={(selected) => update(choice, { selected })}
                      onConnected={() =>
                        void run('models', [
                          {
                            kind: 'wait-models',
                            providerId: choice.providerId,
                            title: choice.title,
                          },
                        ])
                      }
                    />
                  ) : null,
                )}
              </Rows>
            </Section>
          ) : null}

          <Section
            title="API keys"
            hint={
              snapshot.detections !== null && !keysFound
                ? `none found in your shell profile or ${snapshot.envFile}`
                : 'billed by the provider'
            }
          >
            <div className="grid gap-2 @sm:grid-cols-2">
              {[...keyed, ...connectedOthers].map(renderTile)}
            </div>
            {needsKey.map(({ choice, key }) => (
              <div
                key={choice.providerId}
                className="flex flex-col rounded-lg border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950"
              >
                <span className="flex h-9 items-center gap-2 border-b border-neutral-200 px-3.5 font-sans text-[13px] font-medium text-ink dark:border-neutral-800">
                  <ProviderMark
                    id={choice.providerId}
                    label={choice.title}
                    className="size-4 text-ink"
                  />
                  {choice.title} key
                  <span className="ml-auto font-mono text-[11px] font-normal text-neutral-500 dark:text-neutral-400">
                    {choice.provider.envVar}
                  </span>
                </span>
                <KeyField
                  envVar={choice.provider.envVar}
                  detection={choice.detection}
                  value={key}
                  onChange={(next) => update(choice, { key: next })}
                  keysUrl={choice.provider.keysUrl}
                  stores={ROUTER_KEY_STORES}
                  envFile={snapshot.envFile}
                  className="px-3.5 py-3"
                />
              </div>
            ))}
          </Section>

          {extra.length > 0 ? (
            <Section
              title="More providers"
              hint="adds the worker; finish setup in the model picker"
            >
              <div className="grid gap-2 @sm:grid-cols-2">
                {extra.map(renderTile)}
              </div>
            </Section>
          ) : null}
        </>
      )}

      <EngineLog plan={plan} entries={log} running={running === 'models'} />
    </StepLayout>
  )
}

/**
 * After Connect: every checkbox stays as the person left it — a recommended
 * choice they unchecked must not come back checked — and typed keys are
 * dropped so a key is not kept in memory once stored.
 */
export function draftsAfterConnect(
  drafts: ReadonlyMap<string, Draft>,
): ReadonlyMap<string, Draft> {
  return new Map(
    [...drafts].map(([id, draft]) => [id, { selected: draft.selected }]),
  )
}

/** The person's draft over the default; a dropped key falls back to it. */
export function resolveDraft(draft: Draft | undefined, fallback: Draft): Draft {
  if (!draft) return fallback
  return { ...draft, key: draft.key ?? fallback.key }
}

/**
 * Whether a choice starts checked. Recommendations are a starting point for
 * the first connection only: once a provider is connected, coming back to
 * this step (Back, or reopening setup) never re-checks the ones the person
 * left out.
 */
export function preselected(
  choice: ProviderChoice,
  nothingConnected: boolean,
): boolean {
  return (
    nothingConnected && choice.recommended && !choice.ready && usable(choice)
  )
}

/**
 * A provider that signs in from here with a device code (GitHub Copilot):
 * the sign-in stands where the checkbox goes until it lands.
 */
function DeviceRow({
  choice,
  selected,
  disabled,
  onToggle,
  onConnected,
}: {
  choice: Extract<ProviderChoice, { kind: 'device' }>
  selected: boolean
  disabled: boolean
  onToggle: (selected: boolean) => void
  onConnected: () => void
}) {
  const id = useId()
  return (
    <div
      className={cn(
        'flex flex-col gap-1 px-3.5 py-2.5 transition-colors duration-150 ease-[var(--motion-ease-standard)]',
        selected && choice.ready && 'bg-neutral-100 dark:bg-neutral-900',
      )}
    >
      <div className="flex items-start gap-3">
        <ProviderMark
          id={choice.providerId}
          label={choice.title}
          className="mt-0.5 size-4 shrink-0 text-ink"
        />
        <label
          htmlFor={id}
          className={cn(
            'flex min-w-0 flex-1 flex-col gap-px',
            !disabled && choice.ready && 'cursor-pointer',
          )}
        >
          <span className="font-sans text-[13px] font-medium leading-5 text-ink">
            {choice.title}
          </span>
          <span className="text-pretty font-sans text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
            {choice.reason}
          </span>
        </label>
        <span className="flex h-5 items-center gap-3">
          <ChoiceStatus choice={choice} selected={selected} />
          {choice.ready ? (
            <Checkbox
              id={id}
              aria-label={`Connect ${choice.title}`}
              checked={selected}
              disabled={disabled}
              onChange={(event) => onToggle(event.currentTarget.checked)}
            />
          ) : null}
        </span>
      </div>
      {choice.ready ? null : (
        <div className="pl-7">
          <DeviceSignIn
            provider={choice.provider}
            installed={choice.installed}
            onConnected={onConnected}
          />
        </div>
      )}
    </div>
  )
}

function connectLabel(adds: number, removes: number): string {
  if (removes === 0) {
    return adds > 1 ? `Connect ${adds} providers` : 'Connect'
  }
  if (adds === 0) {
    return removes > 1 ? `Remove ${removes} providers` : 'Remove provider'
  }
  return `Apply ${adds + removes} changes`
}

/**
 * The provider's state, said in the place the provider already is: its
 * model count once connected, what unchecking will do, a key that was
 * found.
 */
function ChoiceStatus({
  choice,
  selected,
}: {
  choice: ProviderChoice
  selected: boolean
}) {
  if (choice.ready) {
    return selected ? (
      <StatusChip tone="ok">
        {choice.modelCount} {choice.modelCount === 1 ? 'model' : 'models'}
      </StatusChip>
    ) : (
      <StatusChip tone="warn">Will be removed</StatusChip>
    )
  }
  if (choice.kind === 'subscription') {
    if (choice.usable) return <StatusChip tone="neutral">Signed in</StatusChip>
    return choice.tool?.installed ? (
      <StatusChip tone="warn">Not signed in</StatusChip>
    ) : (
      <StatusChip tone="neutral">Not installed</StatusChip>
    )
  }
  if (choice.kind === 'key' && choice.recommended) {
    return <StatusChip tone="neutral">Key found</StatusChip>
  }
  return null
}

/**
 * A compact provider: the whole tile toggles it, the checkbox stays the
 * accessible control.
 */
function ProviderTile({
  choice,
  selected,
  disabled,
  onToggle,
}: {
  choice: ProviderChoice
  selected: boolean
  disabled: boolean
  onToggle: (selected: boolean) => void
}) {
  return (
    <Checkbox
      aria-label={`Connect ${choice.title}`}
      title={choice.reason}
      checked={selected}
      disabled={disabled}
      onChange={(event) => onToggle(event.currentTarget.checked)}
      className={cn(
        'flex h-10 flex-row-reverse rounded-lg border px-3 transition-[background-color,border-color] duration-150 ease-[var(--motion-ease-standard)]',
        selected
          ? 'border-neutral-400 bg-neutral-100 dark:border-neutral-600 dark:bg-neutral-900'
          : 'border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950',
        !disabled &&
          !selected &&
          'hover:bg-neutral-50 dark:hover:bg-neutral-900',
      )}
      label={
        <span className="flex min-w-0 items-center gap-2.5">
          <ProviderMark
            id={choice.providerId}
            label={choice.title}
            className="size-4 shrink-0 text-ink"
          />
          <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-ink">
            {choice.title}
          </span>
          <ChoiceStatus choice={choice} selected={selected} />
        </span>
      }
    />
  )
}

/**
 * A coding agent: what it pays with, and — until it is signed in on this
 * machine — the exact commands that get it there.
 */
function SubscriptionRow({
  choice,
  selected,
  disabled,
  onToggle,
  scanning,
  onScan,
}: {
  choice: SubscriptionChoice
  selected: boolean
  disabled: boolean
  onToggle: (selected: boolean) => void
  scanning: boolean
  onScan: () => void
}) {
  const { tool, provider } = choice
  const id = useId()
  const needsSetup = !choice.ready && !choice.usable
  return (
    <div
      className={cn(
        'flex flex-col gap-1 px-3.5 py-2.5 transition-colors duration-150 ease-[var(--motion-ease-standard)]',
        selected && 'bg-neutral-100 dark:bg-neutral-900',
      )}
    >
      <div className="flex items-start gap-3">
        <ProviderMark
          id={choice.providerId}
          label={choice.title}
          className="mt-0.5 size-4 shrink-0 text-ink"
        />
        <label
          htmlFor={id}
          className={cn(
            'flex min-w-0 flex-1 flex-col gap-px',
            !disabled && 'cursor-pointer',
          )}
        >
          <span className="font-sans text-[13px] font-medium leading-5 text-ink">
            {choice.title}
          </span>
          <span className="text-pretty font-sans text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
            {subscriptionLine(choice)}
          </span>
          {tool?.signed_in && tool.credentials_path ? (
            <span className="sr-only">
              Sign-in at{' '}
              <span className="font-mono text-xs">{tool.credentials_path}</span>
            </span>
          ) : null}
        </label>
        <span className="flex h-5 items-center gap-3">
          <ChoiceStatus choice={choice} selected={selected} />
          <Checkbox
            id={id}
            aria-label={`Connect ${choice.title}`}
            checked={selected}
            disabled={disabled}
            onChange={(event) => onToggle(event.currentTarget.checked)}
          />
        </span>
      </div>
      {needsSetup ? (
        <div className="mt-1.5 flex flex-col gap-1.5 pl-7">
          <Terminal>
            {!tool?.installed ? (
              <CommandLine command={provider.install} />
            ) : null}
            <CommandLine command={provider.signIn} note={provider.signInNote} />
          </Terminal>
          <span className="flex min-h-7 flex-wrap items-center justify-between gap-x-3 gap-y-1 pl-1">
            <span className="font-sans text-xs text-neutral-500 dark:text-neutral-400">
              Run {tool?.installed ? 'this' : 'these'} in a terminal, then scan
              again.
            </span>
            <ScanButton scanning={scanning} onScan={onScan} />
          </span>
        </div>
      ) : null}
    </div>
  )
}

function subscriptionLine(choice: SubscriptionChoice): React.ReactNode {
  const { tool, provider } = choice
  if (choice.ready || choice.usable) return `Uses ${provider.plan}.`
  if (!tool?.installed) {
    return `Not on this machine yet. Install it and sign in to use ${provider.plan}.`
  }
  if (tool.sign_in_note) return `${sentence(tool.sign_in_note)}.`
  return `Installed, but not signed in. Sign in to use ${provider.plan}.`
}

function sentence(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

const CATALOG_ORDER = new Map(
  [...SUBSCRIPTION_PROVIDERS, ...KEY_PROVIDERS].map((provider, index) => [
    provider.worker,
    index,
  ]),
)

/**
 * Catalog order: `providerChoices` ranks connected ones first, which would
 * move a provider the moment it connects.
 */
function catalogOrder(choices: ProviderChoice[]): ProviderChoice[] {
  const index = (choice: ProviderChoice) =>
    CATALOG_ORDER.get(choice.worker) ?? CATALOG_ORDER.size
  return [...choices].sort((a, b) => index(a) - index(b))
}

/** A subscription provider needs its CLI signed in on this machine. */
function usable(choice: ProviderChoice): boolean {
  return choice.kind !== 'subscription' || choice.usable || choice.ready
}
