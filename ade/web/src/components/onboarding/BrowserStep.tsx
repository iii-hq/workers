import { AppWindow, Check, CircleAlert, LoaderCircle } from 'lucide-react'
import { useState } from 'react'
import { CopyMessageButton } from '@/components/chat/CopyMessageButton'
import { Button } from '@/components/ui/Button'
import { Skeleton } from '@/components/ui/Skeleton'
import {
  BROWSER_WORKER,
  type ChromiumInstallProgress,
  type ChromiumState,
  describeProgress,
  manualInstall,
  megabytes,
  progressFraction,
} from '@/lib/onboarding/chromium'
import { cn } from '@/lib/utils'
import {
  ActivityLog,
  StepHeader,
  StepLayout,
  type StepPosition,
  stepEyebrow,
} from './parts'
import type { OnboardingController } from './use-onboarding'

const LEAD =
  'Agents open the pages they build in Chromium to make sure they work.'

/**
 * Setup's browser step: shown when the project runs the browser worker and
 * the machine has no Chromium for it. One button downloads a private copy
 * (followed live through the worker's progress events); people who would
 * rather install Chrome themselves get the one command for their system and
 * a way to check again. It says what a person exploring iii needs — the
 * size, that it happens once — and leaves out where it lands on disk.
 */
export function BrowserStep({
  onboarding,
  position,
  onBack,
  onNext,
}: {
  onboarding: OnboardingController
  position?: StepPosition
  onBack: () => void
  onNext: () => void
}) {
  const {
    snapshot,
    scanning,
    activity,
    running,
    installChromium,
    checkChromium,
    chromiumProgress,
  } = onboarding
  const state = snapshot.browser
  const log = activity.filter((entry) => entry.group === 'browser')
  const installedHere = log.some((entry) => entry.status === 'done')
  const installing = running === 'browser'
  const busy = running !== null
  const [failure, setFailure] = useState<{
    error: string
    hint?: string
  } | null>(null)
  const [checking, setChecking] = useState(false)
  const [stillMissing, setStillMissing] = useState(false)

  // `tools` stays null until the first look at the machine finished.
  const scanned = snapshot.tools !== null && !scanning
  const hasWorker = snapshot.installed.has(BROWSER_WORKER)
  const workerAbsent = scanned && state === null && !hasWorker
  const readFailed =
    scanned && state === null && hasWorker && snapshot.browserError !== null
  const loading = state === null && !workerAbsent && !readFailed
  const found = state?.found === true || state?.engine === 'lightpanda'
  const canInstall = state?.canInstall === true

  const download = async () => {
    setFailure(null)
    setStillMissing(false)
    const outcome = await installChromium()
    if (!outcome.ok && outcome.error !== 'cancelled') {
      setFailure({ error: outcome.error, hint: outcome.hint })
    }
  }

  const checkAgain = async () => {
    setChecking(true)
    const next = await checkChromium()
    setChecking(false)
    setStillMissing(next !== null && !next.found)
  }

  const primary =
    found || workerAbsent ? (
      <Button onClick={onNext} disabled={busy}>
        Continue
      </Button>
    ) : canInstall ? (
      <Button onClick={() => void download()} disabled={busy || loading}>
        {installing ? (
          <LoaderCircle className="iii-ui-spin" aria-hidden />
        ) : null}
        {installing
          ? 'Downloading…'
          : failure
            ? 'Retry download'
            : 'Download Chromium'}
      </Button>
    ) : (
      <Button
        onClick={() => void checkAgain()}
        disabled={busy || loading || checking}
      >
        {checking ? 'Checking…' : 'Check again'}
      </Button>
    )

  return (
    <StepLayout
      footer={
        <>
          <Button variant="ghost" onClick={onBack} disabled={busy}>
            Back
          </Button>
          <span className="flex items-center gap-2">
            {found || workerAbsent ? null : (
              <Button variant="ghost" onClick={onNext} disabled={busy}>
                Skip
              </Button>
            )}
            {primary}
          </span>
        </>
      }
    >
      <StepHeader
        eyebrow={stepEyebrow(position, true)}
        title="Let agents check their work in a browser"
        lead={
          state !== null && !found
            ? `${LEAD} This machine doesn't have it yet.`
            : LEAD
        }
      />

      {loading ? (
        <div
          role="status"
          aria-label="Looking for Chromium"
          className="flex flex-col gap-2 rounded-md bg-surface px-4 py-4"
        >
          <Skeleton className="h-4 w-1/2" />
          <Skeleton className="h-3 w-5/6" />
        </div>
      ) : readFailed ? (
        <p
          role="alert"
          className="rounded-md bg-surface px-4 py-3 font-sans text-[14px] text-ink"
        >
          The browser worker did not say whether it has Chromium:{' '}
          {snapshot.browserError}
        </p>
      ) : workerAbsent ? (
        <p className="rounded-md bg-surface px-4 py-3 font-sans text-[14px] text-ink">
          This project doesn't run the browser worker, so there is nothing to
          set up here.
        </p>
      ) : found && state ? (
        <ReadyPanel state={state} installedHere={installedHere} />
      ) : installing ? (
        <ProgressPanel progress={chromiumProgress} state={state} />
      ) : failure ? (
        <FailurePanel failure={failure} />
      ) : canInstall && state ? (
        <DownloadPanel state={state} />
      ) : state ? (
        <p className="rounded-md bg-surface px-4 py-3 text-pretty font-sans text-[14px] leading-relaxed text-ink">
          {state.installReason ??
            'Setup cannot download Chromium on this system.'}{' '}
          Install it with the command below, then check again.
        </p>
      ) : null}

      {state && !found && !installing ? (
        <ManualInstall
          state={state}
          // Without a download, the manual path is the step: open, and the
          // footer's primary action is the check.
          open={!canInstall}
          showCheck={canInstall}
          checking={checking}
          busy={busy}
          stillMissing={stillMissing}
          onCheck={() => void checkAgain()}
        />
      ) : null}

      <ActivityLog
        entries={log}
        title={installing ? 'Downloading Chromium' : 'What setup did'}
      />
    </StepLayout>
  )
}

function DownloadPanel({ state }: { state: ChromiumState }) {
  return (
    <section
      aria-label="Download Chromium"
      className="flex gap-3 rounded-md bg-card-highlight px-4 py-4"
    >
      <span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-surface text-ink">
        <AppWindow className="size-4" aria-hidden />
      </span>
      <span className="flex min-w-0 flex-col gap-1">
        <h3 className="font-sans text-[14px] font-semibold text-ink">
          Download Chromium, just for agents
        </h3>
        <p className="text-pretty font-sans text-[13px] leading-relaxed text-ink">
          About {state.approxDownloadMb} MB, downloaded once for this machine.
          It leaves any Chrome you install yourself untouched.
        </p>
      </span>
    </section>
  )
}

function ProgressPanel({
  progress,
  state,
}: {
  progress: ChromiumInstallProgress | null
  state: ChromiumState | null
}) {
  const fraction = progressFraction(progress)
  const percent =
    fraction === undefined ? undefined : Math.round(fraction * 100)
  const line = progress ? describeProgress(progress) : 'Starting the download…'
  return (
    <section
      aria-label="Chromium download"
      className="flex flex-col gap-3 rounded-md bg-surface px-4 py-4"
    >
      <div className="flex items-baseline justify-between gap-3">
        <span
          role="status"
          className="min-w-0 font-sans text-[14px] font-medium text-ink"
        >
          {line}
        </span>
        {percent !== undefined ? (
          <span className="shrink-0 font-mono text-[12px] tabular-nums text-ink">
            {percent}%
          </span>
        ) : null}
      </div>
      <span
        role="progressbar"
        aria-label="Download progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-valuetext={line}
        className="block h-2 overflow-hidden rounded-full bg-surface-active"
      >
        <span
          className={cn(
            'block h-full rounded-full bg-ink transition-[width] duration-300',
            percent === undefined && 'animate-pulse',
          )}
          style={{ width: `${percent ?? 100}%` }}
        />
      </span>
      <p className="font-sans text-[13px] leading-relaxed text-ink">
        {progress?.bytes_total
          ? `${megabytes(progress.bytes_total)} in all`
          : `About ${state?.approxDownloadMb ?? 200} MB in all`}
        . This usually takes a minute or two.
      </p>
    </section>
  )
}

function FailurePanel({
  failure,
}: {
  failure: { error: string; hint?: string }
}) {
  return (
    <section
      role="alert"
      aria-label="Download failed"
      className="flex gap-3 rounded-md bg-surface px-4 py-4"
    >
      <CircleAlert className="mt-0.5 size-4 shrink-0 text-alert" aria-hidden />
      <span className="flex min-w-0 flex-col gap-1">
        <span className="font-sans text-[14px] font-medium text-alert-strong">
          Chromium could not be downloaded
        </span>
        <span className="break-words font-sans text-[13px] leading-relaxed text-ink">
          {failure.error}
        </span>
        {failure.hint ? (
          <span className="break-words font-sans text-[13px] leading-relaxed text-ink">
            {failure.hint}
          </span>
        ) : null}
        <span className="font-sans text-[13px] text-ink">
          Retry the download, or install it yourself below.
        </span>
      </span>
    </section>
  )
}

function ReadyPanel({
  state,
  installedHere,
}: {
  state: ChromiumState
  installedHere: boolean
}) {
  const name = 'Chromium'
  if (state.engine === 'lightpanda' && !state.found) {
    return (
      <p className="rounded-md bg-ok-muted px-4 py-3 font-sans text-[14px] text-ink">
        The browser worker runs Lightpanda, which needs no Chromium.
      </p>
    )
  }
  return (
    <section
      aria-label="Chromium ready"
      className="flex gap-3 rounded-md bg-ok-muted px-4 py-4"
    >
      <span className="mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full bg-ok-muted text-ok">
        <Check className="size-4" aria-hidden />
      </span>
      <span className="flex min-w-0 flex-col gap-1">
        <span className="font-sans text-[14px] font-medium text-ink">
          {installedHere ? `${name} is ready` : `${name} is already here`}
        </span>
        <span className="font-sans text-[13px] leading-relaxed text-ink">
          Agents can open the pages they build and check that they work.
        </span>
      </span>
    </section>
  )
}

function ManualInstall({
  state,
  open,
  showCheck,
  checking,
  busy,
  stillMissing,
  onCheck,
}: {
  state: ChromiumState
  open: boolean
  showCheck: boolean
  checking: boolean
  busy: boolean
  stillMissing: boolean
  onCheck: () => void
}) {
  const manual = manualInstall(state.platform)
  return (
    <details
      className="group rounded-md bg-surface px-4 py-3"
      open={open || undefined}
    >
      <summary className="cursor-pointer font-sans text-[14px] font-medium text-ink">
        I'd rather install it myself
      </summary>
      <div className="mt-3 flex flex-col gap-2">
        <p className="font-sans text-[13px] text-ink">
          On {manual.system}, run:
        </p>
        <div className="flex items-center gap-2 rounded-sm bg-bg px-3 py-2">
          <code className="min-w-0 flex-1 break-all font-mono text-[13px] text-ink">
            {manual.command}
          </code>
          <CopyMessageButton text={manual.command} label="copy command" />
        </div>
        <p className="font-sans text-[13px] leading-relaxed text-ink">
          {manual.alternative} The browser worker finds it by itself.
        </p>
        {stillMissing ? (
          <p role="status" className="font-sans text-[13px] text-alert-strong">
            Still no Chromium. Once the install finishes, check again.
          </p>
        ) : null}
        {showCheck ? (
          <span>
            <Button
              variant="pill"
              size="sm"
              onClick={onCheck}
              disabled={busy || checking}
            >
              {checking ? 'Checking…' : 'Check again'}
            </Button>
          </span>
        ) : null}
      </div>
    </details>
  )
}
