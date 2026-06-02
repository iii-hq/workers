import { CopyCommandButton } from '@/components/chat/sandbox/terminal/CopyCommandButton'
import { Terminal } from '@/components/chat/sandbox/terminal/Terminal'
import { Button } from '@/components/ui/Button'
import { Prompt } from '@/components/ui/Prompt'
import type { InstallStage } from '@/hooks/use-harness-status'
import { normalizeErrorMessage } from '@/lib/providers'
import { cn } from '@/lib/utils'

/**
 * The chat empty state, as a small set of presentational variants:
 *
 *   - `ready`          — harness present + a model available (the hero copy)
 *   - `no-provider`    — harness present, no model configured -> configure CTA
 *   - `no-harness`     — harness worker missing -> install CTA + cli hint
 *   - `installing`     — `worker::add` in flight -> live console
 *   - `install-failed` — add failed -> console + retry
 *
 * The component is intentionally dumb: `MessageList` derives the variant from
 * `ConversationsContext` (harness presence + model catalog) and passes the
 * callbacks. Keeping it props-only is what lets every state render in
 * Storybook without a provider.
 */

export type EmptyStateVariant =
  | 'ready'
  | 'no-provider'
  | 'no-harness'
  | 'installing'
  | 'install-failed'

export interface EmptyStateProps {
  variant: EmptyStateVariant
  density?: 'route' | 'dock'
  /** Console lines for `installing` / `install-failed`. */
  stages?: InstallStage[]
  /** Surfaced under the console when an add fails with no stage detail. */
  errorMessage?: string | null
  /** `no-harness` primary CTA. */
  onInstallHarness?: () => void
  /** `install-failed` retry CTA. */
  onRetryInstall?: () => void
  /** `no-provider` CTA (defaults to opening the harness configuration). */
  onConfigureProvider?: () => void
}

const HARNESS_INSTALL_COMMAND = 'iii worker add harness'

const HEADING_CLASS =
  'font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase'
const BODY_CLASS =
  'font-mono text-[14px] leading-[1.7] text-ink-faint lowercase'

export function EmptyState({
  variant,
  density = 'route',
  stages = [],
  errorMessage,
  onInstallHarness,
  onRetryInstall,
  onConfigureProvider,
}: EmptyStateProps) {
  const emptyPad = density === 'dock' ? 'px-4' : 'px-9'
  const eyebrow =
    variant === 'ready' || variant === 'no-provider' ? 'new session' : 'setup'

  return (
    <div className={cn('flex-1 flex items-center justify-center', emptyPad)}>
      <div className="max-w-[520px] w-full flex flex-col gap-6">
        <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          <Prompt symbol="$">{eyebrow}</Prompt>
        </div>

        {variant === 'ready' ? <ReadyBody /> : null}
        {variant === 'no-provider' ? (
          <NoProviderBody onConfigureProvider={onConfigureProvider} />
        ) : null}
        {variant === 'no-harness' ? (
          <NoHarnessBody onInstallHarness={onInstallHarness} />
        ) : null}
        {variant === 'installing' || variant === 'install-failed' ? (
          <InstallingBody
            failed={variant === 'install-failed'}
            stages={stages}
            errorMessage={errorMessage}
            onRetryInstall={onRetryInstall}
          />
        ) : null}
      </div>
    </div>
  )
}

/* ---------------- ready (unchanged hero) ---------------- */

function ReadyBody() {
  return (
    <>
      <h1 className={HEADING_CLASS}>how can i help.</h1>
      <p className={BODY_CLASS}>
        pick a mode and a model, attach files if you need to, then send a
        message. responses are mocked locally — swap{' '}
        <code className="font-mono text-[12.5px] border border-rule-2 bg-paper-2 text-ink px-1">
          lib/backend/real.ts
        </code>{' '}
        for a real provider when you're ready.
      </p>
      <ul className="font-mono text-[13px] leading-[1.7] text-ink-faint flex flex-col gap-1">
        <li>
          · <span className="text-ink">plan</span> — outline an approach before
          doing.
        </li>
        <li>
          · <span className="text-ink">ask</span> — answer a question with
          context.
        </li>
        <li>
          · <span className="text-ink">agent</span> — take action and report
          back.
        </li>
      </ul>
    </>
  )
}

/* ---------------- no provider configured ---------------- */

function NoProviderBody({
  onConfigureProvider,
}: {
  onConfigureProvider?: () => void
}) {
  return (
    <>
      <h1 className={HEADING_CLASS}>configure a provider.</h1>
      <p className={BODY_CLASS}>
        the harness worker is running, but no model providers are set up yet.
        add an api key — or point at a local server — then pick a model and
        send.
      </p>
      <div>
        <Button variant="primary" onClick={onConfigureProvider}>
          configure a provider
        </Button>
      </div>
    </>
  )
}

/* ---------------- harness not installed ---------------- */

function NoHarnessBody({
  onInstallHarness,
}: {
  onInstallHarness?: () => void
}) {
  return (
    <>
      <h1 className={HEADING_CLASS}>install the harness worker.</h1>
      <p className={BODY_CLASS}>
        chat runs on the <span className="text-ink">harness</span> worker, and
        it isn't connected yet. add it to start a session.
      </p>
      <div className="flex flex-col gap-4">
        <div>
          <Button variant="primary" onClick={onInstallHarness}>
            add harness
          </Button>
        </div>
        <div className="flex flex-col gap-1.5">
          <span className="font-mono text-[12px] text-ink-ghost lowercase">
            prefer the terminal? run:
          </span>
          <Terminal
            command={HARNESS_INSTALL_COMMAND}
            chips={<CopyCommandButton text={HARNESS_INSTALL_COMMAND} />}
          />
        </div>
      </div>
    </>
  )
}

/* ---------------- installing / install failed ---------------- */

function InstallingBody({
  failed,
  stages,
  errorMessage,
  onRetryInstall,
}: {
  failed: boolean
  stages: InstallStage[]
  errorMessage?: string | null
  onRetryInstall?: () => void
}) {
  return (
    <>
      <h1 className={HEADING_CLASS}>
        {failed ? 'install failed.' : 'installing harness…'}
      </h1>
      <p className={BODY_CLASS}>
        {failed
          ? "the worker manager couldn't finish adding harness. you can retry here, or run it from your terminal."
          : 'adding the harness worker. this mirrors what the cli does — you could run the same command in your terminal.'}
      </p>
      <InstallConsole stages={stages} running={!failed} />
      {failed ? (
        <div className="flex flex-col gap-3">
          {errorMessage && stages.length === 0 ? (
            <span className="font-mono text-[12.5px] text-alert lowercase break-all">
              {errorMessage}
            </span>
          ) : null}
          <div>
            <Button variant="primary" onClick={onRetryInstall}>
              retry
            </Button>
          </div>
        </div>
      ) : null}
    </>
  )
}

/**
 * Live install console — `Terminal` chrome over the streamed `worker`
 * lifecycle stages, framed as the equivalent `iii worker add harness` run.
 */
function InstallConsole({
  stages,
  running,
}: {
  stages: InstallStage[]
  running: boolean
}) {
  return (
    <Terminal command={HARNESS_INSTALL_COMMAND}>
      <div className="bg-bg px-3 py-3 font-mono text-[12.5px] leading-[1.6] flex flex-col gap-0.5">
        {stages.length === 0 ? (
          <div className="text-ink-faint italic animate-pulse">
            · connecting to the worker manager…
          </div>
        ) : (
          stages.map((s, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: append-only log; entries never reorder or get removed
            <StageLine key={`${s.stage}-${s.at}-${i}`} stage={s} />
          ))
        )}
        {running && stages.length > 0 ? (
          <div className="text-ink-ghost animate-pulse">· working…</div>
        ) : null}
      </div>
    </Terminal>
  )
}

function StageLine({ stage }: { stage: InstallStage }) {
  const failed = stage.stage === 'failed'
  const done = stage.stage === 'done'
  const tone = failed ? 'text-alert' : done ? 'text-accent' : 'text-ink-faint'
  const symbol = failed ? '×' : done ? '✓' : '→'
  const showProgress = typeof stage.progress === 'number' && !done && !failed
  return (
    <div className={cn('flex items-baseline gap-2', tone)}>
      <span aria-hidden className="text-ink-ghost">
        {symbol}
      </span>
      <span className="break-all">{stageLabel(stage)}</span>
      {showProgress ? (
        <span className="ml-auto text-ink-ghost tabular-nums">
          {Math.round((stage.progress ?? 0) * 100)}%
        </span>
      ) : null}
    </div>
  )
}

function stageLabel(stage: InstallStage): string {
  switch (stage.stage) {
    case 'started':
      return 'resolving harness from the registry…'
    case 'downloading':
      return 'downloading harness…'
    case 'downloaded':
      return 'downloaded'
    case 'done':
      return 'done · harness ready'
    case 'failed':
      return stage.error
        ? `failed · ${normalizeErrorMessage(stage.error)}`
        : 'failed'
    default:
      return stage.stage
  }
}
