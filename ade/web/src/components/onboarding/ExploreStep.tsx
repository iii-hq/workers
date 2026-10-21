import {
  ArrowLeft,
  ArrowRight,
  Compass,
  LoaderCircle,
  MessageSquareText,
  Sparkles,
} from 'lucide-react'
import { useId, useState } from 'react'
import { DEFAULT_AGENT_ID } from '@/components/chat/agent-defaults'
import { Skeleton } from '@/components/ui/Skeleton'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { cn } from '@/lib/utils'
import { Button } from './controls'
import { Section, StatusChip, StepLayout } from './parts'

/** The learn-iii tour, as Explore iii starts it. */
export type TourState =
  | { kind: 'idle' }
  /** Getting the tour ready: adding its worker, waiting for its command. */
  | { kind: 'preparing' }
  | { kind: 'failed'; error: string }

/** The name the new-chat gallery shows for an agent profile, when known. */
function agentLabel(
  id: string,
  names: ReadonlyMap<string, string>,
): string | null {
  return names.get(id) ?? (id === DEFAULT_AGENT_ID ? 'Default' : null)
}

const OPTION_CLASS =
  'flex w-full min-w-0 items-start gap-3 rounded-lg border border-neutral-200 bg-transparent px-3.5 py-3 text-left transition-[background-color,border-color] duration-150 ease-[var(--motion-ease-standard)] hover:bg-neutral-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus focus-visible:ring-offset-2 focus-visible:ring-offset-white active:scale-[0.99] disabled:pointer-events-none disabled:opacity-50 dark:border-neutral-800 dark:hover:bg-neutral-900 dark:focus-visible:ring-offset-neutral-950'

/** One of the three ways in. */
function Option({
  icon: Icon,
  title,
  body,
  spinning,
  disabled,
  onClick,
  children,
}: {
  icon: typeof Compass
  title: string
  body?: string
  spinning?: boolean
  disabled?: boolean
  onClick: () => void
  children?: React.ReactNode
}) {
  return (
    <li className="flex min-w-0 flex-col gap-1">
      <button
        type="button"
        disabled={disabled}
        onClick={onClick}
        className={OPTION_CLASS}
      >
        <Icon
          className={cn(
            'mt-0.5 size-4 shrink-0 text-ink',
            spinning && 'animate-spin motion-reduce:animate-none',
          )}
          strokeWidth={1.75}
          aria-hidden
        />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="font-sans text-[13px] font-medium leading-5 text-ink">
            {title}
          </span>
          {body ? (
            <span className="text-pretty font-sans text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
              {body}
            </span>
          ) : null}
        </span>
      </button>
      {children}
    </li>
  )
}

/** The project's example prompts, one card each: a click starts a chat that sends it. */
export function ExampleList({
  prompts,
  agentNames,
  onPrompt,
}: {
  prompts: readonly ExamplePrompt[] | null
  agentNames: ReadonlyMap<string, string>
  onPrompt: (prompt: ExamplePrompt) => void
}) {
  const ids = useId()
  return (
    <Section
      title="Build an example project"
      hint="starts a new chat and sends it"
    >
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
                      <span className="text-pretty pl-6 font-sans text-[13px] leading-5 text-neutral-600 dark:text-neutral-400">
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
  )
}

/**
 * The step after Ready: build an example (the project's example prompts,
 * one more page in), take the learn-iii tour, or close setup and explore.
 * The first two need a model to answer them.
 */
export function ExploreStep({
  connected,
  prompts,
  agentNames,
  onPrompt,
  tour,
  onTour,
  onOwn,
  onBack,
}: {
  /** A model is connected: the examples and the tour can run. */
  connected: boolean
  /** The project's example prompts; `null` while they are being read. */
  prompts: readonly ExamplePrompt[] | null
  /** Agent profile display names by id, for each prompt's chip. */
  agentNames: ReadonlyMap<string, string>
  /** Close the wizard and open a new chat that sends this prompt. */
  onPrompt: (prompt: ExamplePrompt) => void
  tour: TourState
  /** Get the tour ready, close the wizard and show it. */
  onTour: () => void
  /** Close the wizard and hand the composer the focus. */
  onOwn: () => void
  /** Back to Ready. */
  onBack: () => void
}) {
  const [examples, setExamples] = useState(false)
  const preparing = tour.kind === 'preparing'
  const needsModel = connected ? undefined : 'Connect a model first.'

  if (examples) {
    return (
      <StepLayout
        footer={
          <Button variant="ghost" onClick={() => setExamples(false)}>
            <ArrowLeft aria-hidden />
            Back
          </Button>
        }
      >
        <ExampleList
          prompts={prompts}
          agentNames={agentNames}
          onPrompt={onPrompt}
        />
      </StepLayout>
    )
  }

  return (
    <StepLayout
      centered
      footer={
        <Button variant="ghost" onClick={onBack} disabled={preparing}>
          <ArrowLeft aria-hidden />
          Back
        </Button>
      }
    >
      <div className="flex flex-col gap-1">
        <h2 className="font-sans text-lg font-semibold leading-7 tracking-[-0.01em] text-ink">
          Explore iii
        </h2>
        {needsModel ? (
          <p className="font-sans text-sm leading-5 text-neutral-600 dark:text-neutral-400">
            {needsModel} The model picker in the composer connects one any time.
          </p>
        ) : null}
      </div>
      <ul className="flex flex-col gap-2">
        <Option
          icon={Sparkles}
          title="Build an example project"
          body="See the power of iii and its harness by building an example project in minutes."
          disabled={!connected || prompts?.length === 0 || preparing}
          onClick={() => setExamples(true)}
        />
        <Option
          icon={preparing ? LoaderCircle : Compass}
          title={preparing ? 'Preparing the tour…' : 'Take the learn-iii tour'}
          body="Want to learn more about iii, the engine that powers this harness and everything you build? Take a 5 minute tour to learn how iii does things differently."
          spinning={preparing}
          disabled={!connected || preparing}
          onClick={onTour}
        >
          {tour.kind === 'failed' ? (
            <p
              role="alert"
              className="px-3.5 font-sans text-[13px] leading-5 text-alert-strong"
            >
              The tour could not start: {tour.error}
            </p>
          ) : null}
        </Option>
        <Option
          icon={ArrowRight}
          title="Explore on my own"
          disabled={preparing}
          onClick={onOwn}
        />
      </ul>
    </StepLayout>
  )
}
