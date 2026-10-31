import {
  Ban,
  BookOpen,
  Check,
  ChevronRight,
  CircleAlert,
  FileWarning,
  FolderInput,
  Gauge,
  History,
  Hourglass,
  Info,
  KeyRound,
  type LucideIcon,
  RotateCcw,
  ServerCrash,
  Settings2,
  TriangleAlert,
  Unplug,
  Wallet,
} from 'lucide-react'
import { Fragment, useId, useState } from 'react'
import { Chip, type ChipTone } from '@/components/ui/Chip'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from '@/components/ui/CollapsibleCard'
import { Fact, FactList } from '@/components/ui/FactCard'
import { Card, CardHighlight } from '@/components/ui/Surface'
import { rememberedProvider } from '@/lib/models-catalog'
import { skillUpdateSummary } from '@/lib/skill-update'
import {
  classifyTurnFailure,
  type TurnFailureCategory,
  type TurnFailureOwner,
} from '@/lib/turn-failure'
import { cn } from '@/lib/utils'
import type {
  SkillCatalogUpdate,
  SystemMessage,
  SystemNoticeTechnicalDetails,
  WorkingDirScope,
} from '@/types/chat'
import { CopyMessageButton } from './CopyMessageButton'
import { ModelNoteMarker } from './ModelNoteMarker'
import { formatProviderLabel } from './model-picker-presentation'
import { splitNotice } from './system-notice-copy'
import {
  TimelineActivityDisclosure,
  TimelineActivityTrail,
} from './TimelineActivityTrail'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * Every `role: 'system'` entry that is not a compaction marker or an
 * observer fire. Three presentations, chosen by `kind`:
 *
 * - `working-dir` — a session scope change, in the same activity-row grammar
 *   as function calls and observer fires (status icon, kind trail, one line,
 *   disclosure), with the folder mark in `workdir`.
 * - `skills` — the harness re-sent the model its skill index. Same grammar:
 *   one line saying the index moved and how many skills it holds, the list
 *   itself behind the disclosure. The model needs the whole block; a reader
 *   does not.
 * - `turn-failure` — a turn the provider or iii could not finish: the
 *   diagnosis card, which leads with WHO has to act (a chip and one plain
 *   sentence) before what happened and what to do.
 * - `model-note` — text the harness or a hook showed the model: the
 *   quietest activity row, what it said behind the disclosure (see
 *   `ModelNoteMarker`).
 * - everything else — a one-line operational status on the StatusPanel
 *   recipe (tinted fill, small icon, headline + detail). No stripe, no
 *   outline, no caps transform.
 */
export function SystemNotice({ message }: { message: SystemMessage }) {
  if (message.kind === 'working-dir' && message.scope) {
    return <WorkingDirMarker message={message} scope={message.scope} />
  }
  if (message.kind === 'skills' && message.skills) {
    return <SkillsMarker message={message} update={message.skills} />
  }
  if (message.kind === 'turn-failure') {
    return <TurnFailureCard message={message} />
  }
  if (message.kind === 'model-note' && message.note) {
    return <ModelNoteMarker message={message} note={message.note} />
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
        'flex w-fit max-w-full min-w-0 items-start gap-3 rounded-xl px-4 py-3 sm:px-3 sm:py-2.5',
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
  const details = message.technicalDetails
  const known = details?.provider
    ? rememberedProvider(details.provider)
    : undefined
  const presentation = classifyTurnFailure(message, {
    providerHint: known?.context_overflow_hint,
    providerName: details?.provider
      ? (known?.display_name ??
        formatProviderLabel(details.provider) ??
        undefined)
      : undefined,
  })
  const failure = message.failure
  const Icon = CATEGORY_ICON[presentation.category]
  // A transient failure is amber (it will pass); anything someone has to fix
  // — the user's account or iii itself — is red.
  const severity: 'warn' | 'alert' =
    presentation.owner === 'environment' ? 'warn' : 'alert'
  // What the harness said in its own words sits with the raw error: the card
  // leads with the plain-language title and sentence.
  const summary = failure?.summary ?? splitNotice(message.content).headline
  const rows: TechnicalRow[] = [
    ...(summary ? [['Message', summary] as TechnicalRow] : []),
    ...technicalRows(details),
  ]
  const attempts = failure?.recoveryAttempted
  const maxAttempts = failure?.recoveryMaxAttempts ?? attempts
  // What else is true about the turn, as short facts under the explanation.
  const facts: Array<{ key: string; Icon: LucideIcon; text: string }> = [
    attempts
      ? {
          key: 'retried',
          Icon: History,
          text: `Already retried automatically ${attempts} of ${maxAttempts} ${
            attempts === 1 && maxAttempts === 1 ? 'time' : 'times'
          }.`,
        }
      : null,
    failure?.partialResultAvailable
      ? {
          key: 'partial',
          Icon: FileWarning,
          text: 'The reply above stopped partway, so it’s incomplete.',
        }
      : null,
    // "Temporary" already says it for network and provider hiccups.
    failure?.retryable && presentation.owner !== 'environment'
      ? {
          key: 'retryable',
          Icon: RotateCcw,
          text: 'Safe to try again once that’s fixed.',
        }
      : null,
  ].filter((fact) => fact !== null)

  return (
    <article
      className="w-full"
      data-message-role="turn-failure"
      data-message-tone="error"
      data-failure-category={presentation.category}
      data-failure-owner={presentation.owner}
      aria-labelledby={titleId}
    >
      <Card className="@container gap-0 py-0">
        <div className="flex flex-col gap-4 p-4 sm:p-3.5">
          <header className="flex min-w-0 items-start gap-3">
            <div
              className={cn(
                'flex size-9 shrink-0 items-center justify-center rounded-lg sm:size-8',
                severity === 'warn' ? 'bg-warn-muted' : 'bg-alert-muted',
              )}
            >
              <Icon
                aria-hidden
                className={cn(
                  'size-4 shrink-0',
                  severity === 'warn' ? 'stroke-warn' : 'stroke-alert',
                )}
              />
            </div>
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <h3
                id={titleId}
                className="min-w-0 text-balance font-inter text-base font-medium text-ink sm:text-sm"
              >
                {presentation.title}
              </h3>
              {details?.provider || details?.model ? (
                <div className="flex min-w-0 items-center gap-1.5 font-mono text-xs text-ink-faint">
                  {details.provider ? (
                    <span className="shrink-0">{details.provider}</span>
                  ) : null}
                  {details.provider && details.model ? (
                    <span aria-hidden>/</span>
                  ) : null}
                  {details.model ? (
                    <span className="min-w-0 truncate">{details.model}</span>
                  ) : null}
                </div>
              ) : null}
            </div>
            <Chip tone={OWNER_CHIP[presentation.owner]} className="shrink-0">
              {presentation.ownerLabel}
            </Chip>
          </header>

          <p
            data-failure-ownership
            className="text-pretty wrap-break-word font-inter text-base text-ink-faint sm:text-sm"
          >
            {presentation.ownership}
          </p>

          {facts.length > 0 ? (
            <ul className="flex flex-col gap-1.5 font-inter text-sm text-ink-faint sm:text-[0.8125rem]">
              {facts.map(({ key, Icon: FactIcon, text }) => (
                <li
                  key={key}
                  data-failure-fact={key}
                  className="flex min-w-0 items-start gap-2"
                >
                  <FactIcon
                    aria-hidden
                    className="mt-0.5 size-4 shrink-0 stroke-ink-faint"
                  />
                  <span className="text-pretty">{text}</span>
                </li>
              ))}
            </ul>
          ) : null}

          <FailureSteps
            actions={presentation.actions}
            note={presentation.providerNote}
          />
        </div>

        {rows.length > 0 ? <FailureDetails rows={rows} /> : null}
      </Card>
    </article>
  )
}

/** The fix, as numbered steps in their own quiet panel. */
function FailureSteps({ actions, note }: { actions: string[]; note?: string }) {
  if (actions.length === 0 && !note) return null
  return (
    <section
      data-message-next-actions
      className="flex flex-col gap-2 rounded-lg bg-surface p-3 font-inter text-base sm:text-sm"
    >
      <h4 className="text-xs font-medium text-ink-faint">What you can do</h4>
      <ol className="flex flex-col gap-2 text-pretty text-ink [counter-reset:step]">
        {actions.map((action) => (
          <li
            key={action}
            data-step
            className="relative pl-7 [counter-increment:step] before:absolute before:top-0.5 before:left-0 before:flex before:size-5 before:items-center before:justify-center before:rounded-full before:bg-panel-raised before:font-mono before:text-[11px] before:text-ink-faint before:ring-1 before:ring-edge before:content-[counter(step)]"
          >
            {action}
          </li>
        ))}
      </ol>
      {note ? (
        <p
          data-provider-hint
          className="text-pretty wrap-break-word text-ink-faint"
        >
          {note}
        </p>
      ) : null}
    </section>
  )
}

/** The card's footer band: the raw error behind a disclosure, with a copy
    button that stays reachable while it is closed. */
function FailureDetails({ rows }: { rows: TechnicalRow[] }) {
  const copyText = () => rows.map(([k, v]) => `${k}: ${v}`).join('\n')
  return (
    <details
      data-message-technical-details
      className="group border-t border-edge bg-surface/40"
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 px-4 py-2.5 font-inter text-sm text-ink-faint select-none hover:text-ink focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-rule-focus sm:px-3.5 [&::-webkit-details-marker]:hidden">
        <ChevronRight
          aria-hidden
          className="size-4 shrink-0 transition-transform duration-(--motion-duration-control) ease-(--motion-ease-standard) group-open:rotate-90"
        />
        <span className="flex-1">Technical details</span>
        <span className="hidden font-mono text-xs group-open:hidden @md:inline">
          {rows.find(([label]) => label === 'Code')?.[1]}
        </span>
      </summary>
      <div className="flex flex-col gap-3 px-4 pb-4 sm:px-3.5 sm:pb-3.5">
        <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 rounded-lg bg-panel-raised p-3 ring-1 ring-edge">
          {rows.map(([label, value]) => (
            <Fragment key={label}>
              <dt className="font-inter text-xs leading-5 text-ink-faint">
                {label}
              </dt>
              <dd
                data-technical-detail={label.toLowerCase()}
                className="min-w-0 font-mono text-xs leading-5 break-words text-ink"
              >
                {value}
              </dd>
            </Fragment>
          ))}
        </dl>
        <div className="flex items-center gap-1.5 font-inter text-sm text-ink-faint">
          <CopyMessageButton text={copyText} label="copy technical details" />
          <span>Copy for a bug report</span>
        </div>
      </div>
    </details>
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
          '@container observer-activity-collapsible',
          !open && 'observer-activity-collapsible--compact',
        )}
      >
        <CollapsibleCardTrigger
          className="group observer-activity-collapsible__trigger select-none"
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
                <span className="@lg:hidden">Folder </span>
                <span className="hidden @lg:inline">Working directory </span>
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
              <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-workdir-muted sm:size-9">
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
              <FactList className="gap-y-2">
                {scope.cause !== 'selected' || previous !== null ? (
                  <Fact label="Before" mono muted>
                    {previous ?? 'unscoped'}
                  </Fact>
                ) : null}
                <Fact
                  label="Now"
                  mono
                  className="flex min-w-0 items-baseline gap-2"
                >
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
                </Fact>
              </FactList>
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

/* ───────────────────────── skill index rows ────────────────────────── */

function SkillsMarker({
  message,
  update,
}: {
  message: SystemMessage
  update: SkillCatalogUpdate
}) {
  const [open, setOpen] = useState(false)
  const copy = skillsCopy(update)
  return (
    <article
      className="w-full"
      data-message-role="skills"
      data-message-id={message.id}
      data-skills-available={update.available}
      data-skills-count={update.entries.length}
      data-expanded={open}
    >
      <CollapsibleCard
        open={open}
        onOpenChange={setOpen}
        className={cn(
          '@container observer-activity-collapsible',
          !open && 'observer-activity-collapsible--compact',
        )}
      >
        <CollapsibleCardTrigger
          className="group observer-activity-collapsible__trigger select-none"
          aria-label={`${open ? 'Hide' : 'Show'} the skill index`}
        >
          <div className="flex w-full min-w-0 items-center gap-2">
            <span
              aria-hidden="true"
              className="activity-status-icon"
              data-status={update.available ? 'done' : 'error'}
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
            <TimelineActivityTrail kind="skills" />
            <div
              data-message-summary
              className="min-w-0 flex-1 truncate font-sans text-sm text-muted-foreground sm:text-[0.8125rem]"
              title={copy.title}
            >
              <span>Skills </span>
              {skillUpdateSummary(update)}
            </div>
            <TimelineActivityDisclosure />
          </div>
        </CollapsibleCardTrigger>

        <CollapsibleCardContent>
          <div className="flex flex-col gap-4 border-t border-edge p-4 sm:p-3">
            <div className="flex min-w-0 items-start gap-3">
              <div
                className={cn(
                  'flex size-10 shrink-0 items-center justify-center rounded-lg sm:size-9',
                  update.available ? 'bg-surface' : 'bg-warn-muted',
                )}
              >
                <BookOpen
                  aria-hidden
                  className={cn(
                    'size-5 shrink-0 sm:size-4',
                    update.available ? 'stroke-ink-faint' : 'stroke-warn',
                  )}
                />
              </div>
              <div className="flex min-w-0 flex-1 flex-col gap-1 font-sans text-base sm:text-sm">
                <div className="font-semibold text-ink">{copy.title}</div>
                <p className="text-pretty wrap-break-word text-ink-faint">
                  {copy.description}
                </p>
              </div>
            </div>
            {update.entries.length > 0 ? (
              <CardHighlight className="p-3">
                <ul data-skills-list className="flex flex-col gap-2.5 sm:gap-2">
                  {update.entries.map((entry) => (
                    <li
                      key={entry.id}
                      className="grid min-w-0 grid-cols-1 gap-x-4 gap-y-0.5 @2xl:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]"
                    >
                      <span
                        data-skill-id
                        className="min-w-0 truncate font-mono text-[12px] leading-5 text-ink"
                        title={entry.id}
                      >
                        {entry.id}
                      </span>
                      {entry.description ? (
                        <span
                          className="line-clamp-2 min-w-0 font-sans text-[12.5px] leading-5 text-ink-faint"
                          title={entry.description}
                        >
                          {entry.description}
                        </span>
                      ) : null}
                    </li>
                  ))}
                </ul>
              </CardHighlight>
            ) : null}
          </div>
        </CollapsibleCardContent>
      </CollapsibleCard>
    </article>
  )
}

function skillsCopy(update: SkillCatalogUpdate): {
  title: string
  description: string
} {
  if (!update.available) {
    return {
      title: 'Skill guidance withdrawn',
      description:
        'Skill lookup is no longer reachable from this session, so the agent was told not to rely on any skill listed earlier. Turns before this one keep the index they ran with.',
    }
  }
  const n = update.entries.length
  return {
    title: 'Skill index updated',
    description: `The skill directory changed, so the agent was handed a fresh index of ${n} ${n === 1 ? 'skill' : 'skills'}. It replaces the earlier list; a skill is read in full only when a task calls for it.`,
  }
}

/* ───────────────────────────── shared bits ─────────────────────────── */

function NextActions({
  actions,
  className,
}: {
  actions: string[]
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
      <div className="font-medium text-ink">What you can do</div>
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
  className,
}: {
  rows: TechnicalRow[]
  className?: string
}) {
  const copyText = () => rows.map(([k, v]) => `${k}: ${v}`).join('\n')
  return (
    <details data-message-technical-details className={cn('group', className)}>
      <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 font-sans text-base font-medium text-ink-faint select-none hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent sm:text-sm [&::-webkit-details-marker]:hidden">
        <ChevronRight
          aria-hidden
          className="size-4 shrink-0 stroke-ink-ghost transition-transform duration-(--motion-duration-control) ease-(--motion-ease-standard) group-open:rotate-90"
        />
        <span>Technical details</span>
      </summary>
      <div className="mt-2 flex flex-col gap-2">
        <FactList className="gap-y-1.5">
          {rows.map(([label, value]) => (
            <Fact
              key={label}
              label={label}
              mono
              muted
              data-technical-detail={label.toLowerCase()}
            >
              {value}
            </Fact>
          ))}
        </FactList>
        <div className="flex items-center gap-1.5 font-sans text-sm text-ink-faint sm:text-[0.8125rem]">
          <CopyMessageButton text={copyText} label="copy technical details" />
          <span>Copy for a bug report</span>
        </div>
      </div>
    </details>
  )
}
