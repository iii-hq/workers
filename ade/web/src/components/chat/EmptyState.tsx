import { CopyCommandButton } from '@/components/chat/sandbox/terminal/CopyCommandButton'
import { Terminal } from '@/components/chat/sandbox/terminal/Terminal'
import { Button } from '@/components/ui/Button'
import { Wordmark } from '@/components/ui/Wordmark'
import { cn } from '@/lib/utils'
import type { WorktreePickerOptions } from './DirectoryPicker'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * The chat empty state, as a small set of presentational variants:
 *
 *   - `ready`          — harness present + a model available (the hero copy)
 *   - `no-provider`    — harness present, no model configured -> configure CTA
 *   - `no-harness`     — harness worker missing -> install CTA + cli hint
 *   - `installing`     — `compose::add` in flight -> live console
 *   - `install-failed` — add failed -> console + retry
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
  | 'installing'
  | 'install-failed'

export interface EmptyStateProps {
  variant: EmptyStateVariant
  density?: 'route' | 'dock'
  /** Surfaced under the console when an add fails. */
  errorMessage?: string | null
  /** `no-harness` primary CTA. */
  onInstallHarness?: () => void
  /** `install-failed` retry CTA. */
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

const HARNESS_INSTALL_COMMAND = 'iii trigger compose::add worker=harness'

const HEADING_CLASS =
  'text-balance font-sans text-3xl font-semibold tracking-tight text-ink'
const BODY_CLASS = 'text-pretty font-sans text-base/7 text-ink-faint'

export function EmptyState({
  variant,
  density = 'route',
  errorMessage,
  onInstallHarness,
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
        {variant === 'installing' || variant === 'install-failed' ? (
          <InstallingBody
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
        <h1 className="text-balance font-inter text-2xl font-medium tracking-tight text-ink @2xl:text-[28px]">
          What would you like to build?
        </h1>
        <p className="max-w-md text-pretty font-inter text-sm leading-relaxed text-ink-faint">
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
      <h1 className={HEADING_CLASS}>Connect a model.</h1>
      <p className={BODY_CLASS}>
        the harness is running, but no model is connected yet. setup finds
        claude code, codex or an api key already on this machine and connects it
        in a couple of clicks.
      </p>
      <div>
        <Button variant="primary" onClick={onConfigureProvider}>
          set up a model
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
      <h1 className={HEADING_CLASS}>Install the harness worker.</h1>
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
          <span className="font-mono text-[12px] text-ink-ghost">
            Prefer the terminal? Run:
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
  errorMessage,
  onRetryInstall,
}: {
  failed: boolean
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
          ? "compose couldn't finish adding harness. you can retry here, or run it from your terminal."
          : 'adding the harness worker. this mirrors what the cli does — you could run the same command in your terminal.'}
      </p>
      <InstallConsole running={!failed} />
      {failed ? (
        <div className="flex flex-col gap-3">
          {errorMessage ? (
            <span className="font-mono text-[12.5px] text-alert break-all">
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
 * Install console — `Terminal` chrome framed as the equivalent
 * `iii trigger compose::add worker=harness` run.
 */
function InstallConsole({ running }: { running: boolean }) {
  return (
    <Terminal command={HARNESS_INSTALL_COMMAND}>
      <div className="bg-bg px-3 py-3 font-mono text-[12.5px] leading-[1.6] flex flex-col gap-0.5">
        {running ? (
          <div className="text-ink-faint italic animate-pulse">
            · adding harness to the compose project…
          </div>
        ) : (
          <div className="text-alert">× compose::add did not finish</div>
        )}
      </div>
    </Terminal>
  )
}
