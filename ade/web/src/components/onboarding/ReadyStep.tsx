import { ArrowRight, Check } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Wordmark } from '@/components/ui/Wordmark'
import { type JudgeOption, PILLARS } from '@/lib/onboarding/catalog'
import { Section, StepLayout } from './parts'
import type { OnboardingController } from './use-onboarding'
import { PILLAR_ICONS } from './WelcomeStep'

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
  onStart,
}: {
  onboarding: OnboardingController
  judge: JudgeOption | null
  /** Close the wizard; with a prompt, hand it to the composer first. */
  onStart: (prompt?: string) => void
}) {
  const { snapshot, activity } = onboarding
  const connected = (snapshot.providers ?? []).filter(
    (provider) => provider.modelCount > 0,
  )
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
  const secretRefs = connected.flatMap((provider) =>
    provider.credentialRef ? [provider.credentialRef] : [],
  )

  const lines: { title: string; detail?: string }[] = [
    ...connected.map((provider) => ({
      title: `${provider.title} connected`,
      detail: `${provider.modelCount} ${provider.modelCount === 1 ? 'model' : 'models'}${provider.credentialRef ? ` · key at ${provider.credentialRef}` : ''}`,
    })),
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
            title: 'Your keys stay out of git',
            detail: 'configuration holds only secret:// references',
          },
        ]
      : []),
  ]

  return (
    <StepLayout
      footer={
        <>
          <span className="font-sans text-[12px] text-ink-faint">
            Reopen this from the command palette: Set up the harness.
          </span>
          <Button onClick={() => onStart()}>
            Start building
            <ArrowRight aria-hidden />
          </Button>
        </>
      }
    >
      <div className="flex flex-col items-center gap-4 pt-2 text-center">
        <Wordmark appearance="assemble" className="size-14" />
        <div className="flex flex-col items-center gap-1.5">
          <h2 className="onboarding-rise font-sans text-[22px] font-semibold tracking-[-0.01em] text-ink [animation-delay:520ms]">
            {connected.length > 0 ? 'Your harness is ready' : 'You’re all set'}
          </h2>
          <p className="onboarding-rise font-sans text-[13px] text-ink-faint [animation-delay:600ms]">
            {connected.length > 0 ? (
              <>
                <span className="font-mono text-[15px] font-medium tabular-nums text-ink">
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
                <span className="font-sans text-[13px] font-medium text-ink">
                  {line.title}
                </span>
                {line.detail ? (
                  <span className="font-sans text-[12px] text-ink-faint">
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
          <p className="font-sans text-[12px] leading-relaxed text-ink-faint">
            Each one is declared in{' '}
            <span className="font-mono">worker-compose.yaml</span>, so the
            project starts the same way on the next{' '}
            <span className="font-mono">iii compose --up</span>.
          </p>
          <div className="flex flex-wrap gap-1.5">
            {added.map((worker) => (
              <span
                key={worker}
                className="rounded-sm bg-surface px-2 py-1 font-mono text-[11px] text-ink"
              >
                {worker}
              </span>
            ))}
          </div>
        </Section>
      ) : null}

      <Section title="Try one of these first">
        <div className="grid gap-2 @2xl:grid-cols-2">
          {PILLARS.map((pillar, index) => {
            const Icon = PILLAR_ICONS[pillar.id]
            return (
              <button
                key={pillar.id}
                type="button"
                onClick={() => onStart(pillar.starter.prompt)}
                className="onboarding-rise group flex items-start gap-3 rounded-md bg-surface px-3 py-3 text-left hover:bg-surface-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
                style={{ animationDelay: `${900 + index * 60}ms` }}
              >
                <Icon
                  className="mt-0.5 size-4 shrink-0 text-ink-faint group-hover:text-ink"
                  aria-hidden
                />
                <span className="flex min-w-0 flex-col gap-0.5">
                  <span className="font-mono text-[11px] text-ink-ghost">
                    {pillar.title}
                  </span>
                  <span className="font-sans text-[13px] font-medium text-ink">
                    {pillar.starter.label}
                  </span>
                </span>
              </button>
            )
          })}
        </div>
        <p className="font-sans text-[12px] leading-relaxed text-ink-faint">
          Picking one puts its prompt in the chat for you to read and send.
        </p>
      </Section>
    </StepLayout>
  )
}
