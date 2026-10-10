import { X } from 'lucide-react'
import {
  domAnimation,
  LazyMotion,
  MotionConfig,
  m,
  useReducedMotion,
} from 'motion/react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Wordmark } from '@/components/ui/Wordmark'
import { type AgentEntry, listAgents } from '@/lib/backend/directory-prompts'
import { requestComposerFocus } from '@/lib/composer-insert'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import {
  fetchOnboardingState,
  type OnboardingStatus,
  readableError,
  saveOnboardingState,
} from '@/lib/onboarding/api'
import { type JudgeOption, TOUR_PAGE } from '@/lib/onboarding/catalog'
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
import { prepareTour } from '@/lib/onboarding/tour'
import { requestPanelOpen } from '@/lib/panel-context'
import { cn } from '@/lib/utils'
import { BrowserStep } from './BrowserStep'
import { openExamplePrompt } from './example-prompt'
import { JudgeStep } from './JudgeStep'
import { ModelsStep } from './ModelsStep'
import type { StepPosition } from './parts'
import { ReadyStep, type TourState } from './ReadyStep'
import { Stepper, type StepperStep } from './Stepper'
import { connectedModelCount, useOnboarding } from './use-onboarding'
import { WelcomeStep } from './WelcomeStep'

const STEPS: readonly StepperStep[] = [
  { id: 'welcome', title: 'Welcome', description: 'What the ADE does' },
  { id: 'models', title: 'Models', description: 'Connect a provider' },
  // Listed only while it has something to do (see `showBrowser`).
  {
    id: 'browser',
    title: 'Browser',
    description: 'Chromium for agents',
    optional: true,
  },
  {
    id: 'judge',
    title: 'Judge',
    description: 'Small decision models',
    optional: true,
  },
  { id: 'ready', title: 'Ready', description: 'Start building' },
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
 * automation; never once a model is connected — see
 * `shouldAutoOpenOnboarding`), and whenever something calls
 * `requestOnboardingWizard` — the chat's "configure a provider" call to
 * action, or the command palette.
 *
 * Finishing records `completed` and skipping records `dismissed`, beside the
 * workspace layout in the ADE's data directory, so it never reopens on its
 * own after either.
 *
 * Once a model is connected, Ready offers the guided tour. Accepting adds
 * the `onboarding` worker that carries it — quietly: it is how the tour is
 * delivered, not a choice in setup — and opens its page beside the chat.
 */
export function OnboardingWizardHost() {
  const reduceMotion = useReducedMotion()
  const ctx = useConversationsCtxOptional()
  const live = ctx?.backend.id === 'real'
  const [open, setOpen] = useState(false)
  const [step, setStep] = useState<WizardStepId>('welcome')
  const [visited, setVisited] = useState<ReadonlySet<WizardStepId>>(
    () => new Set(['welcome']),
  )
  const [judges, setJudges] = useState<JudgeOption[]>([])
  const [prompts, setPrompts] = useState<ExamplePrompt[] | null>(null)
  const [agents, setAgents] = useState<AgentEntry[] | null>(null)
  const ctxRef = useRef(ctx)
  ctxRef.current = ctx
  const [tour, setTour] = useState<TourState>({ kind: 'idle' })
  const status = useRef<OnboardingStatus | null>(null)
  const refreshModels = ctx?.refreshModels
  const onboarding = useOnboarding(open, () => {
    // The composer's picker follows router events, but a provider that
    // registers between two of them would otherwise wait for the next one.
    void refreshModels?.()
  })
  const busy = onboarding.running !== null || tour.kind === 'preparing'
  // The Browser step joins the list when the project runs the browser worker
  // and the machine has no Chromium for it (or someone asks for it), and
  // stays listed once shown.
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
    (judgeChoices: JudgeOption[]) => {
      setJudges(judgeChoices)
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
        judge: judgeChoices[0]?.id ?? null,
        judges: judgeChoices.map((option) => option.id),
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

  const start = useCallback(() => {
    setOpen(false)
    window.requestAnimationFrame(requestComposerFocus)
  }, [])

  // Ready reads the project's example prompts, and the agent profiles they
  // name, each time it shows: the project's file may have changed since.
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

  /** Close setup and open a new chat with the prompt waiting in the composer. */
  const startPrompt = useCallback(
    async (prompt: ExamplePrompt) => {
      setOpen(false)
      // Profiles still loading (a quick click): ask for them once more.
      const profiles =
        agents ??
        (await getIiiClient()
          .then(listAgents)
          .catch(() => []))
      const api = ctxRef.current
      if (!api) return
      openExamplePrompt(
        {
          createNew: api.createNew,
          setAgentProfile: api.setAgentProfile,
          setModel: api.setModel,
          setThinkingLevel: api.setThinkingLevel,
          openConversation: api.select,
          modelOptions: api.modelOptions,
        },
        prompt,
        profiles,
      )
      window.requestAnimationFrame(requestComposerFocus)
    },
    [agents],
  )

  const startTour = useCallback(async () => {
    setTour({ kind: 'preparing' })
    try {
      await prepareTour()
    } catch (error) {
      setTour({ kind: 'failed', error: readableError(error) })
      return
    }
    setTour({ kind: 'idle' })
    setOpen(false)
    requestPanelOpen({ pageId: TOUR_PAGE })
  }, [])

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

  // Finished steps stay a click away until Ready: setup is recorded then.
  const reachable = new Set<WizardStepId>(
    busy || step === 'ready' ? [] : visited,
  )

  return (
    <LazyMotion features={domAnimation} strict>
      <MotionConfig reducedMotion="user">
        <Dialog
          open={open}
          onOpenChange={(next) => {
            if (!next) close()
          }}
        >
          <DialogContent
            className="@container flex h-[min(620px,calc(100dvh-32px))] max-h-none w-[min(760px,calc(100vw-32px))] max-w-none flex-row overflow-hidden rounded-xl border border-neutral-200 bg-white p-0 font-sans text-sm font-normal leading-normal tracking-normal text-ink shadow-floating dark:border-neutral-800 dark:bg-neutral-950 [&>button:last-child]:hidden"
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
            <aside className="hidden w-[200px] shrink-0 flex-col border-r border-neutral-200 bg-neutral-50 px-5 pt-5 pb-5 dark:border-neutral-800 dark:bg-neutral-900/50 @lg:flex">
              <div
                aria-hidden
                className="flex h-6 items-center gap-2 text-[13px] font-medium leading-none text-ink"
              >
                <Wordmark className="size-4" />
                Set up the harness
              </div>
              <Stepper
                orientation="vertical"
                steps={steps}
                current={step}
                reachable={reachable}
                onSelect={go}
                className="mt-7"
              />
            </aside>
            <div ref={content} className="flex min-w-0 flex-1 flex-col">
              {/* The close button's own strip: the body scrolls below it, never under it. */}
              <div className="hidden h-12 shrink-0 items-start justify-end px-4 pt-4 @lg:flex">
                <CloseButton />
              </div>
              <header className="flex shrink-0 flex-col gap-3 border-b border-neutral-200 px-4 pt-3 pb-3.5 dark:border-neutral-800 @md:px-6 @lg:hidden">
                <div className="flex h-8 items-center gap-2 text-[13px] font-medium leading-none text-ink">
                  <Wordmark className="size-4" aria-hidden />
                  <span aria-hidden>Set up the harness</span>
                  <CloseButton className="ml-auto" />
                </div>
                <Stepper
                  steps={steps}
                  current={step}
                  reachable={reachable}
                  onSelect={go}
                />
              </header>
              <m.div
                key={step}
                initial={reduceMotion ? false : { opacity: 0, y: 4 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{
                  duration: reduceMotion ? 0 : 0.16,
                  ease: [0.2, 0, 0, 1],
                }}
                className="flex min-h-0 flex-1 flex-col"
              >
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
                    onBack={() => go(showBrowser ? 'browser' : 'models')}
                    onNext={finishTo}
                  />
                ) : (
                  <ReadyStep
                    onboarding={onboarding}
                    judges={judges}
                    prompts={prompts}
                    agentNames={agentNames}
                    onPrompt={(prompt) => void startPrompt(prompt)}
                    tour={tour}
                    onStartTour={() => void startTour()}
                    onStart={start}
                  />
                )}
              </m.div>
            </div>
          </DialogContent>
        </Dialog>
      </MotionConfig>
    </LazyMotion>
  )
}

function CloseButton({ className }: { className?: string }) {
  return (
    <DialogClose
      aria-label="Close"
      className={cn(
        'flex size-8 shrink-0 items-center justify-center rounded-full text-neutral-500 transition-colors duration-150 hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus focus-visible:ring-offset-2 focus-visible:ring-offset-white dark:text-neutral-400 dark:focus-visible:ring-offset-neutral-950',
        className,
      )}
    >
      <X className="size-4" aria-hidden />
    </DialogClose>
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
