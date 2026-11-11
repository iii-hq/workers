import { Button } from '@/components/ui/Button'
import { CommandCard } from '@/components/ui/CommandCard'
import { Wordmark } from '@/components/ui/Wordmark'
import type { HarnessAction } from '@/hooks/use-harness-status'
import { cn } from '@/lib/utils'
import type { WorktreePickerOptions } from './DirectoryPicker'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * The chat empty state, as a small set of presentational variants:
 *
 *   - `ready`           — harness present + a model available (the hero copy)
 *   - `no-provider`     — harness present, no model configured -> configure CTA
 *   - `no-harness`      — harness worker missing -> install CTA + cli hint
 *   - `harness-stopped` — harness declared in compose but not running ->
 *                         start / update-and-start CTAs + cli hint (an add
 *                         would change nothing here)
 *   - `installing`      — the compose action (`action`) in flight -> live console
 *   - `install-failed`  — that action failed -> console + retry
 *
 * `MessageList` derives the variant from `ConversationsContext` (harness
 * presence + model catalog) and passes the callbacks. The ready state loads
 * agent profiles from Directory unless a deterministic catalog is supplied by a
 * story or test.
 */

export type EmptyStateVariant =
  | 'ready'
  | 'no-provider'
  | 'no-harness'
  | 'harness-stopped'
  | 'installing'
  | 'install-failed'

export interface EmptyStateProps {
  variant: EmptyStateVariant
  density?: 'route' | 'dock'
  /** Surfaced under the console when an action fails. */
  errorMessage?: string | null
  /**
   * Which compose action `installing` / `install-failed` shows: an add
   * (default), a start, or an update-and-start.
   */
  action?: HarnessAction
  /** `no-harness` primary CTA. */
  onInstallHarness?: () => void
  /** `harness-stopped` primary CTA. */
  onStartHarness?: () => void
  /** `harness-stopped` secondary CTA. */
  onUpdateHarness?: () => void
  /** `install-failed` retry CTA (repeats `action`). */
  onRetryInstall?: () => void
  /** `no-provider` CTA (opens the model/provider picker). */
  onConfigureProvider?: () => void
  /** `ready` project context and shared directory-picker behavior. */
  workingDir?: string | null
  onWorkingDirChange?: (next: string) => void
  workingDirError?: string | null
  defaultWorkingDir?: string | null
  worktreePicker?: WorktreePickerOptions
}

const HARNESS_INSTALL_COMMAND = 'iii call compose::add worker=harness'
const HARNESS_START_COMMAND = 'iii call compose::up container=harness'
const HARNESS_UPDATE_COMMAND = 'iii call compose::update worker=harness'

/** Copy for each compose action's console, running and failed. */
const ACTION_COPY: Record<
  HarnessAction,
  {
    command: string
    running: { heading: string; body: string; line: string }
    failed: { heading: string; body: string; line: string; retry: string }
  }
> = {
  add: {
    command: HARNESS_INSTALL_COMMAND,
    running: {
      heading: 'Installing harness…',
      body: 'Adding the harness worker. This runs the same command as the CLI, shown below.',
      line: '· adding harness to the compose project…',
    },
    failed: {
      heading: 'Harness install failed',
      body: "Compose couldn't finish adding the harness. Retry here, or run the command below in your terminal.",
      line: '× compose::add did not finish',
      retry: 'Retry install',
    },
  },
  start: {
    command: HARNESS_START_COMMAND,
    running: {
      heading: 'Starting harness…',
      body: 'Starting the harness container. This runs the same command as the CLI, shown below.',
      line: '· starting the harness container…',
    },
    failed: {
      heading: "Harness didn't start",
      body: "Compose couldn't start the harness container. Retry here, or run the command below in your terminal.",
      line: '× compose::up did not finish',
      retry: 'Retry start',
    },
  },
  update: {
    command: HARNESS_UPDATE_COMMAND,
    running: {
      heading: 'Updating harness…',
      body: 'Updating the harness worker to the latest release, then starting it. This runs the same command as the CLI, shown below.',
      line: '· updating harness, then starting it…',
    },
    failed: {
      heading: 'Harness update failed',
      body: "Compose couldn't finish updating the harness. Retry here, or run the command below in your terminal.",
      line: '× compose::update did not finish',
      retry: 'Retry update',
    },
  },
}

// One heading and one body style for every variant, the welcome hero's.
const HEADING_CLASS =
  'text-balance font-inter text-2xl font-medium tracking-tight text-ink @2xl:text-[28px]'
const BODY_CLASS =
  'max-w-md text-pretty font-inter text-sm leading-relaxed text-ink-faint'

export function EmptyState({
  variant,
  density = 'route',
  errorMessage,
  action = 'add',
  onInstallHarness,
  onStartHarness,
  onUpdateHarness,
  onRetryInstall,
  onConfigureProvider,
}: EmptyStateProps) {
  const emptyPad =
    density === 'dock' ? 'px-3 @2xl:px-4' : 'px-3 @2xl:px-6 @5xl:px-9'
  const eyebrow = variant === 'no-provider' ? 'New session' : 'Setup'

  return (
    <div
      className={cn(
        'flex-1 min-h-0 overflow-y-auto flex justify-center',
        variant === 'ready' && 'chat-welcome-heading',
        emptyPad,
      )}
    >
      <div
        className={cn(
          'my-auto flex w-full flex-col py-6',
          variant === 'ready'
            ? 'max-w-[680px] items-center gap-5 text-center'
            : 'max-w-[520px] gap-6',
        )}
      >
        {variant === 'ready' ? (
          <ReadyBody />
        ) : (
          <div className="font-sans text-base font-medium text-ink-faint sm:text-sm">
            {eyebrow}
          </div>
        )}
        {variant === 'no-provider' ? (
          <NoProviderBody onConfigureProvider={onConfigureProvider} />
        ) : null}
        {variant === 'no-harness' ? (
          <NoHarnessBody onInstallHarness={onInstallHarness} />
        ) : null}
        {variant === 'harness-stopped' ? (
          <StoppedHarnessBody
            onStartHarness={onStartHarness}
            onUpdateHarness={onUpdateHarness}
          />
        ) : null}
        {variant === 'installing' || variant === 'install-failed' ? (
          <InstallingBody
            action={action}
            failed={variant === 'install-failed'}
            errorMessage={errorMessage}
            onRetryInstall={onRetryInstall}
          />
        ) : null}
      </div>
    </div>
  )
}

/* ---------------- ready (welcome hero) ---------------- */

function ReadyBody() {
  return (
    <>
      <Wordmark appearance="inset" className="size-10 fill-ink-faint/60" />
      <div className="flex max-w-full flex-col items-center gap-3">
        <h1 className={HEADING_CLASS}>What would you like to build?</h1>
        <p className={BODY_CLASS}>
          Explore your project, fix an issue, or start something new.
        </p>
      </div>
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
      <h1 className={HEADING_CLASS}>Connect a model</h1>
      <p className={BODY_CLASS}>
        The harness is running, but no model is connected yet. Setup finds
        Claude Code, Codex or an API key already on this machine and connects it
        in a couple of clicks.
      </p>
      <div>
        <Button variant="primary" onClick={onConfigureProvider}>
          Set up a model
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
      <h1 className={HEADING_CLASS}>Install the harness worker</h1>
      <p className={BODY_CLASS}>
        Chat runs on the <span className="text-ink">harness</span> worker, which
        isn't connected yet. Add it to start a chat.
      </p>
      <div className="flex flex-col gap-4">
        <div>
          <Button variant="primary" onClick={onInstallHarness}>
            Add harness
          </Button>
        </div>
        <div className="flex flex-col gap-2">
          <span className="font-inter text-sm text-ink-faint">
            Prefer the terminal? Run:
          </span>
          <CommandCard command={HARNESS_INSTALL_COMMAND} />
        </div>
      </div>
    </>
  )
}

/* ---------------- harness declared but stopped ---------------- */

/**
 * The harness is in the compose file but its container is down. Adding it
 * again would change nothing, so the choices are a start as declared, or an
 * update to the latest release followed by a start.
 */
function StoppedHarnessBody({
  onStartHarness,
  onUpdateHarness,
}: {
  onStartHarness?: () => void
  onUpdateHarness?: () => void
}) {
  return (
    <>
      <h1 className={HEADING_CLASS}>Start the harness worker</h1>
      <p className={BODY_CLASS}>
        Chat runs on the <span className="text-ink">harness</span> worker. It is
        in this project's compose file but isn't running. Start it to chat, or
        update it to the latest release first.
      </p>
      <div className="flex flex-col gap-4">
        <div className="flex flex-wrap gap-2">
          <Button variant="primary" onClick={onStartHarness}>
            Start harness
          </Button>
          <Button variant="secondary" onClick={onUpdateHarness}>
            Update and start harness
          </Button>
        </div>
        <div className="flex flex-col gap-2">
          <span className="font-inter text-sm text-ink-faint">
            Prefer the terminal? Run:
          </span>
          <CommandCard command={HARNESS_START_COMMAND} />
        </div>
      </div>
    </>
  )
}

/* ---------------- installing / install failed ---------------- */

function InstallingBody({
  action,
  failed,
  errorMessage,
  onRetryInstall,
}: {
  action: HarnessAction
  failed: boolean
  errorMessage?: string | null
  onRetryInstall?: () => void
}) {
  const copy = ACTION_COPY[action]
  return (
    <>
      <h1 className={HEADING_CLASS}>
        {failed ? copy.failed.heading : copy.running.heading}
      </h1>
      <p className={BODY_CLASS}>
        {failed ? copy.failed.body : copy.running.body}
      </p>
      <InstallConsole action={action} running={!failed} />
      {failed ? (
        <div className="flex flex-col gap-3">
          {errorMessage ? (
            <span className="font-mono text-[12.5px] text-alert break-all">
              {errorMessage}
            </span>
          ) : null}
          <div>
            <Button variant="primary" onClick={onRetryInstall}>
              {copy.failed.retry}
            </Button>
          </div>
        </div>
      ) : null}
    </>
  )
}

/**
 * Install console — a `CommandCard` framed as the equivalent CLI run of the
 * action (`iii call compose::add worker=harness` for an add).
 */
function InstallConsole({
  action,
  running,
}: {
  action: HarnessAction
  running: boolean
}) {
  const copy = ACTION_COPY[action]
  return (
    <CommandCard command={copy.command}>
      <div className="flex flex-col gap-0.5 text-[12.5px]">
        {running ? (
          <div className="text-ink-faint italic animate-pulse">
            {copy.running.line}
          </div>
        ) : (
          <div className="text-alert">{copy.failed.line}</div>
        )}
      </div>
    </CommandCard>
  )
}
