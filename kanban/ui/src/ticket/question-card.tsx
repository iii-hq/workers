import { Button, Card, CardBody, CardHeader, type Host, Input, StatusDot } from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useEffect, useId, useRef, useState } from 'react'
import { mayDismiss, pickOption, questionAnswers, textSelection, type Question, type QuestionAnswer, type StructuredAnswer } from '../question-rules'
import { api, type AgentProfile, type Comment } from '../shared'
import { AssigneeAvatar, assigneeName } from '../ui/avatar'
import { Notice } from '../ui/feedback'

type Attempt = { answers: StructuredAnswer[] } | { dismissed: true }

export function QuestionCard({
  host,
  ticketId,
  comment,
  question,
  reply,
  agents,
  disabled,
  onRefresh,
  onDraft,
}: {
  host: Host
  ticketId: string
  comment: Comment
  question: Question
  reply?: Comment
  agents: AgentProfile[]
  disabled: boolean
  onRefresh: () => void
  onDraft: (id: string, dirty: boolean) => void
}) {
  const uid = useId()
  const [selections, setSelections] = useState(() => question.questions.map((item) => textSelection(item)))
  const [phase, setPhase] = useState<'idle' | 'sending' | 'failed'>('idle')
  const [attempt, setAttempt] = useState<Attempt | null>(null)
  const [result, setResult] = useState<QuestionAnswer | null>(null)
  const [failure, setFailure] = useState('')
  const [conflict, setConflict] = useState('')
  const sending = useRef(false)
  const answer = reply?.answer ?? result
  const closed = Boolean(answer || conflict)
  const answers = questionAnswers(question.questions, selections)
  const dirty = !closed && selections.some((selection, index) =>
    selection.other !== '' || JSON.stringify(selection.picked) !== JSON.stringify(textSelection(question.questions[index]!).picked),
  )
  useEffect(() => {
    onDraft(comment.id, dirty)
    return () => onDraft(comment.id, false)
  }, [comment.id, dirty, onDraft])

  const send = async (next: Attempt) => {
    if (sending.current || closed || disabled) return
    sending.current = true
    setAttempt(next)
    setPhase('sending')
    setFailure('')
    try {
      const delivered = await api.answerQuestion(host.iii, {
        ticket_id: ticketId,
        comment_id: comment.id,
        user: 'user',
        ...next,
      })
      setResult(delivered)
      setPhase('idle')
      onRefresh()
    } catch (caught) {
      const message = errorMessage(caught)
      const answered = message.match(/already answered(?: by ([\w-]+))?/i)
      if (answered || /withdrawn by the asker/i.test(message)) {
        setConflict(answered ? `Already answered${answered[1] ? ` by ${answered[1]}` : ''}` : 'Withdrawn by the asker')
        setPhase('idle')
        onRefresh()
      } else {
        setFailure(message)
        setPhase('failed')
      }
    } finally {
      sending.current = false
    }
  }

  const author = assigneeName(comment.recorded_by ?? comment.author, agents)
  const askedAt = new Date(question.asked_at).toISOString()
  const sentText = attempt && ('dismissed' in attempt ? 'Let the agent decide' : attempt.answers.map((item) =>
    `${item.header}: ${'other' in item ? item.other : 'choice' in item ? item.choice : item.choices.join(', ')}`,
  ).join('\n'))
  const outcomeStatus = answer?.outcome === 'not_delivered' ? 'Not delivered · the session is gone'
    : answer?.outcome === 'withdrawn' ? 'Withdrawn by the asker' : 'Delivered'

  return (
    <Card aria-label={`Question from ${author}`} className="kb-question">
      <CardHeader className="kb-question__head">
        <AssigneeAvatar agents={agents} id={comment.recorded_by ?? comment.author} />
        <strong className="kb-question__author" title={question.session_id}>{author} asks</strong>
        <span className="kb-question__meta">question · {question.about?.class ?? 'scope'}</span>
        <time className="kb-question__meta" dateTime={askedAt} title={new Date(question.asked_at).toLocaleString()}>
          {formatRelative(askedAt)}
        </time>
        {question.about?.blocks.length ? <span className="kb-question__meta">blocks {question.about.blocks.length}</span> : null}
      </CardHeader>
      <CardBody className="kb-question__body">
        {closed ? (
          <>
            <p className="kb-question__prompt">{question.questions.map((item) => item.question).join(' · ')}</p>
            {answer ? (
              <>
                <p className="kb-question__answer">{answer.answer_text}</p>
                <p className="kb-question__by">
                  {answer.by ? `${answer.by.user ?? answer.by.channel} · ${answer.by.channel}` : author}
                  {' · '}<span className="kb-question__meta">{answer.outcome}</span>
                </p>
              </>
            ) : null}
            <div className="kb-question__status" role="status">
              <StatusDot tone={answer?.outcome === 'not_delivered' ? 'alert' : 'ink'} />
              {conflict || outcomeStatus}
            </div>
          </>
        ) : phase === 'sending' || phase === 'failed' ? (
          <>
            <p className="kb-question__answer">{sentText}</p>
            {phase === 'sending' ? (
              <div className="kb-question__status" role="status"><StatusDot tone="ink" />Sending…</div>
            ) : (
              <Notice actionLabel="Retry" detail={failure} onAction={() => attempt && void send(attempt)} title="Failed to send" />
            )}
          </>
        ) : (
          <>
            {question.questions.map((item, index) => {
              const selection = selections[index]!
              return (
                <fieldset className="kb-question__fieldset" disabled={disabled} key={index}>
                  <legend className="kb-question__prompt" id={`${uid}-question-${index}`}>{item.question}</legend>
                  {index === 0 && question.about?.why ? <p className="kb-question__why">{question.about.why}</p> : null}
                  <div className="kb-question__options">
                    {item.options.map((option) => (
                      <label className="kb-question__option" data-selected={selection.picked.includes(option.label) || undefined} key={option.label}>
                        <input
                          checked={selection.picked.includes(option.label)}
                          name={`${uid}-${index}`}
                          onChange={() => setSelections((current) => current.map((value, at) =>
                            at === index ? pickOption(item, value, option.label) : value,
                          ))}
                          type={item.multi_select ? 'checkbox' : 'radio'}
                        />
                        <span className="kb-question__option-copy">
                          <span className="kb-question__option-label">{option.label}
                            {option.recommended === true ? <span className="kb-question__recommended">Recommended</span> : null}
                          </span>
                          {option.description ? <span className="kb-question__why">{option.description}</span> : null}
                        </span>
                      </label>
                    ))}
                  </div>
                  <label className="kb-question__why" htmlFor={`${uid}-other-${index}`}>Or answer in your own words</label>
                  <Input
                    aria-describedby={`${uid}-question-${index}`}
                    disabled={disabled}
                    id={`${uid}-other-${index}`}
                    onChange={(text) => setSelections((current) => current.map((value, at) =>
                      at === index ? textSelection(item, text) : value,
                    ))}
                    placeholder="Type a different answer"
                    value={selection.other}
                  />
                </fieldset>
              )
            })}
            <div className="kb-question__actions">
              <Button disabled={disabled || !answers} onClick={() => answers && void send({ answers })} type="button" variant="primary">Answer</Button>
              {mayDismiss(question) ? <Button disabled={disabled} onClick={() => void send({ dismissed: true })} type="button" variant="ghost">Let the agent decide</Button> : null}
            </div>
          </>
        )}
      </CardBody>
    </Card>
  )
}
