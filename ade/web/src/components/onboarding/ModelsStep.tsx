import { ChevronDown, ChevronRight, RefreshCw } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { DeviceSignIn } from '@/components/chat/DeviceSignIn'
import { ProviderIcon } from '@/components/chat/ProviderIcon'
import { Button } from '@/components/ui/Button'
import { Checkbox } from '@/components/ui/Checkbox'
import { Skeleton } from '@/components/ui/Skeleton'
import { SECRETS_WORKER } from '@/lib/onboarding/catalog'
import {
  connectPlan,
  type KeyInput,
  type PlanStep,
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

const SECRETS_PLAN: PlanStep[] = [
  {
    kind: 'add-workers',
    workers: [SECRETS_WORKER],
    why: {
      [SECRETS_WORKER]:
        'Looks for keys in your shell profile and this project’s .env, and stores the ones you pick encrypted. Values never reach the browser.',
    },
  },
]

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
  const {
    snapshot,
    scanning,
    refresh,
    activity,
    running,
    run,
    secretsInstalled,
  } = onboarding
  const [registry, setRegistry] = useState<RegistryProviderRow[]>([])
  const [drafts, setDrafts] = useState<ReadonlyMap<string, Draft>>(new Map())
  const [showMore, setShowMore] = useState(true)
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
  const draftFor = (choice: ProviderChoice): Draft =>
    resolveDraft(drafts.get(choice.providerId), {
      selected: choice.recommended && !choice.ready && usable(choice),
      key:
        choice.kind === 'key' ? defaultKeyInput(choice.detection) : undefined,
    })
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
  const plan = connectPlan(selections, snapshot.installed)
  const log = activity.filter(
    (entry) => entry.group === 'keys' || entry.group === 'models',
  )
  const busy = running !== null
  const anyReady = ready.length > 0
  // Once something is connected or recommended, the long tail waits behind
  // a toggle instead of pushing the result down.
  const collapsible = anyReady || recommended.length > 0

  const connect = async () => {
    const ok = await run('models', plan)
    if (ok) {
      setConnected(true)
      setDrafts(draftsAfterConnect)
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
          {choice.kind === 'device' ? (
            // Signs in on its own below, not through Connect.
            <span className="mt-1.5 size-4 shrink-0" aria-hidden />
          ) : (
            <Checkbox
              aria-label={`Connect ${choice.title}`}
              checked={draft.selected}
              disabled={disabled}
              onChange={(event) =>
                update(choice, { selected: event.currentTarget.checked })
              }
              className="mt-1.5"
            />
          )}
          <ProviderIcon
            label={choice.title}
            className="mt-1.5 size-4 text-ink"
          />
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="flex flex-wrap items-center gap-2">
              <span className="font-sans text-[14px] font-medium text-ink">
                {choice.title}
              </span>
              {tool?.installed && !tool.signed_in ? (
                <StatusChip tone="warn">Not signed in</StatusChip>
              ) : choice.kind === 'subscription' || choice.kind === 'device' ? (
                <StatusChip tone="neutral">No API key</StatusChip>
              ) : null}
            </span>
            <span className="text-pretty font-sans text-[13px] leading-relaxed text-ink">
              {choice.reason}
            </span>
            {tool?.installed || tool?.signed_in ? (
              <ToolDetails tool={tool} />
            ) : null}
            {!choice.installed && choice.kind !== 'device' ? (
              <span className="font-mono text-[12px] text-ink">
                adds {choice.worker}
                {version ? `@${version}` : ''}
              </span>
            ) : null}
          </span>
        </div>
        {choice.kind === 'device' ? (
          <div className="px-3 pb-3 pl-10">
            <DeviceSignIn
              provider={choice.provider}
              installed={choice.installed}
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
          </div>
        ) : null}
        {draft.selected && choice.kind === 'key' ? (
          <KeyField
            envVar={choice.provider.envVar}
            detection={choice.detection}
            value={draft.key}
            onChange={(key) => update(choice, { key })}
            keysUrl={choice.provider.keysUrl}
            secretsReady={secretsInstalled}
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
        <p className="font-sans text-[13px] text-alert-strong">
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
                  className="size-4 text-ink"
                />
                <span className="flex-1 font-sans text-[14px] text-ink">
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
        title={
          running === 'models'
            ? 'Connecting'
            : running === 'keys'
              ? 'Adding the secrets worker'
              : 'What setup did'
        }
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

      {firstScan ? null : !secretsInstalled ? (
        <Section title="Keys you already have">
          <div className="flex flex-col gap-3 rounded-md bg-surface px-3 py-3">
            <p className="font-sans text-[14px] leading-relaxed text-ink">
              Looking for keys you've already exported uses the{' '}
              <span className="font-mono">secrets</span> worker. It keeps API
              keys out of every file you commit — today a key pasted into
              configuration is saved as plain text under{' '}
              <span className="font-mono">./config</span>.
            </p>
            <PlanPreview steps={SECRETS_PLAN} title="Adding it does this" />
            <div>
              <Button
                variant="pill"
                onClick={() => void run('keys', SECRETS_PLAN)}
                disabled={busy}
              >
                {running === 'keys'
                  ? 'Adding the secrets worker…'
                  : 'Add secrets worker and check for keys'}
              </Button>
            </div>
          </div>
        </Section>
      ) : snapshot.detections !== null && !keysFound ? (
        <p className="rounded-md bg-surface px-3 py-3 font-sans text-[14px] text-ink">
          No provider keys in your shell profile or this project's .env. Paste
          one below, or sign in to a coding agent and scan again.
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

/**
 * After Connect: every checkbox stays as the person left it — a recommended
 * choice they unchecked must not come back checked — and typed keys are
 * dropped, so a key is not kept in memory once stored.
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

/** Where the coding agent's CLI and its sign-in live — paths, never content. */
function ToolDetails({ tool }: { tool: ToolScan }) {
  const cli = [tool.binary_path, tool.version].filter(Boolean).join(' · ')
  return (
    <>
      {tool.signed_in && tool.credentials_path ? (
        <span className="truncate font-sans text-[13px] text-ink">
          Sign-in at{' '}
          <span className="font-mono text-[12px]">{tool.credentials_path}</span>
        </span>
      ) : null}
      {cli ? (
        <span className="truncate font-mono text-[12px] text-ink">{cli}</span>
      ) : null}
    </>
  )
}

/** A subscription provider needs its CLI signed in on this machine. */
function usable(choice: ProviderChoice): boolean {
  return choice.kind !== 'subscription' || choice.usable
}

/**
 * A coding agent installed here but not signed in, or a provider that signs
 * in from here (GitHub Copilot): one sign-in from usable, so it stays beside
 * the recommendations, saying what to do, instead of in the long tail.
 */
function oneSignInAway(choice: ProviderChoice): boolean {
  if (choice.kind === 'device') return true
  return (
    choice.kind === 'subscription' &&
    !choice.usable &&
    choice.tool?.installed === true
  )
}
