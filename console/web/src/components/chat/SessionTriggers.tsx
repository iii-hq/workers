import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Trash2,
  Zap,
} from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import { JsonHighlight } from '@/lib/syntax'

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

/** The row's lifecycle ghost text, from lifecycle data alone. */
export function lifecycleNote(trigger: SessionTriggerInfo): string | null {
  if (trigger.fired) return 'fired · unregistered'
  const parts: string[] = []
  if (trigger.once) parts.push('once')
  if ((trigger.fires ?? 0) > 0)
    parts.push(`${trigger.fires} fire${trigger.fires === 1 ? '' : 's'}`)
  if (trigger.maxFires !== undefined) parts.push(`max ${trigger.maxFires}`)
  if (trigger.expiresAt !== undefined)
    parts.push(`until ${new Date(trigger.expiresAt).toLocaleString()}`)
  return parts.length > 0 ? parts.join(' · ') : null
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
          if (typeof navigator === 'undefined' || !navigator.clipboard) return
          void navigator.clipboard.writeText(value).then(() => {
            setCopied(true)
            window.setTimeout(() => setCopied(false), 1200)
          })
        }}
        className="shrink-0 self-center text-ink-ghost hover:text-ink transition-colors"
        aria-label={copied ? 'copied' : 'copy id'}
        title={copied ? 'copied' : 'copy'}
      >
        {copied ? (
          <Check size={11} aria-hidden />
        ) : (
          <Copy size={11} aria-hidden />
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

function TriggerRow({
  trigger,
  busy,
  onOpen,
  onUnregister,
  stateNote,
}: TriggerRowProps) {
  const name = trigger.label ?? null
  const summary = stateNote ?? summarizeTriggerConfig(trigger.config)
  const lifecycle = lifecycleNote(trigger)
  return (
    <div
      className={`flex items-center gap-2 border-b border-rule-2 px-3 py-1.5 text-[12px] last:border-b-0${trigger.fired ? ' opacity-55' : ''}`}
    >
      <Zap size={12} className="shrink-0 text-ink-ghost" aria-hidden />
      <button
        type="button"
        onClick={onOpen}
        className="min-w-0 flex-1 truncate text-left hover:text-ink transition-colors"
        title="show subscription detail"
      >
        {name || trigger.triggerType}
        <span className="text-ink-ghost">
          {name ? ` · ${trigger.triggerType}` : ''}
          {` · ${deliveryLabel(trigger)}`}
          {summary ? ` · ${summary}` : ''}
          {lifecycle ? ` · ${lifecycle}` : ''}
        </span>
      </button>
      <button
        type="button"
        disabled={busy}
        onClick={onUnregister}
        className="shrink-0 lowercase text-ink-ghost hover:text-ink transition-colors disabled:opacity-50"
        aria-label={`${trigger.fired ? 'dismiss' : 'unregister'} ${name ?? trigger.triggerType}`}
        title={trigger.fired ? 'dismiss' : 'unregister'}
      >
        {busy ? '…' : '✕'}
      </button>
    </div>
  )
}

/**
 * The conversation's registered trigger subscriptions, stacked above the
 * composer next to the queued-messages strip. Collapsed by default to a
 * count header; expanding shows one generic row per subscription — event
 * source, delivery, config summary, lifecycle — straight from the harness's
 * binding rows, with no source- or target-specific interpretation. Click a
 * row for the full detail dialog; ✕ (or the dialog button) tears the
 * subscription down.
 */
export function SessionTriggers({
  triggers,
  onUnregister,
  onClearAll,
  checkStateKey,
}: SessionTriggersProps) {
  const [expanded, setExpanded] = useState(false)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [clearArming, setClearArming] = useState(false)
  const [clearing, setClearing] = useState(false)
  // Fired ghost rows the user dismissed — local per-tab view state; they
  // resurrect from the transcript on reload, so no persistence needed.
  const [dismissed, setDismissed] = useState<Set<string>>(() => new Set())

  const visibleTriggers = useMemo(
    () => triggers.filter((t) => !dismissed.has(t.id)),
    [triggers, dismissed],
  )
  const liveTriggers = useMemo(
    () => visibleTriggers.filter((t) => !t.fired),
    [visibleTriggers],
  )
  const registeredCount = liveTriggers.length
  const firedCount = visibleTriggers.length - registeredCount

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

  // A fired ghost row has no live binding — its ✕ dismisses locally; a live
  // row's ✕ tears the subscription down.
  const rowAction = (t: SessionTriggerInfo) =>
    t.fired ? dismiss(t.id) : void unregister(t.id)

  const clearAll = async () => {
    setClearing(true)
    try {
      await onClearAll?.()
      // Live bindings are unregistered by onClearAll; fired ghosts have no
      // live binding, so sweep them from view here too.
      setDismissed((prev) => {
        const next = new Set(prev)
        for (const t of visibleTriggers) if (t.fired) next.add(t.id)
        return next
      })
      setSelectedId(null)
    } finally {
      setClearing(false)
      setClearArming(false)
    }
  }

  return (
    <>
      <section
        className="mb-1 border border-rule bg-bg"
        aria-label="registered triggers"
      >
        {clearArming ? (
          <div className="flex items-center gap-2 px-3 py-1.5 text-[12px]">
            <Trash2 size={12} className="shrink-0 text-alert" aria-hidden />
            <span className="min-w-0 flex-1 truncate">
              unregister all {registeredCount} triggers?
              <span className="text-ink-ghost">
                {' '}
                nothing will notify this chat afterwards.
              </span>
            </span>
            <button
              type="button"
              onClick={() => void clearAll()}
              disabled={clearing}
              className="shrink-0 lowercase text-alert hover:text-alert/80 transition-colors disabled:opacity-50"
            >
              {clearing ? 'clearing…' : 'clear all'}
            </button>
            <button
              type="button"
              onClick={() => setClearArming(false)}
              disabled={clearing}
              className="shrink-0 lowercase text-ink-ghost hover:text-ink transition-colors disabled:opacity-50"
            >
              cancel
            </button>
          </div>
        ) : (
          <div className="flex items-center">
            <button
              type="button"
              onClick={() => setExpanded((current) => !current)}
              aria-expanded={expanded}
              className="flex min-w-0 flex-1 items-center gap-2 py-1.5 pl-3 text-[12px] hover:text-ink transition-colors"
            >
              <Zap size={12} className="shrink-0 text-ink-ghost" aria-hidden />
              <span className="min-w-0 flex-1 truncate text-left">
                {registeredCount} trigger{registeredCount === 1 ? '' : 's'}{' '}
                registered
                <span className="text-ink-ghost">
                  {firedCount > 0 ? ` · ${firedCount} fired` : ''}
                </span>
              </span>
            </button>
            {onClearAll ? (
              <div className="flex shrink-0 items-center gap-1 pr-1 font-mono text-[11px] uppercase tracking-[0.06em]">
                <button
                  type="button"
                  onClick={() => setClearArming(true)}
                  className="flex items-center gap-1 px-2 py-1.5 text-ink-faint hover:text-alert transition-colors"
                  title="unregister every trigger"
                >
                  <Trash2 size={12} aria-hidden />
                  clear all
                </button>
              </div>
            ) : null}
            <button
              type="button"
              onClick={() => setExpanded((current) => !current)}
              aria-label={expanded ? 'collapse triggers' : 'expand triggers'}
              className="shrink-0 px-2 py-1.5 text-ink-ghost hover:text-ink transition-colors"
            >
              {expanded ? (
                <ChevronDown size={12} aria-hidden />
              ) : (
                <ChevronRight size={12} aria-hidden />
              )}
            </button>
          </div>
        )}
        {expanded ? (
          <div className="border-t border-rule-2">
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
          </div>
        ) : null}
      </section>

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
              <Zap size={13} />
            </span>
            {selected ? selected.label || selected.triggerType : ''}
          </DialogTitle>
          <DialogDescription className="mt-1">
            trigger subscription registered by this conversation's agent.
          </DialogDescription>
          {selected ? (
            <div className="mt-4 space-y-4 font-mono text-[12px]">
              <dl className="grid grid-cols-[max-content_1fr] items-baseline gap-x-4 gap-y-1.5">
                <dt className="lowercase text-ink-ghost">fires on</dt>
                <dd className="text-ink">{selected.triggerType}</dd>
                <dt className="lowercase text-ink-ghost">delivers</dt>
                <dd className="text-ink">{deliveryLabel(selected)}</dd>
                <dt className="lowercase text-ink-ghost">lifetime</dt>
                <dd className="text-ink">
                  {selected.fired
                    ? 'fired — already unregistered'
                    : selected.once
                      ? 'once — retires after first fire'
                      : (lifecycleNote(selected) ?? 'until unregistered')}
                </dd>
                {selected.createdAt !== undefined ? (
                  <>
                    <dt className="lowercase text-ink-ghost">registered</dt>
                    <dd className="text-ink-faint">
                      {new Date(selected.createdAt).toLocaleString()}
                    </dd>
                  </>
                ) : null}
                <dt className="lowercase text-ink-ghost">subscription</dt>
                <dd className="text-ink-faint">
                  <CopyableId value={selected.id} />
                </dd>
                {selected.triggerId ? (
                  <>
                    <dt className="lowercase text-ink-ghost">trigger id</dt>
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
                {/* A fired row has no live binding left to unregister —
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
