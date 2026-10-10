import {
  ArrowRight,
  Check,
  Compass,
  LoaderCircle,
  MessageSquareText,
} from 'lucide-react'
import { useId } from 'react'
import { DEFAULT_AGENT_ID } from '@/components/chat/agent-defaults'
import { Skeleton } from '@/components/ui/Skeleton'
import { Wordmark } from '@/components/ui/Wordmark'
import type { JudgeOption } from '@/lib/onboarding/catalog'
import { servesUsableModels } from '@/lib/onboarding/plan'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { Button } from './controls'
import { ProviderMark } from './ProviderMark'
import { Disclosure, Rows, Section, StatusChip, StepLayout } from './parts'
import type { OnboardingController } from './use-onboarding'

/** The guided tour offered once a model is connected. */
export type TourState =
  | { kind: 'idle' }
  /** Getting the tour ready: adding its worker, waiting for its page. */
  | { kind: 'preparing' }
  | { kind: 'failed'; error: string }

/** The name the new-chat gallery shows for an agent profile, when known. */
function agentLabel(
  id: string,
  names: ReadonlyMap<string, string>,
): string | null {
  return names.get(id) ?? (id === DEFAULT_AGENT_ID ? 'Default' : null)
}

/**
 * The last step: what setup connected, the project's example prompts — one
 * click opens a new chat with the prompt ready to send, its agent profile
 * and its model chosen — and the guided tour.
 */
export function ReadyStep({
  onboarding,
  judges,
  prompts,
  agentNames,
  onPrompt,
  tour,
  onStartTour,
  onStart,
}: {
  onboarding: OnboardingController
  /** The strategies Judge answers with, default first. */
  judges: readonly JudgeOption[]
  /** The project's example prompts; `null` while they are being read. */
  prompts: readonly ExamplePrompt[] | null
  /** Agent profile display names by id, for each prompt's chip. */
  agentNames: ReadonlyMap<string, string>
  /** Close the wizard and open a new chat with this prompt in the composer. */
  onPrompt: (prompt: ExamplePrompt) => void
  tour: TourState
  /** Accept the tour: the wizard gets it ready, then opens it. */
  onStartTour: () => void
  /** Close the wizard and hand the composer the focus. */
  onStart: () => void
}) {
  const { snapshot, activity } = onboarding
  const ids = useId()
  const connected = (snapshot.providers ?? []).filter(servesUsableModels)
  const totalModels = connected.reduce(
    (sum, provider) => sum + provider.modelCount,
    0,
  )
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
  const lines: { title: string; detail?: string; providerId?: string }[] = [
    ...connected.map((provider) => ({
      title: `${provider.title} connected`,
      providerId: provider.id,
      detail: `${provider.modelCount} ${provider.modelCount === 1 ? 'model' : 'models'}`,
    })),
    ...(chromium
      ? [
          {
            title: 'Chromium is ready for agents',
            detail: 'they open and check the pages they build',
          },
        ]
      : []),
    ...(judges.length > 0
      ? [
          {
            title: `Judge answers with ${judges[0].title}`,
            detail:
              judges.length > 1
                ? `${judges
                    .slice(1)
                    .map((option) => option.title)
                    .join(' and ')} also available`
                : 'function search, argument repair, browser decisions',
          },
        ]
      : []),
  ]

  // The tour's first stage is a message to the agent: it needs a model.
  const offerTour = connected.length > 0
  const preparing = tour.kind === 'preparing'
  // A prompt needs a model to answer it.
  const offerPrompts = connected.length > 0 && prompts?.length !== 0

  return (
    <StepLayout
      centered
      footer={
        <>
          <span className="hidden font-sans text-xs text-neutral-500 dark:text-neutral-400 @lg:inline">
            Reopen setup any time from the command palette.
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
                  <LoaderCircle
                    className="animate-spin motion-reduce:animate-none"
                    aria-hidden
                  />
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
      <div className="flex flex-col items-center gap-3 text-center">
        <Wordmark appearance="assemble" className="size-9" />
        <div className="flex flex-col items-center gap-1">
          <h2 className="font-sans text-lg font-semibold leading-7 tracking-[-0.01em] text-ink">
            {connected.length > 0 ? 'Your harness is ready' : 'You’re all set'}
          </h2>
          <p className="max-w-[48ch] text-pretty font-sans text-sm leading-5 text-neutral-600 dark:text-neutral-400">
            {connected.length > 0 ? (
              <>
                <span className="font-medium tabular-nums text-ink">
                  {totalModels}
                </span>{' '}
                {totalModels === 1 ? 'model' : 'models'} from{' '}
                <span className="font-medium tabular-nums text-ink">
                  {connected.length}
                </span>{' '}
                {connected.length === 1 ? 'provider' : 'providers'} are
                connected.
              </>
            ) : (
              'Connect a model any time from the model picker in the composer.'
            )}
          </p>
        </div>
      </div>

      {lines.length > 0 ? (
        <Rows as="ul">
          {lines.map((line) => (
            <li
              key={line.title}
              className="flex items-start gap-3 px-3.5 py-2.5"
            >
              <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center">
                {line.providerId ? (
                  <ProviderMark
                    id={line.providerId}
                    label={line.title}
                    className="size-4 text-ink"
                  />
                ) : (
                  <Check
                    className="size-4 text-ok"
                    strokeWidth={2.25}
                    aria-hidden
                  />
                )}
              </span>
              <span className="flex min-w-0 flex-col gap-px">
                <span className="font-sans text-[13px] font-medium leading-5 text-ink">
                  {line.title}
                </span>
                {line.detail ? (
                  <span className="break-words font-sans text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
                    {line.detail}
                  </span>
                ) : null}
              </span>
            </li>
          ))}
        </Rows>
      ) : null}

      {offerPrompts ? (
        <Section title="Try an example" hint="opens a new chat, ready to send">
          {prompts === null ? (
            <div
              role="status"
              aria-label="Loading example prompts"
              className="grid gap-2 @md:grid-cols-2"
            >
              {[0, 1, 2, 3].map((key) => (
                <Skeleton key={key} className="h-[76px] rounded-lg" />
              ))}
            </div>
          ) : (
            <ul className="grid gap-2 @md:grid-cols-2">
              {prompts.map((prompt, index) => {
                const agent = agentLabel(prompt.agent, agentNames)
                const described = `${ids}-prompt-${index}`
                return (
                  <li
                    // biome-ignore lint/suspicious/noArrayIndexKey: the list is read whole; two prompts may share a title
                    key={index}
                    className="flex min-w-0"
                  >
                    <button
                      type="button"
                      aria-label={`Start a chat: ${prompt.title}`}
                      aria-describedby={described}
                      onClick={() => onPrompt(prompt)}
                      className="flex w-full min-w-0 flex-col gap-1.5 rounded-lg border border-neutral-200 bg-transparent px-3 py-2.5 text-left transition-[background-color,border-color] duration-150 ease-[var(--motion-ease-standard)] hover:bg-neutral-50 focus-visible:outline-none dark:hover:bg-neutral-900 focus-visible:ring-2 focus-visible:ring-rule-focus focus-visible:ring-offset-2 focus-visible:ring-offset-white active:scale-[0.99] dark:border-neutral-800 dark:focus-visible:ring-offset-neutral-950"
                    >
                      <span className="flex items-start gap-2">
                        <MessageSquareText
                          className="mt-0.5 size-4 shrink-0 text-ink"
                          strokeWidth={1.75}
                          aria-hidden
                        />
                        <span className="min-w-0 flex-1 font-sans text-[13px] font-medium leading-5 text-ink">
                          {prompt.title}
                        </span>
                      </span>
                      <span id={described} className="flex flex-col gap-1.5">
                        {prompt.description ? (
                          <span className="text-pretty pl-6 font-sans text-xs leading-4 text-neutral-600 dark:text-neutral-400">
                            {prompt.description}
                          </span>
                        ) : null}
                        {agent ? (
                          <span className="pl-6">
                            <StatusChip tone="neutral">{agent}</StatusChip>
                          </span>
                        ) : null}
                      </span>
                    </button>
                  </li>
                )
              })}
            </ul>
          )}
        </Section>
      ) : null}

      {added.length > 0 ? (
        <Disclosure label="Workers added" summary={`${added.length} installed`}>
          <ul className="grid gap-x-4 gap-y-1 rounded-lg border border-neutral-200 bg-neutral-50 px-3.5 py-2.5 font-mono text-xs leading-5 dark:border-neutral-800 dark:bg-neutral-900 @md:grid-cols-2">
            {added.map((worker) => (
              <li key={worker} className="flex min-w-0 items-center gap-2">
                <span
                  aria-hidden
                  className="select-none text-neutral-500 dark:text-neutral-400"
                >
                  +
                </span>
                <span className="truncate text-ink">{worker}</span>
              </li>
            ))}
          </ul>
        </Disclosure>
      ) : null}

      {offerTour ? (
        <section
          aria-label="Guided tour"
          className="flex gap-3 rounded-lg border border-neutral-200 bg-neutral-50 px-3.5 py-3 dark:border-neutral-800 dark:bg-neutral-900"
        >
          <Compass
            className="mt-0.5 size-4 shrink-0 text-ink"
            strokeWidth={1.75}
            aria-hidden
          />
          <span className="flex min-w-0 flex-col gap-0.5">
            <h3 className="font-sans text-[13px] font-medium leading-5 text-ink">
              Keep going with a guided tour
            </h3>
            <p className="text-pretty font-sans text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
              Send your first message and see your agent use tools, react to
              events, and trace its work. We’ll show you around as you go.
            </p>
            {tour.kind === 'failed' ? (
              <p
                role="alert"
                className="mt-1 font-sans text-[13px] leading-5 text-alert-strong"
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
