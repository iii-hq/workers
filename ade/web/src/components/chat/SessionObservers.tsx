import { Check, ChevronRight, Copy, Loader2, Trash2, X } from 'lucide-react'
import { useEffect, useId, useMemo, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { filterChips } from '@/components/observer-activity/ObserverSource'
import { ObserverIcon } from '@/components/ui/ObserverIcon'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import { copyTextToClipboard } from '@/lib/clipboard'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import { composerCardClass, toolbarIconButtonClass } from './composer-chrome'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

interface SessionObserversProps {
  observers: SessionObserverInfo[]
  onDispose: (observerId: string) => Promise<void> | void
  /** Remove every observer at once (see ChatView). */
  onClearAll?: () => Promise<void> | void
  /** Backend probe: does this state key currently exist? (`null` = unknown) */
  checkStateKey?: (
    scope: string | undefined,
    key: string,
  ) => Promise<boolean | null>
  /** Start with the rows shown (stories, tests); the user's toggle still wins. */
  defaultExpanded?: boolean
}

/**
 * What a fire delivers, from the row's data alone: a wake into this chat, or
 * a plain function call. No observed function or target is special-cased —
 * a function that does not exist yet renders exactly like the known ones.
 */
export function deliveryLabel(observer: SessionObserverInfo): string {
  return observer.delivery.kind === 'call'
    ? `calls ${observer.delivery.functionId}`
    : 'notifies this chat'
}

/** The `$eq` operand of one filter field, when it is a non-empty string. */
function eqFilter(
  filters: Record<string, unknown> | undefined,
  field: string,
): string | undefined {
  const condition = filters?.[field]
  if (!condition || typeof condition !== 'object') return undefined
  const value = (condition as { $eq?: unknown }).$eq
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

/** The `(scope?, key)` a keyed `state::on-changed` observer watches, for the presence probe. */
export function stateWatch(
  observer: SessionObserverInfo,
): { scope?: string; key: string } | null {
  if (observer.functionId !== 'state::on-changed') return null
  const key = eqFilter(observer.filters, 'input.key')
  if (!key) return null
  return { scope: eqFilter(observer.filters, 'input.scope'), key }
}

/**
 * A compact, SOURCE-GENERIC summary: up to three scalar args, then the
 * filters as `field op value`. Known and unknown functions get the same
 * treatment — the row never interprets a source, so functions that do not
 * exist yet render sanely.
 */
export function summarizeObserver(
  observer: SessionObserverInfo,
): string | null {
  const args = Object.entries(observer.args ?? {})
    .filter(
      ([, v]) =>
        typeof v === 'string' ||
        typeof v === 'number' ||
        typeof v === 'boolean',
    )
    .map(([k, v]) => `${k}: ${String(v)}`)
  const filters = filterChips(observer.filters).map(
    ({ field, value }) => `${field} ${value}`,
  )
  const parts = [...args, ...filters].slice(0, 3)
  return parts.length > 0 ? parts.join(' · ') : null
}

/**
 * The row's lifecycle text, from structured lifecycle data alone. Historical
 * `retired` records do not say how the observer ended, so keep their copy
 * deliberately neutral instead of presenting every retirement as a removal.
 */
export function lifecycleNote(observer: SessionObserverInfo): string | null {
  if (observer.fired) {
    switch (observer.retirementReason) {
      case 'once_consumed':
        return 'once · consumed automatically'
      case 'max_fires':
        return 'fire limit reached'
      case 'expired':
        return 'expired'
      case 'unregistered':
        return 'removed'
      case 'invalidated':
        return 'invalidated'
      case 'exhausted':
        return 'exhausted'
    }

    // Records normally repeat these lifecycle events as both outcome and
    // retirement reason. Tolerate a partially enriched record while
    // preserving the same distinct labels.
    switch (observer.outcome) {
      case 'expired':
        return 'expired'
      case 'unregistered':
        return 'removed'
      case 'invalidated':
        return 'invalidated'
      default:
        return 'retired'
    }
  }
  const parts: string[] = []
  if (observer.once) parts.push('once')
  if ((observer.fires ?? 0) > 0)
    parts.push(`${observer.fires} fire${observer.fires === 1 ? '' : 's'}`)
  if (observer.maxFires !== undefined) parts.push(`max ${observer.maxFires}`)
  if (observer.expiresAt !== undefined)
    parts.push(`until ${new Date(observer.expiresAt).toLocaleString()}`)
  return parts.length > 0 ? parts.join(' · ') : null
}

/**
 * What "clear all" is about to do, from the counts alone. Live observers are
 * removed on the backend; inactive ghosts only leave this view.
 */
export function clearAllDescription(live: number, inactive: number): string {
  const parts: string[] = []
  if (live > 0)
    parts.push(
      `${live} ${live === 1 ? 'observer' : 'observers'} will be removed — nothing will notify this chat afterwards.`,
    )
  if (inactive > 0)
    parts.push(
      `${inactive} inactive ${inactive === 1 ? 'row' : 'rows'} will be dismissed.`,
    )
  return parts.join(' ')
}

function isEmptyObject(value: unknown): boolean {
  if (value === null || value === undefined) return true
  if (typeof value === 'object')
    return Object.keys(value as Record<string, unknown>).length === 0
  return false
}

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

/**
 * A full-width text row that owns the click (expand, open detail). Quiet
 * until hovered, the composer toolbar's height on either breakpoint, and its
 * leading icon lands on the same 16px inset as the composer's paperclip. The
 * hover shape nests concentrically inside the card (12px radius − 4px gutter).
 */
const rowButtonClass = cn(
  'flex h-12 min-w-0 flex-1 items-center gap-2 rounded-lg pr-2 pl-3 text-left font-sans',
  'hover:bg-surface-hover sm:h-8',
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
)

/** FCM-style labeled JSON section: tracked header row + wrapped highlight. */
function JsonSection({ label, value }: { label: string; value: unknown }) {
  return (
    <div className="border border-rule-2">
      <div className="border-b border-rule-2 bg-paper-2 px-3 py-1.5 iii-ui-eyebrow">
        {label}
      </div>
      <JsonHighlight code={formatJson(value)} wrap />
    </div>
  )
}

/** Monospace value with a copy affordance for long opaque ids. */
function CopyableId({ value }: { value: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <span className="inline-flex min-w-0 items-baseline gap-1.5">
      <span className="break-all">{value}</span>
      <button
        type="button"
        onClick={() => {
          // Helper, not navigator.clipboard: the API is undefined over
          // `http://<LAN-IP>` (insecure context) and the raw call no-ops.
          void copyTextToClipboard(value).then((ok) => {
            if (!ok) return
            setCopied(true)
            window.setTimeout(() => setCopied(false), 1200)
          })
        }}
        className="shrink-0 self-center text-ink-ghost hover:text-ink transition-colors"
        aria-label={copied ? 'copied' : 'copy id'}
        title={copied ? 'copied' : 'copy'}
      >
        {copied ? (
          <Check size={16} aria-hidden />
        ) : (
          <Copy size={16} aria-hidden />
        )}
      </button>
    </span>
  )
}

interface ObserverRowProps {
  observer: SessionObserverInfo
  busy: boolean
  onOpen: () => void
  onDispose: () => void
  /** Watched state key + whether it exists yet ("on scope/key — not written yet"). */
  stateNote?: string | null
}

/**
 * One observer: name, then the source-generic facts as a quiet suffix.
 * An inactive ghost keeps the same shape a step fainter — never dimmed with
 * opacity, so its hover and focus still read at full strength.
 */
function ObserverRow({
  observer,
  busy,
  onOpen,
  onDispose,
  stateNote,
}: ObserverRowProps) {
  const inactive = Boolean(observer.fired)
  const title = observer.label ?? observer.functionId
  const summary = stateNote ?? summarizeObserver(observer)
  const meta = [
    title === observer.functionId ? null : observer.functionId,
    deliveryLabel(observer),
    summary,
    lifecycleNote(observer),
  ].filter((part): part is string => part !== null)
  return (
    <li className="flex min-w-0 items-center gap-1">
      <button
        type="button"
        onClick={onOpen}
        title="show observer detail"
        className={cn(rowButtonClass, 'text-sm sm:text-[12px]')}
      >
        <ObserverIcon
          size={16}
          className={cn(
            'shrink-0',
            inactive ? 'fill-ink-ghost' : 'fill-ink-faint',
          )}
          aria-hidden
        />
        <span className="min-w-0 flex-1 truncate">
          <span className={inactive ? 'text-ink-faint' : 'text-ink'}>
            {title}
          </span>
          <span className={inactive ? 'text-ink-ghost' : 'text-ink-faint'}>
            {meta.map((part) => ` · ${part}`).join('')}
          </span>
        </span>
      </button>
      <button
        type="button"
        disabled={busy}
        onClick={onDispose}
        className={toolbarIconButtonClass}
        aria-label={`${inactive ? 'dismiss' : 'remove'} ${title}`}
        title={inactive ? 'dismiss' : 'remove'}
      >
        {busy ? (
          <Loader2 aria-hidden className="size-4 shrink-0 animate-spin" />
        ) : (
          <X aria-hidden className="size-4 shrink-0" />
        )}
      </button>
    </li>
  )
}

/**
 * One observer's detail: the function it observes with its args and
 * filters, what it delivers, its lifetime and id, with the remove (or, for
 * an inactive row, dismiss) button. Shared by the composer strip and the
 * chat's side panel.
 */
export function ObserverDetailDialog({
  observer,
  busy,
  onOpenChange,
  onDispose,
  onDismiss,
}: {
  observer: SessionObserverInfo | null
  busy: boolean
  onOpenChange: (open: boolean) => void
  onDispose: (id: string) => void
  /** An inactive row's dismiss button (it has nothing left to remove). */
  onDismiss?: (id: string) => void
}) {
  return (
    <Dialog open={observer !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogTitle className="text-[14px]">
          <span
            className="mr-2 inline-flex align-baseline text-ink-ghost"
            aria-hidden
          >
            <ObserverIcon size={16} className="fill-ink-ghost" />
          </span>
          {observer ? observer.label || observer.functionId : ''}
        </DialogTitle>
        <DialogDescription className="mt-1">
          observer created by this conversation's agent.
        </DialogDescription>
        {observer ? (
          <div className="mt-4 space-y-4 font-mono text-[12px]">
            <dl className="grid grid-cols-[max-content_1fr] items-baseline gap-x-4 gap-y-1.5">
              <dt className="text-ink-ghost">Observes</dt>
              <dd className="text-ink">{observer.functionId}</dd>
              {observer.namespace ? (
                <>
                  <dt className="text-ink-ghost">Namespace</dt>
                  <dd className="text-ink-faint">{observer.namespace}</dd>
                </>
              ) : null}
              <dt className="text-ink-ghost">Delivers</dt>
              <dd className="text-ink">{deliveryLabel(observer)}</dd>
              <dt className="text-ink-ghost">Lifetime</dt>
              <dd className="text-ink">
                {observer.fired
                  ? (lifecycleNote(observer) ?? 'retired')
                  : observer.once
                    ? 'once — retires after first fire'
                    : (lifecycleNote(observer) ?? 'until removed')}
              </dd>
              {observer.createdAt !== undefined ? (
                <>
                  <dt className="text-ink-ghost">Created</dt>
                  <dd className="text-ink-faint">
                    {new Date(observer.createdAt).toLocaleString()}
                  </dd>
                </>
              ) : null}
              <dt className="text-ink-ghost">Observer</dt>
              <dd className="text-ink-faint">
                <CopyableId value={observer.id} />
              </dd>
            </dl>
            {isEmptyObject(observer.args) ? null : (
              <JsonSection label="args" value={observer.args} />
            )}
            {isEmptyObject(observer.filters) ? null : (
              <JsonSection label="filters" value={observer.filters} />
            )}
            {observer.conditions && observer.conditions.length > 0 ? (
              <JsonSection label="conditions" value={observer.conditions} />
            ) : null}
            <div className="flex justify-end">
              {/* An inactive row has no live observer left to remove —
                  offering it would only produce a guaranteed error. */}
              {observer.fired ? (
                <Button
                  type="button"
                  variant="pill"
                  size="sm"
                  onClick={() => onDismiss?.(observer.id)}
                >
                  dismiss
                </Button>
              ) : (
                <Button
                  type="button"
                  variant="pill"
                  size="sm"
                  disabled={busy}
                  onClick={() => onDispose(observer.id)}
                >
                  {busy ? 'removing…' : 'remove'}
                </Button>
              )}
            </div>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  )
}

/**
 * The observers the conversation's agent created, stacked above the
 * composer next to the queued-messages strip, in the composer's own material
 * (see `composer-chrome`) so the footer reads as one instrument. Collapsed by
 * default to a one-line count; expanding unfolds one generic row per
 * observer — observed function, delivery, args and filters, lifecycle —
 * straight from the harness's rows, with no source- or target-specific
 * interpretation. A row opens the detail dialog; its ✕ (or the dialog
 * button) removes the observer; the trash at the edge clears everything
 * after a confirmation.
 */
export function SessionObservers({
  observers,
  onDispose,
  onClearAll,
  checkStateKey,
  defaultExpanded = false,
}: SessionObserversProps) {
  const [expanded, setExpanded] = useState(defaultExpanded)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [confirmingClear, setConfirmingClear] = useState(false)
  const [clearing, setClearing] = useState(false)
  // Inactive ghost rows the user dismissed — local per-tab view state; they
  // resurrect from the transcript on reload, so no persistence needed.
  const [dismissed, setDismissed] = useState<Set<string>>(() => new Set())
  const listId = useId()

  const visibleObservers = useMemo(
    () => observers.filter((t) => !dismissed.has(t.id)),
    [observers, dismissed],
  )
  const liveObservers = useMemo(
    () => visibleObservers.filter((t) => !t.fired),
    [visibleObservers],
  )
  const activeCount = liveObservers.length
  const inactiveCount = visibleObservers.length - activeCount

  // Whether each keyed state observer's watched key exists yet — the
  // row-level diagnosis for a wake armed on a key nothing ever writes.
  // ponytail: refetches on each observer-list refresh while open; cache if it matters.
  const [keyPresence, setKeyPresence] = useState<Record<string, boolean>>({})
  useEffect(() => {
    if (!expanded || !checkStateKey) return
    let alive = true
    for (const observer of visibleObservers) {
      const watch = stateWatch(observer)
      if (!watch) continue
      void checkStateKey(watch.scope, watch.key).then((present) => {
        if (alive && present !== null)
          setKeyPresence((m) => ({ ...m, [observer.id]: present }))
      })
    }
    return () => {
      alive = false
    }
  }, [expanded, checkStateKey, visibleObservers])

  const stateNote = (observer: SessionObserverInfo): string | null => {
    const watch = stateWatch(observer)
    if (!watch) return null
    const label = watch.scope ? `${watch.scope}/${watch.key}` : watch.key
    const present = keyPresence[observer.id]
    if (present === undefined) return `on ${label}`
    return present ? `on ${label} — written` : `on ${label} — not written yet`
  }
  const selected = visibleObservers.find((t) => t.id === selectedId) ?? null

  if (visibleObservers.length === 0) return null

  const dispose = async (id: string) => {
    setBusyId(id)
    try {
      await onDispose(id)
      setSelectedId((current) => (current === id ? null : current))
    } finally {
      setBusyId(null)
    }
  }

  const dismiss = (id: string) => {
    setDismissed((prev) => new Set(prev).add(id))
    setSelectedId((current) => (current === id ? null : current))
  }

  // An inactive ghost row has no live observer — its ✕ dismisses locally; a
  // live row's ✕ removes the observer.
  const rowAction = (t: SessionObserverInfo) =>
    t.fired ? dismiss(t.id) : void dispose(t.id)

  const clearAll = async () => {
    setClearing(true)
    try {
      await onClearAll?.()
      // Live observers are removed by onClearAll; inactive ghosts have no
      // live observer, so sweep them from view here too.
      setDismissed((prev) => {
        const next = new Set(prev)
        for (const t of visibleObservers) if (t.fired) next.add(t.id)
        return next
      })
      setSelectedId(null)
    } finally {
      setClearing(false)
    }
  }

  return (
    <>
      <section
        className={cn('mb-1', composerCardClass)}
        aria-label="active observers"
      >
        {/* Header: the count is the disclosure (chevron rides with the text,
            as on the model picker); the trash sits at the edge, where the
            composer keeps its action button. */}
        <div className="flex min-w-0 items-center gap-1 p-1">
          <button
            type="button"
            onClick={() => setExpanded((current) => !current)}
            aria-expanded={expanded}
            aria-controls={listId}
            className={cn(rowButtonClass, 'text-sm sm:text-[13px]')}
          >
            <ObserverIcon
              size={16}
              className="shrink-0 fill-ink-faint"
              aria-hidden
            />
            <span className="min-w-0 truncate">
              {/* The noun rides only where it fits: on a phone the icon
                  says "observers" and the line keeps both counts instead. */}
              <span className="font-medium text-ink">
                {activeCount}{' '}
                <span className="hidden @lg:inline">
                  observer{activeCount === 1 ? '' : 's'}{' '}
                </span>
                active
              </span>
              {inactiveCount > 0 ? (
                <span className="text-ink-ghost">
                  {' '}
                  · {inactiveCount} inactive
                </span>
              ) : null}
            </span>
            <ChevronRight
              aria-hidden
              className={cn(
                'size-4 shrink-0 text-ink-ghost transition-transform duration-(--motion-duration-control) ease-(--motion-ease-standard)',
                expanded && 'rotate-90',
              )}
            />
          </button>
          {onClearAll ? (
            <button
              type="button"
              onClick={() => setConfirmingClear(true)}
              disabled={clearing}
              aria-label="clear all observers"
              title="clear all observers"
              className={toolbarIconButtonClass}
            >
              {clearing ? (
                <Loader2 aria-hidden className="size-4 shrink-0 animate-spin" />
              ) : (
                <Trash2 aria-hidden className="size-4 shrink-0" />
              )}
            </button>
          ) : null}
        </div>

        {/* The rows unfold like the composer's editor grows: a tracked
            height, not a pop. They stay mounted while folded (the probe
            effect above is what gates the network), inert so nothing
            hidden takes a tab stop. */}
        <div
          id={listId}
          className={cn(
            'grid transition-[grid-template-rows] duration-(--motion-duration-panel) ease-(--motion-ease-standard)',
            expanded ? 'grid-rows-[1fr]' : 'grid-rows-[0fr]',
          )}
        >
          <div
            className="min-h-0 overflow-hidden"
            inert={expanded ? undefined : true}
          >
            <ul className="px-1 pb-1">
              {visibleObservers.map((observer) => (
                <ObserverRow
                  key={observer.id}
                  observer={observer}
                  stateNote={stateNote(observer)}
                  busy={busyId === observer.id}
                  onOpen={() => setSelectedId(observer.id)}
                  onDispose={() => rowAction(observer)}
                />
              ))}
            </ul>
          </div>
        </div>
      </section>

      <ConfirmDialog
        open={confirmingClear}
        onOpenChange={setConfirmingClear}
        title="Clear all observers?"
        description={clearAllDescription(activeCount, inactiveCount)}
        confirmLabel="Clear all"
        cancelLabel="Keep them"
        onConfirm={() => void clearAll()}
      />

      <ObserverDetailDialog
        observer={selected}
        busy={selected !== null && busyId === selected.id}
        onOpenChange={(open) => {
          if (!open) setSelectedId(null)
        }}
        onDispose={(id) => void dispose(id)}
        onDismiss={dismiss}
      />
    </>
  )
}
