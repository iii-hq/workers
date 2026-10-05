import { Check } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Eyebrow } from '@/components/ui/Eyebrow'
import { insertIntoComposer, requestComposerFocus } from '@/lib/composer-insert'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import {
  fetchOnboardingState,
  type OnboardingStatus,
  saveOnboardingState,
} from '@/lib/onboarding/api'
import type { JudgeOption } from '@/lib/onboarding/catalog'
import {
  onOnboardingWizardRequest,
  type WizardStepId,
} from '@/lib/onboarding/open'
import { cn } from '@/lib/utils'
import { JudgeStep } from './JudgeStep'
import { MachineStep } from './MachineStep'
import { ModelsStep } from './ModelsStep'
import { ReadyStep } from './ReadyStep'
import { useOnboarding } from './use-onboarding'
import { WelcomeStep } from './WelcomeStep'

const STEPS: { id: WizardStepId; title: string; optional?: boolean }[] = [
  { id: 'welcome', title: 'Welcome' },
  { id: 'machine', title: 'Your machine' },
  { id: 'models', title: 'Models' },
  { id: 'judge', title: 'Judge', optional: true },
  { id: 'ready', title: 'Ready' },
]

/**
 * The first-run setup wizard. Mounted once in `App`: it opens by itself the
 * first time this machine's ADE loads (`console::onboarding::get` reports
 * `new`), and again whenever something calls `requestOnboardingWizard` — the
 * chat's "configure a provider" call to action, or the command palette.
 *
 * Finishing records `completed` and skipping records `dismissed`, beside the
 * workspace layout in the ADE's data directory, so it never reopens on its
 * own after either.
 */
export function OnboardingWizardHost() {
  const ctx = useConversationsCtxOptional()
  const live = ctx?.backend.id === 'real'
  const [open, setOpen] = useState(false)
  const [step, setStep] = useState<WizardStepId>('welcome')
  const [visited, setVisited] = useState<ReadonlySet<WizardStepId>>(
    () => new Set(['welcome']),
  )
  const [judge, setJudge] = useState<JudgeOption | null>(null)
  const status = useRef<OnboardingStatus | null>(null)
  const refreshModels = ctx?.refreshModels
  const onboarding = useOnboarding(open, () => {
    // The composer's picker follows router events, but a provider that
    // registers between two of them would otherwise wait for the next one.
    void refreshModels?.()
  })
  const busy = onboarding.running !== null

  useEffect(() => {
    if (!live) return
    let cancelled = false
    void fetchOnboardingState()
      .then((state) => {
        if (cancelled || !state) return
        status.current = state.status
        if (state.status === 'new') setOpen(true)
        // Someone who finished setup may jump straight to any part of it.
        if (state.status === 'completed') {
          setVisited(new Set(STEPS.map((entry) => entry.id)))
        }
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [live])

  useEffect(
    () =>
      onOnboardingWizardRequest((target) => {
        const next = target ?? 'welcome'
        setStep(next)
        setVisited((current) => new Set([...current, next]))
        setOpen(true)
      }),
    [],
  )

  const go = useCallback((next: WizardStepId) => {
    setStep(next)
    setVisited((current) => new Set([...current, next]))
  }, [])

  const record = useCallback((next: OnboardingStatus, summary?: unknown) => {
    // Never demote a finished setup to dismissed by closing it again.
    if (next === 'dismissed' && status.current === 'completed') return
    status.current = next
    void saveOnboardingState(next, summary).catch(() => undefined)
  }, [])

  const close = useCallback(() => {
    if (busy) return
    setOpen(false)
    if (step === 'ready') return
    record('dismissed')
  }, [busy, record, step])

  const finishTo = useCallback(
    (judgeChoice: JudgeOption | null) => {
      setJudge(judgeChoice)
      go('ready')
      const providers = (onboarding.snapshot.providers ?? [])
        .filter((provider) => provider.modelCount > 0)
        .map((provider) => ({
          id: provider.id,
          models: provider.modelCount,
          credential_ref: provider.credentialRef,
        }))
      const workers = [
        ...new Set(
          onboarding.activity
            .filter((entry) => entry.status === 'done')
            .flatMap((entry) => entry.workers ?? []),
        ),
      ]
      record('completed', {
        providers,
        judge: judgeChoice?.id ?? null,
        workers_added: workers,
      })
    },
    [go, onboarding.activity, onboarding.snapshot.providers, record],
  )

  const start = useCallback((prompt?: string) => {
    setOpen(false)
    if (prompt) insertIntoComposer(prompt)
    window.requestAnimationFrame(requestComposerFocus)
  }, [])

  const index = STEPS.findIndex((entry) => entry.id === step)
  const content = useRef<HTMLDivElement>(null)
  // A new step moves the caret to its primary action, so the keyboard path
  // through setup is Enter, Enter, Enter.
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-runs on purpose when the step changes; the DOM it reads is the new step's
  useEffect(() => {
    if (!open) return
    const frame = window.requestAnimationFrame(() =>
      focusPrimary(content.current),
    )
    return () => window.cancelAnimationFrame(frame)
  }, [open, step])

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close()
      }}
    >
      <DialogContent
        className="@container flex h-[min(760px,calc(100dvh-24px))] max-h-none w-[min(960px,calc(100vw-24px))] max-w-none flex-row overflow-hidden p-0"
        onOpenAutoFocus={(event) => {
          // Land on the step's primary action, not the first button in it.
          event.preventDefault()
          focusPrimary(event.currentTarget)
        }}
        onEscapeKeyDown={(event) => {
          if (busy) event.preventDefault()
        }}
        onPointerDownOutside={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
      >
        <DialogTitle className="sr-only">Set up the harness</DialogTitle>
        <DialogDescription className="sr-only">
          Connect a model provider and see what the harness can do.
        </DialogDescription>
        <nav
          aria-label="Setup steps"
          className="hidden w-[208px] shrink-0 flex-col gap-1 bg-sidebar px-3 py-6 @2xl:flex"
        >
          <Eyebrow className="mb-3 px-2">Set up the harness</Eyebrow>
          {STEPS.map((entry, position) => {
            const current = entry.id === step
            const done = position < index || (entry.id === 'ready' && current)
            const reachable = !busy && visited.has(entry.id) && step !== 'ready'
            return (
              <button
                key={entry.id}
                type="button"
                disabled={!reachable || current}
                aria-current={current ? 'step' : undefined}
                onClick={() => go(entry.id)}
                className={cn(
                  'flex h-9 items-center gap-2.5 rounded-sm px-2 text-left font-sans text-[13px] text-ink-faint',
                  current && 'bg-surface-selected text-ink',
                  reachable &&
                    !current &&
                    'hover:bg-surface-hover hover:text-ink',
                  'disabled:cursor-default',
                )}
              >
                <StepMark done={done && !current} current={current} />
                <span className="min-w-0 flex-1 truncate">{entry.title}</span>
                {entry.optional ? (
                  <span className="font-sans text-[11px] text-ink-ghost">
                    optional
                  </span>
                ) : null}
              </button>
            )
          })}
          <ChangesCounter count={onboarding.activity.length} />
        </nav>
        <div ref={content} className="flex min-w-0 flex-1 flex-col">
          <div className="flex items-center gap-3 px-5 pt-4 pr-14 @2xl:hidden">
            <Eyebrow>Set up the harness</Eyebrow>
            <span
              role="progressbar"
              aria-label="Setup progress"
              aria-valuemin={1}
              aria-valuemax={STEPS.length}
              aria-valuenow={index + 1}
              className="h-1 flex-1 overflow-hidden rounded-full bg-surface"
            >
              <span
                className="block h-full rounded-full bg-ink transition-[width] duration-300"
                style={{ width: `${((index + 1) / STEPS.length) * 100}%` }}
              />
            </span>
          </div>
          {step === 'welcome' ? (
            <WelcomeStep
              onStart={() => go('machine')}
              onSkip={() => {
                setOpen(false)
                record('dismissed')
              }}
            />
          ) : step === 'machine' ? (
            <MachineStep
              onboarding={onboarding}
              onBack={() => go('welcome')}
              onNext={() => go('models')}
            />
          ) : step === 'models' ? (
            <ModelsStep
              onboarding={onboarding}
              onBack={() => go('machine')}
              onNext={() => go('judge')}
            />
          ) : step === 'judge' ? (
            <JudgeStep
              onboarding={onboarding}
              onBack={() => go('models')}
              onNext={finishTo}
            />
          ) : (
            <ReadyStep onboarding={onboarding} judge={judge} onStart={start} />
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}

/** Focus the step footer's primary action (the last enabled footer button). */
function focusPrimary(root: EventTarget | HTMLElement | null) {
  if (!(root instanceof HTMLElement)) return
  const buttons = root.querySelectorAll<HTMLButtonElement>(
    'footer button:not([disabled])',
  )
  buttons[buttons.length - 1]?.focus()
}

function StepMark({ done, current }: { done: boolean; current: boolean }) {
  if (done) {
    return (
      <span className="flex size-4 shrink-0 items-center justify-center rounded-full bg-ok-muted text-ok">
        <Check className="size-4 p-0.5" aria-hidden />
      </span>
    )
  }
  return (
    <span
      aria-hidden
      className={cn(
        'flex size-4 shrink-0 items-center justify-center rounded-full',
        current ? 'bg-ink' : 'bg-surface-active',
      )}
    >
      {current ? <span className="size-1.5 rounded-full bg-bg" /> : null}
    </span>
  )
}

/** The rail's running tally of what setup changed, so none of it is hidden. */
function ChangesCounter({ count }: { count: number }) {
  return (
    <p className="mt-auto px-2 font-sans text-[12px] leading-relaxed text-ink-ghost">
      {count === 0
        ? 'Nothing changed yet.'
        : `${count} ${count === 1 ? 'change' : 'changes'} made — each one is listed in its step.`}
    </p>
  )
}
