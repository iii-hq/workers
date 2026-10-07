import { ArrowRight, Check, Compass, LoaderCircle } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Wordmark } from '@/components/ui/Wordmark'
import type { JudgeOption } from '@/lib/onboarding/catalog'
import { browserLabel } from '@/lib/onboarding/chromium'
import { servesUsableModels } from '@/lib/onboarding/plan'
import { Section, StepLayout } from './parts'
import type { OnboardingController } from './use-onboarding'

/** The guided tour offered once a model is connected. */
export type TourState =
  | { kind: 'idle' }
  /** Getting the tour ready: adding its worker, waiting for its page. */
  | { kind: 'preparing' }
  | { kind: 'failed'; error: string }

/** Counts up to `target` once; lands on it at once under reduced motion. */
function useCountUp(target: number, durationMs = 900): number {
  const [value, setValue] = useState(0)
  useEffect(() => {
    const reduce =
      typeof window.matchMedia === 'function' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (reduce || target === 0) {
      setValue(target)
      return
    }
    let frame = 0
    const start = performance.now()
    const tick = (now: number) => {
      const t = Math.min(1, (now - start) / durationMs)
      // ease-out cubic: fast start, gentle landing
      setValue(Math.round(target * (1 - (1 - t) ** 3)))
      if (t < 1) frame = window.requestAnimationFrame(tick)
    }
    frame = window.requestAnimationFrame(tick)
    return () => window.cancelAnimationFrame(frame)
  }, [target, durationMs])
  return value
}

export function ReadyStep({
  onboarding,
  judge,
  tour,
  onStartTour,
  onStart,
}: {
  onboarding: OnboardingController
  judge: JudgeOption | null
  tour: TourState
  /** Accept the tour: the wizard gets it ready, then opens it. */
  onStartTour: () => void
  /** Close the wizard and hand the composer the focus. */
  onStart: () => void
}) {
  const { snapshot, activity } = onboarding
  const connected = (snapshot.providers ?? []).filter(servesUsableModels)
  const totalModels = connected.reduce(
    (sum, provider) => sum + provider.modelCount,
    0,
  )
  const counted = useCountUp(totalModels)
  const added = [
    ...new Set(
      activity
        .filter((entry) => entry.status === 'done')
        .flatMap((entry) => entry.workers ?? []),
    ),
  ]
  // Chromium this setup downloaded for the browser worker.
  const chromium = activity.some(
    (entry) => entry.group === 'browser' && entry.status === 'done',
  )
    ? { version: snapshot.browser?.version, path: snapshot.browser?.path }
    : null
  const secretRefs = connected.flatMap((provider) =>
    provider.credentialRef ? [provider.credentialRef] : [],
  )
  // `secret://` (encrypted) and `env://` (this project's .env), as used.
  const schemes = [
    ...new Set(secretRefs.map((ref) => `${ref.split('://')[0]}://`)),
  ]

  const lines: { title: string; detail?: string }[] = [
    ...connected.map((provider) => ({
      title: `${provider.title} connected`,
      detail: `${provider.modelCount} ${provider.modelCount === 1 ? 'model' : 'models'}${provider.credentialRef ? ` · key at ${provider.credentialRef}` : ''}`,
    })),
    ...(chromium
      ? [
          {
            title: `${browserLabel(chromium.version)} is ready for agents`,
            detail: chromium.path
              ? `they open and check the pages they build · ${chromium.path}`
              : 'they open and check the pages they build',
          },
        ]
      : []),
    ...(judge
      ? [
          {
            title: `Judge answers with ${judge.title}`,
            detail: 'function search, argument repair, browser decisions',
          },
        ]
      : []),
    ...(secretRefs.length > 0
      ? [
          {
            title: schemes.includes('env://')
              ? 'Your keys stay out of configuration'
              : 'Your keys stay out of git',
            detail: `configuration holds only ${schemes.join(' and ')} references`,
          },
        ]
      : []),
  ]

  // The tour's first stage is a message to the agent: it needs a model.
  const offerTour = connected.length > 0
  const preparing = tour.kind === 'preparing'

  return (
    <StepLayout
      footer={
        <>
          <span className="hidden font-sans text-[13px] text-ink @2xl:inline">
            Reopen this from the command palette: Set up the harness.
          </span>
          {offerTour ? (
            <span className="ml-auto flex items-center gap-2">
              <Button
                variant="ghost"
                onClick={() => onStart()}
                disabled={preparing}
              >
                Skip the tour
              </Button>
              <Button onClick={onStartTour} disabled={preparing}>
                {preparing ? (
                  <LoaderCircle className="iii-ui-spin" aria-hidden />
                ) : null}
                {preparing
                  ? 'Preparing the tour…'
                  : tour.kind === 'failed'
                    ? 'Try again'
                    : 'Start the tour'}
                {preparing ? null : <ArrowRight aria-hidden />}
              </Button>
            </span>
          ) : (
            <Button className="ml-auto" onClick={() => onStart()}>
              Start building
              <ArrowRight aria-hidden />
            </Button>
          )}
        </>
      }
    >
      <div className="flex flex-col items-center gap-4 pt-2 text-center">
        <Wordmark appearance="assemble" className="size-14" />
        <div className="flex flex-col items-center gap-1.5">
          <h2 className="onboarding-rise font-sans text-[24px] font-semibold tracking-[-0.01em] text-ink [animation-delay:520ms]">
            {connected.length > 0 ? 'Your harness is ready' : 'You’re all set'}
          </h2>
          <p className="onboarding-rise font-sans text-[14px] text-ink [animation-delay:600ms]">
            {connected.length > 0 ? (
              <>
                <span className="font-mono text-[16px] font-medium tabular-nums text-ink">
                  {counted}
                </span>{' '}
                {totalModels === 1 ? 'model' : 'models'} from {connected.length}{' '}
                {connected.length === 1 ? 'provider' : 'providers'}, ready in
                every chat.
              </>
            ) : (
              'Connect a model any time from the model picker in the composer.'
            )}
          </p>
        </div>
      </div>

      {lines.length > 0 ? (
        <ul className="flex flex-col gap-2 rounded-md bg-surface px-4 py-3">
          {lines.map((line, index) => (
            <li
              key={line.title}
              className="onboarding-rise flex items-start gap-3"
              style={{ animationDelay: `${700 + index * 110}ms` }}
            >
              <span
                className="onboarding-pop mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full bg-ok-muted text-ok"
                style={{ animationDelay: `${760 + index * 110}ms` }}
              >
                <Check className="size-4" aria-hidden />
              </span>
              <span className="flex min-w-0 flex-col">
                <span className="font-sans text-[14px] font-medium text-ink">
                  {line.title}
                </span>
                {line.detail ? (
                  <span className="font-sans text-[13px] text-ink">
                    {line.detail}
                  </span>
                ) : null}
              </span>
            </li>
          ))}
        </ul>
      ) : null}

      {added.length > 0 ? (
        <Section title={`Workers added during setup (${added.length})`}>
          <p className="font-sans text-[13px] leading-relaxed text-ink">
            Each one is declared in{' '}
            <span className="font-mono">worker-compose.yaml</span>, so the
            project starts the same way on the next{' '}
            <span className="font-mono">iii compose --up</span>.
          </p>
          <div className="flex flex-wrap gap-1.5">
            {added.map((worker) => (
              <span
                key={worker}
                className="rounded-sm bg-surface px-2 py-1 font-mono text-[12px] text-ink"
              >
                {worker}
              </span>
            ))}
          </div>
        </Section>
      ) : null}

      {offerTour ? (
        <section
          aria-label="Guided tour"
          className="onboarding-rise flex gap-3 rounded-md bg-card-highlight px-4 py-4 [animation-delay:900ms]"
        >
          <span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-surface text-ink">
            <Compass className="size-4" aria-hidden />
          </span>
          <span className="flex min-w-0 flex-col gap-1">
            <h3 className="font-sans text-[14px] font-semibold text-ink">
              Keep going with a guided tour
            </h3>
            <p className="text-pretty font-sans text-[13px] leading-relaxed text-ink">
              It runs right here in the ADE, with the models you just connected,
              one stage at a time: send your first message, then watch the agent
              add a worker, call your backend's functions, react to a trigger
              and trace what it did. Each stage points at the part of the ADE it
              talks about.
            </p>
            {tour.kind === 'failed' ? (
              <p
                role="alert"
                className="font-sans text-[13px] text-alert-strong"
              >
                The tour could not start: {tour.error}
              </p>
            ) : null}
          </span>
        </section>
      ) : null}
    </StepLayout>
  )
}
