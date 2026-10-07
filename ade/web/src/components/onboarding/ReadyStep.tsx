import { ArrowRight, Boxes, Check, MessageSquareText } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import { DEFAULT_AGENT_ID } from '@/components/chat/agent-defaults'
import { Button } from '@/components/ui/Button'
import { Skeleton } from '@/components/ui/Skeleton'
import { Wordmark } from '@/components/ui/Wordmark'
import type { JudgeOption } from '@/lib/onboarding/catalog'
import { servesUsableModels } from '@/lib/onboarding/plan'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { Section, StatusChip, StepLayout } from './parts'
import type { OnboardingController } from './use-onboarding'

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

/** The name the new-chat gallery shows for an agent profile, when known. */
function agentLabel(
  id: string,
  names: ReadonlyMap<string, string>,
): string | null {
  return names.get(id) ?? (id === DEFAULT_AGENT_ID ? 'Default' : null)
}

/**
 * The last step: what setup connected, every worker it added (iii is
 * composable, so each one is new behavior in the project), and the
 * project's example prompts — one click opens a new chat with the prompt
 * ready to send, its agent profile and its model chosen.
 */
export function ReadyStep({
  onboarding,
  judge,
  prompts,
  agentNames,
  onPrompt,
  onFinish,
}: {
  onboarding: OnboardingController
  judge: JudgeOption | null
  /** The project's example prompts; `null` while they are being read. */
  prompts: readonly ExamplePrompt[] | null
  /** Agent profile display names by id, for each prompt's chip. */
  agentNames: ReadonlyMap<string, string>
  /** Close the wizard and open a new chat with this prompt in the composer. */
  onPrompt: (prompt: ExamplePrompt) => void
  /** Close the wizard and hand the composer the focus. */
  onFinish: () => void
}) {
  const { snapshot, activity } = onboarding
  const ids = useId()
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
    ? { version: snapshot.browser?.version }
    : null

  const lines: { title: string; detail?: string }[] = [
    ...connected.map((provider) => ({
      title: `${provider.title} connected`,
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
    ...(judge
      ? [
          {
            title: `Judge answers with ${judge.title}`,
            detail: 'the small decisions agents make along the way',
          },
        ]
      : []),
  ]

  // A prompt needs a model to answer it.
  const offerPrompts = connected.length > 0 && prompts?.length !== 0

  return (
    <StepLayout
      footer={
        <>
          <span className="hidden font-sans text-[13px] text-ink @2xl:inline">
            Reopen this from the command palette: Set up the harness.
          </span>
          <Button className="ml-auto" onClick={() => onFinish()}>
            Finish
            <ArrowRight aria-hidden />
          </Button>
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
        <Section title={`Workers added to your project (${added.length})`}>
          <div className="flex gap-3 rounded-md bg-card-highlight px-4 py-3">
            <Boxes className="mt-0.5 size-4 shrink-0 text-accent" aria-hidden />
            <div className="flex min-w-0 flex-col gap-2">
              <p className="text-pretty font-sans text-[13px] leading-relaxed text-ink">
                iii is composable: each worker adds new behavior to your
                project. They live in{' '}
                <span className="font-mono">worker-compose.yaml</span>, so the
                project starts with them every time.
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
            </div>
          </div>
        </Section>
      ) : null}

      {offerPrompts ? (
        <Section title="Try an example">
          {prompts === null ? (
            <div
              role="status"
              aria-label="Loading example prompts"
              className="grid gap-2 @2xl:grid-cols-2"
            >
              {[0, 1].map((key) => (
                <Skeleton key={key} className="h-24 rounded-md" />
              ))}
            </div>
          ) : (
            <ul className="grid gap-2 @2xl:grid-cols-2">
              {prompts.map((prompt, index) => {
                const agent = agentLabel(prompt.agent, agentNames)
                const described = `${ids}-prompt-${index}`
                return (
                  <li
                    // biome-ignore lint/suspicious/noArrayIndexKey: the list is read whole; two prompts may share a title
                    key={index}
                    className="onboarding-rise flex min-w-0"
                    style={{ animationDelay: `${900 + index * 80}ms` }}
                  >
                    <button
                      type="button"
                      aria-label={`Start a chat: ${prompt.title}`}
                      aria-describedby={described}
                      onClick={() => onPrompt(prompt)}
                      className="group flex w-full min-w-0 flex-col gap-1.5 rounded-md bg-surface px-3 py-3 text-left hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-rule-focus"
                    >
                      <span className="flex items-start gap-2">
                        <MessageSquareText
                          className="mt-0.5 size-4 shrink-0 text-ink"
                          aria-hidden
                        />
                        <span className="min-w-0 flex-1 font-sans text-[14px] font-medium text-ink">
                          {prompt.title}
                        </span>
                      </span>
                      <span id={described} className="flex flex-col gap-1.5">
                        {prompt.description ? (
                          <span className="text-pretty pl-6 font-sans text-[13px] leading-relaxed text-ink">
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
          <p className="font-sans text-[13px] leading-relaxed text-ink">
            Opens a new chat with the message ready for you to send.
          </p>
        </Section>
      ) : null}
    </StepLayout>
  )
}
