import {
  BookmarkPlus,
  Check,
  Clock,
  Copy,
  Layers,
  MessageSquareQuote,
  MessagesSquare,
  Trash2,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import { Button } from '@/components/ui/Button'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { EmptyState } from '@/components/ui/EmptyState'
import { Eyebrow } from '@/components/ui/Eyebrow'
import { IconButton } from '@/components/ui/IconButton'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import { PageSidebar } from '@/components/ui/PageChrome'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import {
  DISMISSED_PRS_KEY,
  dismissedPullRequests,
  findSideChat,
  lastSpokenEntry,
  SIDE_CHAT_KEY,
  SIDE_CHAT_QUOTES_KEY,
  type SideChatQuote,
  sideChatFrom,
  sideChatMessages,
  sideChatMetadata,
  sideChatQuotes,
  sideChatsMadeHere,
  withSideChatQuote,
} from '@/lib/chat-aside'
import { copyTextToClipboard } from '@/lib/clipboard'
import { useConversationsCtx } from '@/lib/conversations-context'
import { errText } from '@/lib/errors'
import { getIiiClient } from '@/lib/iii-client'
import { deleteSession, setSessionMeta } from '@/lib/sessions/api'
import type { SessionMeta, TranscriptItem } from '@/lib/sessions/types'
import type { Conversation } from '@/types/chat'
import { ChatContextPanel } from './ChatContextPanel'
import { ChatView } from './ChatView'

export type AsideTab = 'context' | 'side-chat'

/** An outside ask: show a tab, and for the side chat maybe seed a quote. */
export interface AsideRequest {
  seq: number
  tab: AsideTab
  /** A quote just handed over: the composer takes the caret to ask about it. */
  quote?: SideChatQuote
}

const TAB_STORAGE_KEY = 'iii-chat-aside-tab'

/** When a side chat ends on its own (ChatPanel runs the rules). */
const SIDE_CHAT_LIFETIME =
  'Deleted after 2 hours unused, when every ADE tab closes, or with this chat.'

function readStoredTab(): AsideTab {
  try {
    return window.localStorage.getItem(TAB_STORAGE_KEY) === 'side-chat'
      ? 'side-chat'
      : 'context'
  } catch {
    return 'context'
  }
}

function storeTab(tab: AsideTab) {
  try {
    window.localStorage.setItem(TAB_STORAGE_KEY, tab)
  } catch {
    // A remembered tab is a convenience; private windows may refuse it.
  }
}

interface ChatAsideProps {
  conversation: Conversation
  /** The pane edge it hugs: opposite the conversation list. */
  side: 'left' | 'right'
  /** Phone viewport: a bottom sheet instead of a column. */
  phone: boolean
  sheetOpen: boolean
  onSheetOpenChange: (open: boolean) => void
  request: AsideRequest | null
  triggers: readonly SessionTriggerInfo[]
  onUnregisterTrigger: (subscriptionId: string) => Promise<void> | void
  landBlocked?: boolean
  /** The quote waiting in the side chat's composer (owned by the main chat). */
  pendingQuote: SideChatQuote | null
  onPendingQuoteChange: (quote: SideChatQuote | null) => void
  /** Land the main chat on a message (a quote card's way back). */
  onJumpToMessage: (messageId: string) => void
  /** Hand the main chat's agent an instruction (see ChatContextPanel). */
  onAskAgent?: (text: string, send: boolean) => void
}

/**
 * The chat's side panel: the conversation's Context, and a temporary Side
 * chat that sees the conversation up to when it opened. The side chat is a
 * `session::fork` tagged `side_chat_of`, kept out of the sidebar, rendered
 * by a second ChatView so it has the same tools and approvals.
 */
export function ChatAside({
  conversation,
  side,
  phone,
  sheetOpen,
  onSheetOpenChange,
  request,
  triggers,
  onUnregisterTrigger,
  landBlocked,
  pendingQuote,
  onPendingQuoteChange,
  onJumpToMessage,
  onAskAgent,
}: ChatAsideProps) {
  const ctx = useConversationsCtx()
  const [tab, setTabState] = useState<AsideTab>(readStoredTab)
  const setTab = useCallback((next: AsideTab) => {
    setTabState(next)
    storeTab(next)
  }, [])
  const [expandSeq, setExpandSeq] = useState(0)

  // Whether the column is open, from the attribute PageSidebar sets: it
  // covers the stored preference, the toggle and the narrow drawer alike.
  const contentRef = useRef<HTMLDivElement>(null)
  const [expanded, setExpanded] = useState(true)
  useEffect(() => {
    if (phone) return
    const aside = contentRef.current?.closest('aside')
    if (!aside) return
    const read = () => setExpanded(!aside.hasAttribute('data-collapsed'))
    read()
    const observer = new MutationObserver(read)
    observer.observe(aside, {
      attributes: true,
      attributeFilter: ['data-collapsed'],
    })
    return () => observer.disconnect()
  }, [phone])
  const visible = phone ? sheetOpen : expanded

  // ── the side chat ──
  const [forkId, setForkId] = useState<string | null>(null)
  const forkIdRef = useRef(forkId)
  forkIdRef.current = forkId
  const forkStartedAt = useRef<number | null>(null)
  // Discarded ids stay hidden while their delete is in flight.
  const [discarding, setDiscarding] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const listed = forkId
    ? ctx.conversations.find((c) => c.id === forkId)
    : undefined
  const fork =
    (listed && !discarding.has(listed.id) ? listed : null) ??
    findSideChat(
      ctx.conversations.filter((c) => !discarding.has(c.id)),
      conversation.id,
    )
  // Forked, but its session::created has not reached the list yet.
  const forkPending = forkId !== null && !listed && !discarding.has(forkId)
  const forkRef = useRef(fork)
  forkRef.current = fork
  const [focusSide, setFocusSide] = useState(false)
  const [actionError, setActionError] = useState<{
    headline: string
    detail: string
  } | null>(null)
  const [forking, setForking] = useState(false)
  const [forkError, setForkError] = useState<string | null>(null)
  const forkingRef = useRef<Promise<string | null> | null>(null)
  const [composerKey, setComposerKey] = useState(0)
  const [kept, setKept] = useState<{ id: string } | null>(null)
  const [confirmDiscard, setConfirmDiscard] = useState(false)
  const mainStarted =
    conversation.started === true || conversation.messages.length > 0

  const { watchConversation } = ctx
  const watchedForkId = fork?.id ?? null
  useEffect(
    () => (watchedForkId ? watchConversation(watchedForkId) : undefined),
    [watchedForkId, watchConversation],
  )

  const ensureFork = useCallback(async (): Promise<string | null> => {
    if (forkRef.current) return forkRef.current.id
    if (forkIdRef.current) return forkIdRef.current
    if (forkingRef.current) return forkingRef.current
    const run = (async () => {
      setForking(true)
      setForkError(null)
      setKept(null)
      let forkedId: string | null = null
      try {
        const client = await getIiiClient()
        const tail = (sessionId: string, limit: number) =>
          client.trigger<{ messages?: TranscriptItem[] }>(
            'session::messages-tail',
            {
              session_id: sessionId,
              limit,
              include_custom: false,
              include_image_data: false,
            },
          )
        // Cut at the last spoken entry: its copy anchors the side chat.
        const point = lastSpokenEntry(
          (await tail(conversation.id, 20))?.messages ?? [],
        )
        if (!point) return null
        const forked = await client.trigger<{
          session_id: string
          meta: SessionMeta
        }>('session::fork', {
          session_id: conversation.id,
          entry_id: point,
          title: `Side chat · ${conversation.title}`,
        })
        forkedId = forked.session_id
        const copied = (await tail(forked.session_id, 1))?.messages ?? []
        await setSessionMeta({
          session_id: forked.session_id,
          metadata: sideChatMetadata(
            forked.meta.metadata,
            conversation.id,
            copied.at(-1)?.entry_id,
          ),
        })
        sideChatsMadeHere.add(forked.session_id)
        forkStartedAt.current = forked.meta.created_at
        setForkId(forked.session_id)
        return forked.session_id
      } catch (error) {
        // Untagged, the fork would show up as an ordinary chat everywhere.
        if (forkedId) void deleteSession(forkedId).catch(() => {})
        setForkError(errText(error))
        return null
      } finally {
        setForking(false)
        forkingRef.current = null
      }
    })()
    forkingRef.current = run
    return run
  }, [conversation.id, conversation.title])

  const listLoading = ctx.conversationsLoading === true
  const show = useCallback(
    (next: AsideTab) => {
      setTab(next)
      setExpandSeq((seq) => seq + 1)
      if (phone) onSheetOpenChange(true)
      if (next !== 'side-chat') return
      // Choosing the side chat is choosing to type in it.
      setFocusSide(true)
      // Not before the list is in: an existing side chat would go unseen.
      if (mainStarted && !listLoading) void ensureFork()
    },
    [setTab, phone, onSheetOpenChange, mainStarted, listLoading, ensureFork],
  )

  // An outside request (a message's "Ask in side chat", the palette).
  const handledRequest = useRef(0)
  useEffect(() => {
    if (!request || request.seq === handledRequest.current) return
    handledRequest.current = request.seq
    show(request.tab)
    if (request.tab !== 'side-chat' || !request.quote) return
    void ensureFork().then((id) => {
      // The composer takes the caret on mount: a remount aims it at the
      // question about the quote, whatever was typed there stays.
      if (id) setComposerKey((key) => key + 1)
    })
  }, [request, show, ensureFork])

  // Kept only once the server dropped the tag: a write lost on the way
  // would bring it back, hidden, for the sweep to delete.
  const [keeping, setKeeping] = useState(false)
  const keep = async () => {
    if (!fork || keeping) return
    const target = fork
    const { [SIDE_CHAT_KEY]: _tag, ...metadata } = target.sessionMetadata ?? {}
    setKeeping(true)
    setActionError(null)
    try {
      await setSessionMeta({ session_id: target.id, metadata })
    } catch (error) {
      setActionError({ headline: 'Side chat not kept', detail: errText(error) })
      return
    } finally {
      setKeeping(false)
    }
    ctx.setSessionMetadata(target.id, { [SIDE_CHAT_KEY]: undefined })
    sideChatsMadeHere.delete(target.id)
    onPendingQuoteChange(null)
    setKept({ id: target.id })
    setForkId(null)
  }
  const discard = () => {
    const target = fork
    setConfirmDiscard(false)
    setForkId(null)
    setActionError(null)
    onPendingQuoteChange(null)
    if (!target) return
    const hide = (on: boolean) =>
      setDiscarding((current) => {
        const next = new Set(current)
        if (on) next.add(target.id)
        else next.delete(target.id)
        return next
      })
    hide(true)
    sideChatsMadeHere.delete(target.id)
    void ctx.remove(target.id).catch((error) => {
      hide(false)
      setActionError({
        headline: 'Side chat not discarded',
        detail: errText(error),
      })
    })
  }

  const since = fork ? (fork.createdAt ?? forkStartedAt.current ?? 0) : 0
  const sent = sideChatQuotes(fork)
  const from = sideChatFrom(fork)
  const own = fork ? sideChatMessages(fork.messages, since, from) : []
  // Only an answer counts as news: the turn's notes and tool rows land first.
  const replies = own.filter(
    (message) => message.role === 'assistant' && message.content.trim(),
  ).length

  // A reply that lands while the side chat is out of sight marks its tab and
  // icon. Counted from when its transcript first loads here: coming back to
  // a conversation shows what was already there as read.
  const [seen, setSeen] = useState<{ id: string; count: number } | null>(null)
  const sideVisible = tab === 'side-chat' && visible
  const loadedForkId = fork && fork.hydrated !== false ? fork.id : null
  useEffect(() => {
    if (!loadedForkId) return
    setSeen((current) =>
      current?.id === loadedForkId && !sideVisible
        ? current
        : { id: loadedForkId, count: replies },
    )
  }, [loadedForkId, sideVisible, replies])
  const unread =
    !sideVisible &&
    seen !== null &&
    seen.id === loadedForkId &&
    replies > seen.count

  const tabs = (
    <SegmentedControl<AsideTab>
      aria-label="Side panel"
      value={tab}
      onChange={show}
      className="min-w-0 flex-1"
      options={[
        {
          value: 'context',
          label: 'Context',
          icon: <Layers aria-hidden className="size-4" />,
        },
        {
          value: 'side-chat',
          // A reply that landed while the Context tab was up.
          label: unread ? (
            <>
              Side chat
              <span
                aria-hidden
                data-side-chat-unread=""
                className="inline-block size-1.5 rounded-full bg-accent"
              />
              <span className="sr-only">, new reply</span>
            </>
          ) : (
            'Side chat'
          ),
          icon: <MessagesSquare aria-hidden className="size-4" />,
        },
      ]}
    />
  )

  const content =
    tab === 'context' ? (
      <ChatContextPanel
        conversation={conversation}
        conversations={ctx.conversations}
        connectionState={ctx.connectionState}
        triggers={triggers}
        onUnregisterTrigger={onUnregisterTrigger}
        onOpenConversation={ctx.openConversationInPanel}
        shellAvailable={ctx.shellAvailable}
        githubAvailable={ctx.githubAvailable}
        kanbanAvailable={ctx.kanbanAvailable}
        onAskAgent={onAskAgent}
        onDismissPullRequest={(key) =>
          ctx.setSessionMetadata(conversation.id, {
            [DISMISSED_PRS_KEY]: [...dismissedPullRequests(conversation), key],
          })
        }
        landBlocked={landBlocked}
        active={visible}
      />
    ) : (
      <div className="flex min-h-0 flex-1 flex-col">
        {actionError ? (
          <StatusPanel
            variant="alert"
            role="alert"
            className="mx-2 mb-2"
            headline={actionError.headline}
            detail={actionError.detail}
          />
        ) : null}
        {kept ? (
          <StatusPanel
            variant="success"
            role="status"
            className="mx-2 mb-2"
            headline="Kept as a conversation"
            detail="It is in the sidebar with the other chats."
            action={
              <Button
                variant="ghost"
                size="sm"
                onClick={() => ctx.openConversationInPanel(kept.id)}
              >
                Open
              </Button>
            }
          />
        ) : null}
        {fork ? (
          <>
            <div className="flex h-8 shrink-0 items-center gap-1.5 pr-2 pl-4 text-[12px] text-ink-faint">
              <Clock aria-hidden className="size-4 shrink-0" />
              <span title={SIDE_CHAT_LIFETIME}>Temporary</span>
              <span className="min-w-0 truncate text-ink-ghost">
                · sees the chat up to{' '}
                <span className="font-mono text-[11px] tabular-nums">
                  {new Date(since).toLocaleTimeString([], {
                    hour: '2-digit',
                    minute: '2-digit',
                  })}
                </span>
              </span>
              <span className="flex-1" />
              <IconButton
                label="Keep as conversation"
                disabled={keeping}
                onClick={() => void keep()}
              >
                <BookmarkPlus aria-hidden />
              </IconButton>
              <IconButton
                label="Discard side chat"
                onClick={() =>
                  own.some((message) => message.role === 'user')
                    ? setConfirmDiscard(true)
                    : discard()
                }
              >
                <Trash2 aria-hidden />
              </IconButton>
            </div>
            <ChatView
              key={`${fork.id}:${composerKey}`}
              conversation={fork}
              side={{
                since,
                fromEntry: from,
                empty: <SideChatHint />,
                autoFocus: focusSide,
                quote: pendingQuote,
                onRemoveQuote: () => onPendingQuoteChange(null),
                onQuoteSent: (quote) => {
                  ctx.setSessionMetadata(fork.id, {
                    [SIDE_CHAT_QUOTES_KEY]: withSideChatQuote(sent, quote),
                  })
                  onPendingQuoteChange(null)
                },
                quotes: sent,
                onJumpToQuote: (quote) => onJumpToMessage(quote.messageId),
              }}
              backend={ctx.backend}
              modelOptions={ctx.modelOptions}
              catalogLoading={ctx.catalogLoading}
              density="dock"
              onUpdateModel={ctx.setModel}
              onUpdateThinkingLevel={ctx.setThinkingLevel}
              onUpdateWorkingDir={ctx.setWorkingDir}
              onAppendMessage={ctx.appendMessage}
              onPatchMessage={ctx.updateMessage}
              onCompactConversation={ctx.compactConversation}
            />
            <p className="shrink-0 px-4 pb-3 text-[11px] leading-4 text-ink-ghost">
              {SIDE_CHAT_LIFETIME}
            </p>
          </>
        ) : forking || forkPending ? (
          <div className="flex flex-col gap-2 px-4 py-3" aria-busy="true">
            <Skeleton className="h-3 w-48" />
            <Skeleton className="h-3 w-32" />
          </div>
        ) : forkError ? (
          <StatusPanel
            variant="alert"
            role="alert"
            className="mx-2"
            headline="Side chat not started"
            detail={forkError}
            action={
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void ensureFork()}
              >
                Retry
              </Button>
            }
          />
        ) : (
          <div className="flex flex-1 flex-col justify-center px-4">
            <SideChatHint
              action={
                mainStarted
                  ? {
                      label: 'Start side chat',
                      onClick: () => void ensureFork(),
                    }
                  : undefined
              }
              description={
                mainStarted
                  ? undefined
                  : 'Send a message in this chat first: the side chat starts from it.'
              }
            />
          </div>
        )}
        <ConfirmDialog
          open={confirmDiscard}
          onOpenChange={setConfirmDiscard}
          title="Discard the side chat?"
          description="Its messages are deleted. The main chat is not touched."
          confirmLabel="Discard"
          tone="danger"
          onConfirm={discard}
        />
      </div>
    )

  if (phone) {
    return (
      <BottomSheet open={sheetOpen} onOpenChange={onSheetOpenChange}>
        <BottomSheetContent heading="Side panel" className="h-[85dvh]">
          {/* Portaled to the body, outside the app's sans default. */}
          <div className="flex min-h-0 flex-1 flex-col gap-2 font-sans">
            <div className="flex px-3">{tabs}</div>
            {content}
          </div>
        </BottomSheetContent>
      </BottomSheet>
    )
  }

  return (
    <PageSidebar
      label="Side panel"
      side={side}
      storageKey="console:chat:aside"
      defaultWidth={340}
      minWidth={280}
      maxWidth={480}
      collapsible
      resizable
      // Inline only while the chat keeps its own 560px beside the panel's 340.
      narrowBelow={900}
      narrowMode="drawer"
      closeDrawerOnSelect={false}
      expandRequest={expandSeq}
      header={tabs}
      collapsedActions={
        <>
          <IconButton
            label="Context"
            tooltipSide={side === 'right' ? 'left' : 'right'}
            className="size-7"
            onClick={() => show('context')}
          >
            <Layers aria-hidden />
          </IconButton>
          <IconButton
            label={unread ? 'Side chat, new reply' : 'Side chat'}
            tooltipSide={side === 'right' ? 'left' : 'right'}
            className="relative size-7"
            onClick={() => show('side-chat')}
          >
            <MessagesSquare aria-hidden />
            {unread ? (
              <span
                aria-hidden
                className="absolute top-1 right-1 size-1.5 rounded-full bg-accent"
              />
            ) : null}
          </IconButton>
        </>
      }
    >
      <div ref={contentRef} className="flex min-h-0 flex-1 flex-col">
        {content}
      </div>
    </PageSidebar>
  )
}

function SideChatHint({
  action,
  description = 'Sees this conversation up to now and has the same tools. Nothing here lands in the main chat.',
}: {
  action?: { label: string; onClick: () => void }
  description?: string
}) {
  return (
    <EmptyState
      icon={MessagesSquare}
      title="Ask on the side"
      description={description}
      action={action}
    />
  )
}

/** The quote waiting in a side chat's composer, removable before it is sent. */
export function QuoteChip({
  quote,
  onRemove,
}: {
  quote: SideChatQuote
  onRemove?: () => void
}) {
  return (
    <div
      className="flex items-start gap-2 bg-surface px-3 py-2"
      data-side-chat-quote=""
    >
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <Eyebrow className="text-ink-ghost">
          Quoted · {quote.agent || 'Agent'}
        </Eyebrow>
        <span className="line-clamp-2 text-[12px] leading-[18px] text-ink-faint">
          {quote.text}
        </span>
      </div>
      {onRemove ? (
        <IconButton
          label="Remove quote"
          className="shrink-0"
          onClick={onRemove}
        >
          <X aria-hidden />
        </IconButton>
      ) : null}
    </div>
  )
}

/** Half the toolbar's width, to keep it on screen near the edges. */
const TOOLBAR_HALF = 112
const toolbarButton =
  'inline-flex h-7 items-center gap-1.5 rounded-md px-2 font-sans text-[13px] text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-2 focus-visible:outline-accent [&>svg]:size-4 [&>svg]:shrink-0'

/** What a selection toolbar points at: the quote, where it is, and the
 *  transcript box it must stay inside. */
export interface SelectedQuote {
  quote: SideChatQuote
  rect: DOMRect
  bounds: DOMRect
}

/**
 * Floats Copy and "Ask in side chat" over text selected in the transcript.
 * `read` says what is selected (see `selectionQuote`); the toolbar shows
 * once the pointer lets go, follows the selection while it scrolls, and
 * hides on Escape, a press elsewhere, or the selection leaving the box.
 */
export function SelectionToolbar({
  read,
  onAsk,
}: {
  read: () => SelectedQuote | null
  onAsk: (quote: SideChatQuote) => void
}) {
  const [shown, setShown] = useState<SelectedQuote | null>(null)
  const [copied, setCopied] = useState(false)
  const readRef = useRef(read)
  readRef.current = read
  const toolbarRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    let pressed = false
    let pressedInTranscript = false
    let frame = 0
    const update = () => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        if (pressed) return
        const picked = readRef.current()
        const visible =
          picked &&
          picked.rect.bottom > picked.bounds.top &&
          picked.rect.top < picked.bounds.bottom
        setShown((current) => {
          // "Copied" belongs to the text it copied.
          if (current?.quote.text !== picked?.quote.text) setCopied(false)
          return visible ? picked : null
        })
      })
    }
    const down = (event: PointerEvent) => {
      if (toolbarRef.current?.contains(event.target as Node)) return
      pressed = true
      pressedInTranscript = Boolean(
        (event.target as Element).closest?.('[data-message-list]'),
      )
      setShown(null)
    }
    // Also on a cancelled press: dragging selected text ends in a drag, not
    // a pointerup. A press elsewhere (a button, the sidebar) leaves the
    // selection alone and does not bring the toolbar back.
    const up = () => {
      if (!pressed) return
      pressed = false
      if (pressedInTranscript) update()
    }
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setShown(null)
    }
    document.addEventListener('selectionchange', update)
    document.addEventListener('pointerdown', down, true)
    document.addEventListener('pointerup', up, true)
    document.addEventListener('pointercancel', up, true)
    document.addEventListener('dragend', up, true)
    document.addEventListener('keydown', key)
    window.addEventListener('scroll', update, true)
    window.addEventListener('resize', update)
    return () => {
      cancelAnimationFrame(frame)
      document.removeEventListener('selectionchange', update)
      document.removeEventListener('pointerdown', down, true)
      document.removeEventListener('pointerup', up, true)
      document.removeEventListener('pointercancel', up, true)
      document.removeEventListener('dragend', up, true)
      document.removeEventListener('keydown', key)
      window.removeEventListener('scroll', update, true)
      window.removeEventListener('resize', update)
    }
  }, [])
  if (!shown) return null
  const { quote, rect, bounds } = shown
  // Above the selection, or below it at the top of the transcript and on
  // touch screens, where the system's own selection menu sits above.
  const below =
    rect.top - 48 < bounds.top ||
    window.matchMedia?.('(pointer: coarse)').matches
  const center = Math.min(
    Math.max(rect.left + rect.width / 2, TOOLBAR_HALF + 8),
    window.innerWidth - TOOLBAR_HALF - 8,
  )
  return createPortal(
    <div
      ref={toolbarRef}
      role="toolbar"
      aria-label="Selection"
      data-selection-toolbar=""
      className="fixed z-50 flex items-center gap-0.5 rounded-md bg-panel-raised p-0.5 shadow-floating"
      style={{
        left: center,
        top: below ? rect.bottom + 8 : rect.top - 8,
        translate: below ? '-50% 0' : '-50% -100%',
      }}
      // Keep the selection: a press on a button would collapse it.
      onMouseDown={(event) => event.preventDefault()}
    >
      <button
        type="button"
        className={toolbarButton}
        onClick={() => {
          void copyTextToClipboard(quote.text).then((ok) => {
            if (ok) setCopied(true)
          })
        }}
      >
        {copied ? <Check aria-hidden /> : <Copy aria-hidden />}
        {copied ? 'Copied' : 'Copy'}
      </button>
      <button
        type="button"
        className={toolbarButton}
        onClick={() => {
          setShown(null)
          window.getSelection()?.removeAllRanges()
          onAsk(quote)
        }}
      >
        <MessageSquareQuote aria-hidden />
        Ask in side chat
      </button>
    </div>,
    document.body,
  )
}
