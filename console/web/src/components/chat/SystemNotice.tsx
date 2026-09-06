import {
  Ban,
  Check,
  ChevronRight,
  CircleAlert,
  FolderInput,
  Gauge,
  Hourglass,
  Info,
  KeyRound,
  type LucideIcon,
  ServerCrash,
  Settings2,
  TriangleAlert,
  Unplug,
  Wallet,
  Wrench,
} from 'lucide-react'
import { useId, useState } from 'react'
import { Chip, type ChipTone } from '@/components/ui/Chip'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from '@/components/ui/CollapsibleCard'
import {
  Card,
  CardBody,
  CardHeader,
  CardHighlight,
} from '@/components/ui/Surface'
import {
  classifyTurnFailure,
  type TurnFailureCategory,
  type TurnFailureOwner,
} from '@/lib/turn-failure'
import { cn } from '@/lib/utils'
import type {
  SystemMessage,
  SystemNoticeTechnicalDetails,
  WorkingDirScope,
} from '@/types/chat'
import { CopyMessageButton } from './CopyMessageButton'
import { splitNotice } from './system-notice-copy'
import {
  TimelineActivityDisclosure,
  TimelineActivityTrail,
} from './TimelineActivityTrail'

/**
 * Every `role: 'system'` entry that is not a compaction marker or a trigger
 * fire. Three presentations, chosen by `kind`:
 *
 * - `working-dir` — a session scope change, in the same activity-row grammar
 *   as function calls and trigger fires (status icon, kind trail, one line,
 *   disclosure), with the folder mark in `workdir`.
 * - `turn-failure` — a turn the provider or iii could not finish: the
 *   diagnosis card, which leads with WHO has to act (a chip and one plain
 *   sentence) before what happened and what to do.
 * - everything else — a one-line operational status on the StatusPanel
 *   recipe (tinted fill, small icon, headline + detail). No stripe, no
 *   outline, no uppercase.
 */
export function SystemNotice({ message }: { message: SystemMessage }) {
  if (message.kind === 'working-dir' && message.scope) {
    return <WorkingDirMarker message={message} scope={message.scope} />
  }
  if (message.kind === 'turn-failure') {
    return <TurnFailureCard message={message} />
  }
  return <InlineNotice message={message} />
}

/* ───────────────────────── one-line notices ────────────────────────── */

type NoticeTone = NonNullable<SystemMessage['tone']>

const INLINE_TONE: Record<
  NoticeTone,
  { fill: string; Icon: LucideIcon; icon: string }
> = {
  info: { fill: 'bg-surface', Icon: Info, icon: 'stroke-ink-faint' },
  warn: { fill: 'bg-warn-muted', Icon: TriangleAlert, icon: 'stroke-warn' },
  error: { fill: 'bg-alert-muted', Icon: CircleAlert, icon: 'stroke-alert' },
}

function InlineNotice({ message }: { message: SystemMessage }) {
  const tone: NoticeTone = message.tone ?? 'info'
  const { fill, Icon, icon } = INLINE_TONE[tone]
  const { headline, detail } = splitNotice(message.content)
  const rows = technicalRows(message.technicalDetails)
  const actions = message.nextActions?.filter((a) => a.trim().length > 0) ?? []
  return (
    <article
      data-message-role="system-notice"
      data-message-tone={tone}
      className={cn(
        'flex w-fit max-w-full min-w-0 items-start gap-3 rounded-md px-4 py-3 sm:px-3 sm:py-2.5',
        fill,
      )}
    >
      <Icon
        aria-hidden
        className={cn('mt-0.5 size-5 shrink-0 sm:size-4', icon)}
      />
      <div className="flex min-w-0 flex-1 flex-col gap-1 font-sans text-base sm:text-sm">
        <div
          data-message-summary
          className="text-pretty wrap-break-word font-medium text-ink"
        >
          {headline}
        </div>
        {detail ? (
          <div
            data-message-detail
            className="text-pretty wrap-break-word text-ink-faint"
          >
            {detail}
          </div>
        ) : null}
        {actions.length > 0 ? (
          <NextActions actions={actions} className="mt-1" />
        ) : null}
        {rows.length > 0 ? (
          <TechnicalDetails rows={rows} className="mt-1" />
        ) : null}
      </div>
    </article>
  )
}

/* ─────────────────────────── failed turns ──────────────────────────── */

const CATEGORY_ICON: Record<TurnFailureCategory, LucideIcon> = {
  auth: KeyRound,
  billing: Wallet,
  configuration: Settings2,
  context: Gauge,
  'rate-limit': Hourglass,
  connection: Unplug,
  rejected: Ban,
  send: Unplug,
  internal: ServerCrash,
  unknown: CircleAlert,
}

const OWNER_CHIP: Record<TurnFailureOwner, ChipTone> = {
  user: 'warning',
  environment: 'neutral',
  iii: 'danger',
}

function TurnFailureCard({ message }: { message: SystemMessage }) {
  const titleId = useId()
  const presentation = classifyTurnFailure(message)
  const details = message.technicalDetails
  const failure = message.failure
  const Icon = CATEGORY_ICON[presentation.category]
  // A transient failure is amber (it will pass); anything someone has to fix
  // — the user's account or iii itself — is red.
  const severity: 'warn' | 'alert' =
    presentation.owner === 'environment' ? 'warn' : 'alert'
  const rows = technicalRows(details)
  const summary = failure?.summary ?? splitNotice(message.content).headline
  const notes = [
    failure?.partialResultAvailable
      ? 'The partial response above was kept as evidence; treat it as incomplete.'
      : null,
    failure?.recoveryAttempted
      ? `iii retried automatically ${failure.recoveryAttempted} of ${
          failure.recoveryMaxAttempts ?? failure.recoveryAttempted
        } ${failure.recoveryAttempted === 1 && (failure.recoveryMaxAttempts ?? 1) === 1 ? 'time' : 'times'} before giving up.`
      : null,
  ].filter((note): note is string => note !== null)

  return (
    <article
      className="w-full"
      data-message-role="turn-failure"
      data-message-tone="error"
      data-failure-category={presentation.category}
      data-failure-owner={presentation.owner}
      aria-labelledby={titleId}
    >
      <Card className="@container">
        <CardHeader className="items-start gap-3 border-b border-edge p-4 sm:p-3">
          <div
            className={cn(
              'flex size-10 shrink-0 items-center justify-center rounded-md sm:size-9',
              severity === 'warn' ? 'bg-warn-muted' : 'bg-alert-muted',
            )}
          >
            <Icon
              aria-hidden
              className={cn(
                'size-5 shrink-0 sm:size-4',
                severity === 'warn' ? 'stroke-warn' : 'stroke-alert',
              )}
            />
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <h3
                id={titleId}
                className="min-w-0 text-balance font-sans text-base font-semibold text-ink sm:text-sm"
              >
                {presentation.title}
              </h3>
              <Chip tone={OWNER_CHIP[presentation.owner]}>
                {presentation.ownerLabel}
              </Chip>
              {failure?.retryable ? <Chip tone="accent">Retryable</Chip> : null}
            </div>
            {details?.provider || details?.model ? (
              <div className="flex min-w-0 flex-wrap items-center gap-x-2 font-mono text-[12px] font-normal text-ink-faint">
                {details.provider ? <span>{details.provider}</span> : null}
                {details.provider && details.model ? (
                  <span aria-hidden className="text-ink-ghost">
                    ·
                  </span>
                ) : null}
                {details.model ? (
                  <span className="min-w-0 truncate" title={details.model}>
                    {details.model}
                  </span>
                ) : null}
              </div>
            ) : null}
          </div>
        </CardHeader>

        <CardBody className="flex flex-col gap-4 p-4 sm:p-3">
          <div className="flex flex-col gap-1.5 font-sans text-base sm:text-sm">
            <p
              data-message-summary
              className="text-pretty wrap-break-word text-ink"
            >
              {summary}
            </p>
            <p
              data-failure-ownership
              className="text-pretty wrap-break-word text-ink-faint"
            >
              {presentation.ownership}
            </p>
            {notes.map((note) => (
              <p key={note} className="text-pretty text-ink-faint">
                {note}
              </p>
            ))}
          </div>
          <CardHighlight className="flex items-start gap-3 p-3">
            <Wrench
              aria-hidden
              className="mt-0.5 size-5 shrink-0 stroke-ink-faint sm:size-4"
            />
            <NextActions actions={presentation.actions} heading />
          </CardHighlight>
        </CardBody>

        {rows.length > 0 ? <TechnicalDetails rows={rows} framed /> : null}
      </Card>
    </article>
  )
}

/* ─────────────────────── working directory rows ────────────────────── */

function WorkingDirMarker({
  message,
  scope,
}: {
  message: SystemMessage
  scope: WorkingDirScope
}) {
  const [open, setOpen] = useState(false)
  const previous = scope.previousPath ?? null
  const copy = workingDirCopy(scope)
  return (
    <article
      className="w-full"
      data-message-role="working-dir"
      data-message-id={message.id}
      data-working-dir-cause={scope.cause}
      data-expanded={open}
    >
      <CollapsibleCard
        open={open}
        onOpenChange={setOpen}
        className={cn(
          '@container trigger-activity-collapsible',
          !open && 'trigger-activity-collapsible--compact',
        )}
      >
        <CollapsibleCardTrigger
          className="group trigger-activity-collapsible__trigger select-none"
          aria-label={`${open ? 'Hide' : 'Show'} working directory details`}
        >
          <div className="flex w-full min-w-0 items-center gap-2">
            <span
              aria-hidden="true"
              className="activity-status-icon"
              data-status={scope.cause === 'unavailable' ? 'error' : 'done'}
            >
              <span data-activity-status-layer="error">
                <CircleAlert strokeWidth={2.5} className="size-4 stroke-warn" />
              </span>
              <span data-activity-status-layer="done">
                <Check
                  strokeWidth={2.5}
                  className="size-4 stroke-muted-foreground"
                />
              </span>
            </span>
            <TimelineActivityTrail kind="working-dir" />
            <div
              data-message-summary
              className="flex min-w-0 flex-1 items-baseline gap-1.5 font-sans text-sm text-muted-foreground sm:text-[0.8125rem]"
              title={message.content}
            >
              {/* The noun rides only where it fits: on a phone the folder
                  glyph says "working directory" and the line keeps the path. */}
              <span className="shrink-0 truncate">
                <span className="sm:hidden">Folder </span>
                <span className="hidden sm:inline">Working directory </span>
                {copy.verb}
              </span>
              {copy.path ? (
                <PathTail path={copy.path} muted={copy.pathMuted} />
              ) : null}
            </div>
            <TimelineActivityDisclosure />
          </div>
        </CollapsibleCardTrigger>

        <CollapsibleCardContent>
          <div className="flex flex-col gap-4 border-t border-edge p-4 sm:p-3">
            <div className="flex min-w-0 items-start gap-3">
              <div className="flex size-10 shrink-0 items-center justify-center rounded-md bg-workdir-muted sm:size-9">
                <FolderInput
                  aria-hidden
                  className="size-5 shrink-0 stroke-workdir sm:size-4"
                />
              </div>
              <div className="flex min-w-0 flex-1 flex-col gap-1 font-sans text-base sm:text-sm">
                <div className="font-semibold text-ink">{copy.title}</div>
                <p className="text-pretty wrap-break-word text-ink-faint">
                  {copy.description}
                </p>
              </div>
            </div>
            <CardHighlight className="p-3">
              <dl className="grid grid-cols-[max-content_minmax(0,1fr)] items-baseline gap-x-4 gap-y-2 font-mono text-[12px]">
                {scope.cause !== 'selected' || previous !== null ? (
                  <>
                    <dt className="text-ink-ghost">Before</dt>
                    <dd className="min-w-0 wrap-break-word text-ink-faint">
                      {previous ?? 'unscoped'}
                    </dd>
                  </>
                ) : null}
                <dt className="text-ink-ghost">Now</dt>
                <dd className="flex min-w-0 items-baseline gap-2 text-ink">
                  <span className="min-w-0 wrap-break-word">
                    {scope.path ?? 'unscoped'}
                  </span>
                  {scope.path ? (
                    <CopyMessageButton
                      text={scope.path}
                      label="copy path"
                      className="shrink-0 self-center"
                    />
                  ) : null}
                </dd>
              </dl>
            </CardHighlight>
          </div>
        </CollapsibleCardContent>
      </CollapsibleCard>
    </article>
  )
}

/**
 * A path that keeps its folder name when the row runs out of room: the parent
 * directories truncate from the right, the last segment never shrinks, so a
 * narrow phone still reads `…/workers/harness` rather than `/Us…`.
 */
function PathTail({ path, muted = false }: { path: string; muted?: boolean }) {
  const cut = path.lastIndexOf('/')
  const head = cut > 0 ? path.slice(0, cut) : ''
  const tail = cut >= 0 ? path.slice(cut) : path
  return (
    <span
      className={cn(
        'flex min-w-0 items-baseline overflow-hidden font-mono',
        muted ? 'text-ink-faint' : 'text-ink',
      )}
      data-working-dir-path={path}
    >
      {/* Only the parent path gives way (down to its ellipsis); the folder
          name never shrinks. Flex would otherwise hand the name a sub-pixel
          share of the shortfall and clip it to `/worke…` while the parent
          still has room. A name too long for a phone clips at the row edge. */}
      {head ? (
        <span className="min-w-4 truncate text-ink-faint">{head}</span>
      ) : null}
      <span className="shrink-0">{tail}</span>
    </span>
  )
}

function workingDirCopy(scope: WorkingDirScope): {
  /** Compact-row verb after the noun; the path (when any) follows it. */
  verb: string
  path: string | null
  /** The path names a folder that no longer exists. */
  pathMuted?: boolean
  title: string
  description: string
} {
  const previous = scope.previousPath ?? null
  switch (scope.cause) {
    case 'recovered':
      return {
        verb: 'moved to',
        path: scope.path,
        title: 'Working directory moved',
        description: `The saved folder ${previous ?? ''} is no longer available, so from the next message on this session runs in the harness default folder. Pick another folder from the composer if that is not what you want.`,
      }
    case 'unavailable':
      return {
        verb: 'gone',
        path: previous,
        pathMuted: true,
        title: 'Working directory unavailable',
        description: `The saved folder ${previous ?? ''} is no longer available and there is no default to fall back to. From the next message on this session runs unscoped; pick a folder from the composer to scope it again.`,
      }
    default:
      return scope.path === null
        ? {
            verb: 'cleared · session unscoped',
            path: null,
            title: 'Working directory cleared',
            description:
              'From the next message on this session runs unscoped: shell and file operations are no longer confined to a project folder. Earlier turns keep the scope they ran in.',
          }
        : {
            verb: 'changed to',
            path: scope.path,
            title: 'Working directory changed',
            description:
              'Shell and file operations from the next message on run inside this folder. Earlier turns keep the scope they ran in.',
          }
  }
}

/* ───────────────────────────── shared bits ─────────────────────────── */

function NextActions({
  actions,
  heading = false,
  className,
}: {
  actions: string[]
  heading?: boolean
  className?: string
}) {
  return (
    <div
      data-message-next-actions
      className={cn(
        'flex min-w-0 flex-1 flex-col gap-1.5 font-sans text-base sm:text-sm',
        className,
      )}
    >
      <div
        className={cn('text-ink', heading ? 'font-semibold' : 'font-medium')}
      >
        What you can do
      </div>
      <ol className="flex list-decimal flex-col gap-1 pl-4 text-pretty text-ink-faint marker:text-ink-ghost">
        {actions.map((action) => (
          <li key={action}>{action}</li>
        ))}
      </ol>
    </div>
  )
}

type TechnicalRow = [label: string, value: string]

const TECHNICAL_LABELS: Array<[keyof SystemNoticeTechnicalDetails, string]> = [
  ['provider', 'Provider'],
  ['model', 'Model'],
  ['code', 'Code'],
  ['class', 'Class'],
  ['detail', 'Detail'],
]

function technicalRows(
  details: SystemNoticeTechnicalDetails | undefined,
): TechnicalRow[] {
  if (!details) return []
  return TECHNICAL_LABELS.flatMap(([key, label]) => {
    const value = details[key]
    return typeof value === 'string' && value.length > 0
      ? [[label, value] as TechnicalRow]
      : []
  })
}

function TechnicalDetails({
  rows,
  framed = false,
  className,
}: {
  rows: TechnicalRow[]
  /** Card footer variant: divider above, card padding inside. */
  framed?: boolean
  className?: string
}) {
  const copyText = () => rows.map(([k, v]) => `${k}: ${v}`).join('\n')
  return (
    <details
      data-message-technical-details
      className={cn('group', framed && 'border-t border-edge', className)}
    >
      <summary
        className={cn(
          'flex w-fit cursor-pointer list-none items-center gap-1.5 font-sans text-base font-medium text-ink-faint select-none hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent sm:text-sm [&::-webkit-details-marker]:hidden',
          framed && 'w-full px-4 py-3 sm:px-3 sm:py-2.5',
        )}
      >
        <ChevronRight
          aria-hidden
          className="size-4 shrink-0 stroke-ink-ghost transition-transform duration-(--motion-duration-control) ease-(--motion-ease-standard) group-open:rotate-90"
        />
        <span>Technical details</span>
      </summary>
      <div
        className={cn(
          'flex flex-col gap-2',
          framed ? 'px-4 pb-4 sm:px-3 sm:pb-3' : 'mt-2',
        )}
      >
        <dl className="grid grid-cols-[max-content_minmax(0,1fr)] items-baseline gap-x-4 gap-y-1.5 font-mono text-[12px]">
          {rows.map(([label, value]) => (
            <div key={label} className="contents">
              <dt className="text-ink-ghost">{label}</dt>
              <dd
                data-technical-detail={label.toLowerCase()}
                className="min-w-0 wrap-break-word text-ink-faint"
              >
                {value}
              </dd>
            </div>
          ))}
        </dl>
        <div className="flex items-center gap-1.5 font-sans text-sm text-ink-faint sm:text-[0.8125rem]">
          <CopyMessageButton text={copyText} label="copy technical details" />
          <span>Copy for a bug report</span>
        </div>
      </div>
    </details>
  )
}
