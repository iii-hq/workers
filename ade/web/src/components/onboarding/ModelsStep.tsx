import { ChevronDown, ChevronRight, RefreshCw } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { ProviderIcon } from '@/components/chat/ProviderIcon'
import { Button } from '@/components/ui/Button'
import { Checkbox } from '@/components/ui/Checkbox'
import { Skeleton } from '@/components/ui/Skeleton'
import {
  connectPlan,
  type KeyInput,
  type ProviderChoice,
  type ProviderSelection,
  providerChoices,
  type RegistryProviderRow,
  registryChoices,
  type ToolScan,
} from '@/lib/onboarding/plan'
import { cn } from '@/lib/utils'
import { fetchRegistryProviders } from '@/lib/workers-registry'
import {
  ActivityLog,
  defaultKeyInput,
  KeyField,
  keyInputReady,
  PlanPreview,
  Rows,
  Section,
  StatusChip,
  StepHeader,
  StepLayout,
} from './parts'
import type { OnboardingController } from './use-onboarding'

interface Draft {
  selected: boolean
  key?: KeyInput
}

/**
 * llm-router resolves `secret://` and `env://` references alike, so a
 * provider key can stay encrypted or live in this project's `.env`.
 */
const ROUTER_KEY_STORES = ['vault', 'env'] as const

/**
 * The one step that gets a model connected. It starts from what this
 * machine already has — a coding agent signed in, a key already exported —
 * and recommends exactly that, so most of the time there is nothing to type.
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
  const [showMore, setShowMore] = useState(false)
  const [connected, setConnected] = useState(false)

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
  const versions = useMemo(
    () => new Map(registry.map((row) => [row.name, row.version])),
    [registry],
  )

  // Recommended choices start selected; the user's own clicks win after.
  const draftFor = (choice: ProviderChoice): Draft => {
    const draft = drafts.get(choice.providerId)
    if (draft) return draft
    return {
      selected: choice.recommended && !choice.ready && usable(choice),
      key:
        choice.kind === 'key' ? defaultKeyInput(choice.detection) : undefined,
    }
  }
  const update = (choice: ProviderChoice, next: Partial<Draft>) =>
    setDrafts((current) =>
      new Map(current).set(choice.providerId, { ...draftFor(choice), ...next }),
    )

  const firstScan = snapshot.tools === null
  const ready = choices.filter((choice) => choice.ready)
  const recommended = choices.filter(
    (choice) => !choice.ready && (choice.recommended || oneSignInAway(choice)),
  )
  const others: ProviderChoice[] = [
    ...choices.filter(
      (choice) =>
        !choice.ready && !choice.recommended && !oneSignInAway(choice),
    ),
    ...extra,
  ]
  const keysFound = choices.some(
    (choice) =>
      choice.kind === 'key' &&
      choice.detection !== null &&
      (choice.detection.stored || choice.detection.sources.length > 0),
  )

  const selections: ProviderSelection[] = [...choices, ...extra]
    .filter((choice) => !choice.ready && draftFor(choice).selected)
    .map((choice) => ({ choice, key: draftFor(choice).key }))
  const incomplete = selections.some(
    ({ choice, key }) => choice.kind === 'key' && !keyInputReady(key),
  )
  const plan = connectPlan(selections, snapshot.installed, snapshot.envFile)
  const log = activity.filter((entry) => entry.group === 'models')
  const busy = running !== null
  const anyReady = ready.length > 0
  // Once something is connected or recommended, the long tail waits behind
  // a toggle instead of pushing the result down.
  const collapsible = anyReady || recommended.length > 0

  const connect = async () => {
    const ok = await run('models', plan)
    if (ok) {
      setConnected(true)
      setDrafts(new Map())
    }
  }

  const renderChoice = (choice: ProviderChoice) => {
    const draft = draftFor(choice)
    const version = versions.get(choice.worker)
    const disabled = busy || !usable(choice)
    const tool = choice.kind === 'subscription' ? choice.tool : null
    return (
      <div key={choice.providerId} className="flex flex-col">
        <div
          className={cn(
            'flex items-start gap-3 px-3 py-3',
            !usable(choice) && 'opacity-60',
          )}
        >
          <Checkbox
            aria-label={`Connect ${choice.title}`}
            checked={draft.selected}
            disabled={disabled}
            onChange={(event) =>
              update(choice, { selected: event.currentTarget.checked })
            }
            className="mt-1.5"
          />
          <ProviderIcon
            label={choice.title}
            className="mt-1.5 size-4 text-ink-faint"
          />
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="flex flex-wrap items-center gap-2">
              <span className="font-sans text-[13px] font-medium text-ink">
                {choice.title}
              </span>
              {tool?.installed && !tool.signed_in ? (
                <StatusChip tone="warn">Not signed in</StatusChip>
              ) : choice.kind === 'subscription' ? (
                <StatusChip tone="neutral">No API key</StatusChip>
              ) : null}
            </span>
            <span className="text-pretty font-sans text-[12px] leading-relaxed text-ink-faint">
              {choice.reason}
            </span>
            {tool?.installed ? <ToolDetails tool={tool} /> : null}
            {!choice.installed ? (
              <span className="font-mono text-[11px] text-ink-ghost">
                adds {choice.worker}
                {version ? `@${version}` : ''}
              </span>
            ) : null}
          </span>
        </div>
        {draft.selected && choice.kind === 'key' ? (
          <KeyField
            envVar={choice.provider.envVar}
            detection={choice.detection}
            value={draft.key}
            onChange={(key) => update(choice, { key })}
            keysUrl={choice.provider.keysUrl}
            stores={ROUTER_KEY_STORES}
            envFile={snapshot.envFile}
          />
        ) : null}
      </div>
    )
  }

  const primary = connected || (selections.length === 0 && anyReady)
  return (
    <StepLayout
      footer={
        <>
          <Button variant="ghost" onClick={onBack} disabled={busy}>
            Back
          </Button>
          <span className="flex items-center gap-2">
            {!primary && !anyReady && selections.length === 0 ? (
              <Button variant="ghost" onClick={onNext} disabled={busy}>
                Skip for now
              </Button>
            ) : null}
            {primary ? (
              <Button onClick={onNext} disabled={busy}>
                Continue
              </Button>
            ) : (
              <Button
                onClick={() => void connect()}
                disabled={busy || selections.length === 0 || incomplete}
              >
                {running === 'models'
                  ? 'Connecting…'
                  : selections.length > 1
                    ? `Connect ${selections.length} providers`
                    : 'Connect'}
              </Button>
            )}
          </span>
        </>
      }
    >
      <StepHeader
        eyebrow="Step 1 of 2"
        title="Connect a model"
        lead="Pick at least one. What's recommended comes from what this machine already has: a coding agent you're signed in to needs no API key, and a key you've already exported is reused without copying it anywhere."
        action={
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void refresh()}
            disabled={scanning || busy}
          >
            <RefreshCw
              className={scanning ? 'iii-ui-spin' : undefined}
              aria-hidden
            />
            Scan again
          </Button>
        }
      />

      {snapshot.toolsError ? (
        <p className="font-sans text-[12px] text-alert-strong">
          Could not scan this machine: {snapshot.toolsError}
        </p>
      ) : null}

      {ready.length > 0 ? (
        <Section title="Connected">
          <Rows>
            {ready.map((choice) => (
              <div
                key={choice.providerId}
                className="flex items-center gap-3 px-3 py-2.5"
              >
                <ProviderIcon
                  label={choice.title}
                  className="size-4 text-ink-faint"
                />
                <span className="flex-1 font-sans text-[13px] text-ink">
                  {choice.title}
                </span>
                <StatusChip tone="ok">
                  {choice.modelCount}{' '}
                  {choice.modelCount === 1 ? 'model' : 'models'}
                </StatusChip>
              </div>
            ))}
          </Rows>
        </Section>
      ) : null}

      {/* While it runs, and once it has, the log stands where the plan was:
          the same steps, now with what actually happened. */}
      <ActivityLog
        entries={log}
        title={running === 'models' ? 'Connecting' : 'What setup did'}
      />

      {firstScan ? (
        <Section title="Looking at this machine">
          <Rows>
            {[0, 1].map((key) => (
              <div key={key} className="flex items-center gap-3 px-3 py-3">
                <Skeleton className="size-4" />
                <Skeleton className="h-4 flex-1" />
              </div>
            ))}
          </Rows>
        </Section>
      ) : recommended.length > 0 ? (
        <Section title="Recommended for this machine">
          <Rows>{recommended.map(renderChoice)}</Rows>
        </Section>
      ) : null}

      {!firstScan && snapshot.detections !== null && !keysFound ? (
        <p className="rounded-md bg-surface px-3 py-3 font-sans text-[13px] text-ink-faint">
          No provider keys in your shell profile or this project's{' '}
          {snapshot.envFile}. Paste one below, or sign in to a coding agent and
          scan again.
        </p>
      ) : null}

      {!firstScan && others.length > 0 ? (
        <Section
          title={
            anyReady
              ? 'Add another provider'
              : recommended.length > 0
                ? 'Other providers'
                : 'Providers'
          }
          aside={
            collapsible ? (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setShowMore((value) => !value)}
                aria-expanded={showMore}
              >
                {showMore ? (
                  <ChevronDown aria-hidden />
                ) : (
                  <ChevronRight aria-hidden />
                )}
                {showMore ? 'Hide' : `Show ${others.length}`}
              </Button>
            ) : null
          }
        >
          {showMore || !collapsible ? (
            <Rows>{others.map(renderChoice)}</Rows>
          ) : null}
        </Section>
      ) : null}

      {running !== 'models' ? <PlanPreview steps={plan} /> : null}
    </StepLayout>
  )
}

/** Where the coding agent's CLI and its sign-in live — paths, never content. */
function ToolDetails({ tool }: { tool: ToolScan }) {
  const cli = [tool.binary_path, tool.version].filter(Boolean).join(' · ')
  return (
    <>
      {tool.signed_in && tool.credentials_path ? (
        <span className="truncate font-sans text-[12px] text-ink-faint">
          Sign-in at{' '}
          <span className="font-mono text-[11px]">{tool.credentials_path}</span>
        </span>
      ) : null}
      {cli ? (
        <span className="truncate font-mono text-[11px] text-ink-ghost">
          {cli}
        </span>
      ) : null}
    </>
  )
}

/** A subscription provider needs its CLI signed in on this machine. */
function usable(choice: ProviderChoice): boolean {
  return choice.kind !== 'subscription' || choice.usable
}

/**
 * A coding agent installed here but not signed in: one sign-in and a rescan
 * from usable, so it stays beside the recommendations, saying what to do,
 * instead of in the long tail.
 */
function oneSignInAway(choice: ProviderChoice): boolean {
  return (
    choice.kind === 'subscription' &&
    !choice.usable &&
    choice.tool?.installed === true
  )
}
