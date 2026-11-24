import { formatRelative } from '@iii-dev/console-ui/format'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import {
  BookmarkPlus,
  ChevronDown,
  ChevronRight,
  ChevronsUp,
  ChevronUp,
  Copy,
  EllipsisVertical,
  ExternalLink,
  EyeOff,
  FileCode,
  FileDiff,
  FileIcon,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  GitBranch,
  GitPullRequest,
  GitPullRequestCreate,
  GitPullRequestDraft,
  Globe,
  ImageIcon,
  Layers,
  MessagesSquare,
  Plus,
  SquareTerminal,
  Ticket,
  Trash2,
  TriangleAlert,
  X,
} from 'lucide-react'
import {
  type ButtonHTMLAttributes,
  type CSSProperties,
  createContext,
  Fragment,
  forwardRef,
  type ReactNode,
  useContext,
  useEffect,
  useId,
  useMemo,
  useState,
} from 'react'
import { Button } from '@/components/ui/Button'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { EmptyState } from '@/components/ui/EmptyState'
import { Hint } from '@/components/ui/Hint'
import {
  ImageThumbnailButton,
  ImageViewer,
  useImageViewer,
} from '@/components/ui/ImageViewer'
import { LiveRegion } from '@/components/ui/LiveRegion'
import { ObserverIcon } from '@/components/ui/ObserverIcon'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { useLazyAttachment } from '@/hooks/use-lazy-attachment'
import { useLiveAnnouncer } from '@/hooks/use-live-announcer'
import {
  type AutoFixSeen,
  autoFixBaseline,
  autoFixState,
  ghViewer,
  readPrActivity,
} from '@/lib/auto-fix'
import type { ContextItems } from '@/lib/backend/context-items'
import {
  firedGhostRow,
  type SessionObserverInfo,
} from '@/lib/backend/observers'
import {
  type ChatFile,
  capRows,
  collectChatFiles,
  collectChatImageSources,
  collectChatLinks,
  collectChatTickets,
  createPullRequestPrompt,
  dismissedPullRequests,
  folderGh,
  type GhRunner,
  type GitSummary,
  ghSource,
  githubWorkerGh,
  isDefaultBranch,
  type KanbanBoard,
  linkSourceMessages,
  type PullRequestAction,
  type PullRequestRef,
  type PullRequestRow,
  pullRequestKey,
  readGitSummary,
  readKanbanBoard,
  readPullRequests,
  runPullRequestAction,
  type SideChatRow,
  sideChatRowMeta,
  type TicketRow,
  ticketRows,
} from '@/lib/chat-aside'
import { copyTextToClipboard } from '@/lib/clipboard'
import {
  moveSection,
  sectionSource,
  toggleGroup,
  useContextSections,
  useFoldedGroups,
  useHiddenSections,
} from '@/lib/context-groups'
import { openChatFile } from '@/lib/file-navigation'
import { getIiiClient, type IIIConnectionState } from '@/lib/iii-client'
import { requestPanelOpen } from '@/lib/panel-context'
import { entryIdOfMessage } from '@/lib/sessions/entry-mapper'
import { shortAge } from '@/lib/short-age'
import {
  type RegisteredContextSection,
  useExtContextSections,
} from '@/lib/ui-slots'
import { cn } from '@/lib/utils'
import type { Attachment, Conversation, SystemMessage } from '@/types/chat'
import type { ContextSectionSummary } from '@/types/injectable-ui'
import { SUBAGENT_ICON_COMPONENTS } from './ActiveSubagentChips'
import {
  downloadStoredAttachment,
  formatSize,
  isLazyImage,
} from './AttachmentChip'
import {
  collectSubagentDescendants,
  deriveSubagentVisualStatus,
  resolveSubagentAppearance,
  type SubagentVisualStatus,
} from './active-subagents'
import { ContextTrackingBody, useContextTracking } from './ContextTracking'
import {
  attachmentSize,
  copyLinkEntry,
  fileIconFor,
  type GridImage,
  ImageGrid,
  ItemMenu,
  type ItemMenuEntry,
  menuIcons,
  shareEntry,
  subagentStatusText,
} from './context-items-rows'
import {
  ObserverDetailDialog,
  ObserverStatus,
  observerTrigger,
} from './ObserverDetailDialog'
import { ReorderHandle, type ReorderTarget } from './ReorderHandle'

/**
 * Coloured trailing text is a plain span: the tree recipe's `__meta` class
 * sets its own colour unlayered, so a colour utility on it would lose.
 */
const deltaClassName = 'font-mono text-meta leading-4 tabular-nums'

/** Sub-agents are what people come here for; every other section shows less. */
const SUBAGENT_CAP = 5
const SECTION_CAP = 3
/** One row of the four-across strip: three pictures and a "+N". */
const IMAGE_CAP = 3

interface ChatContextPanelProps {
  conversation: Conversation
  conversations: readonly Conversation[]
  connectionState: IIIConnectionState
  /** The session's observer rows (ChatView's merged list); inactive ones are skipped. */
  observers: readonly SessionObserverInfo[]
  /**
   * What the observers did in this chat, oldest first: the events a roll-up
   * in the transcript counts. Listed even after their observer has ended.
   */
  observerEvents?: readonly SystemMessage[]
  /** The observer list has loaded: an observer missing from it has ended. */
  observersListed?: boolean
  onDisposeObserver: (observerId: string) => Promise<void> | void
  onOpenConversation: (sessionId: string) => void
  /** git is read through `shell::exec`, so the folder's state needs that worker. */
  shellAvailable: boolean
  githubAvailable: boolean
  /** The `kanban` worker is connected: the board names the tickets. */
  kanbanAvailable?: boolean
  /** The managed worktree's land is blocked on conflicts. */
  landBlocked?: boolean
  /** Whether the panel is in sight; git and GitHub are read only then. */
  active?: boolean
  /**
   * Hand the chat's agent an instruction: sent at once, or left in this
   * chat's composer for the person to send.
   */
  onAskAgent?: (text: string, send: boolean) => void
  /** Stop listing a pull request in this chat's panel (`repo#number`). */
  onDismissPullRequest?: (key: string) => void
  /** Start (a baseline) or stop (null) auto-fix on a PR (`repo#number`). */
  onAutoFixChange?: (key: string, seen: AutoFixSeen | null) => void
  /**
   * What the engine lists for this conversation (`harness::context-items`,
   * chat-projects D10). Absent or null: the pane parses the transcript it
   * has loaded, as before.
   */
  items?: ContextItems | null
  /**
   * "Show in conversation": land the transcript on an entry (the engine's
   * lists) or on a loaded row (the local parse).
   */
  onShowInConversation?: (target: {
    entryId?: string
    messageId?: string
  }) => void
  /** The phone drawer: the filter chips under the header. */
  phone?: boolean
  /**
   * The engine's side chats (chat-projects §2.3): the Sidechats section,
   * first, with "+ New". Absent: no section (the pane's own side chat tab).
   */
  sideChats?: {
    rows: readonly SideChatRow[]
    onOpen: (id: string) => void
    onNew: () => void
    onKeep: (id: string) => void
    onDelete: (id: string) => void
    /** A side chat is being opened: "+ New" waits. */
    opening?: boolean
  }
}

/** A link row, from the engine or the local parse. */
interface LinkRow {
  url: string
  host: string
  /** The Markdown link text, when the engine has one. */
  title?: string
  entryId?: string
  messageId?: string
}

/** The phone drawer's filter chips (§1.3.6), and the sections each shows. */
type ContextFilter =
  | 'all'
  | 'sidechats'
  | 'subagents'
  | 'prs'
  | 'links'
  | 'attachments'
  | 'images'

const COMMON_SECTIONS: readonly Exclude<ContextFilter, 'all'>[] = [
  'sidechats',
  'subagents',
  'prs',
  'links',
  'attachments',
  'images',
]

const FILTER_LABELS: Record<Exclude<ContextFilter, 'all'>, string> = {
  sidechats: 'Side',
  subagents: 'Agents',
  prs: 'PRs',
  links: 'Links',
  attachments: 'Files',
  images: 'Images',
}

const PR_STATE_LABEL: Record<PullRequestRow['state'], string> = {
  open: 'Open',
  draft: 'Draft',
  merged: 'Merged',
  closed: 'Closed',
}

/** The pull request icon's tint by state (open, merged, closed; a draft stays quiet). */
const PR_STATE_COLOR: Record<PullRequestRow['state'], string | undefined> = {
  open: 'green',
  draft: undefined,
  merged: 'purple',
  closed: 'rose',
}

function openSourceControl(cwd: string) {
  requestPanelOpen({ pageId: 'ide', context: { type: 'source-control', cwd } })
}

function openExternal(url: string) {
  window.open(url, '_blank', 'noopener,noreferrer')
}

function basename(path: string): string {
  const trimmed = path.replace(/\/+$/, '')
  return trimmed.slice(trimmed.lastIndexOf('/') + 1) || trimmed
}

function parentDir(path: string): string {
  const trimmed = path.replace(/\/+$/, '')
  return trimmed.slice(0, trimmed.lastIndexOf('/')) || '/'
}

/**
 * The conversation's working context: its folder and branch, pull requests,
 * sub-agents, observers and the links it mentions. A section with nothing to
 * show is left out; a long one shows its first rows and a "Show N more".
 */
export function ChatContextPanel({
  conversation,
  conversations,
  connectionState,
  observers,
  observerEvents = [],
  observersListed = false,
  onDisposeObserver,
  onOpenConversation,
  shellAvailable,
  githubAvailable,
  kanbanAvailable = false,
  landBlocked,
  active = true,
  onAskAgent,
  onDismissPullRequest,
  onAutoFixChange,
  items = null,
  onShowInConversation,
  phone = false,
  sideChats,
}: ChatContextPanelProps) {
  const [filter, setFilter] = useState<ContextFilter>('all')
  const workingDir = conversation.workingDir ?? null
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})
  const toggle = (key: string) =>
    setExpanded((current) => ({ ...current, [key]: !current[key] }))

  // git, re-read when a turn ends (status changes) and when the window
  // regains focus: that is when the folder has most likely changed.
  const [git, setGit] = useState<{
    dir: string
    summary: GitSummary | null
  } | null>(null)
  const [focusTick, setFocusTick] = useState(0)
  useEffect(() => {
    const onFocus = () => setFocusTick((tick) => tick + 1)
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [])
  // biome-ignore lint/correctness/useExhaustiveDependencies: status and focusTick are re-read tokens, not inputs
  useEffect(() => {
    if (!active || !workingDir || !shellAvailable) return
    let alive = true
    void (async () => {
      const client = await getIiiClient()
      const summary = await readGitSummary(
        client.call.bind(client),
        workingDir,
      ).catch(() => null)
      if (alive) setGit({ dir: workingDir, summary })
    })()
    return () => {
      alive = false
    }
  }, [active, workingDir, shellAvailable, conversation.status, focusTick])
  const summary = git?.dir === workingDir ? git.summary : null
  const gitLoading = Boolean(
    workingDir && shellAvailable && git?.dir !== workingDir,
  )

  // What the conversation gathered: the engine's lists when it has
  // `harness::context-items`, else the parse of the loaded transcript.
  const fromEngine = items !== null
  const parsed = useMemo(
    () =>
      fromEngine
        ? null
        : {
            ...collectChatLinks(conversation.messages),
            sources: linkSourceMessages(conversation.messages),
          },
    [fromEngine, conversation.messages],
  )
  const links = useMemo<LinkRow[]>(
    () =>
      items
        ? items.links.map((link) => ({
            url: link.url,
            host: link.host,
            title: link.title,
            entryId: link.entry_id || undefined,
          }))
        : (parsed?.links ?? []).map((link) => ({
            url: link.url,
            host: link.host,
            messageId: parsed?.sources.get(link.url),
          })),
    [items, parsed],
  )
  const mentioned = useMemo<PullRequestRef[]>(
    () =>
      items
        ? items.pull_requests.map(({ repo, number, url }) => ({
            repo,
            number,
            url,
          }))
        : (parsed?.pullRequests ?? []),
    [items, parsed],
  )
  /** Where each pull request was named, for "Show in conversation". */
  const prOrigin = (
    pr: PullRequestRef,
  ): { entryId?: string; messageId?: string } | null => {
    const key = `${pr.repo}#${pr.number}`
    const fromItems = items?.pull_requests.find(
      (item) => `${item.repo}#${item.number}` === key,
    )
    if (fromItems?.entry_id) return { entryId: fromItems.entry_id }
    const messageId = parsed?.sources.get(key)
    return messageId ? { messageId } : null
  }
  const show = onShowInConversation
  const showEntry = (
    target: { entryId?: string; messageId?: string } | null,
  ): ItemMenuEntry[] =>
    show && target && (target.entryId || target.messageId)
      ? [
          {
            label: 'Show in conversation',
            icon: menuIcons.show,
            onSelect: () => show(target),
          },
        ]
      : []
  const mentionedKey = mentioned.map((pr) => `${pr.repo}#${pr.number}`).join()
  const repo = summary?.repo ?? null
  const branch = summary?.branch ?? null
  // `gh` for reads and actions: the github worker when it is there,
  // otherwise `gh` in the folder.
  const ghVia = githubAvailable
    ? 'worker'
    : shellAvailable && workingDir
      ? 'folder'
      : null
  const withGh = async (): Promise<GhRunner | null> => {
    if (!ghVia) return null
    const client = await getIiiClient()
    const call = client.call.bind(client)
    return ghVia === 'worker' || !workingDir
      ? githubWorkerGh(call)
      : folderGh(call, workingDir)
  }
  const [pullRequests, setPullRequests] = useState<PullRequestRow[]>([])
  /** The repo and branch whose PRs were read: no Create PR before that. */
  const [prsReadFor, setPrsReadFor] = useState<string | null>(null)
  const [prTick, setPrTick] = useState(0)
  // biome-ignore lint/correctness/useExhaustiveDependencies: mentionedKey stands for `mentioned`; status and prTick re-read
  useEffect(() => {
    if (!active) return
    if (!ghVia || (!(repo && branch) && mentioned.length === 0)) {
      setPullRequests([])
      return
    }
    let alive = true
    void (async () => {
      const gh = await withGh()
      const read = gh
        ? await readPullRequests(ghSource(gh), {
            repo,
            branch,
            mentioned,
          }).catch(() => null)
        : null
      if (!alive) return
      setPullRequests(read?.rows ?? [])
      // Only a read that answered for the branch says it has no PR.
      setPrsReadFor(read?.branchListed ? `${repo}@${branch}` : null)
    })()
    return () => {
      alive = false
    }
  }, [
    active,
    ghVia,
    workingDir,
    repo,
    branch,
    mentionedKey,
    conversation.status,
    prTick,
    focusTick,
  ])
  const sessionMetadata = conversation.sessionMetadata
  const dismissed = useMemo(
    () => new Set(dismissedPullRequests({ sessionMetadata })),
    [sessionMetadata],
  )
  // The engine's own reading of each PR (its state and checks come from the
  // github worker, cached 120 s): listed where this tab's gh read has no row
  // for it (no gh, or gh in the folder failed or is signed out).
  const engineRows = useMemo<PullRequestRow[]>(
    () =>
      !items
        ? []
        : items.pull_requests.map((pr) => ({
            repo: pr.repo,
            number: pr.number,
            url: pr.url,
            // No title read (no github worker): the repo, after the "#482".
            title: pr.title ?? pr.repo,
            state: pr.state ?? 'open',
            checks:
              pr.checks === 'passing'
                ? 'pass'
                : pr.checks === 'failing'
                  ? 'fail'
                  : pr.checks === 'pending'
                    ? 'pending'
                    : null,
            failing: pr.checks === 'failing' ? 1 : 0,
            additions: null,
            deletions: null,
            autoMerge: false,
            headRefName: null,
            mentioned: pr.source === 'mentioned',
          })),
    [items],
  )
  const readKeys = new Set(pullRequests.map(pullRequestKey))
  const listedPrs = [
    ...pullRequests,
    ...engineRows.filter((pr) => !readKeys.has(pullRequestKey(pr))),
  ].filter((pr) => !dismissed.has(pullRequestKey(pr)))
  /** Engine rows whose state nobody read (no github worker): no chip, no tint. */
  const unknownState = new Set(
    (items?.pull_requests ?? [])
      .filter((pr) => !pr.state && !readKeys.has(`${pr.repo}#${pr.number}`))
      .map((pr) => `${pr.repo}#${pr.number}`),
  )
  const [prBusy, setPrBusy] = useState<Record<string, boolean>>({})
  const busy = (pr: PullRequestRow, on: boolean) =>
    setPrBusy((current) => ({ ...current, [pullRequestKey(pr)]: on }))
  const [prError, setPrError] = useState<string | null>(null)
  const actOnPr = async (pr: PullRequestRow, action: PullRequestAction) => {
    const gh = await withGh()
    if (!gh) return
    busy(pr, true)
    setPrError(null)
    try {
      const error = await runPullRequestAction(gh, pr, action)
      if (error) setPrError(`#${pr.number}: ${error}`)
      else setPrTick((tick) => tick + 1)
    } finally {
      busy(pr, false)
    }
  }
  // Auto-fix: the side panel reads the PR with gh while this chat is open
  // and tells the agent what changed (lib/auto-fix). Turning it on records
  // where it starts.
  const autoFixing = useMemo(
    () => autoFixState({ sessionMetadata }),
    [sessionMetadata],
  )
  const setAutoFix = async (pr: PullRequestRow, on: boolean) => {
    if (!onAutoFixChange) return
    if (!on) {
      onAutoFixChange(pullRequestKey(pr), null)
      return
    }
    const gh = await withGh()
    if (!gh) return
    busy(pr, true)
    try {
      const now = await readPrActivity(gh, pr, await ghViewer(gh))
      onAutoFixChange(pullRequestKey(pr), autoFixBaseline(pr, now))
    } finally {
      busy(pr, false)
    }
  }
  // The folder's branch has work and no pull request of its own yet.
  const canCreatePr = Boolean(
    onAskAgent &&
      workingDir &&
      repo &&
      branch &&
      !isDefaultBranch(branch) &&
      summary &&
      (summary.changed > 0 || summary.ahead > 0) &&
      prsReadFor === `${repo}@${branch}` &&
      !pullRequests.some(
        (pr) => !pr.mentioned && (pr.state === 'open' || pr.state === 'draft'),
      ),
  )

  const images = useMemo<GridImage[]>(() => {
    const showIn = (target: { entryId?: string; messageId?: string }) =>
      show && (target.entryId || target.messageId)
        ? [
            {
              label: 'Show in conversation',
              icon: menuIcons.show,
              onSelect: () => show(target),
            },
          ]
        : []
    if (items) {
      // An inline picture (no stored attachment) draws from the loaded
      // transcript when its entry is in the window; otherwise its tile
      // stays a placeholder that "Show in conversation" still reaches.
      const loaded = new Map<string, Attachment[]>()
      for (const source of collectChatImageSources(conversation.messages)) {
        const entry = entryIdOfMessage(source.messageId)
        loaded.set(entry, [...(loaded.get(entry) ?? []), source.image])
      }
      const used = new Map<string, number>()
      return items.images.map((image, index) => {
        let inline: Attachment | undefined
        if (!image.attachment_id) {
          const seen = used.get(image.entry_id) ?? 0
          inline = loaded
            .get(image.entry_id)
            ?.filter((a) => !a.attachmentId)
            ?.at(seen)
          used.set(image.entry_id, seen + 1)
        }
        const share = shareEntry(
          conversation.id,
          image.attachment_id,
          `image-${index + 1}`,
        )
        return {
          key: `${image.entry_id}:${image.block_index}:${index}`,
          direction: image.direction,
          image: inline,
          attachmentId: image.attachment_id,
          mime: image.mime,
          name: `Image ${index + 1}`,
          menu: [
            ...(share ? [share] : []),
            ...showIn({ entryId: image.entry_id || undefined }),
          ],
        }
      })
    }
    return collectChatImageSources(conversation.messages).map((source) => {
      const share = shareEntry(
        conversation.id,
        source.image.attachmentId,
        source.image.name,
      )
      return {
        key: `${source.messageId}:${source.image.id}`,
        direction: source.direction,
        image: source.image,
        name: source.image.name,
        menu: [
          ...(share ? [share] : []),
          ...showIn({ messageId: source.messageId }),
        ],
      }
    })
  }, [items, conversation.messages, conversation.id, show])
  const files = useMemo(
    () =>
      fromEngine ? [] : collectChatFiles(conversation.messages, workingDir),
    [fromEngine, conversation.messages, workingDir],
  )
  const attachmentCount = items ? items.attachments.length : files.length
  /** The loaded row a locally parsed file came from. */
  const fileSource = (file: ChatFile): string | undefined =>
    conversation.messages.find((message) =>
      file.attachment
        ? message.role === 'user' &&
          (message.attachments ?? []).some((a) => a.id === file.attachment?.id)
        : message.role === 'function-call' &&
          JSON.stringify(message.input ?? '').includes(file.path),
    )?.id

  const subagents = useMemo(() => {
    const { descendants } = collectSubagentDescendants(
      conversations,
      conversation.id,
    )
    return descendants.map(({ conversation: child }) => ({
      child,
      status: deriveSubagentVisualStatus(child, connectionState),
      appearance: resolveSubagentAppearance(child),
    }))
    // Tree order, not status order: a row sorted by status jumps the moment
    // its status changes, which is exactly when the person clicked it (the
    // chat opening loads it, and "disconnected" becomes "completed"). The
    // dot carries the status; the row stays where it was.
  }, [conversations, conversation.id, connectionState])
  // Children the engine knows of that this tab's list has not loaded.
  const engineSubagents = useMemo(
    () =>
      items
        ? items.subagents.filter(
            (item) =>
              !subagents.some(({ child }) => child.id === item.session_id),
          )
        : [],
    [items, subagents],
  )
  const subagentTotal = subagents.length + engineSubagents.length
  const activeCount = subagents.filter(
    (s) => subagentRank(s.status) === 0,
  ).length

  // Tickets: what the chat touched, named by the board. Read again when a
  // turn ends or a new ticket shows up in the chat.
  const touches = useMemo(
    () =>
      collectChatTickets(
        conversation.messages,
        subagents.map(({ child, appearance }) => ({
          name: appearance.name,
          title: child.title,
          at: child.updatedAt,
        })),
      ),
    [conversation.messages, subagents],
  )
  const touchKey = touches
    .map((touch) => touch.ref)
    .sort()
    .join()
  const [board, setBoard] = useState<KanbanBoard | null>()
  // biome-ignore lint/correctness/useExhaustiveDependencies: touchKey and status are re-read tokens
  useEffect(() => {
    if (!active || !kanbanAvailable || !touchKey) return
    let alive = true
    void (async () => {
      const client = await getIiiClient()
      const read = await readKanbanBoard(client.call.bind(client)).catch(
        () => null,
      )
      if (alive) setBoard(read)
    })()
    return () => {
      alive = false
    }
  }, [active, kanbanAvailable, touchKey, conversation.status])
  const tickets = useMemo(
    () => (board && kanbanAvailable ? ticketRows(touches, board) : []),
    [board, kanbanAvailable, touches],
  )
  // Only a kanban call is sure to be a ticket: a bare key may be UTF-8.
  const ticketsLoading =
    kanbanAvailable &&
    board === undefined &&
    touches.some((touch) => touch.action)

  const liveObservers = useMemo(
    () => observers.filter((observer) => !observer.fired),
    [observers],
  )
  // One row per observer, not per event: a standing observer checks many
  // times, and each check is a transcript event. Ended observers stay while
  // this chat holds their events, so the transcript's roll-up has a row here.
  const observerListing = useMemo(() => {
    const eventsOf = new Map<string, SystemMessage[]>()
    const latest = new Map<string, NonNullable<SystemMessage['observer']>>()
    for (const event of observerEvents) {
      const fired = event.observer
      if (!fired) continue
      const list = eventsOf.get(fired.observer_id) ?? []
      list.push(event)
      eventsOf.set(fired.observer_id, list)
      latest.set(fired.observer_id, fired)
    }
    const live = new Set(liveObservers.map((observer) => observer.id))
    // Ended when something says so: the loaded list leaves it out, or a
    // retirement record. Absent from a list still loading (or that failed)
    // says nothing, and such a row must not claim it ended.
    const ended = new Set<string>()
    const others = [...latest]
      .filter(([id]) => !live.has(id))
      .map(([id, fired]) => {
        const listed = observers.find((observer) => observer.id === id)
        if (observersListed || listed?.fired || fired.retired) ended.add(id)
        return listed ?? firedGhostRow(fired)
      })
    return { rows: [...liveObservers, ...others], eventsOf, ended, live }
  }, [observerEvents, liveObservers, observers, observersListed])
  const [openObserverId, setOpenObserverId] = useState<string | null>(null)
  // The sub-agent whose chat this panel opened last: its row keeps the
  // selected wash, so the person sees which one they are looking at.
  const [openSubagentId, setOpenSubagentId] = useState<string | null>(null)
  const [busyObserverId, setBusyObserverId] = useState<string | null>(null)
  const [confirmObserver, setConfirmObserver] =
    useState<SessionObserverInfo | null>(null)
  const dispose = async (id: string) => {
    setBusyObserverId(id)
    try {
      await onDisposeObserver(id)
      setOpenObserverId((current) => (current === id ? null : current))
    } finally {
      setBusyObserverId(null)
    }
  }
  const openObserver =
    observerListing.rows.find((observer) => observer.id === openObserverId) ??
    null

  const ticketRowsShown = capRows(tickets, SECTION_CAP, !!expanded.tickets)
  const prRows = capRows(listedPrs, SECTION_CAP, !!expanded.prs)
  const subagentRows = capRows(subagents, SUBAGENT_CAP, !!expanded.subagents)
  const observerRows = capRows(
    observerListing.rows,
    SECTION_CAP,
    !!expanded.observers,
  )
  const linkRows = capRows(links, SECTION_CAP, !!expanded.links)
  const imageRows = capRows(images, IMAGE_CAP, !!expanded.images)
  const fileRows = capRows(files, SECTION_CAP, !!expanded.files)
  const attachmentRows = capRows(
    items?.attachments ?? [],
    SECTION_CAP,
    !!expanded.files,
  )
  const ticketsWorked = tickets.filter((row) => row.action || row.by).length
  const failingPrs = listedPrs.filter((pr) => pr.failing > 0).length

  const tracking = useContextTracking({
    sessionId: conversation.id,
    workingDir,
    status: conversation.status,
    active,
    connectionState,
  })
  const order = useContextSections()
  const hiddenSections = useHiddenSections()
  const workerSections = useExtContextSections()
  const { announcement, announce } = useLiveAnnouncer()
  const onMove = (id: string, target: ReorderTarget) => {
    moveSection(
      order.map((section) => section.id),
      id,
      target,
    )
    const name = (key: string) =>
      order.find((section) => section.id === key)?.label ?? key
    announce(
      'before' in target
        ? `${name(id)}, now above ${name(target.before)}`
        : `${name(id)}, now below ${name(target.after)}`,
    )
  }

  // Each section by id, null while it has nothing to show. The order
  // kept in this browser places them; hidden ones are left out.
  const groups: Record<string, ReactNode> = {
    tracking: (
      <Section
        id="tracking"
        label="Tracking"
        count={tracking.items.length}
        brief={
          <span className="truncate text-ink-faint">
            {tracking.items[0]
              ? `${tracking.items[0].title}${tracking.items[0].progress !== null ? ` · ${tracking.items[0].progress}%` : ''}`
              : 'Ask in chat to add tracking'}
          </span>
        }
      >
        <ContextTrackingBody {...tracking} onAskAgent={onAskAgent} />
      </Section>
    ),
    project: (
      <Section
        id="project"
        label="Project"
        brief={
          workingDir ? (
            <span className="truncate font-mono text-ink-ghost">
              {branch ?? basename(workingDir)}
              {summary && summary.changed > 0
                ? ` · ${summary.changed} changed`
                : ''}
            </span>
          ) : null
        }
      >
        {workingDir ? (
          <>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Row
                  icon={<FolderOpen aria-hidden />}
                  label={basename(workingDir)}
                  suffix={parentDir(workingDir)}
                  title={workingDir}
                  ariaLabel={`Folder ${workingDir}`}
                  meta={<ChevronDown aria-hidden className="size-4" />}
                />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                <DropdownMenuItem
                  onSelect={() =>
                    requestPanelOpen({
                      pageId: 'ide',
                      context: {
                        type: 'agent-terminal',
                        cwd: workingDir,
                        command: '',
                      },
                    })
                  }
                >
                  <SquareTerminal
                    aria-hidden
                    className="size-4 text-ink-faint"
                  />
                  Open in terminal
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() => void copyTextToClipboard(workingDir)}
                >
                  <Copy aria-hidden className="size-4 text-ink-faint" />
                  Copy path
                </DropdownMenuItem>
                {repo ? (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      onSelect={() =>
                        openExternal(`https://github.com/${repo}`)
                      }
                    >
                      <ExternalLink
                        aria-hidden
                        className="size-4 text-ink-faint"
                      />
                      Open repository on GitHub
                    </DropdownMenuItem>
                  </>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
            {gitLoading ? (
              <div className="flex flex-col gap-2 px-3 py-1.5" aria-hidden>
                <Skeleton className="h-3 w-40" />
                <Skeleton className="h-3 w-28" />
              </div>
            ) : null}
            {branch ? (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Row
                    icon={<GitBranch aria-hidden />}
                    label={branch}
                    mono
                    ariaLabel={`Branch ${branch}`}
                    meta={
                      <>
                        {summary && summary.ahead > 0 ? (
                          <span className={uiClasses.treeItemMeta}>
                            {summary.ahead} ahead
                          </span>
                        ) : null}
                        <ChevronDown aria-hidden className="size-4" />
                      </>
                    }
                  />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start">
                  <DropdownMenuItem
                    onSelect={() => void copyTextToClipboard(branch)}
                  >
                    <Copy aria-hidden className="size-4 text-ink-faint" />
                    Copy branch name
                  </DropdownMenuItem>
                  {repo ? (
                    <DropdownMenuItem
                      onSelect={() =>
                        openExternal(
                          `https://github.com/${repo}/tree/${encodeURIComponent(branch)}`,
                        )
                      }
                    >
                      <ExternalLink
                        aria-hidden
                        className="size-4 text-ink-faint"
                      />
                      Open branch on GitHub
                    </DropdownMenuItem>
                  ) : null}
                </DropdownMenuContent>
              </DropdownMenu>
            ) : null}
            {summary && summary.changed > 0 ? (
              <Row
                icon={<FileDiff aria-hidden />}
                label={`${summary.changed} changed ${summary.changed === 1 ? 'file' : 'files'}`}
                title="Open the changes in the IDE"
                onActivate={() => openSourceControl(workingDir)}
                meta={
                  <>
                    {summary.added > 0 ? (
                      <span className={cn(deltaClassName, 'text-ok')}>
                        +{summary.added}
                      </span>
                    ) : null}
                    {summary.removed > 0 ? (
                      <span className={cn(deltaClassName, 'text-alert-strong')}>
                        −{summary.removed}
                      </span>
                    ) : null}
                    <ChevronRight aria-hidden className="size-4" />
                  </>
                }
              />
            ) : null}
            {canCreatePr && workingDir && repo && branch ? (
              <Row
                icon={<GitPullRequestCreate aria-hidden />}
                label="Create pull request"
                title="Ask the agent to commit, push and open a pull request"
                onActivate={() =>
                  onAskAgent?.(
                    createPullRequestPrompt({
                      folder: workingDir,
                      branch,
                      repo,
                    }),
                    false,
                  )
                }
                meta={
                  <span className={uiClasses.treeItemMeta}>draft in chat</span>
                }
              />
            ) : null}
          </>
        ) : (
          <EmptyState
            compact
            icon={FolderOpen}
            title="No folder"
            description="Pick a folder in the composer to see its branch and changes here."
          />
        )}
      </Section>
    ),
    tickets: ticketsLoading ? (
      <Section id="tickets" label="Tickets">
        <div className="flex flex-col gap-2 px-3 py-1.5" aria-hidden>
          <Skeleton className="h-3 w-44" />
          <Skeleton className="h-3 w-28" />
        </div>
      </Section>
    ) : tickets.length > 0 ? (
      <Section
        id="tickets"
        label="Tickets"
        count={tickets.length}
        brief={plural([
          [ticketsWorked, 'worked here', 'worked here'],
          [tickets.length - ticketsWorked, 'mentioned', 'mentioned'],
        ])}
      >
        {ticketRowsShown.shown.map((row) => (
          <TicketItem key={row.ticket.key} row={row} />
        ))}
        <ShowMore
          hidden={ticketRowsShown.hidden}
          expanded={!!expanded.tickets}
          total={tickets.length}
          cap={SECTION_CAP}
          onToggle={() => toggle('tickets')}
        />
      </Section>
    ) : null,
    prs:
      listedPrs.length > 0 ? (
        <Section
          id="prs"
          label="Pull requests"
          count={listedPrs.length}
          brief={
            failingPrs > 0 ? (
              <>
                <StatusDot tone="alert" />
                <span className="text-ink">{failingPrs} failing</span>
              </>
            ) : null
          }
        >
          {prRows.shown.map((pr) => {
            const stateRead = !unknownState.has(pullRequestKey(pr))
            return (
              <Row
                key={`${pr.repo}#${pr.number}`}
                icon={<GitPullRequest aria-hidden />}
                color={stateRead ? PR_STATE_COLOR[pr.state] : undefined}
                label={
                  <>
                    <span className="mr-1.5 font-mono text-xs text-ink-faint">
                      #{pr.number}
                    </span>
                    {pr.title}
                  </>
                }
                title={pr.url}
                ariaLabel={
                  stateRead
                    ? `#${pr.number} ${pr.title}, ${PR_STATE_LABEL[pr.state]}`
                    : `#${pr.number} ${pr.title}`
                }
                onActivate={() => openExternal(pr.url)}
                description={
                  stateRead ? (
                    <PullRequestFacts pr={pr} />
                  ) : (
                    // Without the github worker the engine reads no state
                    // or title (spec §1.2.4): where the PR came from.
                    <span className="truncate">
                      {pr.mentioned
                        ? 'Mentioned in the conversation'
                        : 'Created here'}
                    </span>
                  )
                }
                meta={
                  stateRead ? (
                    <span
                      data-pr-state={pr.state}
                      className="rounded-sm bg-surface px-1.5 text-meta leading-4 text-ink-faint"
                    >
                      {PR_STATE_LABEL[pr.state]}
                    </span>
                  ) : null
                }
                actions={
                  <PullRequestMenu
                    pr={pr}
                    onShow={
                      show && prOrigin(pr)
                        ? () => {
                            const origin = prOrigin(pr)
                            if (origin) show(origin)
                          }
                        : undefined
                    }
                    busy={!!prBusy[pullRequestKey(pr)]}
                    canWrite={ghVia !== null}
                    autoFix={pullRequestKey(pr) in autoFixing}
                    autoFixAvailable={
                      ghVia !== null && Boolean(onAutoFixChange && onAskAgent)
                    }
                    onViewChanges={() =>
                      workingDir && pr.headRefName === branch
                        ? openSourceControl(workingDir)
                        : openExternal(`${pr.url}/files`)
                    }
                    onAction={(action) => void actOnPr(pr, action)}
                    onAutoFix={(on) => void setAutoFix(pr, on)}
                    onDismiss={
                      onDismissPullRequest
                        ? () => onDismissPullRequest(pullRequestKey(pr))
                        : undefined
                    }
                  />
                }
              />
            )
          })}
          <ShowMore
            hidden={prRows.hidden}
            expanded={!!expanded.prs}
            total={listedPrs.length}
            cap={SECTION_CAP}
            onToggle={() => toggle('prs')}
          />
          {prError ? (
            <StatusPanel
              variant="alert"
              role="alert"
              className="mx-2 mt-1"
              headline="Pull request not changed"
              detail={prError}
              action={
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => setPrError(null)}
                >
                  Dismiss
                </Button>
              }
            />
          ) : null}
        </Section>
      ) : null,
    images:
      images.length > 0 ? (
        <Section
          id="images"
          label="Images"
          count={images.length}
          extra={
            <span className="flex items-center gap-1 text-meta font-normal text-ink-faint">
              <span aria-hidden>· ↑</span> Sent <span aria-hidden>· ↓</span>{' '}
              Received
            </span>
          }
        >
          <ImageGrid
            images={imageRows.shown}
            sessionId={conversation.id}
            hidden={imageRows.hidden}
            onShowMore={() => toggle('images')}
          />
          {/* The "+N" tile unfolds the pictures; this folds them back. */}
          {expanded.images ? (
            <ShowMore
              hidden={0}
              expanded
              total={images.length}
              cap={IMAGE_CAP}
              onToggle={() => toggle('images')}
            />
          ) : null}
        </Section>
      ) : null,
    attachments:
      attachmentCount > 0 ? (
        <Section id="attachments" label="Attachments" count={attachmentCount}>
          {items
            ? attachmentRows.shown.map((item) => {
                const Icon = fileIconFor(item.name, item.mime)
                const size = attachmentSize(item.size)
                const share = shareEntry(
                  conversation.id,
                  item.attachment_id,
                  item.name,
                )
                const open = item.attachment_id
                  ? () =>
                      void downloadStoredAttachment(
                        conversation.id,
                        item.attachment_id as string,
                        item.name,
                      ).catch(() => {})
                  : item.path
                    ? () => openChatFile({ path: item.path as string }, null)
                    : undefined
                return (
                  <Row
                    key={`${item.entry_id}:${item.attachment_id ?? item.path ?? item.name}`}
                    icon={<Icon aria-hidden />}
                    label={item.name}
                    title={item.path ?? item.name}
                    ariaLabel={`Open ${item.name}`}
                    onActivate={open}
                    meta={
                      size ? (
                        <span
                          className={cn(uiClasses.treeItemMeta, 'font-mono')}
                        >
                          {size}
                        </span>
                      ) : null
                    }
                    actions={
                      <ItemMenu
                        label={`Actions for ${item.name}`}
                        entries={[
                          ...(open
                            ? [
                                {
                                  label: 'Open',
                                  icon: menuIcons.open,
                                  onSelect: open,
                                },
                              ]
                            : []),
                          ...(share ? [share] : []),
                          ...showEntry({
                            entryId: item.entry_id || undefined,
                          }),
                        ]}
                      />
                    }
                  />
                )
              })
            : fileRows.shown.map((file) => (
                <FileRow
                  key={file.attachment ? file.attachment.id : file.path}
                  file={file}
                  sessionId={conversation.id}
                  menu={showEntry({ messageId: fileSource(file) })}
                />
              ))}
          <ShowMore
            hidden={items ? attachmentRows.hidden : fileRows.hidden}
            expanded={!!expanded.files}
            total={attachmentCount}
            cap={SECTION_CAP}
            onToggle={() => toggle('files')}
          />
        </Section>
      ) : null,
    sidechats: sideChats ? (
      <Section
        id="sidechats"
        label="Sidechats"
        count={sideChats.rows.length || undefined}
        extra={
          sideChats.rows.some((row) => row.running) ? (
            <span className="flex items-center gap-1.5 text-meta font-normal text-ink">
              <StatusDot tone="accent" pulse />
              {sideChats.rows.filter((row) => row.running).length} running
            </span>
          ) : null
        }
        action={
          <button
            type="button"
            data-side-chat-new=""
            disabled={sideChats.opening}
            onClick={sideChats.onNew}
            className="flex h-7 shrink-0 items-center gap-1 rounded-md px-1.5 font-inter text-xs text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:opacity-50 pointer-coarse:h-11"
          >
            <Plus aria-hidden className="size-4" />
            New
          </button>
        }
      >
        {sideChats.rows.length === 0 ? (
          <p className="px-2 py-1.5 font-inter text-xs text-ink-faint">
            No side chats yet. “Side chat” on a reply, or New, starts one.
          </p>
        ) : null}
        {sideChats.rows.map((row) => (
          <Row
            key={row.id}
            data-side-chat-row={row.id}
            icon={
              row.running ? (
                <StatusDot tone="accent" pulse />
              ) : (
                <MessagesSquare aria-hidden />
              )
            }
            label={row.title}
            title={row.title}
            ariaLabel={`Open side chat ${row.title}${row.running ? ' (running)' : ''}`}
            onActivate={() => sideChats.onOpen(row.id)}
            meta={
              <span className={uiClasses.treeItemMeta} data-keep-mono="">
                {sideChatRowMeta(row, (at) => shortAge(at))}
              </span>
            }
            actions={
              <ItemMenu
                label={`Actions for ${row.title}`}
                entries={[
                  {
                    label: 'Open',
                    icon: menuIcons.open,
                    onSelect: () => sideChats.onOpen(row.id),
                  },
                  {
                    label: 'Keep as a conversation',
                    icon: (
                      <BookmarkPlus
                        aria-hidden
                        className="size-4 text-ink-faint"
                      />
                    ),
                    onSelect: () => sideChats.onKeep(row.id),
                  },
                  {
                    label: 'Delete…',
                    icon: (
                      <Trash2 aria-hidden className="size-4 text-ink-faint" />
                    ),
                    onSelect: () => sideChats.onDelete(row.id),
                  },
                ]}
              />
            }
          />
        ))}
      </Section>
    ) : null,
    subagents:
      subagentTotal > 0 ? (
        <Section
          id="subagents"
          label="Subagents"
          count={subagentTotal}
          extra={
            activeCount > 0 ? (
              <span className="flex items-center gap-1.5 text-meta font-normal text-ink">
                <StatusDot tone="accent" pulse />
                {activeCount} active
              </span>
            ) : null
          }
        >
          {subagentRows.shown.map(({ child, status, appearance }) => {
            const Icon = SUBAGENT_ICON_COMPONENTS[appearance.icon]
            return (
              <Row
                key={child.id}
                icon={<Icon aria-hidden />}
                color={appearance.color}
                label={
                  <>
                    <span className="mr-1.5 font-mono text-xs text-ink-faint">
                      {appearance.name}
                    </span>
                    {child.title}
                  </>
                }
                title={child.title}
                ariaLabel={`Open ${appearance.name} in a new panel (${status})`}
                aria-current={openSubagentId === child.id ? 'true' : undefined}
                onActivate={() => {
                  setOpenSubagentId(child.id)
                  onOpenConversation(child.id)
                }}
                meta={<SubagentStatus status={status} at={child.updatedAt} />}
                actions={
                  <ItemMenu
                    label={`Actions for ${appearance.name}`}
                    entries={[
                      {
                        label: 'Open',
                        icon: menuIcons.open,
                        onSelect: () => {
                          setOpenSubagentId(child.id)
                          onOpenConversation(child.id)
                        },
                      },
                      ...showEntry({
                        messageId: conversation.messages.find(
                          (message) =>
                            message.role === 'function-call' &&
                            child.parentFunctionCallId !== undefined &&
                            message.functionCallId ===
                              child.parentFunctionCallId,
                        )?.id,
                      }),
                    ]}
                  />
                }
              />
            )
          })}
          {expanded.subagents || subagentRows.hidden === 0
            ? engineSubagents.map((item) => {
                const failed = /fail|error|cancel/.test(item.status)
                const running = /running|working|queued|waiting|started/.test(
                  item.status,
                )
                return (
                  <Row
                    key={item.session_id}
                    icon={
                      <StatusDot
                        tone={failed ? 'alert' : running ? 'warn' : 'ok'}
                        pulse={running}
                      />
                    }
                    label={
                      <>
                        {item.display ? (
                          <span className="mr-1.5 font-mono text-xs text-ink-faint">
                            {item.display}
                          </span>
                        ) : null}
                        {item.title}
                      </>
                    }
                    title={item.title}
                    ariaLabel={`Open ${item.title} in a new panel`}
                    onActivate={() => onOpenConversation(item.session_id)}
                    meta={
                      <span className={uiClasses.treeItemMeta}>
                        {subagentStatusText(
                          item.status,
                          item.started_at_ms,
                          item.ended_at_ms,
                        )}
                      </span>
                    }
                  />
                )
              })
            : null}
          <ShowMore
            hidden={subagentRows.hidden}
            expanded={!!expanded.subagents}
            total={subagents.length}
            cap={SUBAGENT_CAP}
            onToggle={() => toggle('subagents')}
          />
        </Section>
      ) : null,
    observers:
      observerListing.rows.length > 0 ? (
        <Section
          id="observers"
          label="Observers"
          count={observerListing.rows.length}
          brief={
            <span className="truncate font-mono text-ink-ghost">
              {[...new Set(observerListing.rows.map((t) => t.functionId))].join(
                ' · ',
              )}
            </span>
          }
        >
          {observerRows.shown.map((observer) => {
            const label =
              observer.label ?? observer.action ?? observer.functionId
            const ended = observerListing.ended.has(observer.id)
            const removable = observerListing.live.has(observer.id)
            const events =
              observerListing.eventsOf.get(observer.id)?.length ?? 0
            // The count first: it is what the transcript's roll-up counted,
            // and a long trigger truncates after it, not over it.
            const summary = [
              events > 0
                ? `${events} ${events === 1 ? 'event' : 'events'}`
                : null,
              observerTrigger(observer),
            ]
              .filter(Boolean)
              .join(' · ')
            return (
              <Row
                key={observer.id}
                data-observer-row={ended ? 'ended' : 'live'}
                icon={
                  <ObserverIcon
                    size={16}
                    className={ended ? 'fill-ink-ghost' : 'fill-warn'}
                    aria-hidden
                  />
                }
                label={label}
                mono={label === observer.functionId}
                description={
                  <span className="truncate text-xs text-ink-faint">
                    {summary}
                  </span>
                }
                onActivate={() => setOpenObserverId(observer.id)}
                aria-current={
                  openObserverId === observer.id ? 'true' : undefined
                }
                ariaLabel={`Show ${label}`}
                meta={<ObserverStatus ended={ended} />}
                actions={
                  !removable ? undefined : (
                    <Hint label="Remove">
                      <button
                        type="button"
                        className={uiClasses.treeItemAction}
                        data-tone="alert"
                        aria-label={`Remove ${label}`}
                        disabled={busyObserverId === observer.id}
                        onClick={(event) => {
                          event.stopPropagation()
                          setConfirmObserver(observer)
                        }}
                      >
                        <X aria-hidden />
                      </button>
                    </Hint>
                  )
                }
              />
            )
          })}
          <ShowMore
            hidden={observerRows.hidden}
            expanded={!!expanded.observers}
            total={observerListing.rows.length}
            cap={SECTION_CAP}
            onToggle={() => toggle('observers')}
          />
        </Section>
      ) : null,
    links:
      links.length > 0 ? (
        <Section
          id="links"
          label="Links"
          count={links.length}
          brief={
            <span className="truncate font-mono text-ink-ghost">
              {[...new Set(links.map((link) => link.host))]
                .slice(0, 2)
                .join(' · ')}
            </span>
          }
        >
          {linkRows.shown.map((link) => (
            <Row
              key={link.url}
              icon={<Globe aria-hidden />}
              label={link.title ?? link.host}
              title={link.url}
              onActivate={() => openExternal(link.url)}
              description={
                <span className="truncate font-mono text-ink-ghost">
                  {link.url}
                </span>
              }
              actions={
                <ItemMenu
                  label={`Actions for ${link.url}`}
                  entries={[
                    {
                      label: 'Open',
                      icon: menuIcons.open,
                      onSelect: () => openExternal(link.url),
                    },
                    copyLinkEntry(link.url),
                    ...showEntry({
                      entryId: link.entryId,
                      messageId: link.messageId,
                    }),
                  ]}
                />
              }
            />
          ))}
          <ShowMore
            hidden={linkRows.hidden}
            expanded={!!expanded.links}
            total={links.length}
            cap={SECTION_CAP}
            onToggle={() => toggle('links')}
          />
        </Section>
      ) : null,
  }
  // The six common sections' counts (Sidechats join in phase 2): the empty
  // pane, and the phone drawer's chips.
  const commonCounts = {
    sidechats: sideChats?.rows.length ?? 0,
    subagents: subagentTotal,
    prs: listedPrs.length,
    links: links.length,
    attachments: attachmentCount,
    images: images.length,
    total:
      (sideChats?.rows.length ?? 0) +
      subagentTotal +
      listedPrs.length +
      links.length +
      attachmentCount +
      images.length,
  }
  for (const section of workerSections) {
    groups[section.id] = (
      <WorkerSection
        key={section.registrationId}
        section={section}
        sessionId={conversation.id}
        workingDir={workingDir ?? undefined}
        active={active}
        isStreaming={conversation.status === 'working'}
      />
    )
  }

  return (
    <SectionMover.Provider value={onMove}>
      <div
        data-reorder-list=""
        className="@container/context flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pt-1 pb-3"
      >
        {phone && commonCounts.total > 0 ? (
          <div
            role="toolbar"
            aria-label="Show"
            // The chip pressed now, for tests and reviewers: all, subagents,
            // prs, links, attachments or images (each chip says it too, with
            // aria-pressed; each section is a region named by its label).
            data-context-filter={filter}
            className="-mx-2 flex shrink-0 gap-1.5 overflow-x-auto px-2 pb-1"
          >
            {(['all', ...COMMON_SECTIONS] as const).map((id) => {
              const count = id === 'all' ? null : commonCounts[id]
              if (count === 0) return null
              const selected = filter === id
              return (
                <button
                  key={id}
                  type="button"
                  // Not the section's own header, which also reads "Links 2"
                  // and folds the section: chips sit in the "Show" toolbar.
                  data-context-chip={id}
                  aria-pressed={selected}
                  onClick={() => setFilter(id)}
                  className={cn(
                    'flex h-9 shrink-0 items-center gap-1.5 rounded-full px-3 text-sm focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50',
                    selected
                      ? 'bg-surface-selected text-ink'
                      : 'bg-surface text-ink-faint hover:bg-surface-hover',
                  )}
                >
                  {id === 'all' ? 'All' : FILTER_LABELS[id]}
                  {count !== null ? (
                    <span className="font-mono text-xs tabular-nums">
                      {count}
                    </span>
                  ) : null}
                </button>
              )
            })}
          </div>
        ) : null}
        {commonCounts.total === 0 && filter === 'all' ? (
          <EmptyState
            compact
            icon={Layers}
            title="Nothing gathered yet"
            description="Links, files, images, subagents and pull requests from this conversation show up here."
          />
        ) : null}
        {landBlocked && workingDir ? (
          <StatusPanel
            variant="warn"
            role="status"
            icon={<TriangleAlert className="h-full w-full" />}
            headline="Landing blocked"
            detail="The worktree conflicts with its base. Resolve the conflicts, then land again."
            action={
              <Button
                variant="secondary"
                size="sm"
                onClick={() => openSourceControl(workingDir)}
              >
                Open changes
              </Button>
            }
          />
        ) : null}

        {order.map((section) =>
          hiddenSections.has(section.id) ||
          (filter !== 'all' && section.id !== filter) ? null : (
            <Fragment key={section.id}>{groups[section.id] ?? null}</Fragment>
          ),
        )}

        <ObserverDetailDialog
          observer={openObserver}
          ended={
            openObserver !== null && observerListing.ended.has(openObserver.id)
          }
          events={
            openObserver
              ? (observerListing.eventsOf.get(openObserver.id) ?? [])
              : []
          }
          busy={openObserver !== null && busyObserverId === openObserver.id}
          onOpenChange={(open) => {
            if (!open) setOpenObserverId(null)
          }}
          onDispose={(id) => void dispose(id)}
        />
        <ConfirmDialog
          open={confirmObserver !== null}
          onOpenChange={(open) => {
            if (!open) setConfirmObserver(null)
          }}
          title="Remove this observer?"
          description="Nothing will notify this chat from it afterwards."
          confirmLabel="Remove"
          tone="danger"
          onConfirm={() => {
            if (confirmObserver) void dispose(confirmObserver.id)
            setConfirmObserver(null)
          }}
        />
        <LiveRegion announcement={announcement} />
      </div>
    </SectionMover.Provider>
  )
}

/** Running work first, then failures, then the finished. */
function subagentRank(status: SubagentVisualStatus): number {
  if (
    status === 'working' ||
    status === 'queued' ||
    status === 'waiting' ||
    status === 'disconnected'
  )
    return 0
  return status === 'failed' ? 1 : 2
}

function SubagentStatus({
  status,
  at,
}: {
  status: SubagentVisualStatus
  at: number
}) {
  // An ended child is not a success claim and not live: the sub-agent card
  // in the chat says "Ended" with no green anywhere (SpawnView's
  // SUBAGENT_STATUS_COPY), so the row's dot is the quiet ghost, not the
  // green that reads as "active".
  if (status === 'completed') {
    return (
      <>
        <StatusDot tone="ghost" />
        <span className={uiClasses.treeItemMeta}>{formatRelative(at)}</span>
      </>
    )
  }
  const tone =
    status === 'working'
      ? 'accent'
      : status === 'failed'
        ? 'alert'
        : status === 'waiting'
          ? 'warn'
          : 'ink'
  return (
    <>
      <StatusDot tone={tone} pulse={status === 'working'} />
      <span className="text-meta leading-4 text-ink">{status}</span>
    </>
  )
}

/** The status dot of a board column: the default columns' meaning. */
function columnTone(status: string): 'accent' | 'warn' | 'ok' | 'ink' {
  if (status === 'in_progress') return 'accent'
  if (status === 'in_review') return 'warn'
  if (status === 'done') return 'ok'
  return 'ink'
}

/**
 * One ticket the chat touched: opens the board's own ticket page beside the
 * chat. A mentioned ticket reads quieter; a deleted one is struck through.
 */
function TicketItem({ row }: { row: TicketRow }) {
  const { ticket } = row
  const deleted = ticket.deletedAt !== null
  const status = deleted ? 'Deleted' : row.status
  return (
    <Row
      icon={<Ticket aria-hidden />}
      label={
        <span
          className={cn(
            (deleted || (!row.action && !row.by)) && 'text-ink-faint',
            deleted && 'line-through',
          )}
        >
          {ticket.title}
        </span>
      }
      title={`${ticket.key} · ${ticket.title}`}
      ariaLabel={`Open ${ticket.key} ${ticket.title} in a new panel (${status}${
        ticket.priority === 'urgent' || ticket.priority === 'high'
          ? `, ${ticket.priority} priority`
          : ''
      })`}
      onActivate={() =>
        requestPanelOpen({
          pageId: 'kanban-ticket',
          context: { id: ticket.key },
        })
      }
      description={
        <>
          <span className="font-mono">{ticket.key} ·</span>
          {deleted ? null : <StatusDot tone={columnTone(ticket.status)} />}
          <span className="text-ink">{status}</span>
          <span className="truncate">· {ticketTouchText(row)}</span>
        </>
      }
      meta={
        ticket.priority === 'urgent' ? (
          <ChevronsUp
            role="img"
            aria-label="Urgent priority"
            className="size-4 text-alert"
          />
        ) : ticket.priority === 'high' ? (
          <ChevronUp
            role="img"
            aria-label="High priority"
            className="size-4 text-warn"
          />
        ) : null
      }
      actions={<CopyAction text={ticket.key} label={`Copy ${ticket.key}`} />}
    />
  )
}

/** `moved here 12m ago`, the sub-agent's name, or `mentioned`. */
function ticketTouchText(row: TicketRow): string {
  if (row.action) {
    const ago = formatRelative(row.at)
    return `${row.action} here ${ago === 'just now' ? ago : `${ago} ago`}`
  }
  return row.by ?? 'mentioned'
}

function PullRequestFacts({ pr }: { pr: PullRequestRow }) {
  return (
    <>
      <span className="font-mono">
        #{pr.number} · {pr.state}
        {pr.mentioned ? ' · mentioned' : ''}
      </span>
      {pr.additions !== null && pr.deletions !== null ? (
        <>
          <span className={cn(deltaClassName, 'text-ok')}>+{pr.additions}</span>
          <span className={cn(deltaClassName, 'text-alert-strong')}>
            −{pr.deletions}
          </span>
        </>
      ) : null}
      {pr.checks ? (
        <>
          <StatusDot
            tone={
              pr.checks === 'fail'
                ? 'alert'
                : pr.checks === 'pending'
                  ? 'accent'
                  : 'ok'
            }
            pulse={pr.checks === 'pending'}
          />
          <span className="text-ink">
            {pr.checks === 'fail'
              ? `${pr.failing || 1} failing`
              : pr.checks === 'pending'
                ? 'checks running'
                : 'checks passed'}
          </span>
        </>
      ) : null}
      {pr.autoMerge ? <span>· merge when ready</span> : null}
    </>
  )
}

/**
 * A pull request's actions. Auto-fix and merge when ready show their state
 * as a check; one that cannot run says why beside it.
 */
function PullRequestMenu({
  pr,
  busy,
  canWrite,
  autoFix,
  autoFixAvailable,
  onViewChanges,
  onAction,
  onAutoFix,
  onDismiss,
  onShow,
}: {
  /** "Show in conversation", when the PR was named in a loaded row or entry. */
  onShow?: () => void
  pr: PullRequestRow
  busy: boolean
  canWrite: boolean
  autoFix: boolean
  autoFixAvailable: boolean
  onViewChanges: () => void
  onAction: (action: PullRequestAction) => void
  onAutoFix: (on: boolean) => void
  onDismiss?: () => void
}) {
  const live = pr.state === 'open' || pr.state === 'draft'
  const draft = pr.state === 'draft'
  // Why an item cannot run: shown beside it and read as its description,
  // never as part of its name.
  const ids = useId()
  const reason = (key: string, text: string | null) =>
    text ? (
      <span id={`${ids}-${key}`} className="ml-auto pl-3 text-meta">
        {text}
      </span>
    ) : null
  const autoFixReason = autoFixAvailable ? null : 'needs gh'
  const mergeReason = !canWrite ? 'needs gh' : draft ? 'draft' : null
  return (
    <DropdownMenu>
      <Hint label="More actions">
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className={uiClasses.treeItemAction}
            aria-label={`Actions for ${pr.repo}#${pr.number}`}
            disabled={busy}
            onClick={(event) => event.stopPropagation()}
          >
            <EllipsisVertical aria-hidden />
          </button>
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={() => openExternal(pr.url)}>
          <ExternalLink aria-hidden className="size-4 text-ink-faint" />
          Open on GitHub
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => void copyTextToClipboard(pr.url)}>
          <Copy aria-hidden className="size-4 text-ink-faint" />
          Copy link
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onViewChanges}>
          <FileDiff aria-hidden className="size-4 text-ink-faint" />
          View changes
        </DropdownMenuItem>
        {onShow ? (
          <DropdownMenuItem onSelect={onShow}>
            {menuIcons.show}
            Show in conversation
          </DropdownMenuItem>
        ) : null}
        {live ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuCheckboxItem
              box
              checked={autoFix}
              disabled={!autoFixAvailable || busy}
              aria-label="Auto-fix"
              aria-describedby={autoFixReason ? `${ids}-fix` : undefined}
              onCheckedChange={(on) => onAutoFix(on === true)}
            >
              Auto-fix
              {reason('fix', autoFixReason)}
            </DropdownMenuCheckboxItem>
            <DropdownMenuCheckboxItem
              box
              checked={pr.autoMerge}
              disabled={!canWrite || draft}
              aria-label="Merge when ready"
              aria-describedby={mergeReason ? `${ids}-merge` : undefined}
              onCheckedChange={(on) =>
                onAction(
                  on === true ? 'merge-when-ready' : 'cancel-merge-when-ready',
                )
              }
            >
              Merge when ready
              {reason('merge', mergeReason)}
            </DropdownMenuCheckboxItem>
            <DropdownMenuItem
              disabled={!canWrite}
              onSelect={() => onAction(draft ? 'ready' : 'draft')}
            >
              {draft ? (
                <GitPullRequest aria-hidden className="size-4 text-ink-faint" />
              ) : (
                <GitPullRequestDraft
                  aria-hidden
                  className="size-4 text-ink-faint"
                />
              )}
              {draft ? 'Ready for review' : 'Convert to draft'}
            </DropdownMenuItem>
          </>
        ) : null}
        {onDismiss ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={onDismiss}>
              <EyeOff aria-hidden className="size-4 text-ink-faint" />
              Dismiss
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** `3 worked here · 2 mentioned`, leaving out the parts that are zero. */
function plural(parts: [number, string, string][]): string | null {
  const text = parts
    .filter(([n]) => n > 0)
    .map(([n, one, many]) => `${n} ${n === 1 ? one : many}`)
    .join(' · ')
  return text || null
}

/** Moves a section among the others; absent outside the Context panel. */
const SectionMover = createContext<
  ((id: string, target: ReorderTarget) => void) | null
>(null)

/**
 * A section card whose header folds it. Folded, the header keeps a one-line
 * brief of what is inside, so a failing check or a running agent stays in
 * sight. Its handle moves it among the others. Which sections are folded,
 * and where they go, is remembered in this browser, for every chat.
 */
function Section({
  id,
  label,
  source,
  count,
  extra,
  brief,
  hidden,
  keepMounted,
  action,
  children,
}: {
  id: string
  label: string
  /** A control beside the header ("+ New"), outside its fold button. */
  action?: ReactNode
  /** The worker that adds it. */
  source?: string
  count?: number
  extra?: ReactNode
  brief?: ReactNode
  /** Left out for now, its body still mounted (a worker's, while empty). */
  hidden?: boolean
  /** Keep the body mounted while folded, so a worker's summary keeps up. */
  keepMounted?: boolean
  children: ReactNode
}) {
  const open = !useFoldedGroups().has(id)
  const onMove = useContext(SectionMover)
  return (
    <section
      aria-label={label}
      data-reorder-item={id}
      hidden={hidden}
      className={cn(
        'group/section flex shrink-0 flex-col rounded-xl border border-card-edge bg-card data-[dragging]:relative data-[dragging]:z-10 data-[dragging]:shadow-floating data-[dragging]:ring-1 data-[dragging]:ring-ring/50',
        open && 'pb-1',
      )}
    >
      <div className="flex items-center gap-0.5 pr-1">
        <button
          type="button"
          aria-expanded={open}
          onClick={() => toggleGroup(id)}
          className={cn(
            uiClasses.listGroupLabel,
            'flex min-w-0 flex-1 items-center gap-2 rounded-sm text-left hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50 pointer-coarse:min-h-11',
            // Folded, the header is the whole card: room to read and to hit.
            // Important: the recipe's padding is unlayered and would win.
            !open && 'py-2.5!',
          )}
        >
          <span className="shrink-0">{label}</span>
          {source ? (
            <span className="shrink-0 font-mono text-meta font-normal text-ink-ghost">
              {source}
            </span>
          ) : null}
          <span className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden text-meta font-normal whitespace-nowrap">
            {open ? null : brief}
          </span>
          {extra}
          {count !== undefined ? (
            <span className="font-mono text-meta font-normal text-ink-ghost tabular-nums">
              {count}
            </span>
          ) : null}
          <ChevronDown
            aria-hidden
            className={cn(
              'size-4 shrink-0 text-ink-ghost motion-safe:transition-transform',
              !open && '-rotate-90',
            )}
          />
        </button>
        {action}
        {onMove ? (
          <ReorderHandle
            label={label}
            onMove={(target) => onMove(id, target)}
            className="size-7 opacity-0 group-hover/section:opacity-100 group-data-[dragging]/section:opacity-100 focus-visible:opacity-100 pointer-coarse:hidden"
          />
        ) : null}
      </div>
      {open || keepMounted ? (
        // Rows sit 6px in from the card's edge, so a hover or selected wash
        // never touches its border; the inset tokens give the 6px back, so
        // the text still lines up with the header above it.
        <div
          hidden={!open}
          className={cn(
            uiClasses.tree,
            'mx-1.5 [--iii-ui-tree-inset:4px] [--iii-ui-tree-trailing:6px]',
          )}
        >
          {children}
        </div>
      ) : null}
    </section>
  )
}

/**
 * A worker's section: the panel's header around the worker's body, which
 * reports the count, brief and live mark and leaves the section out while
 * empty.
 */
function WorkerSection({
  section,
  sessionId,
  workingDir,
  active,
  isStreaming,
}: {
  section: RegisteredContextSection
  sessionId: string
  workingDir?: string
  active: boolean
  isStreaming: boolean
}) {
  const [summary, setSummary] = useState<ContextSectionSummary>({})
  const open = !useFoldedGroups().has(section.id)
  const Body = section.render
  return (
    <Section
      id={section.id}
      label={section.title}
      source={sectionSource(section)}
      count={summary.count}
      hidden={summary.empty === true}
      keepMounted
      extra={
        summary.live ? (
          <span className="flex items-center gap-1.5 text-meta font-normal text-ink">
            <StatusDot tone="accent" pulse />
            {summary.live}
          </span>
        ) : null
      }
      brief={
        summary.brief ? (
          <>
            {summary.tone ? <StatusDot tone={summary.tone} /> : null}
            <span className="truncate text-ink">{summary.brief}</span>
          </>
        ) : null
      }
    >
      <Body
        sessionId={sessionId}
        workingDir={workingDir}
        active={active}
        open={open}
        isStreaming={isStreaming}
        setSummary={setSummary}
      />
    </Section>
  )
}

interface RowProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'title'> {
  icon: ReactNode
  /** Glyph tone (`SubagentColor`) for an identity mark. */
  color?: string
  label: ReactNode
  /** Quiet text right after the label, such as a folder's parent path. */
  suffix?: string
  mono?: boolean
  /** A second line under the label (pull requests). */
  description?: ReactNode
  /** Status at the row's end. With `actions` it yields to them on hover. */
  meta?: ReactNode
  /** Revealed on hover and focus, in place of `meta`. */
  actions?: ReactNode
  title?: string
  ariaLabel?: string
  onActivate?: () => void
}

/** A two-line row keeps the recipe's padding and grows to the list height. */
const TWO_LINE_ROW = { '--iii-ui-tree-row': '2.75rem' } as CSSProperties

/**
 * One tree row. Without actions it is the button itself (and can be a menu
 * trigger). With actions the row is a plain frame holding the activating
 * button and the action buttons side by side: buttons never nest, and the
 * recipe's hover hides only the metadata the actions replace.
 */
const Row = forwardRef<HTMLButtonElement, RowProps>(function Row(
  {
    icon,
    color,
    label,
    suffix,
    mono,
    description,
    meta,
    actions,
    title,
    ariaLabel,
    onActivate,
    onClick,
    className,
    style,
    ...rest
  },
  ref,
) {
  const labelNode = (
    <span
      className={cn(
        uiClasses.treeItemLabel,
        'text-ink',
        mono && 'font-mono text-xs',
      )}
    >
      {label}
      {suffix ? (
        <span className="ml-2 font-mono text-meta font-normal text-ink-ghost">
          {suffix}
        </span>
      ) : null}
    </span>
  )
  const body = (
    <>
      <span className={uiClasses.treeItemIcon} data-color={color}>
        {icon}
      </span>
      {description ? (
        <span className="flex min-w-0 flex-1 flex-col">
          {labelNode}
          <span className="flex min-w-0 items-center gap-1.5 truncate text-meta font-normal text-ink-faint">
            {description}
          </span>
        </span>
      ) : (
        labelNode
      )}
    </>
  )
  const activate: RowProps['onClick'] = (event) => {
    onClick?.(event)
    if (!event.defaultPrevented) onActivate?.()
  }
  if (!actions) {
    return (
      <button
        ref={ref}
        type="button"
        title={title}
        aria-label={ariaLabel}
        className={cn(uiClasses.treeItem, className)}
        style={description ? { ...TWO_LINE_ROW, ...style } : style}
        onClick={activate}
        {...rest}
      >
        {body}
        {meta ? (
          <span className="ml-auto flex shrink-0 items-center gap-1.5 pl-2 text-ink-ghost">
            {meta}
          </span>
        ) : null}
      </button>
    )
  }
  return (
    <div
      className={cn(uiClasses.treeItem, className)}
      style={description ? { ...TWO_LINE_ROW, ...style } : style}
    >
      <button
        ref={ref}
        type="button"
        title={title}
        aria-label={ariaLabel}
        className="flex min-w-0 flex-1 items-center self-stretch rounded-sm text-left focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
        onClick={activate}
        {...rest}
      >
        {body}
      </button>
      <span className={cn(uiClasses.treeItemTrailing, 'text-ink-ghost')}>
        {meta}
        <span className={uiClasses.treeItemActions}>{actions}</span>
      </span>
    </div>
  )
})

function CopyAction({ text, label }: { text: string; label: string }) {
  return (
    <Hint label={label}>
      <button
        type="button"
        className={uiClasses.treeItemAction}
        aria-label={label}
        onClick={(event) => {
          event.stopPropagation()
          void copyTextToClipboard(text)
        }}
      >
        <Copy aria-hidden />
      </button>
    </Hint>
  )
}

function ShowMore({
  hidden,
  expanded,
  total,
  cap,
  onToggle,
}: {
  hidden: number
  expanded: boolean
  total: number
  cap: number
  onToggle: () => void
}) {
  // "Show less" only once something was actually folded away.
  if (hidden === 0 && !(expanded && total > cap + 1)) return null
  return (
    <button
      type="button"
      className={uiClasses.treeItem}
      aria-expanded={expanded}
      onClick={onToggle}
    >
      <span className={uiClasses.treeItemIcon} />
      <span className={cn(uiClasses.treeItemLabel, 'text-xs')}>
        {expanded ? 'Show less' : `Show ${hidden} more`}
      </span>
      <span className={uiClasses.treeItemTrailing}>
        <ChevronDown
          aria-hidden
          className={cn('size-4 text-ink-ghost', expanded && 'rotate-180')}
        />
      </span>
    </button>
  )
}

const FILE_ICONS: [RegExp, typeof FileIcon][] = [
  [/\.(pdf|md|txt|docx?)$/i, FileText],
  [/\.(csv|tsv|xlsx?)$/i, FileSpreadsheet],
  [/\.(html?|json|jsx?|tsx?|css|ya?ml|toml|sh|py|rs)$/i, FileCode],
]

/**
 * A file of the conversation. A written one opens in the IDE and copies its
 * path; an attachment downloads its stored original.
 */
function FileRow({
  file,
  sessionId,
  menu = [],
}: {
  file: ChatFile
  sessionId: string
  /** Entries after the row's own ("Show in conversation"). */
  menu?: readonly ItemMenuEntry[]
}) {
  const Icon =
    FILE_ICONS.find(([pattern]) => pattern.test(file.name))?.[1] ?? FileIcon
  const stored = file.attachment?.attachmentId
  const size =
    file.size !== null ? (
      <span className={uiClasses.treeItemMeta}>{formatSize(file.size)}</span>
    ) : null
  if (file.attachment) {
    const open = stored
      ? () =>
          // ponytail: a failed download stays quiet here; the chip in
          // the transcript shows why.
          void downloadStoredAttachment(sessionId, stored, file.name).catch(
            () => {},
          )
      : undefined
    const share = shareEntry(sessionId, stored, file.name)
    const entries: ItemMenuEntry[] = [
      ...(open
        ? [{ label: 'Open', icon: menuIcons.open, onSelect: open }]
        : []),
      ...(share ? [share] : []),
      ...menu,
    ]
    return (
      <Row
        icon={<Icon aria-hidden />}
        label={file.name}
        description="Attached"
        ariaLabel={stored ? `Download ${file.name}` : file.name}
        onActivate={open}
        meta={size}
        actions={
          entries.length > 0 ? (
            <ItemMenu label={`Actions for ${file.name}`} entries={entries} />
          ) : undefined
        }
      />
    )
  }
  const ago = formatRelative(file.at)
  return (
    <Row
      icon={<Icon aria-hidden />}
      label={file.name}
      title={file.path}
      ariaLabel={`Open ${file.name} in the IDE`}
      description={
        <>
          <span className="shrink-0">
            Written {ago === 'just now' ? ago : `${ago} ago`} ·
          </span>
          <span className="truncate font-mono text-ink-ghost">
            {parentDir(file.path)}
          </span>
        </>
      }
      onActivate={() => openChatFile({ path: file.path }, null)}
      meta={size}
      actions={
        <>
          <CopyAction text={file.path} label="Copy path" />
          {menu.length > 0 ? (
            <ItemMenu label={`Actions for ${file.name}`} entries={menu} />
          ) : null}
        </>
      }
    />
  )
}

/**
 * One picture of the conversation, square-cropped; opens in the viewer. A
 * picture whose bytes stayed in the session store loads when it scrolls into
 * view, the same way its chip in the transcript does. Kept for injected
 * callers; the pane's own grid is `ImageGrid` (context-items-rows).
 */
export function ImageTile({
  image,
  sessionId,
}: {
  image: Attachment
  sessionId: string
}) {
  const lazy = isLazyImage(image, sessionId)
  const stored = useLazyAttachment(sessionId, image.attachmentId, lazy)
  const viewer = useImageViewer()
  const src =
    image.dataUrl ??
    (stored.state.phase === 'ready' ? stored.state.dataUrl : undefined)
  return (
    <div
      ref={lazy ? stored.ref : undefined}
      className="aspect-square min-w-0 overflow-hidden rounded-sm bg-surface"
    >
      <ImageThumbnailButton
        title={image.name}
        onClick={() => (src ? viewer.show() : stored.request())}
        className="flex size-full items-center justify-center"
      >
        {src ? (
          <img
            src={src}
            alt=""
            className="size-full object-cover outline -outline-offset-1 outline-black/10 dark:outline-white/10"
          />
        ) : (
          <ImageIcon aria-hidden className="size-4 text-ink-faint" />
        )}
      </ImageThumbnailButton>
      <ImageViewer
        open={viewer.open}
        onOpenChange={viewer.setOpen}
        src={src}
        alt={image.name}
        title={image.name}
      />
    </div>
  )
}
