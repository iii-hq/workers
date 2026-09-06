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
import { TriggerIcon } from '@/components/ui/TriggerIcon'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import { copyTextToClipboard } from '@/lib/clipboard'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import { composerCardClass, toolbarIconButtonClass } from './composer-chrome'

interface SessionTriggersProps {
  triggers: SessionTriggerInfo[]
  onUnregister: (subscriptionId: string) => Promise<void> | void
  /** Unregister every binding at once (see ChatView). */
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
 * a plain function call. No trigger source or target is special-cased — a
 * source type that does not exist yet renders exactly like the known ones.
 */
export function deliveryLabel(trigger: SessionTriggerInfo): string {
  return trigger.delivery.kind === 'call'
    ? `calls ${trigger.delivery.functionId}`
    : 'notifies this chat'
}

/** The `(scope?, key)` a keyed `state` binding watches, for the presence probe. */
export function stateWatch(
  trigger: SessionTriggerInfo,
): { scope?: string; key: string } | null {
  if (trigger.triggerType !== 'state') return null
  const config = (trigger.config ?? {}) as Record<string, unknown>
  const key = typeof config.key === 'string' ? config.key : null
  if (!key) return null
  const scope = typeof config.scope === 'string' ? config.scope : undefined
  return { scope, key }
}

/**
 * A compact, SOURCE-GENERIC config summary: up to three scalar top-level
 * entries. Known and unknown trigger types get the same treatment — the row
 * never interprets a source, so future trigger types render sanely.
 */
export function summarizeTriggerConfig(config: unknown): string | null {
  if (config === null || config === undefined || typeof config !== 'object')
    return null
  const parts = Object.entries(config as Record<string, unknown>)
    .filter(
      ([, v]) =>
        typeof v === 'string' ||
        typeof v === 'number' ||
        typeof v === 'boolean',
    )
    .slice(0, 3)
    .map(([k, v]) => `${k}: ${String(v)}`)
  return parts.length > 0 ? parts.join(' · ') : null
}

/**
 * The row's lifecycle text, from structured lifecycle data alone. Historical
 * `retired` records do not say how the binding ended, so keep their copy
 * deliberately neutral instead of presenting every retirement as an unbind.
 */
export function lifecycleNote(trigger: SessionTriggerInfo): string | null {
  if (trigger.fired) {
    switch (trigger.retirementReason) {
      case 'once_consumed':
        return 'once · consumed automatically'
      case 'max_fires':
        return 'fire limit reached'
      case 'expired':
        return 'expired'
      case 'unregistered':
        return 'unregistered'
      case 'invalidated':
        return 'invalidated'
      case 'exhausted':
        return 'exhausted'
    }

    // Newer records normally repeat these lifecycle events as both outcome
    // and retirement reason. Tolerate a partially enriched record while
    // preserving the same distinct labels.
    switch (trigger.outcome) {
      case 'expired':
        return 'expired'
      case 'unregistered':
        return 'unregistered'
      case 'invalidated':
        return 'invalidated'
      default:
        return 'retired'
    }
  }
  const parts: string[] = []
  if (trigger.once) parts.push('once')
  if ((trigger.fires ?? 0) > 0)
    parts.push(`${trigger.fires} fire${trigger.fires === 1 ? '' : 's'}`)
  if (trigger.maxFires !== undefined) parts.push(`max ${trigger.maxFires}`)
  if (trigger.expiresAt !== undefined)
    parts.push(`until ${new Date(trigger.expiresAt).toLocaleString()}`)
  return parts.length > 0 ? parts.join(' · ') : null
}

/**
 * What "clear all" is about to do, from the counts alone. Live bindings are
 * torn down on the backend; inactive ghosts only leave this view.
 */
export function clearAllDescription(live: number, inactive: number): string {
  const parts: string[] = []
  if (live > 0)
    parts.push(
      `${live} ${live === 1 ? 'trigger' : 'triggers'} will be unregistered — nothing will notify this chat afterwards.`,
    )
  if (inactive > 0)
    parts.push(
      `${inactive} inactive ${inactive === 1 ? 'row' : 'rows'} will be dismissed.`,
    )
  return parts.join(' ')
}

function isEmptyConfig(config: unknown): boolean {
  if (config === null || config === undefined) return true
  if (typeof config === 'object')
    return Object.keys(config as Record<string, unknown>).length === 0
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
      <div className="border-b border-rule-2 bg-paper-2 px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
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

interface TriggerRowProps {
  trigger: SessionTriggerInfo
  busy: boolean
  onOpen: () => void
  onUnregister: () => void
  /** Watched state key + whether it exists yet ("on scope/key — not written yet"). */
  stateNote?: string | null
}

/**
 * One subscription: name, then the source-generic facts as a quiet suffix.
 * An inactive ghost keeps the same shape a step fainter — never dimmed with
 * opacity, so its hover and focus still read at full strength.
 */
function TriggerRow({
  trigger,
  busy,
  onOpen,
  onUnregister,
  stateNote,
}: TriggerRowProps) {
  const inactive = Boolean(trigger.fired)
  const title = trigger.label ?? trigger.triggerType
  const summary = stateNote ?? summarizeTriggerConfig(trigger.config)
  const meta = [
    title === trigger.triggerType ? null : trigger.triggerType,
    deliveryLabel(trigger),
    summary,
    lifecycleNote(trigger),
  ].filter((part): part is string => part !== null)
  return (
    <li className="flex min-w-0 items-center gap-1">
      <button
        type="button"
        onClick={onOpen}
        title="show subscription detail"
        className={cn(rowButtonClass, 'text-sm sm:text-[12px]')}
      >
        <TriggerIcon
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
        onClick={onUnregister}
        className={toolbarIconButtonClass}
        aria-label={`${inactive ? 'dismiss' : 'unregister'} ${title}`}
        title={inactive ? 'dismiss' : 'unregister'}
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
 * The conversation's registered trigger subscriptions, stacked above the
 * composer next to the queued-messages strip, in the composer's own material
 * (see `composer-chrome`) so the footer reads as one instrument. Collapsed by
 * default to a one-line count; expanding unfolds one generic row per
 * subscription — event source, delivery, config summary, lifecycle — straight
 * from the harness's binding rows, with no source- or target-specific
 * interpretation. A row opens the detail dialog; its ✕ (or the dialog button)
 * tears the subscription down; the trash at the edge clears everything after
 * a confirmation.
 */
export function SessionTriggers({
  triggers,
  onUnregister,
  onClearAll,
  checkStateKey,
  defaultExpanded = false,
}: SessionTriggersProps) {
  const [expanded, setExpanded] = useState(defaultExpanded)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [confirmingClear, setConfirmingClear] = useState(false)
  const [clearing, setClearing] = useState(false)
  // Inactive ghost rows the user dismissed — local per-tab view state; they
  // resurrect from the transcript on reload, so no persistence needed.
  const [dismissed, setDismissed] = useState<Set<string>>(() => new Set())
  const listId = useId()

  const visibleTriggers = useMemo(
    () => triggers.filter((t) => !dismissed.has(t.id)),
    [triggers, dismissed],
  )
  const liveTriggers = useMemo(
    () => visibleTriggers.filter((t) => !t.fired),
    [visibleTriggers],
  )
  const registeredCount = liveTriggers.length
  const inactiveCount = visibleTriggers.length - registeredCount

  // Whether each keyed state binding's watched key exists yet — the
  // row-level diagnosis for a wake armed on a key nothing ever writes.
  // ponytail: refetches on each trigger-poll tick while open; cache if it matters.
  const [keyPresence, setKeyPresence] = useState<Record<string, boolean>>({})
  useEffect(() => {
    if (!expanded || !checkStateKey) return
    let alive = true
    for (const trigger of visibleTriggers) {
      const watch = stateWatch(trigger)
      if (!watch) continue
      void checkStateKey(watch.scope, watch.key).then((present) => {
        if (alive && present !== null)
          setKeyPresence((m) => ({ ...m, [trigger.id]: present }))
      })
    }
    return () => {
      alive = false
    }
  }, [expanded, checkStateKey, visibleTriggers])

  const stateNote = (trigger: SessionTriggerInfo): string | null => {
    const watch = stateWatch(trigger)
    if (!watch) return null
    const label = watch.scope ? `${watch.scope}/${watch.key}` : watch.key
    const present = keyPresence[trigger.id]
    if (present === undefined) return `on ${label}`
    return present ? `on ${label} — written` : `on ${label} — not written yet`
  }
  const selected = visibleTriggers.find((t) => t.id === selectedId) ?? null

  if (visibleTriggers.length === 0) return null

  const unregister = async (id: string) => {
    setBusyId(id)
    try {
      await onUnregister(id)
      setSelectedId((current) => (current === id ? null : current))
    } finally {
      setBusyId(null)
    }
  }

  const dismiss = (id: string) => {
    setDismissed((prev) => new Set(prev).add(id))
    setSelectedId((current) => (current === id ? null : current))
  }

  // An inactive ghost row has no live binding — its ✕ dismisses locally; a live
  // row's ✕ tears the subscription down.
  const rowAction = (t: SessionTriggerInfo) =>
    t.fired ? dismiss(t.id) : void unregister(t.id)

  const clearAll = async () => {
    setClearing(true)
    try {
      await onClearAll?.()
      // Live bindings are unregistered by onClearAll; inactive ghosts have no
      // live binding, so sweep them from view here too.
      setDismissed((prev) => {
        const next = new Set(prev)
        for (const t of visibleTriggers) if (t.fired) next.add(t.id)
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
        aria-label="registered triggers"
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
            <TriggerIcon
              size={16}
              className="shrink-0 fill-ink-faint"
              aria-hidden
            />
            <span className="min-w-0 truncate">
              {/* The noun rides only where it fits: on a phone the icon
                  says "triggers" and the line keeps both counts instead. */}
              <span className="font-medium text-ink">
                {registeredCount}{' '}
                <span className="hidden sm:inline">
                  trigger{registeredCount === 1 ? '' : 's'}{' '}
                </span>
                registered
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
              aria-label="clear all triggers"
              title="clear all triggers"
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
              {visibleTriggers.map((trigger) => (
                <TriggerRow
                  key={trigger.id}
                  trigger={trigger}
                  stateNote={stateNote(trigger)}
                  busy={busyId === trigger.id}
                  onOpen={() => setSelectedId(trigger.id)}
                  onUnregister={() => rowAction(trigger)}
                />
              ))}
            </ul>
          </div>
        </div>
      </section>

      <ConfirmDialog
        open={confirmingClear}
        onOpenChange={setConfirmingClear}
        title="Clear all triggers?"
        description={clearAllDescription(registeredCount, inactiveCount)}
        confirmLabel="Clear all"
        cancelLabel="Keep them"
        onConfirm={() => void clearAll()}
      />

      <Dialog
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) setSelectedId(null)
        }}
      >
        <DialogContent className="max-w-lg">
          <DialogTitle className="text-[14px] lowercase">
            <span
              className="mr-2 inline-flex align-baseline text-ink-ghost"
              aria-hidden
            >
              <TriggerIcon size={16} className="fill-ink-ghost" />
            </span>
            {selected ? selected.label || selected.triggerType : ''}
          </DialogTitle>
          <DialogDescription className="mt-1">
            trigger subscription registered by this conversation's agent.
          </DialogDescription>
          {selected ? (
            <div className="mt-4 space-y-4 font-mono text-[12px]">
              <dl className="grid grid-cols-[max-content_1fr] items-baseline gap-x-4 gap-y-1.5">
                <dt className="text-ink-ghost">Fires on</dt>
                <dd className="text-ink">{selected.triggerType}</dd>
                <dt className="text-ink-ghost">Delivers</dt>
                <dd className="text-ink">{deliveryLabel(selected)}</dd>
                <dt className="text-ink-ghost">Lifetime</dt>
                <dd className="text-ink">
                  {selected.fired
                    ? (lifecycleNote(selected) ?? 'retired')
                    : selected.once
                      ? 'once — retires after first fire'
                      : (lifecycleNote(selected) ?? 'until unregistered')}
                </dd>
                {selected.createdAt !== undefined ? (
                  <>
                    <dt className="text-ink-ghost">Registered</dt>
                    <dd className="text-ink-faint">
                      {new Date(selected.createdAt).toLocaleString()}
                    </dd>
                  </>
                ) : null}
                <dt className="text-ink-ghost">Subscription</dt>
                <dd className="text-ink-faint">
                  <CopyableId value={selected.id} />
                </dd>
                {selected.triggerId ? (
                  <>
                    <dt className="text-ink-ghost">Trigger ID</dt>
                    <dd className="text-ink-faint">
                      <CopyableId value={selected.triggerId} />
                    </dd>
                  </>
                ) : null}
              </dl>
              {isEmptyConfig(selected.config) ? null : (
                <JsonSection label="config" value={selected.config} />
              )}
              {selected.conditions && selected.conditions.length > 0 ? (
                <JsonSection label="conditions" value={selected.conditions} />
              ) : null}
              <div className="flex justify-end">
                {/* An inactive row has no live binding left to unregister —
                    offering it would only produce a guaranteed error. */}
                {selected.fired ? (
                  <Button
                    type="button"
                    variant="pill"
                    size="sm"
                    onClick={() => dismiss(selected.id)}
                  >
                    dismiss
                  </Button>
                ) : (
                  <Button
                    type="button"
                    variant="pill"
                    size="sm"
                    disabled={busyId === selected.id}
                    onClick={() => void unregister(selected.id)}
                  >
                    {busyId === selected.id ? 'unregistering…' : 'unregister'}
                  </Button>
                )}
              </div>
            </div>
          ) : null}
        </DialogContent>
      </Dialog>
    </>
  )
}
