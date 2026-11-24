import { ArrowDown, ArrowUp, MessageCircleQuestionMark } from 'lucide-react'

/** What the notice says: how many waiting question cards are out of view. */
export function questionNoticeText(count: number): string {
  return count === 1
    ? '1 question waiting for your answer'
    : `${count} questions waiting for your answer`
}

/**
 * A row in the composer's tray (the queued-message strip's look) for
 * question cards that scrolled out of view: the person is looking at the
 * composer when the agent stops to ask, so the notice sits there, in flow,
 * instead of floating over the transcript. The whole row is the control;
 * the trailing arrow points where the card is.
 */
export function QuestionNotice({
  count,
  direction,
  onShow,
}: {
  count: number
  direction: 'up' | 'down'
  onShow: () => void
}) {
  const Arrow = direction === 'down' ? ArrowDown : ArrowUp
  return (
    <button
      type="button"
      data-question-jump=""
      onClick={onShow}
      className="mb-1 flex min-h-11 w-full items-center gap-2 rounded-xl bg-surface px-3 py-2 text-start text-base text-ink transition-colors duration-(--motion-duration-control) ease-(--motion-ease-standard) hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none @2xl:min-h-9 @2xl:py-1.5 @2xl:text-sm"
    >
      <MessageCircleQuestionMark
        aria-hidden="true"
        className="size-4 shrink-0 text-ink-faint"
      />
      <span className="min-w-0 flex-1">{questionNoticeText(count)}</span>
      <span className="flex shrink-0 items-center gap-1 text-ink-faint">
        Show
        <Arrow aria-hidden="true" className="size-4" />
      </span>
    </button>
  )
}
