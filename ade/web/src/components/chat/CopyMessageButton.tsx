import { Copy, Download, MessageSquareQuote } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { forwardRef, useContext, useRef, useState } from 'react'
import { Hint } from '@/components/ui/Hint'
import { AskInSideChatContext } from '@/lib/chat-aside'
import { copyTextToClipboard } from '@/lib/clipboard'
import {
  messagesToMarkdown,
  triggerMarkdownDownload,
} from '@/lib/export-session'
import { cn } from '@/lib/utils'
import type { AssistantMessage } from '@/types/chat'
import { DrawnCheck } from './ChatGlyphs'

const actionClassName =
  'chat-message-action relative flex size-12 shrink-0 cursor-pointer items-center justify-center text-ink-ghost hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent sm:size-5'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * Copy affordance for a chat message — the PaneShell copy idiom (12px
 * Copy→Check flip on success, nothing on failure) lifted to a message
 * header. Visibility is the parent's job (hover/focus opacity classes);
 * this component only copies and flips.
 */
export const CopyMessageButton = forwardRef<
  HTMLButtonElement,
  {
    /** A thunk defers building large payloads (e.g. serialized function calls)
        until the click, keeping streaming re-renders free. */
    text: string | (() => string)
    /** Accessible name while idle; flips to "copied". The caller wraps
        the button in a `Hint` for the hover label. */
    label?: string
    className?: string
  }
>(function CopyMessageButton({ text, label = 'copy message', className }, ref) {
  const [copied, setCopied] = useState(false)
  const reduceMotion = useReducedMotion()
  const copy = () => {
    const value = typeof text === 'function' ? text() : text
    void copyTextToClipboard(value).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    })
  }
  return (
    <motion.button
      ref={ref}
      whileTap={reduceMotion ? undefined : { scale: 0.9 }}
      data-message-action="copy"
      type="button"
      onClick={copy}
      className={cn(actionClassName, className)}
      aria-label={copied ? 'copied' : label}
    >
      <span
        className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
        aria-hidden="true"
      />
      {copied ? (
        <DrawnCheck className="size-4 shrink-0 text-ok" drawOnMount />
      ) : (
        <Copy className="size-4 shrink-0" aria-hidden />
      )}
    </motion.button>
  )
})

/** Text selected inside the message `button` belongs to, if any. */
function selectionWithin(button: HTMLElement): string {
  const selection = window.getSelection()
  const article = button.closest('article')
  if (!selection || selection.isCollapsed || !article) return ''
  // Both ends: a selection running on into the next message is not this one's.
  const inside = (node: Node | null) => node !== null && article.contains(node)
  return inside(selection.anchorNode) && inside(selection.focusNode)
    ? selection.toString()
    : ''
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
    <Hint label="Ask in side chat" side="top">
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
      >
        <span
          className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
          aria-hidden="true"
        />
        <MessageSquareQuote className="size-4 shrink-0" aria-hidden />
      </button>
    </Hint>
  )
}

/** Save one reply as a Markdown file, named after its id and the moment. */
export function downloadReply(message: AssistantMessage): void {
  const stamp = new Date(message.createdAt || Date.now())
    .toISOString()
    .slice(0, 16)
    .replace(/[:T]/g, '-')
  triggerMarkdownDownload(
    messagesToMarkdown([message]),
    `iii-reply-${message.id.slice(0, 8)}-${stamp}.md`,
  )
}

export const DownloadMessageButton = forwardRef<
  HTMLButtonElement,
  { message: AssistantMessage; className?: string }
>(function DownloadMessageButton({ message, className }, ref) {
  const reduceMotion = useReducedMotion()
  return (
    <Hint label="Download as Markdown" side="top">
      <motion.button
        ref={ref}
        whileTap={reduceMotion ? undefined : { scale: 0.9 }}
        data-message-action="download"
        type="button"
        onClick={() => downloadReply(message)}
        className={cn(actionClassName, className)}
        aria-label="Download reply as Markdown"
      >
        <span
          className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
          aria-hidden="true"
        />
        <Download className="size-4 shrink-0" aria-hidden />
      </motion.button>
    </Hint>
  )
})
