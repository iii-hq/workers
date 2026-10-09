import { Check } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Eyebrow } from '@/components/ui/Eyebrow'
import { type AgentEntry, listAgents } from '@/lib/backend/directory-prompts'
import { requestComposerFocus } from '@/lib/composer-insert'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import {
  fetchOnboardingState,
  type OnboardingStatus,
  saveOnboardingState,
} from '@/lib/onboarding/api'
import type { JudgeOption } from '@/lib/onboarding/catalog'
import { chromiumMissing } from '@/lib/onboarding/chromium'
import {
  browserIsAutomated,
  onOnboardingWizardRequest,
  shouldAutoOpenOnboarding,
  type WizardStepId,
} from '@/lib/onboarding/open'
import { servesUsableModels } from '@/lib/onboarding/plan'
import {
  type ExamplePrompt,
  fetchExamplePrompts,
} from '@/lib/onboarding/prompts'
import { cn } from '@/lib/utils'
import { fetchNewChatWorkingDir } from '@/lib/working-dir'
import { BrowserStep } from './BrowserStep'
import { openExamplePrompt } from './example-prompt'
import { JudgeStep } from './JudgeStep'
import { ModelsStep } from './ModelsStep'
import type { StepPosition } from './parts'
import { ReadyStep } from './ReadyStep'
import { connectedModelCount, useOnboarding } from './use-onboarding'
import { WelcomeStep } from './WelcomeStep'

const STEPS: { id: WizardStepId; title: string; optional?: boolean }[] = [
  { id: 'welcome', title: 'Welcome' },
  { id: 'models', title: 'Models' },
  // Listed only while it has something to do (see `showBrowser`).
  { id: 'browser', title: 'Browser', optional: true },
  { id: 'judge', title: 'Judge', optional: true },
  { id: 'ready', title: 'Ready' },
]

/** "Step N of M" counts the steps that set something up. */
export function stepPosition(
  steps: readonly { id: WizardStepId }[],
  id: WizardStepId,
): StepPosition | undefined {
  const setup = steps.filter(
    (entry) => entry.id !== 'welcome' && entry.id !== 'ready',
  )
  const index = setup.findIndex((entry) => entry.id === id)
  return index < 0 ? undefined : { index: index + 1, total: setup.length }
}

/**
 * The first-run setup wizard. Mounted once in `App`: it opens by itself the
 * first time a person loads this machine's ADE (`console::onboarding::get`
 * reports `new` and does not turn auto-open off; never in a browser under
 * automation — see `shouldAutoOpenOnboarding`), and whenever something calls
 * `requestOnboardingWizard` — the chat's "configure a provider" call to
 * action, or the command palette.
 *
 * Finishing records `completed` and skipping records `dismissed`, beside the
 * workspace layout in the ADE's data directory. After either, it reopens on
 * its own only while no model is connected.
 *
 * The Browser step joins the list when the project runs the browser worker
 * and the machine has no Chromium for it (or someone asks for it — the
 * command palette's "Install Chromium", a chat error that says Chromium is
 * missing), and stays listed once shown.
 *
 * Ready ends setup with Finish, and — once a model is connected — offers
 * the example prompts the project's template declares (`onboarding.yaml`,
 * read through `console::onboarding::prompts`): a click finishes setup and
 * opens a new chat that sends the prompt, its agent profile and model
 * chosen (see `openExamplePrompt`).
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
  const [prompts, setPrompts] = useState<ExamplePrompt[] | null>(null)
  const [agents, setAgents] = useState<AgentEntry[] | null>(null)
  const status = useRef<OnboardingStatus | null>(null)
  const ctxRef = useRef(ctx)
  ctxRef.current = ctx
  const refreshModels = ctx?.refreshModels
  const onboarding = useOnboarding(open, () => {
    // The composer's picker follows router events, but a provider that
    // registers between two of them would otherwise wait for the next one.
    void refreshModels?.()
  })
  const busy = onboarding.running !== null
  const needsBrowser = chromiumMissing(onboarding.snapshot.browser)
  const [browserListed, setBrowserListed] = useState(false)
  useEffect(() => {
    if (needsBrowser) setBrowserListed(true)
  }, [needsBrowser])
  const showBrowser = browserListed || needsBrowser || step === 'browser'
  const steps = STEPS.filter((entry) => entry.id !== 'browser' || showBrowser)

  useEffect(() => {
    if (!live) return
    let cancelled = false
    void fetchOnboardingState()
      .then(async (state) => {
        if (cancelled || !state) return
        status.current = state.status
        // Someone who finished setup may jump straight to any part of it.
        if (state.status === 'completed') {
          setVisited(new Set(STEPS.map((entry) => entry.id)))
        }
        // First run opens whatever the router serves; after setup, only a
        // project with no model connected opens it again.
        const models =
          state.status === 'new'
            ? null
            : await connectedModelCount().catch(() => null)
        if (
          !cancelled &&
          shouldAutoOpenOnboarding(state, browserIsAutomated(), models)
        ) {
          setOpen(true)
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
        if (next === 'browser') setBrowserListed(true)
        setStep(next)
        setVisited((current) => new Set([...current, next]))
        setOpen(true)
      }),
    [],
  )

  const go = useCallback((next: WizardStepId) => {
    if (next === 'browser') setBrowserListed(true)
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
        .filter(servesUsableModels)
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
      const chromiumInstalled = onboarding.activity.some(
        (entry) => entry.group === 'browser' && entry.status === 'done',
      )
      record('completed', {
        providers,
        judge: judgeChoice?.id ?? null,
        workers_added: workers,
        chromium_installed: chromiumInstalled
          ? (onboarding.snapshot.browser?.version ?? true)
          : false,
      })
    },
    [
      go,
      onboarding.activity,
      onboarding.snapshot.browser,
      onboarding.snapshot.providers,
      record,
    ],
  )

  // Ready reads the project's example prompts, and the agent profiles they
  // name, each time it shows: the template's file may have changed since.
  useEffect(() => {
    if (!open || step !== 'ready') return
    if (!live) {
      setPrompts([])
      return
    }
    let cancelled = false
    setPrompts(null)
    void fetchExamplePrompts()
      .catch(() => [])
      .then((next) => {
        if (!cancelled) setPrompts(next)
      })
    void getIiiClient()
      .then(listAgents)
      .catch(() => null)
      .then((next) => {
        if (!cancelled && next) setAgents(next)
      })
    return () => {
      cancelled = true
    }
  }, [open, step, live])

  const agentNames = useMemo(
    () =>
      new Map(
        (agents ?? []).map((entry) => [
          entry.id,
          entry.name.trim() || entry.id,
        ]),
      ),
    [agents],
  )

  /** Close setup for good: it is complete, whichever way Ready was left. */
  const finish = useCallback(() => {
    setOpen(false)
    if (status.current !== 'completed') record('completed')
  }, [record])

  const start = useCallback(() => {
    finish()
    window.requestAnimationFrame(requestComposerFocus)
  }, [finish])

  const startPrompt = useCallback(
    async (prompt: ExamplePrompt) => {
      finish()
      const [profiles, workingDir] = await Promise.all([
        // Profiles still loading (a quick click): ask for them once more.
        agents ??
          getIiiClient()
            .then(listAgents)
            .catch(() => []),
        fetchNewChatWorkingDir().catch(() => null),
      ])
      const api = ctxRef.current
      if (!api) return
      openExamplePrompt(api, prompt, profiles, workingDir)
      window.requestAnimationFrame(requestComposerFocus)
    },
    [agents, finish],
  )

  const index = steps.findIndex((entry) => entry.id === step)
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
        className="@container flex h-[min(920px,calc(100dvh-24px))] max-h-none w-[min(960px,calc(100vw-24px))] max-w-none flex-row overflow-hidden p-0"
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
          <Eyebrow className="mb-3 px-2 text-[12px] text-ink">
            Set up the harness
          </Eyebrow>
          {steps.map((entry, position) => {
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
                  'flex h-9 items-center gap-2.5 rounded-sm px-2 text-left font-sans text-[14px] text-ink',
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
                  <span className="font-sans text-[12px] text-ink">
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
            <Eyebrow className="text-[12px] text-ink">
              Set up the harness
            </Eyebrow>
            <span
              role="progressbar"
              aria-label="Setup progress"
              aria-valuemin={1}
              aria-valuemax={steps.length}
              aria-valuenow={index + 1}
              className="h-1 flex-1 overflow-hidden rounded-full bg-surface"
            >
              <span
                className="block h-full rounded-full bg-ink transition-[width] duration-300"
                style={{ width: `${((index + 1) / steps.length) * 100}%` }}
              />
            </span>
          </div>
          {step === 'welcome' ? (
            <WelcomeStep
              onStart={() => go('models')}
              onSkip={() => {
                setOpen(false)
                record('dismissed')
              }}
            />
          ) : step === 'models' ? (
            <ModelsStep
              onboarding={onboarding}
              position={stepPosition(steps, 'models')}
              onBack={() => go('welcome')}
              onNext={() => go(showBrowser ? 'browser' : 'judge')}
            />
          ) : step === 'browser' ? (
            <BrowserStep
              onboarding={onboarding}
              position={stepPosition(steps, 'browser')}
              onBack={() => go('models')}
              onNext={() => go('judge')}
            />
          ) : step === 'judge' ? (
            <JudgeStep
              onboarding={onboarding}
              position={stepPosition(steps, 'judge')}
              onBack={() => go(showBrowser ? 'browser' : 'models')}
              onNext={finishTo}
            />
          ) : (
            <ReadyStep
              onboarding={onboarding}
              judge={judge}
              prompts={prompts}
              agentNames={agentNames}
              onPrompt={(prompt) => void startPrompt(prompt)}
              onFinish={start}
            />
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
    <p className="mt-auto px-2 font-sans text-[13px] leading-relaxed text-ink">
      {count === 0
        ? 'Nothing changed yet.'
        : `${count} ${count === 1 ? 'change' : 'changes'} made — each one is listed in its step.`}
    </p>
  )
}
