import {
  BookmarkPlus,
  Clock,
  Layers,
  MessagesSquare,
  Trash2,
  X,
} from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
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
  const [actionError, setActionError] = useState<string | null>(null)
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

  const show = useCallback(
    (next: AsideTab) => {
      setTab(next)
      setExpandSeq((seq) => seq + 1)
      if (phone) onSheetOpenChange(true)
      if (next !== 'side-chat') return
      // Choosing the side chat is choosing to type in it.
      setFocusSide(true)
      if (mainStarted) void ensureFork()
    },
    [setTab, phone, onSheetOpenChange, mainStarted, ensureFork],
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

  const keep = () => {
    if (!fork) return
    ctx.setSessionMetadata(fork.id, { [SIDE_CHAT_KEY]: undefined })
    sideChatsMadeHere.delete(fork.id)
    onPendingQuoteChange(null)
    setKept({ id: fork.id })
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
      setActionError(errText(error))
    })
  }

  const since = fork ? (fork.createdAt ?? forkStartedAt.current ?? 0) : 0
  const sent = sideChatQuotes(fork)
  const from = sideChatFrom(fork)
  const own = fork ? sideChatMessages(fork.messages, since, from) : []

  // A reply that lands while the side chat is out of sight marks its icon.
  const [seenCount, setSeenCount] = useState(0)
  const sideVisible = tab === 'side-chat' && visible
  useEffect(() => {
    if (sideVisible) setSeenCount(own.length)
  }, [sideVisible, own.length])
  const unread = !sideVisible && own.length > seenCount

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
          label: 'Side chat',
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
            headline="Side chat not discarded"
            detail={actionError}
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
              <span title="Discarded when you leave this conversation">
                Temporary
              </span>
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
              <IconButton label="Keep as conversation" onClick={keep}>
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
              Discarded when you leave this conversation.
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
          <div className="flex min-h-0 flex-1 flex-col gap-2">
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
