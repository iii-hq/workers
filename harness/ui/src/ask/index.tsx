/**
 * Injected function-trigger renderer for `harness::ask`, registered through
 * `host.functionTriggers` with `metadata.display: true`, so the card stays
 * visible in the chat feed while the raw call details stay collapsed.
 *
 * The card draws the questions from the result `details` (`parseAsk`): per
 * question a fieldset with the header chip and the question, radio buttons
 * (single-select) or checkboxes (multi-select) with the description under
 * each label, and a single-line "Other" field. One Send button hands the
 * formatted answer to the composer (`host.chat.compose`, submitted), which
 * starts the next turn.
 *
 * Open vs answered: the card reads the session's current turn from
 * `harness::status` on mount, when the tab becomes visible again, and when
 * focus enters an open card (so an answer typed in the composer closes it
 * before a second one is sent). There is no interval polling. A session with
 * no turn record (the reply is `null`: the record expired) counts as
 * answered; an unknown turn (not read yet, or a failed call) counts as open
 * (see `turnIdOf` and `isOpen` in lib/ask).
 *
 * Anything `parseAsk` rejects (a refused ask, a running call) returns null
 * and falls through to the console's default card.
 */

import {
  Badge,
  Button,
  Checkbox,
  Chip,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
  Input,
  uiClasses,
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
  type AskQuestionView,
  type AskSelection,
  type AskView,
  type CurrentTurn,
  formatAnswer,
  isOpen,
  parseAsk,
  turnIdOf,
} from '../lib/ask'

const ASK_ID = 'harness::ask'
const STATUS_TIMEOUT_MS = 5000

/**
 * The session's current turn id and a function to re-read it. Re-reads when
 * the tab becomes visible again; a newer read always wins over an older one
 * still in flight, and nothing lands after unmount.
 */
function useCurrentTurnId(host: Host, sessionId: string) {
  const [turnId, setTurnId] = useState<CurrentTurn>(undefined)
  const latest = useRef(0)

  const check = useCallback(() => {
    const request = ++latest.current
    host.iii
      .trigger('harness::status', { session_id: sessionId }, { timeoutMs: STATUS_TIMEOUT_MS })
      .then(turnIdOf, () => undefined)
      .then((next) => {
        if (request === latest.current) setTurnId(next)
      })
  }, [host, sessionId])

  useEffect(() => {
    check()
    const onVisibility = () => {
      if (document.visibilityState === 'visible') check()
    }
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      document.removeEventListener('visibilitychange', onVisibility)
      latest.current += 1
    }
  }, [check])

  return [turnId, check] as const
}

function RadioChoice({
  name,
  label,
  checked,
  describedBy,
  onPick,
}: {
  name: string
  label: string
  checked: boolean
  describedBy?: string
  onPick: () => void
}) {
  // The shared checkbox recipe with a native radio inside: same box, focus
  // ring and checked fill; the scoped sheet rounds it and draws the dot.
  return (
    <label className={`${uiClasses.checkbox} harness-ui-ask-choice harness-ui-ask-radio`}>
      <span className={uiClasses.checkboxControl}>
        <input
          type="radio"
          name={name}
          className={uiClasses.checkboxInput}
          checked={checked}
          aria-describedby={describedBy}
          onChange={(event) => {
            if (event.currentTarget.checked) onPick()
          }}
        />
        <span aria-hidden className="harness-ui-ask-dot" />
      </span>
      <span className={uiClasses.checkboxLabel}>{label}</span>
    </label>
  )
}

function QuestionFields({
  question,
  selection,
  onChange,
}: {
  question: AskQuestionView
  selection: AskSelection
  onChange: (next: AskSelection) => void
}) {
  const baseId = useId()
  const otherId = `${baseId}-other`

  const toggle = (label: string, checked: boolean) => {
    const picked = checked
      ? [...selection.picked, label]
      : selection.picked.filter((picked) => picked !== label)
    onChange({ ...selection, picked })
  }
  // Single-select is a radio group: picking an option clears Other, and
  // typing in Other clears the pick.
  const pickOnly = (label: string) => onChange({ picked: [label], other: '' })
  const setOther = (other: string) =>
    onChange(question.multiSelect ? { ...selection, other } : { picked: [], other })

  return (
    <fieldset className="harness-ui-ask-q">
      <legend className="harness-ui-ask-legend">
        <Chip>{question.header}</Chip>
        <span className="harness-ui-ask-question">{question.question}</span>
      </legend>
      {question.multiSelect ? (
        <p className="harness-ui-ask-note">Choose any that apply.</p>
      ) : null}
      <div className="harness-ui-ask-options">
        {question.options.map((option, index) => {
          const checked = selection.picked.includes(option.label)
          const descriptionId = option.description ? `${baseId}-d${index}` : undefined
          return (
            <div key={option.label} className="harness-ui-ask-option">
              {question.multiSelect ? (
                <Checkbox
                  className="harness-ui-ask-choice"
                  label={option.label}
                  checked={checked}
                  aria-describedby={descriptionId}
                  onChange={(event) => toggle(option.label, event.currentTarget.checked)}
                />
              ) : (
                <RadioChoice
                  name={`${baseId}-choice`}
                  label={option.label}
                  checked={checked}
                  describedBy={descriptionId}
                  onPick={() => pickOnly(option.label)}
                />
              )}
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
  const other = selection?.other?.trim()
  return (
    <div className="harness-ui-ask-q" role="group" aria-labelledby={headingId}>
      <div id={headingId} className="harness-ui-ask-legend">
        <Chip>{question.header}</Chip>
        <span className="harness-ui-ask-question">{question.question}</span>
      </div>
      <ul className="harness-ui-ask-summary">
        {question.options.map((option) => {
          // On a single-select, Other text replaces the pick.
          const counts = question.multiSelect || !other
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
  const [currentTurnId, recheck] = useCurrentTurnId(host, view.sessionId)
  const [selections, setSelections] = useState<AskSelection[]>(() =>
    view.questions.map(() => ({ picked: [] })),
  )
  const [sent, setSent] = useState<string | null>(null)
  const [sendFailed, setSendFailed] = useState(false)
  const hintId = useId()

  const canCompose = typeof host.chat?.compose === 'function'
  const answered = sent !== null || !isOpen(view, currentTurnId)
  const interactive = canCompose && !answered
  const text = formatAnswer(view, selections)

  const update = (index: number, next: AskSelection) =>
    setSelections((previous) => previous.map((entry, i) => (i === index ? next : entry)))

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!interactive || text === null) return
    try {
      host.chat?.compose?.({ text, submit: true })
      setSent(text)
      setSendFailed(false)
    } catch {
      setSendFailed(true)
    }
  }

  // Focus arriving from outside an open card re-reads the turn, so a
  // question already answered in the composer closes before a second send.
  const onFocus = (event: FocusEvent<HTMLFormElement>) => {
    if (!interactive) return
    const from = event.relatedTarget
    if (from instanceof Node && event.currentTarget.contains(from)) return
    recheck()
  }

  let note: string
  if (answered) note = sent !== null ? 'Sent as your reply.' : ''
  else if (!canCompose) note = 'Type your answer in the chat to reply.'
  else if (sendFailed) note = "Couldn't send. Type your answer in the chat instead."
  else note = text === null ? 'Answer each question to send.' : ''

  return (
    <form
      className="harness-ui-ask"
      aria-label="Questions from the agent"
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
            onChange={(next) => update(index, next)}
          />
        ) : (
          <QuestionSummary
            key={`${index}:${question.header}`}
            question={question}
            selection={sent !== null ? selections[index] : undefined}
          />
        ),
      )}
      <div className="harness-ui-ask-foot">
        <span id={hintId} className="harness-ui-ask-note" role="status">
          {note}
        </span>
        {answered ? (
          <Badge variant="ok">Answered</Badge>
        ) : interactive ? (
          <Button
            type="submit"
            variant="primary"
            size="sm"
            disabled={text === null}
            aria-describedby={hintId}
          >
            Send
          </Button>
        ) : null}
      </div>
    </form>
  )
}

export function createAskRenderer(host: Host): FunctionTriggerRenderer {
  const render = (message: FunctionTriggerMessage) => {
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
