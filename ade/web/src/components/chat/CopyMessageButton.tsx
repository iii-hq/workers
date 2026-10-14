import { Check, Copy, MessageSquareQuote } from 'lucide-react'
import { useContext, useRef, useState } from 'react'
import { AskInSideChatContext } from '@/lib/chat-aside'
import { copyTextToClipboard } from '@/lib/clipboard'
import { cn } from '@/lib/utils'

const actionClassName =
  'relative flex size-12 shrink-0 cursor-pointer items-center justify-center text-ink-ghost hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent sm:size-5'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * Copy affordance for a chat message — the PaneShell copy idiom (12px
 * Copy→Check flip on success, nothing on failure) lifted to a message
 * header. Visibility is the parent's job (hover/focus opacity classes);
 * this component only copies and flips.
 */
export function CopyMessageButton({
  text,
  label = 'copy message',
  className,
}: {
  /** A thunk defers building large payloads (e.g. serialized function calls)
      until the click, keeping streaming re-renders free. */
  text: string | (() => string)
  /** Accessible name while idle (aria-label/title); flips to "copied". */
  label?: string
  className?: string
}) {
  const [copied, setCopied] = useState(false)
  const copy = () => {
    const value = typeof text === 'function' ? text() : text
    void copyTextToClipboard(value).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }
  return (
    <button
      data-message-action="copy"
      type="button"
      onClick={copy}
      className={cn(actionClassName, className)}
      aria-label={copied ? 'copied' : label}
      title={copied ? 'copied' : label}
    >
      <span
        className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
        aria-hidden="true"
      />
      {copied ? (
        <Check className="size-4 shrink-0" aria-hidden />
      ) : (
        <Copy className="size-4 shrink-0" aria-hidden />
      )}
    </button>
  )
}

/** Text selected inside the message `button` belongs to, if any. */
function selectionWithin(button: HTMLElement): string {
  const selection = window.getSelection()
  const node = selection?.anchorNode ?? null
  if (!selection || selection.isCollapsed || !node) return ''
  return button.closest('article')?.contains(node) ? selection.toString() : ''
}

/**
 * "Ask in side chat" for a message: quotes the text selected inside it, or
 * the whole message, into the chat's side chat. Renders nothing where the
 * chat has no side panel.
 */
export function AskInSideChatButton({
  text,
  messageId,
  agent,
  className,
}: {
  text: string
  /** The message's transcript row id: where the side chat's card leads back. */
  messageId: string
  agent?: string
  className?: string
}) {
  const sideChat = useContext(AskInSideChatContext)
  // Pressing the button can collapse the selection before the click lands.
  const pressedSelection = useRef('')
  if (!sideChat) return null
  return (
    <button
      data-message-action="ask-side-chat"
      type="button"
      onPointerDown={(event) => {
        pressedSelection.current = selectionWithin(event.currentTarget)
      }}
      onClick={(event) => {
        const quote =
          pressedSelection.current || selectionWithin(event.currentTarget)
        pressedSelection.current = ''
        sideChat.ask({ messageId, text: quote || text, agent })
      }}
      className={cn(actionClassName, className)}
      aria-label="Ask in side chat"
      title="Ask in side chat"
    >
      <span
        className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
        aria-hidden="true"
      />
      <MessageSquareQuote className="size-4 shrink-0" aria-hidden />
    </button>
  )
}
