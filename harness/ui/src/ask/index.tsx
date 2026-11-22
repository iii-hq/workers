/** Durable asks use the harness reservation; only absent entries use the legacy composer. */

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
import { errorMessage } from '@iii-dev/console-ui/format'
import { Check } from 'lucide-react'
import { type FocusEvent, type FormEvent, useCallback, useEffect, useId, useRef, useState } from 'react'
import {
  askCardState,
  type AskEntry,
  mayDismiss,
  readAskEntry,
  recommendedSelection,
  structuredAnswers,
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
  watchAskSession,
} from '../lib/ask'

const ASK_ID = 'harness::ask'
const STATUS_ID = 'harness::status'
const SETTLEMENTS_ID = 'harness::ask::settlements'
const SETTLE_ID = 'harness::ask::settle'
const CALL_TIMEOUT_MS = 5000

/** Session re-reads after a Send, in ms after it: bounded, not polling. */
const CONFIRM_READS_MS = [500, 1500, 3000] as const

function useSettlement(host: Host, sessionId: string, questionId: string) {
  const [settlement, setSettlement] = useState<AskOutcome | undefined>(undefined)
  const [entry, setEntry] = useState<AskEntry | null | undefined>(undefined)
  const [readFailed, setReadFailed] = useState(false)
  const latest = useRef(0)

  const read = useCallback(async () => {
    const request = ++latest.current
    try {
      const next = await readAskEntry(host.iii, sessionId, questionId)
      const legacy =
        next === null
          ? settlementsOf(
              await host.iii.call(SETTLEMENTS_ID, { session_id: sessionId }, { timeoutMs: CALL_TIMEOUT_MS }),
            )
          : undefined
      if (request !== latest.current) return
      setEntry((previous) => next ?? previous ?? null)
      setReadFailed(false)
      if (legacy?.has(questionId)) setSettlement(legacy.get(questionId))
    } catch {
      if (request === latest.current) setReadFailed(true)
    }
  }, [host, sessionId, questionId])

  const acceptEntry = useCallback((next: AskEntry) => {
    latest.current += 1
    setEntry(next)
    setReadFailed(false)
  }, [])

  const markSettled = useCallback(
    (outcome: AskOutcome) => {
      setSettlement(outcome)
      host.iii
        .call(SETTLE_ID, { session_id: sessionId, question_id: questionId, outcome }, { timeoutMs: CALL_TIMEOUT_MS })
        .catch(() => undefined)
    },
    [host, sessionId, questionId],
  )

  useEffect(() => {
    void read()
    const onVisibility = () => {
      if (document.visibilityState === 'visible') void read()
    }
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      latest.current += 1
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [read])

  return { settlement, entry, readFailed, read, markSettled, acceptEntry }
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
    const picked = checked ? [...selection.picked, label] : selection.picked.filter((picked) => picked !== label)
    onChange({ ...selection, picked })
  }
  // Single-select ticks one box at a time: ticking an option clears the
  // others and Other, unticking it leaves nothing picked, and typing in
  // Other clears the pick.
  const pickOnly = (label: string, checked: boolean) =>
    onChange(checked ? { picked: [label], other: '' } : { ...selection, picked: [] })
  const setOther = (other: string) =>
    onChange(
      question.multiSelect
        ? { ...selection, other }
        : { picked: other ? [] : recommendedSelection(question).picked, other },
    )

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
                label={`${option.label}${option.recommended ? ' (Recommended)' : ''}`}
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
function QuestionSummary({ question, selection }: { question: AskQuestionView; selection?: AskSelection }) {
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
            <li key={option.label} className="harness-ui-ask-summary-row" data-chosen={chosen || undefined}>
              <span className="harness-ui-ask-mark">{chosen ? <Check size={16} aria-hidden /> : null}</span>
              <span className="harness-ui-ask-summary-text">
                <span>
                  {chosen ? <span className="harness-ui-ask-sr">Chosen: </span> : null}
                  {option.label}
                </span>
                {option.description ? <span className="harness-ui-ask-desc">{option.description}</span> : null}
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
  const {
    settlement,
    entry,
    readFailed,
    read: recheckSettlement,
    markSettled,
    acceptEntry,
  } = useSettlement(host, view.sessionId, view.questionId)
  const [turn, knownTurn, rereadTurn] = useTurnPosition(host, view.sessionId)
  const [selections, setSelections] = useState<AskSelection[]>(() => view.questions.map(recommendedSelection))
  const [phase, setPhase] = useState<SendPhase>('idle')
  const [intent, setIntent] = useState<AskOutcome>('answered')
  const [sendError, setSendError] = useState('')
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
  const state = askCardState(entry === null ? settlement : undefined, phase, entry)
  const settled = state === 'answered' || state === 'dismissed' || state === 'withdrawn'
  const sending = state === 'sending'
  const interactive = !settled && (entry !== null || canCompose)
  const loading = entry === undefined
  const about = entry?.about ?? view.about
  const allowDismiss = mayDismiss({ ...view, about })
  // This card's own send is what settled it, so the summary can show what
  // was chosen.
  const sentHere = settled && phase === 'sending'
  const text = formatAnswer(view, selections)
  const answeredSelections =
    entry?.answers?.map((answer) => ({
      picked: answer.choices ?? (answer.choice ? [answer.choice] : []),
      other: answer.other,
      skipped: answer.skipped,
    })) ?? (sentHere ? selections : undefined)

  // While the card is open, every change to its session re-reads it: a
  // settlement written anywhere (another tab answered) closes the card, and
  // a turn starting or ending shows the "moved on" note as it happens. One
  // scope per mounted card, so the same card in two panes keeps two handlers.
  const [watchScope] = useState(() => `${view.questionId}-${Math.random().toString(36).slice(2, 10)}`)
  useEffect(() => {
    if (settled) return
    return watchAskSession(host.iii, view.sessionId, watchScope, (change) => {
      void recheckSettlement()
      if (change === 'turn') void rereadTurn()
    })
  }, [host, view.sessionId, watchScope, settled, recheckSettlement, rereadTurn])

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

  const send = async (sent: string, outcome: AskOutcome) => {
    if (!interactive || sending || pending.current || loading) return
    pending.current = true
    setIntent(outcome)
    setSendError('')
    setPhase('sending')
    try {
      // Re-read before sending: a stale tab must never choose the legacy path.
      const current = await readAskEntry(host.iii, view.sessionId, view.questionId)
      if (current !== null || entry) {
        if (current !== null) acceptEntry(current)
        const answered = await host.iii.call('harness::ask::answer', {
          session_id: view.sessionId,
          question_id: view.questionId,
          ...(outcome === 'dismissed' ? { dismissed: true } : { answers: structuredAnswers(view, selections) }),
          by: { channel: 'chat' },
        })
        acceptEntry(answered as AskEntry)
        pending.current = false
        return
      }
      const before = knownTurn.current
      const result = host.chat?.compose?.({ text: sent, submit: true, sessionId: view.sessionId })
      if (!canCompose || result === 'unavailable' || result === 'ambiguous') {
        throw new Error('The chat composer is unavailable.')
      }
      void confirmSend(before, sent, outcome)
    } catch (error) {
      pending.current = false
      setPhase('failed')
      setSendError(errorMessage(error))
      void recheckSettlement()
    }
  }

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (text === null) return
    void send(text, 'answered')
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
  if (state === 'withdrawn') note = 'Withdrawn by the asker.'
  else if (state === 'answered')
    note = entry?.by ? `Answered by ${entry.by.user ?? entry.by.channel}.` : sentHere ? 'Sent as your reply.' : ''
  else if (state === 'dismissed') note = 'Dismissed without answering.'
  else if (loading) note = readFailed ? 'Could not load this question. Retry to answer.' : 'Loading question…'
  else if (entry === null && !canCompose) note = 'Type your answer in the chat to reply.'
  else if (state === 'sending') note = intent === 'dismissed' ? 'Dismissing…' : 'Sending your answer…'
  else if (state === 'not-sent') note = 'Not sent yet. Check the chat composer.'
  else if (state === 'failed') note = sendError || 'Could not send. Try again.'
  else if (entry?.state === 'expired') note = 'The deadline passed. You can still answer.'
  else if (movedOn(view, turn)) note = 'The chat moved on. Your answer still arrives as a message.'
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
      {about?.class ? <p className="harness-ui-ask-note">Question · {about.class.replaceAll('_', ' ')}</p> : null}
      {about?.why ? <p className="harness-ui-ask-note">{about.why}</p> : null}
      {about?.ticket || about?.blocks?.length ? (
        <p className="harness-ui-ask-note">
          {about.ticket ? `Card: ${about.ticket}. ` : ''}
          {about.blocks?.length ? `Blocks: ${about.blocks.join(', ')}.` : ''}
        </p>
      ) : null}
      {view.questions.map((question, index) =>
        interactive ? (
          <QuestionFields
            key={`${index}:${question.header}`}
            question={question}
            selection={selections[index] ?? { picked: [] }}
            disabled={sending || loading}
            onChange={(next) => update(index, next)}
          />
        ) : (
          <QuestionSummary
            key={`${index}:${question.header}`}
            question={question}
            selection={state === 'answered' ? answeredSelections?.[index] : undefined}
          />
        ),
      )}
      <div className="harness-ui-ask-foot">
        <span id={hintId} className="harness-ui-ask-note" role="status">
          {note}
        </span>
        {state === 'answered' ? (
          <Badge variant="ok">Answered</Badge>
        ) : state === 'dismissed' || state === 'withdrawn' ? (
          <Badge>{state === 'withdrawn' ? 'Withdrawn' : 'Dismissed'}</Badge>
        ) : interactive ? (
          <>
            {readFailed ? (
              <Button type="button" variant="secondary" size="sm" onClick={() => void recheckSettlement()}>
                Retry
              </Button>
            ) : null}
            {allowDismiss ? (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                disabled={sending || loading}
                aria-describedby={hintId}
                onClick={() => void send(DISMISS_TEXT, 'dismissed')}
              >
                Let the agent decide
              </Button>
            ) : null}
            <Button
              type="submit"
              variant="primary"
              size="sm"
              disabled={text === null || sending || loading}
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
    return view ? <AskCard key={`${view.sessionId}/${view.questionId}`} host={host} view={view} /> : null
  }
  return {
    id: 'harness/page.js#ask',
    isMatch: (functionId) => functionId === ASK_ID,
    metadata: { display: true },
    tryRender: render,
    tryRenderDisplay: render,
  }
}
