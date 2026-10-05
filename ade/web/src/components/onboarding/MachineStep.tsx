import { KeyRound, RefreshCw, SquareTerminal } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Skeleton } from '@/components/ui/Skeleton'
import { SECRETS_WORKER } from '@/lib/onboarding/catalog'
import {
  type PlanStep,
  preferredSource,
  sourceLabel,
  type ToolScan,
} from '@/lib/onboarding/plan'
import {
  ActivityLog,
  PlanPreview,
  Rows,
  Section,
  StatusChip,
  StepHeader,
  StepLayout,
} from './parts'
import type { OnboardingController } from './use-onboarding'

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

export function MachineStep({
  onboarding,
  onBack,
  onNext,
}: {
  onboarding: OnboardingController
  onBack: () => void
  onNext: () => void
}) {
  const { snapshot, scanning, refresh, activity, running, run } = onboarding
  const tools = snapshot.tools
  const connected = (snapshot.providers ?? []).filter(
    (provider) => provider.modelCount > 0,
  )
  const found = (snapshot.detections ?? []).filter(
    (detection) => detection.stored || detection.sources.length > 0,
  )
  const log = activity.filter((entry) => entry.group === 'machine')
  const busy = running !== null

  return (
    <StepLayout
      footer={
        <>
          <Button variant="ghost" onClick={onBack} disabled={busy}>
            Back
          </Button>
          <Button onClick={onNext} disabled={busy || scanning}>
            Continue
          </Button>
        </>
      }
    >
      <StepHeader
        eyebrow="Step 1 of 3"
        title="What's already on this machine"
        lead="A coding agent you're signed in to gives you models without an API key. A key you've already exported can be reused without copying it anywhere."
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

      <Section title="Coding agents">
        <Rows>
          {tools === null
            ? [0, 1].map((key) => (
                <div key={key} className="flex items-center gap-3 px-3 py-3">
                  <Skeleton className="size-8" />
                  <Skeleton className="h-4 flex-1" />
                </div>
              ))
            : (['claude-code', 'codex'] as const).map((id) => (
                <ToolRow
                  key={id}
                  tool={tools.find((tool) => tool.id === id)}
                  fallbackName={id === 'codex' ? 'Codex' : 'Claude Code'}
                />
              ))}
        </Rows>
        {snapshot.toolsError ? (
          <p className="font-sans text-[12px] text-alert-strong">
            Could not scan this machine: {snapshot.toolsError}
          </p>
        ) : null}
      </Section>

      <Section title="API keys">
        {onboarding.secretsInstalled ? (
          snapshot.detections === null ? (
            <Skeleton className="h-12 w-full" />
          ) : found.length > 0 ? (
            <Rows>
              {found.map((detection) => {
                const source = preferredSource(detection)
                return (
                  <div
                    key={detection.name}
                    className="flex items-center gap-3 px-3 py-2.5"
                  >
                    <KeyRound
                      className="size-4 shrink-0 text-ink-faint"
                      aria-hidden
                    />
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className="font-mono text-[12px] text-ink">
                        {detection.name}
                      </span>
                      <span className="font-sans text-[12px] text-ink-faint">
                        {detection.stored
                          ? 'In the secrets store'
                          : source
                            ? `Found in ${sourceLabel(source)}`
                            : null}
                      </span>
                    </span>
                    <span className="shrink-0 font-mono text-[11px] text-ink-faint">
                      {detection.stored
                        ? detection.stored_hint
                        : (source?.hint ?? '')}
                    </span>
                  </div>
                )
              })}
            </Rows>
          ) : (
            <p className="rounded-md bg-surface px-3 py-3 font-sans text-[13px] text-ink-faint">
              No provider keys in your shell profile or this project's .env. You
              can paste one in the next step, or use a coding agent above.
            </p>
          )
        ) : (
          <div className="flex flex-col gap-3 rounded-md bg-surface px-3 py-3">
            <p className="font-sans text-[13px] leading-relaxed text-ink">
              Checking for keys uses the{' '}
              <span className="font-mono">secrets</span> worker. It keeps API
              keys out of every file you commit — today a key pasted into
              configuration is saved as plain text under{' '}
              <span className="font-mono">./config</span>.
            </p>
            <PlanPreview steps={SECRETS_PLAN} title="Adding it does this" />
            <div>
              <Button
                variant="pill"
                onClick={() => void run('machine', SECRETS_PLAN)}
                disabled={busy}
              >
                {running === 'machine'
                  ? 'Adding the secrets worker…'
                  : 'Add secrets worker and check for keys'}
              </Button>
            </div>
          </div>
        )}
        <ActivityLog entries={log} />
      </Section>

      {connected.length > 0 ? (
        <Section title="Already connected">
          <Rows>
            {connected.map((provider) => (
              <div
                key={provider.id}
                className="flex items-center justify-between gap-3 px-3 py-2.5"
              >
                <span className="font-sans text-[13px] text-ink">
                  {provider.title}
                </span>
                <StatusChip tone="ok">
                  {provider.modelCount}{' '}
                  {provider.modelCount === 1 ? 'model' : 'models'}
                </StatusChip>
              </div>
            ))}
          </Rows>
        </Section>
      ) : null}
    </StepLayout>
  )
}

function ToolRow({
  tool,
  fallbackName,
}: {
  tool: ToolScan | undefined
  fallbackName: string
}) {
  const name = tool?.name ?? fallbackName
  const status = !tool?.installed
    ? { tone: 'neutral' as const, label: 'Not found' }
    : tool.signed_in
      ? { tone: 'ok' as const, label: 'Signed in' }
      : { tone: 'warn' as const, label: 'Not signed in' }
  const meta = [tool?.binary_path, tool?.version].filter(Boolean).join(' · ')
  return (
    <div className="flex items-center gap-3 px-3 py-2.5">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-surface text-ink-faint">
        <SquareTerminal className="size-4" aria-hidden />
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="font-sans text-[13px] font-medium text-ink">
          {name}
        </span>
        {meta ? (
          <span className="truncate font-mono text-[11px] text-ink-ghost">
            {meta}
          </span>
        ) : null}
        {tool?.signed_in && tool.credentials_path ? (
          <span className="truncate font-sans text-[12px] text-ink-faint">
            Sign-in at{' '}
            <span className="font-mono text-[11px]">
              {tool.credentials_path}
            </span>
          </span>
        ) : tool?.installed && tool.sign_in_note ? (
          <span className="font-sans text-[12px] text-ink-faint">
            {tool.sign_in_note}
          </span>
        ) : null}
      </span>
      <StatusChip tone={status.tone}>{status.label}</StatusChip>
    </div>
  )
}
