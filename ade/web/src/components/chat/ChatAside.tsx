import { Dialog as DialogPrimitive } from '@base-ui/react/dialog'
import {
  BookmarkPlus,
  Check,
  ChevronsDownUp,
  ChevronsUpDown,
  Clock,
  Copy,
  EllipsisVertical,
  History,
  Layers,
  MessageSquareOff,
  MessageSquareQuote,
  MessagesSquare,
  SquareFunction,
  Timer,
  Trash2,
  X,
} from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Badge } from '@/components/shadcn/badge'
import { Button } from '@/components/shadcn/button'
import { Sheet, SheetClose, SheetPortal } from '@/components/shadcn/sheet'
import { Tabs, TabsList, TabsTrigger } from '@/components/shadcn/tabs'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/shadcn/tooltip'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { Hint } from '@/components/ui/Hint'
import { IconButton } from '@/components/ui/IconButton'
import { PageSidebar } from '@/components/ui/PageChrome'
import { SheetScrim } from '@/components/ui/SheetScrim'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { useVisualViewportFit } from '@/hooks/use-visual-viewport-fit'
import {
  AUTO_FIX_KEY,
  autoFixRound,
  autoFixState,
  ghViewer,
} from '@/lib/auto-fix'
import type { ContextItems } from '@/lib/backend/context-items'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import {
  closeSideChat,
  describeSideChatError,
  keepSideChat,
  openSideChat,
  summarizeSideChat,
} from '@/lib/backend/side-chats'
import {
  appendToDraft,
  bringBackBlock,
  DISMISSED_PRS_KEY,
  dismissedPullRequests,
  findSideChat,
  folderGh,
  githubWorkerGh,
  isSideChat,
  lastSpokenEntry,
  SIDE_CHAT_KEY,
  SIDE_CHAT_QUOTES_KEY,
  type SideChatQuote,
  sideChatForkPoint,
  sideChatFrom,
  sideChatLock,
  sideChatMessages,
  sideChatMetadata,
  sideChatQuotes,
  sideChatRows,
  sideChatSummaries,
  sideChatsMadeHere,
  withSideChatQuote,
} from '@/lib/chat-aside'
import { copyTextToClipboard } from '@/lib/clipboard'
import { composerForSession, insertIntoComposer } from '@/lib/composer-insert'
import {
  allGroupsFolded,
  foldAllGroups,
  revealGroup,
  useContextSections,
  useFoldedGroups,
} from '@/lib/context-groups'
import { useConversationsCtx } from '@/lib/conversations-context'
import { errText } from '@/lib/errors'
import { getIiiClient } from '@/lib/iii-client'
import { bindingMatchesEvent } from '@/lib/keybindings/bindings'
import {
  deleteSession,
  setSessionDraft,
  setSessionMeta,
} from '@/lib/sessions/api'
import { belongsToEntry } from '@/lib/sessions/entry-mapper'
import type { SessionMeta, TranscriptItem } from '@/lib/sessions/types'
import { PortalScope } from '@/lib/ui-scope'
import type { Conversation, SystemMessage } from '@/types/chat'
import { ChatContextPanel } from './ChatContextPanel'
import { ChatView } from './ChatView'
import { ContextSectionsMenu } from './ContextSectionsMenu'
import {
  ConfirmSideChatDelete,
  forkStripText,
  lockPresentation,
  SideChatForkStrip,
  SideChatHeader,
  SideChatMenu,
  SideChatPage,
  SideChatToast,
} from './side-chats'

export type AsideTab = 'context' | 'side-chat'

/** An outside ask: show a tab, and for the side chat maybe seed a quote. */
export interface AsideRequest {
  seq: number
  tab: AsideTab
  /** A quote just handed over: the composer takes the caret to ask about it. */
  quote?: SideChatQuote
  /** A Context section to unfold and scroll to (`observers`). */
  section?: string
  /** Engine side chats: open a new one forked at this entry ("Side chat"). */
  fork?: { entryId?: string }
  /** Engine side chats: open this one (its mark in the transcript). */
  openId?: string
}

/** A side chat open in the pane widens it to this, when there is room (§0.5). */
const SIDE_CHAT_PANE_WIDTH = 460
/** What the conversation keeps beside a widened pane (§0.5). */
const MIN_CHAT_WIDTH = 480

const TAB_STORAGE_KEY = 'iii-chat-aside-tab'
/** The pane's stored geometry and open state (`PageSidebar`), per device. */
export const ASIDE_STORAGE_KEY = 'console:chat:aside'
/** Show / Hide context pane (§1.3.6): ⌘⌥0 on a Mac, Ctrl+Alt+0 elsewhere. */
export const CONTEXT_PANE_SHORTCUT = 'Mod+Alt+0'
/** How long a phone drawer's close may take before a pending "Show in
 * conversation" lands anyway (the slide is --motion-duration-panel). */
const DRAWER_CLOSE_FALLBACK_MS = 600
/** Where "Show in conversation" lands: an entry, or a loaded message. */
type ShowTarget = { entryId?: string; messageId?: string }
/** The chat's header height beside the pane (PageHeader at sm and up). */
const CHAT_HEADER_PX = 44
/** How often auto-fix reads a watched PR while the chat is open. */
const AUTO_FIX_EVERY_MS = 90_000

/** When a side chat ends on its own (ChatPanel runs the rules). */
const SIDE_CHAT_LIFETIME =
  'Deleted after 2 hours unused, when every ADE tab closes, or with this chat.'

/** A panel tab: the header's full height, a 2px ink rule under the chosen
 *  one on the header's bottom edge, quiet otherwise. The weight never
 *  changes between states, so switching tabs shifts nothing. */
const ASIDE_TAB =
  'h-full flex-none gap-1.5 rounded-none px-2 text-body font-medium text-muted-foreground hover:text-foreground data-active:text-foreground data-active:shadow-none group-data-horizontal/tabs:after:bottom-0 group-data-horizontal/tabs:after:h-0.5 dark:data-active:border-transparent dark:data-active:bg-transparent'

function clockTime(at: number): string {
  return new Date(at).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  })
}

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
  /**
   * Each new value puts the pane away: the column folds, the drawer beside
   * a narrow chat closes (the header's button, Mod+Alt+0, the palette).
   */
  hideRequest?: number
  observers: readonly SessionObserverInfo[]
  /** This chat's observer events, for the Observers section. */
  observerEvents?: readonly SystemMessage[]
  /** The observer list has loaded: an observer missing from it has ended. */
  observersListed?: boolean
  onDisposeObserver: (observerId: string) => Promise<void> | void
  landBlocked?: boolean
  /** The quote waiting in the side chat's composer (owned by the main chat). */
  pendingQuote: SideChatQuote | null
  onPendingQuoteChange: (quote: SideChatQuote | null) => void
  /** Land the main chat on a message (a quote card's way back). */
  onJumpToMessage: (messageId: string) => void
  /** Hand the main chat's agent an instruction (see ChatContextPanel). */
  onAskAgent?: (text: string, send: boolean) => void
  /** The engine's context items (`harness::context-items`), when it has them. */
  contextItems?: ContextItems | null
  /** How many items the pane lists, beside "Context" (0 hides it). */
  itemCount?: number
  /** "Show in conversation" from a pane row; the phone drawer closes first. */
  onShowInConversation?: (target: {
    entryId?: string
    messageId?: string
  }) => void
  /** The pane or drawer came into sight or left it (the header's button). */
  onVisibleChange?: (visible: boolean) => void
  /**
   * The engine has side chats (`harness::side-chat::open`): several per
   * conversation, listed in the Context pane, opened in a side-chat view
   * (a page on phones). Off: the pane's own one-fork Side chat tab.
   */
  engineSideChats?: boolean
  /** The side chats changed here: re-read the context items. */
  onSideChatsChanged?: () => void
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
  hideRequest,
  observers,
  observerEvents,
  observersListed,
  onDisposeObserver,
  landBlocked,
  pendingQuote,
  onPendingQuoteChange,
  onJumpToMessage,
  onAskAgent,
  contextItems = null,
  itemCount = 0,
  onShowInConversation,
  onVisibleChange,
  engineSideChats = false,
  onSideChatsChanged,
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
  const onVisibleChangeRef = useRef(onVisibleChange)
  onVisibleChangeRef.current = onVisibleChange

  // "Show in conversation" from the phone drawer: the drawer closes first
  // and the transcript lands once it has gone (§1.3.6), so the row's 1.2 s
  // flash is all in sight, not under the drawer sliding out. The fallback
  // timer lands it should the close never report its end.
  const onShowInConversationRef = useRef(onShowInConversation)
  onShowInConversationRef.current = onShowInConversation
  const pendingShowRef = useRef<ShowTarget | null>(null)
  const pendingShowTimerRef = useRef<number | null>(null)
  const landPendingShow = useCallback(() => {
    if (pendingShowTimerRef.current !== null) {
      window.clearTimeout(pendingShowTimerRef.current)
      pendingShowTimerRef.current = null
    }
    const target = pendingShowRef.current
    pendingShowRef.current = null
    if (target) onShowInConversationRef.current?.(target)
  }, [])
  useEffect(
    () => () => {
      if (pendingShowTimerRef.current !== null)
        window.clearTimeout(pendingShowTimerRef.current)
    },
    [],
  )
  useEffect(() => {
    onVisibleChangeRef.current?.(visible)
  }, [visible])

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
    )[0] ??
    null
  // Forked, but its session::on-created has not reached the list yet.
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
    // A side chat of a side chat is never wanted, and a loop that forks the
    // fork would fill the session list: refuse outright.
    if (
      isSideChat(conversation) ||
      (conversation.title ?? '').startsWith('Side chat · ')
    ) {
      setForkError('This is already a side chat. Ask in it directly.')
      return null
    }
    const run = (async () => {
      setForking(true)
      setForkError(null)
      setKept(null)
      let forkedId: string | null = null
      try {
        const client = await getIiiClient()
        const tail = (sessionId: string, limit: number) =>
          client.call<{ messages?: TranscriptItem[] }>(
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
        if (!point) {
          setForkError(
            'A side chat sees the conversation up to now, and this one has no messages yet. Send one in the chat first.',
          )
          return null
        }
        const forked = await client.call<{
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

  // ── engine side chats (chat-projects §2.3): several per conversation ──
  const engine = engineSideChats
  const [openId, setOpenId] = useState<string | null>(null)
  const openIdRef = useRef(openId)
  openIdRef.current = openId
  const [opening, setOpening] = useState(false)
  const openingRef = useRef(false)
  const [sideError, setSideError] = useState<{
    headline: string
    detail: string
  } | null>(null)
  // Closed, kept or deleted here: out of the list while the engine catches up.
  const [sideHidden, setSideHidden] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const [busyId, setBusyId] = useState<string | null>(null)
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null)
  const [toast, setToast] = useState<{
    seq: number
    text: string
    action?: { label: string; run: () => void }
  } | null>(null)
  const sideChangedRef = useRef(onSideChatsChanged)
  sideChangedRef.current = onSideChatsChanged
  const sideRows = engine
    ? sideChatRows(
        contextItems?.side_chats,
        ctx.conversations,
        conversation.id,
        sideHidden,
      )
    : []
  const openChat =
    engine && openId
      ? (ctx.conversations.find((c) => c.id === openId) ?? null)
      : null
  useEffect(
    () => (engine && openId ? watchConversation(openId) : undefined),
    [engine, openId, watchConversation],
  )
  const hideSide = useCallback((id: string, on: boolean) => {
    setSideHidden((current) => {
      const next = new Set(current)
      if (on) next.add(id)
      else next.delete(id)
      return next
    })
  }, [])

  const openNewSideChat = useCallback(
    async (target: { entryId?: string; quote?: SideChatQuote }) => {
      if (openingRef.current) return
      openingRef.current = true
      setOpening(true)
      setSideError(null)
      setKept(null)
      setFocusSide(true)
      try {
        const opened = await openSideChat({
          sessionId: conversation.id,
          ...target,
        })
        setOpenId(opened.session_id)
        sideChangedRef.current?.()
      } catch (error) {
        setSideError({
          headline: 'Side chat not started',
          detail: describeSideChatError(error),
        })
      } finally {
        openingRef.current = false
        setOpening(false)
      }
    },
    [conversation.id],
  )
  const openExistingSideChat = useCallback((id: string) => {
    setSideError(null)
    setKept(null)
    setFocusSide(true)
    setOpenId(id)
  }, [])

  const openSince = openChat ? (openChat.createdAt ?? 0) : 0
  const openFrom = sideChatFrom(openChat)
  const openOwn = openChat
    ? sideChatMessages(openChat.messages, openSince, openFrom)
    : []
  // Leaving a side chat in which nothing was sent discards it, without
  // asking (§2.3.2): `close` deletes it only when it is empty. One left
  // before its transcript was read goes to `close` too, since the engine
  // keeps one that was sent in; but not with text typed in it, the draft of
  // an Enter that waited for that read.
  const leaveSideChat = () => {
    const target = openChat
    setOpenId(null)
    setSideError(null)
    onPendingQuoteChange(null)
    if (
      !target ||
      target.status === 'working' ||
      openOwn.some((message) => message.role === 'user')
    ) {
      return
    }
    const unread = target.hydrated === false
    if (unread && ctx.getDraftText(target.id)?.trim()) return
    // A read one is known to be empty: out of the list at once.
    if (!unread) hideSide(target.id, true)
    void closeSideChat(target.id).then(
      (result) => {
        hideSide(target.id, Boolean(result?.deleted))
        sideChangedRef.current?.()
      },
      () => hideSide(target.id, false),
    )
  }
  const leaveRef = useRef(leaveSideChat)
  leaveRef.current = leaveSideChat

  const keepSide = async (id: string) => {
    setBusyId(id)
    setSideError(null)
    try {
      await keepSideChat(id)
    } catch (error) {
      setSideError({
        headline: 'Side chat not kept',
        detail: describeSideChatError(error),
      })
      return
    } finally {
      setBusyId(null)
    }
    hideSide(id, true)
    sideChatsMadeHere.delete(id)
    if (openIdRef.current === id) {
      setOpenId(null)
      onPendingQuoteChange(null)
    }
    setKept({ id })
    sideChangedRef.current?.()
  }
  const deleteSide = (id: string) => {
    hideSide(id, true)
    sideChatsMadeHere.delete(id)
    if (openIdRef.current === id) {
      setOpenId(null)
      onPendingQuoteChange(null)
    }
    void ctx.remove(id).then(
      () => sideChangedRef.current?.(),
      (error) => {
        hideSide(id, false)
        setSideError({
          headline: 'Side chat not deleted',
          detail: errText(error),
        })
      },
    )
  }
  const summarizeSide = async (id: string) => {
    setBusyId(id)
    setSideError(null)
    try {
      await summarizeSideChat(id)
    } catch (error) {
      setSideError({
        headline: 'No summary asked for',
        detail: describeSideChatError(error),
      })
    } finally {
      setBusyId(null)
    }
  }

  // "Bring back": the reply goes into the conversation's draft, unsent
  // (§2.3.2). Its composer, when mounted, takes it as paragraphs of their
  // own and saves the draft (`session::set-draft`); otherwise the draft is
  // written here. Wide: the composer takes the caret; phone: the toast
  // offers the way back.
  const openTitleRef = useRef(openChat?.title ?? 'Side chat')
  openTitleRef.current = openChat?.title ?? 'Side chat'
  const { getDraftText, setDraftText } = ctx
  const bringBack = useCallback(
    (reply: string) => {
      const block = bringBackBlock(openTitleRef.current, reply)
      const composerId = composerForSession(conversation.id)
      if (composerId) {
        // The composer writes one line per paragraph: a paragraph per line
        // keeps the text as it is, and an empty one before it is the blank
        // line after a draft that has text.
        const lines = block.split('\n')
        if ((getDraftText(conversation.id) ?? '').trim()) lines.unshift('')
        for (const line of lines) {
          insertIntoComposer(line, { composerId, focus: !phone })
        }
      } else {
        const merged = appendToDraft(getDraftText(conversation.id), block)
        setDraftText(conversation.id, merged)
        void setSessionDraft(conversation.id, merged).catch(() => {})
      }
      setToast((current) => ({
        seq: (current?.seq ?? 0) + 1,
        text: `Added to the ${conversation.title} composer`,
        ...(phone
          ? { action: { label: 'Open', run: () => leaveRef.current() } }
          : {}),
      }))
    },
    [conversation.id, conversation.title, phone, getDraftText, setDraftText],
  )

  // Auto-fix rounds while this chat is open: one tab at a time (a Web Lock
  // per chat), never while the agent is working, and what was reported
  // kept in the session so every tab shares it (lib/auto-fix).
  const watching = Object.keys(autoFixState(conversation)).sort().join()
  const conversationRef = useRef(conversation)
  conversationRef.current = conversation
  const askRef = useRef(onAskAgent)
  askRef.current = onAskAgent
  const { githubAvailable, shellAvailable, setSessionMetadata } = ctx
  const workingDir = conversation.workingDir ?? null
  useEffect(() => {
    const ghReach = githubAvailable
      ? 'worker'
      : shellAvailable && workingDir
        ? 'folder'
        : null
    if (!watching || !ghReach) return
    let alive = true
    let viewer: string | null | undefined
    const run = async () => {
      const current = conversationRef.current
      const state = autoFixState(current)
      if (!askRef.current || current.status === 'working') return
      if (Object.keys(state).length === 0) return
      const client = await getIiiClient()
      const call = client.call.bind(client)
      const gh =
        ghReach === 'folder' && workingDir
          ? folderGh(call, workingDir)
          : githubWorkerGh(call)
      viewer ??= await ghViewer(gh)
      const { next, message } = await autoFixRound(gh, state, viewer)
      if (!alive) return
      if (JSON.stringify(next) !== JSON.stringify(state)) {
        setSessionMetadata(current.id, { [AUTO_FIX_KEY]: next })
      }
      if (message) askRef.current?.(message, true)
    }
    const round = async () => {
      const locks =
        typeof navigator === 'undefined' ? undefined : navigator.locks
      if (!locks) return run()
      await locks.request(
        `iii-ade-autofix:${conversationRef.current.id}`,
        { ifAvailable: true },
        async (lock) => {
          if (lock) await run()
        },
      )
    }
    void round().catch(() => {})
    const timer = window.setInterval(
      () => void round().catch(() => {}),
      AUTO_FIX_EVERY_MS,
    )
    return () => {
      alive = false
      window.clearInterval(timer)
    }
  }, [
    watching,
    githubAvailable,
    shellAvailable,
    workingDir,
    setSessionMetadata,
  ])

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
      // The engine's side chats open from the pane instead.
      if (!listLoading && !engine) void ensureFork()
    },
    [setTab, phone, onSheetOpenChange, listLoading, ensureFork, engine],
  )

  // An outside request (a message's "Ask in side chat", the palette).
  const handledRequest = useRef(0)
  useEffect(() => {
    if (!request || request.seq === handledRequest.current) return
    handledRequest.current = request.seq
    if (engine && request.tab === 'side-chat') {
      // The side-chat view: in the pane (wide) or a page (phone), never the
      // drawer.
      if (phone) onSheetOpenChange(false)
      else setExpandSeq((seq) => seq + 1)
      if (request.openId) openExistingSideChat(request.openId)
      else if (request.fork) {
        void openNewSideChat({ entryId: request.fork.entryId })
      } else if (request.quote) {
        // Into the side chat on screen, else a new one that opens about it.
        if (openIdRef.current) setComposerKey((key) => key + 1)
        else void openNewSideChat({ quote: request.quote })
      } else if (!openIdRef.current) void openNewSideChat({})
      return
    }
    if (engine && openIdRef.current) leaveRef.current()
    show(request.tab)
    const section = request.section
    if (section) {
      revealGroup(section)
      // After the tab and the unfold have painted.
      requestAnimationFrame(() => {
        const target = contentRef.current?.querySelector<HTMLElement>(
          `[data-reorder-item="${section}"]`,
        )
        if (!target) return
        const still = window.matchMedia(
          '(prefers-reduced-motion: reduce)',
        ).matches
        target.scrollIntoView({
          block: 'nearest',
          behavior: still ? 'auto' : 'smooth',
        })
        // Already in view, the section would not move: a brief ring says
        // this is where the click landed.
        if (!still) {
          target.animate(
            [
              { boxShadow: '0 0 0 2px var(--color-warn)' },
              { boxShadow: '0 0 0 2px transparent' },
            ],
            { duration: 1200, easing: 'ease-out' },
          )
        }
      })
    }
    if (request.tab !== 'side-chat' || !request.quote) return
    void ensureFork().then((id) => {
      // The composer takes the caret on mount: a remount aims it at the
      // question about the quote, whatever was typed there stays.
      if (id) setComposerKey((key) => key + 1)
    })
  }, [
    request,
    show,
    ensureFork,
    engine,
    phone,
    onSheetOpenChange,
    openExistingSideChat,
    openNewSideChat,
  ])

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

  // On a phone the clock says what the dropped "Temporary" row did.
  const SideChatIcon = phone ? Clock : MessagesSquare
  const tabs = (
    <Tabs
      value={tab}
      onValueChange={(next) => show(next as AsideTab)}
      className="min-w-0 flex-1 self-stretch font-inter"
    >
      <TabsList
        variant="line"
        aria-label="Context pane"
        className="w-full justify-start gap-1 p-0 group-data-horizontal/tabs:h-full"
      >
        <TabsTrigger value="context" className={ASIDE_TAB}>
          <Layers aria-hidden className="size-4" />
          Context
          {itemCount > 0 ? (
            <span
              data-context-count=""
              data-keep-mono=""
              className="font-mono text-meta font-normal text-ink-faint tabular-nums"
            >
              {itemCount}
              <span className="sr-only">
                {itemCount === 1 ? ' item' : ' items'}
              </span>
            </span>
          ) : null}
        </TabsTrigger>
        <TabsTrigger value="side-chat" className={ASIDE_TAB}>
          <SideChatIcon aria-hidden className="size-4" />
          Side chat
          {/* A reply that landed while the Context tab was up. */}
          {unread ? (
            <>
              <span
                aria-hidden
                data-side-chat-unread=""
                className="inline-block size-1.5 rounded-full bg-foreground"
              />
              <span className="sr-only">, new reply</span>
            </>
          ) : null}
        </TabsTrigger>
      </TabsList>
    </Tabs>
  )

  // The engine side chat on screen (or opening): the pane's side-chat view.
  const sideOpen = engine && (openId !== null || opening)
  const sectionIds = useContextSections().map((section) => section.id)
  const allFolded = allGroupsFolded(useFoldedGroups(), sectionIds)
  const foldAll = (engine ? !sideOpen : tab === 'context') ? (
    <IconButton
      label={allFolded ? 'Expand all groups' : 'Collapse all groups'}
      className="size-7 shrink-0"
      onClick={() => foldAllGroups(!allFolded, sectionIds)}
    >
      {allFolded ? (
        <ChevronsUpDown aria-hidden />
      ) : (
        <ChevronsDownUp aria-hidden />
      )}
    </IconButton>
  ) : null

  const contextPanel = (
    <ChatContextPanel
      sideChats={
        engine
          ? {
              rows: sideRows,
              opening,
              onOpen: (id) => {
                if (phone) onSheetOpenChange(false)
                openExistingSideChat(id)
              },
              onNew: () => {
                if (phone) onSheetOpenChange(false)
                void openNewSideChat({})
              },
              onKeep: (id) => void keepSide(id),
              onDelete: setConfirmDeleteId,
            }
          : undefined
      }
      conversation={conversation}
      conversations={ctx.conversations}
      connectionState={ctx.connectionState}
      observers={observers}
      observerEvents={observerEvents}
      observersListed={observersListed}
      onDisposeObserver={onDisposeObserver}
      onOpenConversation={ctx.openConversationInPanel}
      shellAvailable={ctx.shellAvailable}
      githubAvailable={ctx.githubAvailable}
      kanbanAvailable={ctx.kanbanAvailable}
      onAskAgent={onAskAgent}
      onAutoFixChange={(key, seen) => {
        const { [key]: _old, ...rest } = autoFixState(conversation)
        ctx.setSessionMetadata(conversation.id, {
          [AUTO_FIX_KEY]: seen ? { ...rest, [key]: seen } : rest,
        })
      }}
      onDismissPullRequest={(key) =>
        ctx.setSessionMetadata(conversation.id, {
          [DISMISSED_PRS_KEY]: [...dismissedPullRequests(conversation), key],
        })
      }
      landBlocked={landBlocked}
      active={visible}
      items={contextItems}
      phone={phone}
      onShowInConversation={
        onShowInConversation
          ? (target) => {
              if (!phone || !sheetOpen) {
                onShowInConversation(target)
                return
              }
              // On a phone the drawer closes first (§1.3.6).
              pendingShowRef.current = target
              if (pendingShowTimerRef.current !== null)
                window.clearTimeout(pendingShowTimerRef.current)
              pendingShowTimerRef.current = window.setTimeout(
                landPendingShow,
                DRAWER_CLOSE_FALLBACK_MS,
              )
              onSheetOpenChange(false)
            }
          : undefined
      }
    />
  )
  const content =
    tab === 'context' ? (
      contextPanel
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
            {phone ? null : (
              <SideChatBar
                since={since}
                keeping={keeping}
                onKeep={() => void keep()}
                onDiscard={() =>
                  own.some((message) => message.role === 'user')
                    ? setConfirmDiscard(true)
                    : discard()
                }
              />
            )}
            <ChatView
              key={`${fork.id}:${composerKey}`}
              conversation={fork}
              side={{
                since,
                fromEntry: from,
                empty: <SideChatIntro since={since} />,
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
                compact: phone,
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
          <div className="flex flex-1 flex-col overflow-y-auto px-3 pt-3 pb-4">
            <SideChatIntro
              action={{
                label: 'Start side chat',
                onClick: () => void ensureFork(),
              }}
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

  // ── the engine side chats' views (chat-projects §2.3.2) ──
  // The pane widens to 460 while a side chat is open, when the chat keeps
  // its 480 beside it (§0.5); the drawer beside a narrow chat keeps its own.
  const [hostWidth, setHostWidth] = useState(0)
  useEffect(() => {
    if (phone || !engine) return
    const host = contentRef.current?.closest('aside')?.parentElement
    if (!host) return
    const update = () => setHostWidth(host.getBoundingClientRect().width)
    update()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(update)
    observer.observe(host)
    return () => observer.disconnect()
  }, [phone, engine])
  const widen =
    sideOpen && !phone && hostWidth >= SIDE_CHAT_PANE_WIDTH + MIN_CHAT_WIDTH

  // Where it was forked, for the strip: the conversation's own entry when
  // it is loaded here, else the side chat's copy of it.
  const forkPoint = sideChatForkPoint(openChat)
  const forkEntry =
    (forkPoint
      ? [...conversation.messages]
          .reverse()
          .find((m) => belongsToEntry(m.id, forkPoint.entryId))
      : undefined) ??
    (openChat && openFrom
      ? [...openChat.messages]
          .reverse()
          .find((m) => belongsToEntry(m.id, openFrom))
      : undefined)
  const lock = openChat
    ? lockPresentation(
        sideChatLock(openChat),
        {
          model: openChat.model ?? conversation.model,
          thinkingLevel:
            openChat.thinkingLevel ?? conversation.thinkingLevel ?? null,
        },
        ctx.modelOptions,
      )
    : null
  const sideMenu = (id: string) => (
    <SideChatMenu
      busy={busyId === id}
      onSummarize={() => void summarizeSide(id)}
      onKeep={() => void keepSide(id)}
      onDelete={() => deleteSide(id)}
      size={phone ? 'page' : 'default'}
    />
  )
  const sideErrorPanel = sideError ? (
    <StatusPanel
      variant="alert"
      role="alert"
      className="mx-2 mb-2"
      headline={sideError.headline}
      detail={sideError.detail}
    />
  ) : null
  const sideView = sideOpen ? (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-side-chat-view={openId ?? ''}
    >
      {sideErrorPanel}
      {openChat && lock ? (
        <>
          <SideChatForkStrip
            text={forkStripText({
              at: forkEntry?.createdAt ?? openSince,
              fromUser: forkEntry?.role === 'user',
              parentTitle: conversation.title,
              compact: phone,
            })}
          />
          <ChatView
            key={`${openChat.id}:${composerKey}`}
            conversation={openChat}
            side={{
              since: openSince,
              fromEntry: openFrom,
              empty: null,
              autoFocus: focusSide,
              quote: pendingQuote,
              onRemoveQuote: () => onPendingQuoteChange(null),
              onQuoteSent: (quote) => {
                ctx.setSessionMetadata(openChat.id, {
                  [SIDE_CHAT_QUOTES_KEY]: withSideChatQuote(
                    sideChatQuotes(openChat),
                    quote,
                  ),
                })
                onPendingQuoteChange(null)
              },
              quotes: sideChatQuotes(openChat),
              onJumpToQuote: (quote) => onJumpToMessage(quote.messageId),
              // A page of its own on a phone: the full composer row, where
              // the locked model label sits (§2.3.2).
              compact: false,
              locked: {
                label: phone ? lock.short : lock.label,
                ...(lock.model ? { model: lock.model } : {}),
              },
              placeholder: 'Ask aside…',
              metaLine: phone ? null : lock.meta,
              summaries: sideChatSummaries(openChat),
              onBringBack: bringBack,
              bringBackLabel: phone
                ? `Bring back to ${conversation.title}`
                : 'Bring back',
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
        </>
      ) : (
        <div className="flex flex-col gap-2 px-4 py-3" aria-busy="true">
          <Skeleton className="h-3 w-48" />
          <Skeleton className="h-3 w-32" />
        </div>
      )}
    </div>
  ) : null
  const listView = (
    <div className="flex min-h-0 flex-1 flex-col">
      {sideOpen ? null : sideErrorPanel}
      {kept ? (
        <StatusPanel
          variant="success"
          role="status"
          className="mx-2 mb-2"
          headline="Kept as a conversation"
          detail="It is in the list with the other chats, in this chat's project."
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
      {contextPanel}
    </div>
  )
  // The pane's title without the tabs: the engine's side chats live in the
  // Context pane, so there is one view to name.
  const contextHeading = (
    <div
      data-context-heading=""
      className="flex min-w-0 flex-1 items-center gap-1.5 self-stretch px-2 font-inter text-body font-medium text-foreground"
    >
      <Layers aria-hidden className="size-4" />
      Context
      {itemCount > 0 ? (
        <span
          data-context-count=""
          data-keep-mono=""
          className="font-mono text-meta font-normal text-ink-faint tabular-nums"
        >
          {itemCount}
          <span className="sr-only">
            {itemCount === 1 ? ' item' : ' items'}
          </span>
        </span>
      ) : null}
    </div>
  )
  const sideExtras = engine ? (
    <>
      <ConfirmSideChatDelete
        open={confirmDeleteId !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmDeleteId(null)
        }}
        onConfirm={() => {
          if (confirmDeleteId) deleteSide(confirmDeleteId)
        }}
      />
      <SideChatToast toast={toast} onDismiss={() => setToast(null)} />
    </>
  ) : null

  if (phone) {
    return (
      <>
        <PhonePanel
          open={sheetOpen}
          onOpenChange={onSheetOpenChange}
          onClosed={landPendingShow}
          title={conversation.title}
          tabs={engine ? contextHeading : tabs}
          options={
            engine ? (
              <ContextSectionsMenu phone />
            ) : tab === 'side-chat' && fork ? (
              <SideChatOptions
                since={since}
                keeping={keeping}
                onKeep={() => void keep()}
                onDiscard={() =>
                  own.some((message) => message.role === 'user')
                    ? setConfirmDiscard(true)
                    : discard()
                }
              />
            ) : tab === 'context' ? (
              <ContextSectionsMenu phone />
            ) : null
          }
        >
          {engine ? listView : content}
        </PhonePanel>
        {engine ? (
          <SideChatPage
            open={sideOpen}
            onBack={leaveSideChat}
            parentTitle={conversation.title}
            title={openChat?.title ?? 'Side chat'}
            menu={openChat ? sideMenu(openChat.id) : null}
          >
            {sideView}
          </SideChatPage>
        ) : null}
        {sideExtras}
      </>
    )
  }

  return (
    <>
      <PageSidebar
        label="Context pane"
        side={side}
        // A hairline where the panel meets the chat, on the edge it shares.
        className={
          side === 'left' ? 'border-r border-edge' : 'border-l border-edge'
        }
        storageKey={ASIDE_STORAGE_KEY}
        defaultWidth={340}
        minWidth={280}
        maxWidth={480}
        width={widen ? SIDE_CHAT_PANE_WIDTH : undefined}
        collapsible
        resizable
        // Inline only while the chat keeps its own 560px beside the panel's 340.
        narrowBelow={900}
        narrowMode="drawer"
        closeDrawerOnSelect={false}
        expandRequest={expandSeq}
        collapseRequest={hideRequest}
        // Beside a narrow chat the drawer opens under the chat's 44px header
        // (PageHeader beside a pane), so the pane button that opened it, now
        // "Hide context pane", stays in reach to close it.
        drawerOffsetTop={CHAT_HEADER_PX}
        header={
          engine ? (
            sideOpen ? (
              <SideChatHeader
                title={openChat?.title ?? 'Side chat'}
                onBack={leaveSideChat}
                menu={openChat ? sideMenu(openChat.id) : null}
              />
            ) : (
              <>
                {contextHeading}
                <ContextSectionsMenu phone={false} />
                {foldAll}
              </>
            )
          ) : (
            <>
              {tabs}
              {tab === 'context' ? <ContextSectionsMenu phone={false} /> : null}
              {foldAll}
            </>
          )
        }
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
            {engine ? null : (
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
            )}
          </>
        }
      >
        <div ref={contentRef} className="flex min-h-0 flex-1 flex-col">
          {engine ? (sideView ?? listView) : content}
        </div>
      </PageSidebar>
      {sideExtras}
    </>
  )
}

/**
 * The desktop side chat's top strip: what this chat is (temporary, with the
 * lifetime behind the badge), how far back it sees, and Keep / Discard.
 */
function SideChatBar({
  since,
  keeping,
  onKeep,
  onDiscard,
}: {
  since: number
  keeping: boolean
  onKeep: () => void
  onDiscard: () => void
}) {
  return (
    <div
      className="flex h-10 shrink-0 items-center gap-2 border-b border-edge pr-1.5 pl-3 font-inter"
      data-side-chat-bar=""
    >
      <Tooltip>
        <TooltipTrigger
          render={
            <Badge
              variant="outline"
              render={<button type="button" />}
              className="h-6 cursor-default gap-1.5 rounded-md px-1.5 text-xs text-foreground [&>svg]:size-4!"
            />
          }
        >
          <Clock aria-hidden />
          Temporary
        </TooltipTrigger>
        <TooltipContent align="start">{SIDE_CHAT_LIFETIME}</TooltipContent>
      </Tooltip>
      <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
        Sees up to{' '}
        <time
          dateTime={new Date(since).toISOString()}
          className="font-geist-mono tabular-nums text-foreground"
        >
          {clockTime(since)}
        </time>
      </span>
      <Hint label="Keep as conversation">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Keep as conversation"
          className="text-muted-foreground"
          disabled={keeping}
          onClick={onKeep}
        >
          <BookmarkPlus aria-hidden />
        </Button>
      </Hint>
      <Hint label="Discard side chat">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Discard side chat"
          className="text-muted-foreground hover:text-destructive"
          onClick={onDiscard}
        >
          <Trash2 aria-hidden />
        </Button>
      </Hint>
    </div>
  )
}

/**
 * What a side chat is, where its transcript would be. With `action`, the
 * chat has not started yet and the card's footer starts it; with `since`,
 * the first fact says how far back it sees.
 */
function SideChatIntro({
  since,
  action,
}: {
  since?: number
  action?: { label: string; onClick: () => void }
}) {
  const facts: { icon: typeof History; label: string; value: string }[] = [
    {
      icon: History,
      label: 'Context',
      value:
        since === undefined
          ? 'This chat, up to now'
          : `This chat, up to ${clockTime(since)}`,
    },
    {
      icon: SquareFunction,
      label: 'Functions',
      value: 'Same functions and approvals',
    },
    {
      icon: MessageSquareOff,
      label: 'Main chat',
      value: 'Nothing lands there',
    },
    {
      icon: Timer,
      label: 'Lifetime',
      value: '2 hours unused, or with this chat',
    },
  ]
  return (
    <section
      aria-labelledby="side-chat-intro-title"
      data-side-chat-intro=""
      className="mx-auto flex w-full max-w-[26rem] flex-col items-center px-1 pt-6 pb-2 text-center font-inter"
    >
      {/* A quiet tile: a hairline ring over a faint wash, no fill to shout. */}
      <span
        aria-hidden
        className="flex size-10 items-center justify-center rounded-lg bg-gradient-to-b from-foreground/[0.07] to-foreground/[0.02] text-foreground ring-1 ring-card-edge"
      >
        <MessagesSquare className="size-5" />
      </span>
      <h2
        id="side-chat-intro-title"
        className="mt-4 text-[15px] font-medium tracking-[-0.01em] text-foreground"
      >
        Ask on the side
      </h2>
      <p className="mt-1.5 text-sm text-balance text-muted-foreground">
        A scratch chat beside this one, for the question you don't want in the
        main thread.
      </p>
      <dl className="mt-5 w-full divide-y divide-foreground/[0.07] rounded-xl bg-card text-left ring-1 ring-card-edge">
        {facts.map(({ icon: Icon, label, value }) => (
          <div key={label} className="flex items-center gap-3 px-3.5 py-2.5">
            <dt className="flex w-24 shrink-0 items-center gap-2 text-xs font-medium text-muted-foreground">
              <Icon aria-hidden className="size-4 shrink-0" />
              {label}
            </dt>
            <dd className="min-w-0 flex-1 truncate text-sm text-foreground">
              {value}
            </dd>
          </div>
        ))}
      </dl>
      {action ? (
        <Button
          variant="secondary"
          size="sm"
          className="mt-5"
          onClick={action.onClick}
        >
          {action.label}
        </Button>
      ) : null}
    </section>
  )
}

/** The quote waiting in a side chat's composer, removable before it is sent. */
export function QuoteChip({
  quote,
  onRemove,
  compact = false,
}: {
  quote: SideChatQuote
  onRemove?: () => void
  /** One line on a phone, its source in the accessible name. */
  compact?: boolean
}) {
  if (compact) {
    return (
      <div
        className="flex h-10 items-center gap-2 border-b border-edge bg-muted/40 pl-3 font-inter"
        data-side-chat-quote=""
      >
        <MessageSquareQuote
          aria-hidden
          className="size-4 shrink-0 text-muted-foreground"
        />
        <span className="sr-only">Quoted · {quote.agent || 'Agent'}: </span>
        <span className="min-w-0 flex-1 truncate text-body text-muted-foreground">
          {quote.text}
        </span>
        {onRemove ? (
          <IconButton
            label="Remove quote"
            className="size-10 shrink-0"
            onClick={onRemove}
          >
            <X aria-hidden />
          </IconButton>
        ) : null}
      </div>
    )
  }
  return (
    <div
      className="flex items-start gap-2 border-b border-edge bg-muted/40 py-2 pr-1.5 pl-3 font-inter"
      data-side-chat-quote=""
    >
      <MessageSquareQuote
        aria-hidden
        className="mt-0.5 size-4 shrink-0 text-muted-foreground"
      />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-xs font-medium text-muted-foreground">
          Quoted · {quote.agent || 'Agent'}
        </span>
        <span className="line-clamp-2 text-body leading-[18px] text-foreground">
          {quote.text}
        </span>
      </div>
      {onRemove ? (
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Remove quote"
          className="-my-0.5 shrink-0 text-muted-foreground"
          onClick={onRemove}
        >
          <X aria-hidden />
        </Button>
      ) : null}
    </div>
  )
}

/** Half the toolbar's width, to keep it on screen near the edges. */
const TOOLBAR_HALF = 112
const toolbarButton =
  'inline-flex h-7 items-center gap-1.5 rounded-md px-2 font-sans text-body text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50 [&>svg]:size-4 [&>svg]:shrink-0'

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

/**
 * The side panel on a phone (max-width 639px): a drawer from the right
 * (chat-projects §0.5), min(screen width − 48, 380) wide on the popover
 * surface over the sheet scrim. It slides in over 220 ms (none with reduced
 * motion) and closes on a tap on the scrim, a swipe to the right, Escape or
 * its close button. It names the conversation above the tabs, and the
 * Context tab carries the item count. It follows the visual viewport, so a
 * raised keyboard shrinks it instead of covering the side chat's composer.
 */
function PhonePanel({
  open,
  onOpenChange,
  onClosed,
  title,
  tabs,
  options,
  children,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The close has finished: its slide out is over, the drawer is gone. */
  onClosed?: () => void
  title: string
  tabs: ReactNode
  options: ReactNode
  children: ReactNode
}) {
  return (
    <Sheet
      open={open}
      onOpenChange={(next) => onOpenChange(next)}
      onOpenChangeComplete={(next) => {
        if (!next) onClosed?.()
      }}
    >
      <SheetPortal>
        <PortalScope>
          <SheetScrim />
          <PhonePanelContent
            title={title}
            tabs={tabs}
            options={options}
            onClose={() => onOpenChange(false)}
          >
            {children}
          </PhonePanelContent>
        </PortalScope>
      </SheetPortal>
    </Sheet>
  )
}

/** How far a finger travels right before the drawer follows it shut. */
const SWIPE_CLOSE_PX = 64

function PhonePanelContent({
  title,
  tabs,
  options,
  onClose,
  children,
}: {
  title: string
  tabs: ReactNode
  options: ReactNode
  onClose: () => void
  children: ReactNode
}) {
  // Mounted only while open, so the fit starts with the panel on screen.
  const ref = useRef<HTMLDivElement>(null)
  useVisualViewportFit(ref)
  const swipe = useRef<{ x: number; y: number; id: number } | null>(null)
  return (
    <DialogPrimitive.Popup
      ref={ref}
      aria-describedby={undefined}
      // The drawer's accessible name; nothing above the conversation's
      // name is shown (§1.3.6: the name, then "Context" and the count).
      aria-label="Context"
      data-side-panel-phone=""
      data-side-panel-drawer=""
      onKeyDown={(event) => {
        // The drawer is portaled out of the chat's workspace pane, where
        // the pane's own commands listen: its shortcut closes it here.
        if (bindingMatchesEvent(CONTEXT_PANE_SHORTCUT, event)) {
          event.preventDefault()
          onClose()
        }
      }}
      onPointerDown={(event) => {
        if (event.pointerType === 'mouse') return
        swipe.current = {
          x: event.clientX,
          y: event.clientY,
          id: event.pointerId,
        }
      }}
      onPointerUp={(event) => {
        const start = swipe.current
        swipe.current = null
        if (!start || start.id !== event.pointerId) return
        const dx = event.clientX - start.x
        const dy = Math.abs(event.clientY - start.y)
        if (dx > SWIPE_CLOSE_PX && dx > dy * 1.5) onClose()
      }}
      onPointerCancel={() => {
        swipe.current = null
      }}
      className="fixed top-0 right-0 z-50 flex h-dvh w-[min(calc(100vw-48px),380px)] flex-col bg-popover font-sans text-ink shadow-floating transition-transform duration-(--motion-duration-panel) ease-(--motion-ease-enter) focus:outline-none side-panel-safe-area data-ending-style:translate-x-full data-starting-style:translate-x-full motion-reduce:transition-none"
    >
      <div className="flex h-12 shrink-0 items-center gap-1 pr-1 pl-4">
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">
          {title}
        </span>
        <SheetClose
          render={
            <IconButton
              label="Close the context pane"
              className="size-11 shrink-0"
            >
              <X aria-hidden />
            </IconButton>
          }
        />
      </div>
      <div className="flex h-11 shrink-0 items-center gap-1 px-1">
        <div className="flex min-w-0 flex-1 self-stretch">{tabs}</div>
        {options ?? null}
      </div>
      <div className="flex min-h-0 flex-1 flex-col">{children}</div>
    </DialogPrimitive.Popup>
  )
}

/**
 * The side chat's options on a phone, behind ⋮: what the desktop shows in
 * its "Temporary" row and footer (when it ends) plus Keep and Discard.
 */
function SideChatOptions({
  since,
  keeping,
  onKeep,
  onDiscard,
}: {
  since: number
  keeping: boolean
  onKeep: () => void
  onDiscard: () => void
}) {
  const [open, setOpen] = useState(false)
  const row =
    'flex h-12 w-full items-center gap-3 rounded-md px-3 text-left text-base hover:bg-surface-hover focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50 disabled:opacity-50 [&>svg]:size-[18px] [&>svg]:shrink-0'
  return (
    <>
      <IconButton
        label="Side chat options"
        className="size-11 shrink-0"
        onClick={() => setOpen(true)}
      >
        <EllipsisVertical aria-hidden />
      </IconButton>
      <BottomSheet open={open} onOpenChange={setOpen}>
        <BottomSheetContent
          heading="Temporary side chat"
          description={`Sees the chat up to ${clockTime(since)}. ${SIDE_CHAT_LIFETIME}`}
          className="mx-auto max-w-[460px] font-sans"
        >
          <div className="flex flex-col gap-1 px-2 pb-1">
            <button
              type="button"
              className={row}
              disabled={keeping}
              onClick={() => {
                setOpen(false)
                onKeep()
              }}
            >
              <BookmarkPlus aria-hidden className="text-ink-faint" />
              Keep as conversation
            </button>
            <button
              type="button"
              className={`${row} text-alert`}
              onClick={() => {
                setOpen(false)
                onDiscard()
              }}
            >
              <Trash2 aria-hidden />
              Discard side chat…
            </button>
          </div>
        </BottomSheetContent>
      </BottomSheet>
    </>
  )
}
