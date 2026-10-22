import {
  ArrowLeft,
  ArrowRight,
  BookOpen,
  Bot,
  Compass,
  Hammer,
  Link2,
  LoaderCircle,
  type LucideIcon,
  MessageSquareText,
  Receipt,
  Sparkles,
} from 'lucide-react'
import { m, useReducedMotion } from 'motion/react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { DEFAULT_AGENT_ID } from '@/components/chat/agent-defaults'
import { Skeleton } from '@/components/ui/Skeleton'
import type { ExamplePrompt } from '@/lib/onboarding/prompts'
import { cn } from '@/lib/utils'
import { Button } from './controls'
import { StepLayout } from './parts'

/** The learn-iii tour, as Explore iii starts it. */
export type TourState =
  | { kind: 'idle' }
  /** Getting the tour ready: adding its worker, waiting for its command. */
  | { kind: 'preparing' }
  | { kind: 'failed'; error: string }

/**
 * The icon an example prompt leads with: what its title says it builds —
 * a link, an expense list, an agent, an explanation — read from the words
 * the title uses, since the prompts come from the project's own file. A
 * title that names none of these falls back to what its agent does: the
 * worker builder builds, the default profile reads and explains.
 */
const PROMPT_ICONS: readonly [RegExp, LucideIcon][] = [
  [/\b(link|url|redirect)/i, Link2],
  [/\b(expense|invoice|receipt|budget|cost)/i, Receipt],
  [/\bagent/i, Bot],
  [/\b(explain|overview|understand|tour)/i, BookOpen],
  [/\b(build|create|make)\b/i, Hammer],
]

function promptIcon(
  prompt: Pick<ExamplePrompt, 'title' | 'agent'>,
): LucideIcon {
  const match = PROMPT_ICONS.find(([words]) => words.test(prompt.title))
  if (match) return match[1]
  if (prompt.agent === DEFAULT_AGENT_ID) return BookOpen
  if (prompt.agent.includes('builder')) return Hammer
  return MessageSquareText
}

/*
 * Both screens here are a choice, made in two moves: pick one of the
 * options, then confirm it with the footer's button, which names what it
 * will do; with nothing picked it is Finish, and closes setup. The options are a
 * radio group, so arrow keys move the pick and Space or a click sets it;
 * the picked one carries an ink border and a filled dot.
 */

const PICK_CLASS =
  'group flex w-full min-w-0 items-start gap-4 rounded-lg border bg-white px-4 py-3.5 text-left transition-[background-color,border-color,box-shadow] duration-150 ease-[var(--motion-ease-standard)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus focus-visible:ring-offset-2 focus-visible:ring-offset-white disabled:pointer-events-none disabled:opacity-50 motion-reduce:transition-none dark:bg-neutral-950 dark:focus-visible:ring-offset-neutral-950'
const PICK_IDLE =
  'border-neutral-200 hover:border-neutral-300 hover:bg-neutral-50 dark:border-neutral-800 dark:hover:border-neutral-700 dark:hover:bg-neutral-900'
const PICK_CHECKED =
  'border-ink bg-neutral-50 shadow-[inset_0_0_0_1px_var(--color-ink)] dark:bg-neutral-900'

/** Rows rise in one after another as the screen opens; nothing waits on it. */
const ROW_ENTER = { duration: 0.2, ease: [0.23, 1, 0.32, 1] as const }
const ROW_STAGGER_S = 0.03

/** The radio mark at a row's end: a ring, filled when the row is picked. */
function Dot({ checked }: { checked: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        'flex size-[18px] shrink-0 items-center justify-center rounded-full border transition-colors duration-150 ease-[var(--motion-ease-standard)] motion-reduce:transition-none',
        checked
          ? 'border-ink'
          : 'border-neutral-300 group-hover:border-neutral-400 dark:border-neutral-600 dark:group-hover:border-neutral-500',
      )}
    >
      <span
        className={cn(
          'size-2 rounded-full bg-ink transition-transform duration-150 ease-[var(--motion-ease-standard)] motion-reduce:transition-none',
          checked ? 'scale-100' : 'scale-0',
        )}
      />
    </span>
  )
}

/** One option of a group: the whole row is the radio. */
function Pick({
  checked,
  tabbable,
  disabled,
  onPick,
  buttonRef,
  lead,
  title,
  body,
  order,
}: {
  checked: boolean
  /** The one row the Tab key lands on: the picked one, else the first. */
  tabbable: boolean
  disabled?: boolean
  onPick: () => void
  buttonRef: (element: HTMLButtonElement | null) => void
  /** What starts the row: an icon, a number. */
  lead: React.ReactNode
  title: string
  body?: string
  /** Its place in the list, for the entrance stagger. */
  order: number
}) {
  const reduceMotion = useReducedMotion()
  return (
    <m.li
      initial={reduceMotion ? false : { opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ ...ROW_ENTER, delay: order * ROW_STAGGER_S }}
    >
      <button
        ref={buttonRef}
        type="button"
        role="radio"
        aria-checked={checked}
        tabIndex={tabbable ? 0 : -1}
        disabled={disabled}
        onClick={onPick}
        className={cn(PICK_CLASS, checked ? PICK_CHECKED : PICK_IDLE)}
      >
        {lead}
        <span className="flex min-w-0 flex-1 flex-col gap-px">
          <span className="text-[16.5px] font-medium leading-6 text-ink">
            {title}
          </span>
          {body ? (
            <span className="text-pretty text-base leading-6 text-neutral-600 dark:text-neutral-400">
              {body}
            </span>
          ) : null}
        </span>
        <span className="flex self-stretch items-center">
          <Dot checked={checked} />
        </span>
      </button>
    </m.li>
  )
}

/**
 * The keyboard half of a radio group: arrow keys move the pick across the
 * enabled rows and put the focus with it. Rows register themselves by key;
 * `focusFirst` is for the screen's arrival, when nothing is picked yet and
 * the footer's button, which setup would focus, is still disabled.
 */
function usePickGroup<K extends string>(
  keys: readonly K[],
  enabled: (key: K) => boolean,
  picked: K | null,
  onPick: (key: K) => void,
  /** The rows are on screen; when they come back, they arrive again. */
  active = true,
) {
  const buttons = useRef(new Map<K, HTMLButtonElement>())
  const register = useCallback(
    (key: K) => (element: HTMLButtonElement | null) => {
      if (element) buttons.current.set(key, element)
      else buttons.current.delete(key)
    },
    [],
  )
  const onKeyDown = (event: React.KeyboardEvent) => {
    const step =
      event.key === 'ArrowDown' || event.key === 'ArrowRight'
        ? 1
        : event.key === 'ArrowUp' || event.key === 'ArrowLeft'
          ? -1
          : 0
    if (step === 0) return
    const open = keys.filter(enabled)
    if (open.length === 0) return
    // From the row that has the focus: on arrival that is the first row
    // with nothing picked yet, and the first key must leave it.
    const focused = open.find(
      (key) => buttons.current.get(key) === document.activeElement,
    )
    const at = open.indexOf(focused ?? picked ?? open[0])
    const next =
      focused === undefined && picked === null
        ? open[0]
        : open[(at + step + open.length) % open.length]
    event.preventDefault()
    onPick(next)
    buttons.current.get(next)?.focus()
  }
  const first = keys.find(enabled) ?? null
  const tabbable = (key: K) => (picked ? key === picked : key === first)
  // On arrival, two frames on — after setup's own focus move, which runs on
  // the first — the first open row takes the focus. The first row can open
  // later (the examples are read after the screen shows): then it takes
  // the focus only if the focus is already in the group, so a reader who
  // has moved on is not pulled back.
  const arrived = useRef(false)
  useEffect(() => {
    if (!active) {
      arrived.current = false
      return
    }
    // On arrival the picked row, else the first open one; after that only
    // the first row, when it opens late and the focus is still in here.
    const key = arrived.current
      ? picked === null
        ? first
        : null
      : (picked ?? first)
    if (key === null) return
    const target = buttons.current.get(key)
    if (!target) return
    const take = () => {
      const within = [...buttons.current.values()].some(
        (button) => button === document.activeElement,
      )
      if (!arrived.current || within) target.focus()
      arrived.current = true
    }
    let inner = 0
    const outer = requestAnimationFrame(() => {
      inner = requestAnimationFrame(take)
    })
    return () => {
      cancelAnimationFrame(outer)
      cancelAnimationFrame(inner)
    }
  }, [picked, first, active])
  return { register, onKeyDown, tabbable }
}

function Lead({
  icon: Icon,
  spinning,
}: {
  icon: LucideIcon
  spinning?: boolean
}) {
  return (
    <Icon
      className={cn(
        'mt-0.5 size-5 shrink-0 text-ink',
        spinning && 'animate-spin motion-reduce:animate-none',
      )}
      strokeWidth={1.75}
      aria-hidden
    />
  )
}

/**
 * The project's example prompts, one row each, led by an icon for what
 * its agent does. Picking one and confirming starts a chat that sends it.
 */
export function ExampleList({
  prompts,
  picked,
  onPick,
}: {
  prompts: readonly ExamplePrompt[] | null
  /** The index of the picked prompt. */
  picked: number | null
  onPick: (index: number) => void
}) {
  const keys = (prompts ?? []).map((_, index) => String(index))
  const group = usePickGroup(
    keys,
    () => true,
    picked === null ? null : String(picked),
    (key) => onPick(Number(key)),
  )
  return (
    <>
      <header className="-mt-2 flex flex-col gap-1.5">
        <h2 className="text-balance text-xl font-semibold leading-7 tracking-[-0.01em] text-ink">
          Build an example project
        </h2>
        <p className="max-w-[56ch] text-pretty text-base leading-6 text-neutral-600 dark:text-neutral-400">
          Pick one, and a new chat starts building it.
        </p>
      </header>
      {prompts === null ? (
        <div
          role="status"
          aria-label="Loading example prompts"
          className="flex flex-col gap-2.5"
        >
          {[0, 1, 2, 3].map((key) => (
            <Skeleton key={key} className="h-[74px] rounded-lg" />
          ))}
        </div>
      ) : (
        <ul
          role="radiogroup"
          aria-label="Example projects"
          className="flex flex-col gap-2.5"
          onKeyDown={group.onKeyDown}
        >
          {prompts.map((prompt, index) => {
            const key = String(index)
            return (
              <Pick
                // biome-ignore lint/suspicious/noArrayIndexKey: the list is read whole; two prompts may share a title
                key={index}
                order={index}
                checked={picked === index}
                tabbable={group.tabbable(key)}
                onPick={() => onPick(index)}
                buttonRef={group.register(key)}
                lead={<Lead icon={promptIcon(prompt)} />}
                title={prompt.title}
                body={prompt.description}
              />
            )
          })}
        </ul>
      )}
    </>
  )
}

type ChoiceId = 'example' | 'tour' | 'own'
const CHOICES: readonly ChoiceId[] = ['example', 'tour', 'own']

/**
 * The step after Ready: build an example (the project's example prompts,
 * one more page in), take the learn-iii tour, or close setup and explore.
 * The first two need a model to answer them. One is picked, then the
 * footer's button — Choose a project, Start the tour, Finish — does it;
 * with nothing picked the button is Finish, and closes setup.
 */
export function ExploreStep({
  connected,
  prompts,
  onPrompt,
  tour,
  onTour,
  onOwn,
  onFinish,
  onBack,
}: {
  /** A model is connected: the examples and the tour can run. */
  connected: boolean
  /** The project's example prompts; `null` while they are being read. */
  prompts: readonly ExamplePrompt[] | null
  /** Close the wizard and open a new chat that sends this prompt. */
  onPrompt: (prompt: ExamplePrompt) => void
  tour: TourState
  /** Get the tour ready, close the wizard and show it. */
  onTour: () => void
  /** Close the wizard and hand the composer the focus: Explore on my own. */
  onOwn: () => void
  /** Close the wizard: Finish, with nothing picked. */
  onFinish: () => void
  /** Back to Ready. */
  onBack: () => void
}) {
  const [examples, setExamples] = useState(false)
  const [picked, setPicked] = useState<ChoiceId | null>(null)
  const [prompt, setPrompt] = useState<number | null>(null)
  // The pick is an index into the list it was made from; a new list (the
  // project's file re-read) may order its prompts differently.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `prompts` changing is the point
  useEffect(() => setPrompt(null), [prompts])
  const preparing = tour.kind === 'preparing'
  const count = prompts?.length ?? 0
  const can: Record<ChoiceId, boolean> = {
    example: connected && count > 0 && !preparing,
    tour: connected && !preparing,
    own: !preparing,
  }
  // A pick that has since gone out of reach (the model left, the examples
  // emptied) counts as no pick: the button goes back to Finish.
  const choice = picked !== null && can[picked] ? picked : null
  const group = usePickGroup(
    CHOICES,
    (id) => can[id],
    choice,
    setPicked,
    !examples,
  )

  if (examples) {
    const chosen = prompt !== null ? prompts?.[prompt] : undefined
    return (
      <StepLayout
        centered
        footer={
          <>
            <Button
              variant="ghost"
              className="-ml-3"
              onClick={() => setExamples(false)}
            >
              <ArrowLeft aria-hidden />
              Back
            </Button>
            <Button
              disabled={!chosen}
              onClick={() => chosen && onPrompt(chosen)}
            >
              Start building
              <ArrowRight aria-hidden />
            </Button>
          </>
        }
      >
        <ExampleList prompts={prompts} picked={prompt} onPick={setPrompt} />
      </StepLayout>
    )
  }

  const go = () => {
    if (choice === 'example') setExamples(true)
    else if (choice === 'tour') onTour()
    else if (choice === 'own') onOwn()
    else onFinish()
  }
  const label = preparing
    ? 'Preparing the tour…'
    : choice === 'example'
      ? 'Choose a project'
      : choice === 'tour'
        ? 'Start the tour'
        : 'Finish'

  return (
    <StepLayout
      centered
      footer={
        <>
          <Button
            variant="ghost"
            className="-ml-3"
            onClick={onBack}
            disabled={preparing}
          >
            <ArrowLeft aria-hidden />
            Back
          </Button>
          <Button disabled={preparing} onClick={go}>
            {preparing ? (
              <LoaderCircle
                className="animate-spin motion-reduce:animate-none"
                aria-hidden
              />
            ) : null}
            {label}
            {preparing ? null : <ArrowRight aria-hidden />}
          </Button>
        </>
      }
    >
      <header className="-mt-2 flex flex-col gap-1.5">
        <h2 className="text-balance text-xl font-semibold leading-7 tracking-[-0.01em] text-ink">
          Explore iii
        </h2>
        <p className="max-w-[56ch] text-pretty text-base leading-6 text-neutral-600 dark:text-neutral-400">
          {connected
            ? 'Choose where to start.'
            : 'Connect a model first. The model picker in the composer connects one any time.'}
        </p>
      </header>
      <ul
        role="radiogroup"
        aria-label="Where to start"
        className="flex flex-col gap-2.5"
        onKeyDown={group.onKeyDown}
      >
        <Pick
          order={0}
          checked={choice === 'example'}
          tabbable={group.tabbable('example')}
          disabled={!can.example}
          onPick={() => setPicked('example')}
          buttonRef={group.register('example')}
          lead={<Lead icon={Sparkles} />}
          title="Build an example project"
          body="See what iii and its harness can do by building a small project in minutes."
        />
        <Pick
          order={1}
          checked={choice === 'tour'}
          tabbable={group.tabbable('tour')}
          disabled={!can.tour}
          onPick={() => setPicked('tour')}
          buttonRef={group.register('tour')}
          lead={<Lead icon={Compass} />}
          title="Take the learn-iii tour"
          body="Learn how iii, the engine under this harness, does things differently."
        />
        <Pick
          order={2}
          checked={choice === 'own'}
          tabbable={group.tabbable('own')}
          disabled={!can.own}
          onPick={() => setPicked('own')}
          buttonRef={group.register('own')}
          lead={<Lead icon={MessageSquareText} />}
          title="Explore on my own"
          body="Close setup and start a chat of your own."
        />
      </ul>
      {tour.kind === 'failed' ? (
        <p
          role="alert"
          className="-mt-2 px-1 text-pretty text-sm leading-5 text-alert-strong"
        >
          The tour could not start: {tour.error}
        </p>
      ) : null}
    </StepLayout>
  )
}
