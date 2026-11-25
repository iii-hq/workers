import {
  ArrowDown,
  ArrowUp,
  ChevronDown,
  MessageCircleQuestionMark,
} from 'lucide-react'
import { useId, useState } from 'react'
import { IconButton } from '@/components/ui/IconButton'
import { Tray, TrayButton } from '@/components/ui/TrayButton'
import type { QuestionJumps } from './question-jump'
import type { AwaitingQuestion } from './use-awaiting-questions'

/** What the notice says: how many question cards wait on the person. Short
 *  enough to stay on one line in a narrow pane. */
export function questionNoticeText(count: number): string {
  return count === 1 ? '1 unanswered question' : `${count} unanswered questions`
}

export interface QuestionNoticeState {
  /** Message id of the card `jumps.next` lands on. */
  next: string | null
  /** Which way that card is from the viewport. */
  direction: 'up' | 'down'
  jumps: QuestionJumps
}

/**
 * The composer tray's notice for question cards the person has not
 * answered, shown while any of them is scrolled out of view. The person is
 * looking at the composer when the agent stops to ask, so it sits there, in
 * flow, rather than over the transcript.
 *
 * The row walks the cards in order: each click lands on the next one,
 * wrapping after the last. With more than one, the chevron opens the list of
 * what each card asks, so any of them is one click away.
 *
 * Always mounted: the status line stays in the accessibility tree, so a
 * screen reader hears the notice appear and its count change while focus
 * stays in the composer. Only the words are announced, not the controls.
 */
export function QuestionNotice({
  questions,
  notice,
}: {
  questions: readonly AwaitingQuestion[]
  notice: QuestionNoticeState | null
}) {
  const [listOpen, setListOpen] = useState(false)
  const listId = useId()
  const count = questions.length
  const text = notice && count > 0 ? questionNoticeText(count) : ''
  const Arrow = notice?.direction === 'down' ? ArrowDown : ArrowUp
  const several = count > 1
  const showList = several && listOpen
  return (
    <>
      <p role="status" aria-live="polite" className="sr-only">
        {text}
      </p>
      {notice && count > 0 ? (
        <Tray className="mb-1" data-question-notice="">
          <div className="flex min-w-0 items-center pr-1">
            <TrayButton
              data-question-jump=""
              className="flex-1"
              onClick={notice.jumps.next}
              icon={<MessageCircleQuestionMark className="size-4" />}
              trailing={
                <>
                  {several ? 'Next' : 'Show'}
                  <Arrow aria-hidden="true" className="size-4" />
                </>
              }
            >
              {text}
            </TrayButton>
            {several ? (
              <IconButton
                label={showList ? 'Hide the questions' : 'List the questions'}
                aria-expanded={showList}
                aria-controls={showList ? listId : undefined}
                onClick={() => setListOpen((open) => !open)}
              >
                <ChevronDown
                  aria-hidden="true"
                  className="size-4 transition-transform duration-(--motion-duration-control) data-[open]:rotate-180"
                  data-open={showList ? '' : undefined}
                />
              </IconButton>
            ) : null}
          </div>
          {showList ? (
            <ul
              id={listId}
              aria-label="Unanswered questions"
              className="max-h-48 overflow-y-auto border-t border-border"
              onKeyDown={(event) => {
                if (event.key !== 'Escape') return
                event.stopPropagation()
                setListOpen(false)
              }}
            >
              {questions.map((question, index) => (
                <li key={question.messageId}>
                  <TrayButton
                    data-question-jump-to={question.messageId}
                    onClick={() => notice.jumps.jumpTo(question.messageId)}
                    icon={
                      <span className="w-4 text-center tabular-nums">
                        {index + 1}
                      </span>
                    }
                    trailing={
                      question.questionCount > 1
                        ? `+${question.questionCount - 1} more`
                        : undefined
                    }
                  >
                    <span className="line-clamp-1">
                      {question.header ? (
                        <span className="text-ink-faint">
                          {question.header} ·{' '}
                        </span>
                      ) : null}
                      {question.title}
                    </span>
                  </TrayButton>
                </li>
              ))}
            </ul>
          ) : null}
        </Tray>
      ) : null}
    </>
  )
}
