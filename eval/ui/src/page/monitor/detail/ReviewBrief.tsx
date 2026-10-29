// The handoff to whoever implements a suggestion: the brief is copied, or
// opened as an unsent draft in a chat. Built from stored fields in the
// browser (`briefMarkdown`), never summarized by a model.
import { Button, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, CircleCheck, Copy, MessageSquare } from 'lucide-react'

export function CopyBriefButton({ brief, narrow }: { brief: string; narrow: boolean }) {
  const { state, copy } = useCopyFlash(brief)
  const Icon = state === 'copied' ? Check : Copy
  return (
    <Button variant="ghost" size={narrow ? 'lg' : 'sm'} onClick={copy}>
      <Icon size={16} aria-hidden="true" />
      <span aria-live="polite">
        {state === 'copied' ? 'Copied' : state === 'failed' ? 'Copy failed' : 'Copy implementation brief'}
      </span>
    </Button>
  )
}

/** Hidden by the caller when the host cannot open a draft; disabled when the analysis ran without a code directory. */
export function DraftButton({
  disabled,
  narrow,
  onClick,
}: {
  disabled: boolean
  narrow: boolean
  onClick: () => void
}) {
  return (
    <Button variant="pill" size={narrow ? 'lg' : 'sm'} disabled={disabled} onClick={onClick}>
      <MessageSquare size={16} aria-hidden="true" />
      Draft in chat
    </Button>
  )
}

/** What the draft did, or why it is off. */
export function DraftNote({ drafted, codeRoot }: { drafted: boolean; codeRoot: string | undefined }) {
  if (!codeRoot) {
    return (
      <p className="eval-ui-ad-quiet eval-ui-rv-note">
        Draft in chat is off: this analysis ran without code access, so there is no directory to open the chat in. The
        brief can still be copied.
      </p>
    )
  }
  if (!drafted) return null
  return (
    <StatusPanel
      variant="success"
      role="status"
      icon={<CircleCheck className={uiClasses.icon} aria-hidden />}
      headline="Draft ready in a new chat"
      detail={
        <>
          The brief is in the composer; nothing was sent. The code it names is in{' '}
          <span className="eval-ui-val-mono">{codeRoot}</span>.
        </>
      }
    />
  )
}
