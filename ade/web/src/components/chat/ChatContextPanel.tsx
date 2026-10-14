import { formatRelative } from '@iii-dev/console-ui/format'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import {
  ChevronDown,
  ChevronRight,
  Copy,
  ExternalLink,
  FileDiff,
  Folder,
  GitBranch,
  GitPullRequest,
  Globe,
  ImageIcon,
  SquareTerminal,
  TriangleAlert,
  X,
} from 'lucide-react'
import {
  type ButtonHTMLAttributes,
  type CSSProperties,
  forwardRef,
  type ReactNode,
  useEffect,
  useMemo,
  useState,
} from 'react'
import { Button } from '@/components/ui/Button'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { EmptyState } from '@/components/ui/EmptyState'
import {
  ImageThumbnailButton,
  ImageViewer,
  useImageViewer,
} from '@/components/ui/ImageViewer'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { TriggerIcon } from '@/components/ui/TriggerIcon'
import { useLazyAttachment } from '@/hooks/use-lazy-attachment'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import {
  capRows,
  collectChatImages,
  collectChatLinks,
  type GitSummary,
  ghCliSource,
  githubWorkerSource,
  type PullRequestRow,
  readGitSummary,
  readPullRequests,
} from '@/lib/chat-aside'
import { copyTextToClipboard } from '@/lib/clipboard'
import { getIiiClient, type IIIConnectionState } from '@/lib/iii-client'
import { requestPanelOpen } from '@/lib/panel-context'
import { cn } from '@/lib/utils'
import type { Attachment, Conversation } from '@/types/chat'
import { SUBAGENT_ICON_COMPONENTS } from './ActiveSubagentChips'
import { isLazyImage } from './AttachmentChip'
import {
  collectSubagentDescendants,
  deriveSubagentVisualStatus,
  resolveSubagentAppearance,
  type SubagentVisualStatus,
} from './active-subagents'
import { TriggerDetailDialog } from './SessionTriggers'

/**
 * Coloured trailing text is a plain span: the tree recipe's `__meta` class
 * sets its own colour unlayered, so a colour utility on it would lose.
 */
const deltaClassName = 'font-mono text-[11px] leading-4 tabular-nums'

/** Sub-agents are what people come here for; every other section shows less. */
const SUBAGENT_CAP = 5
const SECTION_CAP = 3
/** Two rows of the three-across image grid. */
const IMAGE_CAP = 6

interface ChatContextPanelProps {
  conversation: Conversation
  conversations: readonly Conversation[]
  connectionState: IIIConnectionState
  /** Registered trigger rows (ChatView's merged list); inactive ones are skipped. */
  triggers: readonly SessionTriggerInfo[]
  onUnregisterTrigger: (subscriptionId: string) => Promise<void> | void
  onOpenConversation: (sessionId: string) => void
  /** git is read through `shell::exec`, so the folder's state needs that worker. */
  shellAvailable: boolean
  githubAvailable: boolean
  /** The managed worktree's land is blocked on conflicts. */
  landBlocked?: boolean
  /** Whether the panel is in sight; git and GitHub are read only then. */
  active?: boolean
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
 * sub-agents, triggers and the links it mentions. A section with nothing to
 * show is left out; a long one shows its first rows and a "Show N more".
 */
export function ChatContextPanel({
  conversation,
  conversations,
  connectionState,
  triggers,
  onUnregisterTrigger,
  onOpenConversation,
  shellAvailable,
  githubAvailable,
  landBlocked,
  active = true,
}: ChatContextPanelProps) {
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
        client.trigger.bind(client),
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

  const { links, pullRequests: mentioned } = useMemo(
    () => collectChatLinks(conversation.messages),
    [conversation.messages],
  )
  const mentionedKey = mentioned.map((pr) => `${pr.repo}#${pr.number}`).join()
  const repo = summary?.repo ?? null
  const branch = summary?.branch ?? null
  const [pullRequests, setPullRequests] = useState<PullRequestRow[]>([])
  // biome-ignore lint/correctness/useExhaustiveDependencies: mentionedKey stands for `mentioned`; status re-reads after a turn
  useEffect(() => {
    if (!active) return
    // The github worker when it is there; otherwise `gh` in the folder.
    const viaGh = !githubAvailable && shellAvailable && workingDir
    if (
      (!githubAvailable && !viaGh) ||
      (!(repo && branch) && mentioned.length === 0)
    ) {
      setPullRequests([])
      return
    }
    let alive = true
    void (async () => {
      const client = await getIiiClient()
      const trigger = client.trigger.bind(client)
      const source = viaGh
        ? ghCliSource(trigger, viaGh)
        : githubWorkerSource(trigger)
      const rows = await readPullRequests(source, {
        repo,
        branch,
        mentioned,
      }).catch(() => [])
      if (alive) setPullRequests(rows)
    })()
    return () => {
      alive = false
    }
  }, [
    active,
    githubAvailable,
    shellAvailable,
    workingDir,
    repo,
    branch,
    mentionedKey,
    conversation.status,
  ])

  const images = useMemo(
    () => collectChatImages(conversation.messages),
    [conversation.messages],
  )

  const subagents = useMemo(() => {
    const { descendants } = collectSubagentDescendants(
      conversations,
      conversation.id,
    )
    return descendants
      .map(({ conversation: child }) => ({
        child,
        status: deriveSubagentVisualStatus(child, connectionState),
        appearance: resolveSubagentAppearance(child),
      }))
      .sort((a, b) => subagentRank(a.status) - subagentRank(b.status))
  }, [conversations, conversation.id, connectionState])
  const activeCount = subagents.filter(
    (s) => subagentRank(s.status) === 0,
  ).length

  const liveTriggers = useMemo(
    () => triggers.filter((trigger) => !trigger.fired),
    [triggers],
  )
  const [openTriggerId, setOpenTriggerId] = useState<string | null>(null)
  const [busyTriggerId, setBusyTriggerId] = useState<string | null>(null)
  const [confirmTrigger, setConfirmTrigger] =
    useState<SessionTriggerInfo | null>(null)
  const unregister = async (id: string) => {
    setBusyTriggerId(id)
    try {
      await onUnregisterTrigger(id)
      setOpenTriggerId((current) => (current === id ? null : current))
    } finally {
      setBusyTriggerId(null)
    }
  }
  const openTrigger =
    liveTriggers.find((trigger) => trigger.id === openTriggerId) ?? null

  const prRows = capRows(pullRequests, SECTION_CAP, !!expanded.prs)
  const subagentRows = capRows(subagents, SUBAGENT_CAP, !!expanded.subagents)
  const triggerRows = capRows(liveTriggers, SECTION_CAP, !!expanded.triggers)
  const linkRows = capRows(links, SECTION_CAP, !!expanded.links)
  const imageRows = capRows(images, IMAGE_CAP, !!expanded.images)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-2 pt-1 pb-3">
      {landBlocked && workingDir ? (
        <StatusPanel
          variant="warn"
          role="status"
          icon={<TriangleAlert className="h-full w-full" />}
          headline="Landing blocked"
          detail="The worktree conflicts with its base. Resolve the conflicts, then land again."
          action={
            <Button
              variant="ghost"
              size="sm"
              onClick={() => openSourceControl(workingDir)}
            >
              Open changes
            </Button>
          }
        />
      ) : null}

      <Section label="Project">
        {workingDir ? (
          <>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Row
                  icon={<Folder aria-hidden />}
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
          </>
        ) : (
          <EmptyState
            compact
            icon={Folder}
            title="No project folder"
            description="Pick a folder in the composer to see its branch and changes here."
          />
        )}
      </Section>

      {pullRequests.length > 0 ? (
        <Section label="Pull requests" count={pullRequests.length}>
          {prRows.shown.map((pr) => (
            <Row
              key={`${pr.repo}#${pr.number}`}
              icon={<GitPullRequest aria-hidden />}
              label={pr.title}
              title={pr.url}
              onActivate={() => openExternal(pr.url)}
              description={<PullRequestFacts pr={pr} />}
              actions={<CopyAction text={pr.url} label="Copy link" />}
            />
          ))}
          <ShowMore
            hidden={prRows.hidden}
            expanded={!!expanded.prs}
            total={pullRequests.length}
            cap={SECTION_CAP}
            onToggle={() => toggle('prs')}
          />
        </Section>
      ) : null}

      {images.length > 0 ? (
        <Section label="Images" count={images.length}>
          <div className="grid grid-cols-3 gap-1.5 px-2.5 pt-1">
            {imageRows.shown.map((image) => (
              <ImageTile
                key={image.id}
                image={image}
                sessionId={conversation.id}
              />
            ))}
          </div>
          <ShowMore
            hidden={imageRows.hidden}
            expanded={!!expanded.images}
            total={images.length}
            cap={IMAGE_CAP}
            onToggle={() => toggle('images')}
          />
        </Section>
      ) : null}

      {subagents.length > 0 ? (
        <Section
          label="Sub-agents"
          count={subagents.length}
          extra={
            activeCount > 0 ? (
              <span className="flex items-center gap-1.5 text-[11px] font-normal text-ink">
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
                label={appearance.name}
                title={child.title}
                ariaLabel={`Open ${appearance.name} in a new panel (${status})`}
                onActivate={() => onOpenConversation(child.id)}
                meta={<SubagentStatus status={status} at={child.updatedAt} />}
              />
            )
          })}
          <ShowMore
            hidden={subagentRows.hidden}
            expanded={!!expanded.subagents}
            total={subagents.length}
            cap={SUBAGENT_CAP}
            onToggle={() => toggle('subagents')}
          />
        </Section>
      ) : null}

      {liveTriggers.length > 0 ? (
        <Section label="Triggers" count={liveTriggers.length}>
          {triggerRows.shown.map((trigger) => {
            const label = trigger.label ?? trigger.action ?? trigger.triggerType
            return (
              <Row
                key={trigger.id}
                icon={
                  <TriggerIcon size={16} className="fill-current" aria-hidden />
                }
                label={label}
                mono={label === trigger.triggerType}
                onActivate={() => setOpenTriggerId(trigger.id)}
                ariaLabel={`Show ${label}`}
                meta={
                  label === trigger.triggerType ? null : (
                    <span className={cn(uiClasses.treeItemMeta, 'font-mono')}>
                      {trigger.triggerType}
                    </span>
                  )
                }
                actions={
                  <button
                    type="button"
                    className={uiClasses.treeItemAction}
                    data-tone="alert"
                    aria-label={`Unregister ${label}`}
                    title="Unregister"
                    disabled={busyTriggerId === trigger.id}
                    onClick={(event) => {
                      event.stopPropagation()
                      setConfirmTrigger(trigger)
                    }}
                  >
                    <X aria-hidden />
                  </button>
                }
              />
            )
          })}
          <ShowMore
            hidden={triggerRows.hidden}
            expanded={!!expanded.triggers}
            total={liveTriggers.length}
            cap={SECTION_CAP}
            onToggle={() => toggle('triggers')}
          />
        </Section>
      ) : null}

      {links.length > 0 ? (
        <Section label="Links" count={links.length}>
          {linkRows.shown.map((link) => (
            <Row
              key={link.url}
              icon={<Globe aria-hidden />}
              label={link.label}
              title={link.url}
              onActivate={() => openExternal(link.url)}
              meta={
                <span className={cn(uiClasses.treeItemMeta, 'font-mono')}>
                  {link.host}
                </span>
              }
              actions={<CopyAction text={link.url} label="Copy link" />}
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
      ) : null}

      <TriggerDetailDialog
        trigger={openTrigger}
        busy={openTrigger !== null && busyTriggerId === openTrigger.id}
        onOpenChange={(open) => {
          if (!open) setOpenTriggerId(null)
        }}
        onUnregister={(id) => void unregister(id)}
      />
      <ConfirmDialog
        open={confirmTrigger !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmTrigger(null)
        }}
        title="Unregister this trigger?"
        description="Nothing will notify this chat from it afterwards."
        confirmLabel="Unregister"
        tone="danger"
        onConfirm={() => {
          if (confirmTrigger) void unregister(confirmTrigger.id)
          setConfirmTrigger(null)
        }}
      />
    </div>
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
  if (status === 'completed') {
    return (
      <>
        <StatusDot tone="ok" />
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
      <span className="text-[11px] leading-4 text-ink">{status}</span>
    </>
  )
}

function PullRequestFacts({ pr }: { pr: PullRequestRow }) {
  return (
    <>
      <span className="font-mono">
        #{pr.number} · {pr.state}
        {pr.mentioned ? ' · mentioned' : ''}
      </span>
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
              ? 'checks failing'
              : pr.checks === 'pending'
                ? 'checks running'
                : 'checks passed'}
          </span>
        </>
      ) : null}
    </>
  )
}

function Section({
  label,
  count,
  extra,
  children,
}: {
  label: string
  count?: number
  extra?: ReactNode
  children: ReactNode
}) {
  return (
    <section aria-label={label} className="flex flex-col">
      <div className={cn(uiClasses.listGroupLabel, 'flex items-center gap-2')}>
        <span className="flex-1">{label}</span>
        {extra}
        {count !== undefined ? (
          <span className="font-mono text-[11px] font-normal text-ink-ghost tabular-nums">
            {count}
          </span>
        ) : null}
      </div>
      <div className={uiClasses.tree}>{children}</div>
    </section>
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
        <span className="ml-2 font-mono text-[11px] font-normal text-ink-ghost">
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
          <span className="flex min-w-0 items-center gap-1.5 truncate text-[11px] font-normal text-ink-faint">
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
        style={description ? TWO_LINE_ROW : undefined}
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
      style={description ? TWO_LINE_ROW : undefined}
    >
      <button
        ref={ref}
        type="button"
        title={title}
        aria-label={ariaLabel}
        className="flex min-w-0 flex-1 items-center self-stretch rounded-sm text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
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
    <button
      type="button"
      className={uiClasses.treeItemAction}
      aria-label={label}
      title={label}
      onClick={(event) => {
        event.stopPropagation()
        void copyTextToClipboard(text)
      }}
    >
      <Copy aria-hidden />
    </button>
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

/**
 * One picture of the conversation, square-cropped; opens in the viewer. A
 * picture whose bytes stayed in the session store loads when it scrolls into
 * view, the same way its chip in the transcript does.
 */
function ImageTile({
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
          <img src={src} alt="" className="size-full object-cover" />
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
