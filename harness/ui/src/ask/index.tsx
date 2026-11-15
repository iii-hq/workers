/**
 * Injected function-call renderer for `harness::ask`, registered through
 * `host.functionCalls` with `metadata.display: true`, so the card stays
 * visible in the chat feed while the raw call details stay collapsed.
 *
 * The card draws the questions from the result `details` (`parseAsk`): per
 * question a fieldset with the header chip (a long header ends in an
 * ellipsis and shows whole on hover), the question, a Skip toggle, a
 * checkbox per option (one control for both kinds: a single-select question
 * ticks one box and clears the rest) with the description under each label,
 * and a single-line "Other" field. A skipped question sends
 * `Header: (skipped)` and takes no pick. The foot has Dismiss, which sends
 * the dismissal notice instead of answers, and Send, which hands the
 * formatted answer to the composer (`host.chat.compose`, submitted); either
 * starts the next turn.
 *
 * Send is not proof of sending: `compose` returns nothing and the composer
 * drops a submit while it is blocked, keeping the text as a draft. So after
 * Send (or Dismiss) the card waits with its buttons disabled and re-reads
 * the session a bounded number of times; the send landed when the text is
 * parked in the queue, the session is on another turn, or the same turn
 * stepped on (`sendLanded` in lib/ask). Then the card records the outcome
 * through `harness::ask::settle` and closes; when the last read still shows
 * nothing landed, it says so and takes an answer again.
 *
 * Open vs settled: only a settlement closes a card, read from
 * `harness::ask::settlements` on mount, when the tab becomes visible again,
 * and when focus enters an open card (so a card settled in another tab
 * closes before a second send). The session moving to a later turn does
 * not close it: the person may have typed something else in the chat, and
 * the question still needs its answer. The card then says the chat moved on
 * and keeps taking an answer, or a dismissal.
 *
 * Anything `parseAsk` rejects (a refused ask, a running call) returns null
 * and falls through to the console's default card.
 */

import {
  Badge,
  Button,
  Checkbox,
  Chip,
  type FunctionCallMessage,
  type FunctionCallRenderer,
  type Host,
  Input,
} from '@iii-dev/console-ui'
import { Check } from 'lucide-react'
import {
  type FocusEvent,
  type FormEvent,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
} from 'react'
import {
  askCardState,
  type AskOutcome,
  type AskQuestionView,
  type AskSelection,
  type AskView,
  type CurrentTurn,
  DISMISS_TEXT,
  formatAnswer,
  movedOn,
  parseAsk,
  sendLanded,
  type SendPhase,
  settlementsOf,
  turnPositionOf,
} from '../lib/ask'

const ASK_ID = 'harness::ask'
const STATUS_ID = 'harness::status'
const SETTLEMENTS_ID = 'harness::ask::settlements'
const SETTLE_ID = 'harness::ask::settle'
const CALL_TIMEOUT_MS = 5000

/** Session re-reads after a Send, in ms after it: bounded, not polling. */
const CONFIRM_READS_MS = [500, 1500, 3000] as const

/**
 * Whether this card's question is settled, and a function to re-read it.
 * A settlement, once seen, sticks: a later read that fails (or an older
 * harness without the function) cannot reopen a settled card. `markSettled`
 * is for this card's own confirmed send.
 */
function useSettlement(host: Host, sessionId: string, questionId: string) {
  const [settlement, setSettlement] = useState<AskOutcome | undefined>(undefined)
  const mounted = useRef(true)

  const read = useCallback((): Promise<void> => {
    return host.iii
      .call(SETTLEMENTS_ID, { session_id: sessionId }, { timeoutMs: CALL_TIMEOUT_MS })
      .then(settlementsOf, () => undefined)
      .then((settled) => {
        const outcome = settled?.get(questionId)
        if (mounted.current && outcome !== undefined) setSettlement(outcome)
      })
  }, [host, sessionId, questionId])

  const markSettled = useCallback(
    (outcome: AskOutcome) => {
      setSettlement(outcome)
      // Best effort: the local state already reflects a confirmed send; a
      // lost write only means the card reopens after a reload.
      host.iii
        .call(
          SETTLE_ID,
          { session_id: sessionId, question_id: questionId, outcome },
          { timeoutMs: CALL_TIMEOUT_MS },
        )
        .catch(() => undefined)
    },
    [host, sessionId, questionId],
  )

  useEffect(() => {
    mounted.current = true
    void read()
    const onVisibility = () => {
      if (document.visibilityState === 'visible') void read()
    }
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      mounted.current = false
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [read])

  return [settlement, read, markSettled] as const
}

/**
 * Where the session stands (a verbose `harness::status` read) and a function
 * to re-read it, which resolves with the position just read. A newer read
 * always wins over an older one still in flight, and nothing lands after
 * unmount. The position only informs: it tells the card when the chat moved
 * on, and it is the "before" a send is confirmed against.
 */
function useTurnPosition(host: Host, sessionId: string) {
  const [turn, setTurn] = useState<CurrentTurn>(undefined)
  const known = useRef<CurrentTurn>(undefined)
  const latest = useRef(0)

  const read = useCallback((): Promise<CurrentTurn> => {
    const request = ++latest.current
    return host.iii
      .call(STATUS_ID, { session_id: sessionId, verbose: true }, { timeoutMs: CALL_TIMEOUT_MS })
      .then(turnPositionOf, () => undefined)
      .then((position) => {
        if (request !== latest.current) return undefined
        known.current = position
        setTurn(position)
        return position
      })
  }, [host, sessionId])

  useEffect(() => {
    void read()
    const onVisibility = () => {
      if (document.visibilityState === 'visible') void read()
    }
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      document.removeEventListener('visibilitychange', onVisibility)
      latest.current += 1
    }
  }, [read])

  return [turn, known, read] as const
}

function QuestionFields({
  question,
  selection,
  disabled,
  onChange,
}: {
  question: AskQuestionView
  selection: AskSelection
  disabled: boolean
  onChange: (next: AskSelection) => void
}) {
  const baseId = useId()
  const otherId = `${baseId}-other`
  const skipped = selection.skipped === true
  const locked = disabled || skipped

  const toggle = (label: string, checked: boolean) => {
    const picked = checked
      ? [...selection.picked, label]
      : selection.picked.filter((picked) => picked !== label)
    onChange({ ...selection, picked })
  }
  // Single-select ticks one box at a time: ticking an option clears the
  // others and Other, unticking it leaves nothing picked, and typing in
  // Other clears the pick.
  const pickOnly = (label: string, checked: boolean) =>
    onChange(checked ? { picked: [label], other: '' } : { ...selection, picked: [] })
  const setOther = (other: string) =>
    onChange(question.multiSelect ? { ...selection, other } : { picked: [], other })

  return (
    <fieldset className="harness-ui-ask-q" data-skipped={skipped || undefined}>
      <legend className="harness-ui-ask-legend">
        <Chip className="harness-ui-ask-header" title={question.header}>
          <span className="harness-ui-ask-header-text">{question.header}</span>
        </Chip>
        <span className="harness-ui-ask-question">{question.question}</span>
      </legend>
      <div className="harness-ui-ask-row">
        <p className="harness-ui-ask-note" role="status">
          {skipped
            ? 'Skipped: the agent decides this one.'
            : question.multiSelect
              ? 'Choose any that apply.'
              : 'Choose one.'}
        </p>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="harness-ui-ask-skip"
          aria-pressed={skipped}
          disabled={disabled}
          onClick={() => onChange({ ...selection, skipped: !skipped })}
        >
          {skipped ? 'Answer instead' : 'Skip'}
        </Button>
      </div>
      <div className="harness-ui-ask-options">
        {question.options.map((option, index) => {
          const checked = selection.picked.includes(option.label)
          const descriptionId = option.description ? `${baseId}-d${index}` : undefined
          return (
            <div key={option.label} className="harness-ui-ask-option">
              <Checkbox
                className="harness-ui-ask-choice"
                label={option.label}
                checked={checked}
                disabled={locked}
                aria-describedby={descriptionId}
                onChange={(event) =>
                  question.multiSelect
                    ? toggle(option.label, event.currentTarget.checked)
                    : pickOnly(option.label, event.currentTarget.checked)
                }
              />
              {option.description ? (
                <span id={descriptionId} className="harness-ui-ask-desc">
                  {option.description}
                </span>
              ) : null}
            </div>
          )
        })}
      </div>
      <div className="harness-ui-ask-other">
        <label htmlFor={otherId} className="harness-ui-ask-other-label">
          Other
        </label>
        <Input
          id={otherId}
          className="harness-ui-ask-other-input"
          value={selection.other ?? ''}
          disabled={locked}
          onChange={setOther}
          placeholder="Type your own answer"
          autoComplete="off"
        />
      </div>
    </fieldset>
  )
}

/** Read-only question: every option listed, the chosen ones checked. */
function QuestionSummary({
  question,
  selection,
}: {
  question: AskQuestionView
  selection?: AskSelection
}) {
  const headingId = useId()
  const skipped = selection?.skipped === true
  const other = skipped ? '' : selection?.other?.trim()
  return (
    <div className="harness-ui-ask-q" role="group" aria-labelledby={headingId}>
      <div id={headingId} className="harness-ui-ask-legend">
        <Chip className="harness-ui-ask-header" title={question.header}>
          <span className="harness-ui-ask-header-text">{question.header}</span>
        </Chip>
        <span className="harness-ui-ask-question">{question.question}</span>
      </div>
      {skipped ? <p className="harness-ui-ask-note">Skipped: the agent decides this one.</p> : null}
      <ul className="harness-ui-ask-summary">
        {question.options.map((option) => {
          // On a single-select, Other text replaces the pick.
          const counts = !skipped && (question.multiSelect || !other)
          const chosen = counts && (selection?.picked.includes(option.label) ?? false)
          return (
            <li
              key={option.label}
              className="harness-ui-ask-summary-row"
              data-chosen={chosen || undefined}
            >
              <span className="harness-ui-ask-mark">
                {chosen ? <Check size={16} aria-hidden /> : null}
              </span>
              <span className="harness-ui-ask-summary-text">
                <span>
                  {chosen ? <span className="harness-ui-ask-sr">Chosen: </span> : null}
                  {option.label}
                </span>
                {option.description ? (
                  <span className="harness-ui-ask-desc">{option.description}</span>
                ) : null}
              </span>
            </li>
          )
        })}
        {other ? (
          <li className="harness-ui-ask-summary-row" data-chosen>
            <span className="harness-ui-ask-mark">
              <Check size={16} aria-hidden />
            </span>
            <span className="harness-ui-ask-summary-text">
              <span>
                <span className="harness-ui-ask-sr">Chosen: </span>
                Other: {other}
              </span>
            </span>
          </li>
        ) : null}
      </ul>
    </div>
  )
}

function AskCard({ host, view }: { host: Host; view: AskView }) {
  const [settlement, recheckSettlement, markSettled] = useSettlement(
    host,
    view.sessionId,
    view.questionId,
  )
  const [turn, knownTurn, rereadTurn] = useTurnPosition(host, view.sessionId)
  const [selections, setSelections] = useState<AskSelection[]>(() =>
    view.questions.map(() => ({ picked: [] })),
  )
  const [phase, setPhase] = useState<SendPhase>('idle')
  const [intent, setIntent] = useState<AskOutcome>('answered')
  const hintId = useId()
  const confirmation = useRef(0)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  // Held from a Send until its reads run out: two submits in one task both
  // run before React re-renders the disabled button.
  const pending = useRef(false)

  useEffect(
    () => () => {
      confirmation.current += 1
      clearTimeout(timer.current)
    },
    [],
  )

  const canCompose = typeof host.chat?.compose === 'function'
  const state = askCardState(settlement, phase)
  const settled = state === 'answered' || state === 'dismissed'
  const sending = state === 'sending'
  const interactive = canCompose && !settled
  // This card's own send is what settled it, so the summary can show what
  // was chosen.
  const sentHere = settled && phase === 'sending'
  const text = formatAnswer(view, selections)

  const update = (index: number, next: AskSelection) =>
    setSelections((previous) => previous.map((entry, i) => (i === index ? next : entry)))

  // Re-read the session after a send until it landed or the reads run out;
  // nothing lands after unmount or after a newer send.
  const confirmSend = async (before: CurrentTurn, sent: string, outcome: AskOutcome) => {
    const run = ++confirmation.current
    const started = Date.now()
    for (const at of CONFIRM_READS_MS) {
      await new Promise<void>((resolve) => {
        timer.current = setTimeout(resolve, Math.max(0, at - (Date.now() - started)))
      })
      if (run !== confirmation.current) return
      const after = await rereadTurn()
      if (run !== confirmation.current) return
      if (sendLanded(before, after, sent)) {
        pending.current = false
        markSettled(outcome)
        return
      }
    }
    pending.current = false
    setPhase('unconfirmed')
  }

  const send = (sent: string, outcome: AskOutcome) => {
    if (!interactive || sending || pending.current) return
    const before = knownTurn.current
    try {
      host.chat?.compose?.({ text: sent, submit: true })
    } catch {
      setPhase('failed')
      return
    }
    pending.current = true
    setIntent(outcome)
    setPhase('sending')
    void confirmSend(before, sent, outcome)
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (text === null) return
    send(text, 'answered')
  }

  // Focus arriving from outside an open card re-reads the session, so a
  // card settled elsewhere closes before a second send, and a send is
  // confirmed against a fresh position.
  const onFocus = (event: FocusEvent<HTMLFormElement>) => {
    if (!interactive) return
    const from = event.relatedTarget
    if (from instanceof Node && event.currentTarget.contains(from)) return
    void recheckSettlement()
    void rereadTurn()
  }

  let note: string
  if (state === 'answered') note = sentHere ? 'Sent as your reply.' : ''
  else if (state === 'dismissed') note = 'Dismissed without answering.'
  else if (!canCompose) note = 'Type your answer in the chat to reply.'
  else if (state === 'sending')
    note = intent === 'dismissed' ? 'Dismissing…' : 'Sending your answer…'
  else if (state === 'not-sent') note = 'Not sent yet. Check the chat composer.'
  else if (state === 'failed') note = "Couldn't send. Type your answer in the chat instead."
  else if (movedOn(view, turn)) note = 'The chat moved on. You can still answer, or dismiss.'
  else note = text === null ? 'Answer or skip each question to send.' : ''

  return (
    <form
      className="harness-ui-ask"
      aria-label="Questions from the agent"
      aria-busy={sending || undefined}
      noValidate
      onSubmit={submit}
      onFocus={onFocus}
    >
      {view.questions.map((question, index) =>
        interactive ? (
          <QuestionFields
            key={`${index}:${question.header}`}
            question={question}
            selection={selections[index] ?? { picked: [] }}
            disabled={sending}
            onChange={(next) => update(index, next)}
          />
        ) : (
          <QuestionSummary
            key={`${index}:${question.header}`}
            question={question}
            selection={sentHere && state === 'answered' ? selections[index] : undefined}
          />
        ),
      )}
      <div className="harness-ui-ask-foot">
        <span id={hintId} className="harness-ui-ask-note" role="status">
          {note}
        </span>
        {state === 'answered' ? (
          <Badge variant="ok">Answered</Badge>
        ) : state === 'dismissed' ? (
          <Badge>Dismissed</Badge>
        ) : interactive ? (
          <>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={sending}
              aria-describedby={hintId}
              onClick={() => send(DISMISS_TEXT, 'dismissed')}
            >
              Dismiss
            </Button>
            <Button
              type="submit"
              variant="primary"
              size="sm"
              disabled={text === null || sending}
              aria-describedby={hintId}
            >
              Send
            </Button>
          </>
        ) : null}
      </div>
    </form>
  )
}

export function createAskRenderer(host: Host): FunctionCallRenderer {
  const render = (message: FunctionCallMessage) => {
    const view = parseAsk(message)
    return view ? <AskCard key={view.questionId} host={host} view={view} /> : null
  }
  return {
    id: 'harness/page.js#ask',
    isMatch: (functionId) => functionId === ASK_ID,
    metadata: { display: true },
    tryRender: render,
    tryRenderDisplay: render,
  }
}
