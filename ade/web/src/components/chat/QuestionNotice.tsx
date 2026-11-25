import { ArrowDown, ArrowUp, MessageCircleQuestionMark } from 'lucide-react'
import { TrayButton } from '@/components/ui/TrayButton'

/** What the notice says: how many waiting question cards are out of view. */
export function questionNoticeText(count: number): string {
  return count === 1
    ? '1 question waiting for your answer'
    : `${count} questions waiting for your answer`
}

export interface QuestionNoticeState {
  count: number
  direction: 'up' | 'down'
  onShow: () => void
}

/**
 * The composer tray's row for question cards that scrolled out of view: the
 * person is looking at the composer when the agent stops to ask, so the
 * notice sits there, in flow, rather than over the transcript. The trailing
 * arrow points where the card is.
 *
 * Always mounted: the status line beside the button stays in the
 * accessibility tree, so a screen reader hears the notice appear and its
 * count change while focus stays in the composer. Only the words are
 * announced, not the control.
 */
export function QuestionNotice({
  notice,
}: {
  notice: QuestionNoticeState | null
}) {
  const text = notice ? questionNoticeText(notice.count) : ''
  const Arrow = notice?.direction === 'down' ? ArrowDown : ArrowUp
  return (
    <>
      <p role="status" aria-live="polite" className="sr-only">
        {text}
      </p>
      {notice ? (
        <TrayButton
          data-question-jump=""
          className="mb-1"
          onClick={notice.onShow}
          icon={<MessageCircleQuestionMark className="size-4" />}
          trailing={
            <>
              Show
              <Arrow aria-hidden="true" className="size-4" />
            </>
          }
        >
          {text}
        </TrayButton>
      ) : null}
    </>
  )
}
