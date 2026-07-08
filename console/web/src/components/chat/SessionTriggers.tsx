import { Check, Copy, GitMerge, Zap } from 'lucide-react'
import { useMemo, useState } from 'react'
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
  onUnregister: (triggerId: string) => Promise<void> | void
}

function targetLabel(trigger: SessionTriggerInfo): string {
  return trigger.functionId === 'harness::react'
    ? 'spawns sub-agent'
    : 'notifies this chat'
}

/* ------------------------------------------------------------------ */
/* Workflow structure derived from the bindings themselves:            */
/* - join groups: react bindings sharing `metadata.join.id` (fan-in)   */
/* - chain edges: A spawns into session S (`metadata.session_id`) and  */
/*   B watches S complete (`config.session_id` on a turn-event type)   */
/* Rendered as topological stages with a ↓ between them; flat when     */
/* nothing is connected.                                               */
/* ------------------------------------------------------------------ */

interface JoinMeta {
  id: string
  expect: string[]
  key?: string
}

function joinMeta(trigger: SessionTriggerInfo): JoinMeta | null {
  const join = trigger.metadata?.join
  if (!join || typeof join !== 'object') return null
  const j = join as Record<string, unknown>
  if (typeof j.id !== 'string') return null
  return {
    id: j.id,
    expect: Array.isArray(j.expect)
      ? j.expect.filter((k): k is string => typeof k === 'string')
      : [],
    key: typeof j.key === 'string' ? j.key : undefined,
  }
}

/** The session this binding's reaction spawns into (explicit targets only). */
function spawnTarget(trigger: SessionTriggerInfo): string | null {
  if (trigger.functionId !== 'harness::react') return null
  const target = trigger.metadata?.session_id
  return typeof target === 'string' ? target : null
}

/** The session whose turn events this binding watches. */
function watchedSession(trigger: SessionTriggerInfo): string | null {
  if (
    trigger.triggerType !== 'harness::turn-completed' &&
    trigger.triggerType !== 'harness::turn-started'
  ) {
    return null
  }
  const config = trigger.config as Record<string, unknown> | null | undefined
  const watched = config?.session_id
  return typeof watched === 'string' ? watched : null
}

export interface TriggerUnit {
  key: string
  /** Set when this unit is a join fan-in group. */
  join: { id: string; expect: string[] } | null
  /** The bindings in the unit (exactly one unless `join` is set). */
  members: SessionTriggerInfo[]
}

export interface TriggerWorkflow {
  /** Topological stages, upstream first. */
  levels: TriggerUnit[][]
  /** False → nothing is connected; render the flat list. */
  hasStructure: boolean
}

export function buildTriggerWorkflow(
  triggers: SessionTriggerInfo[],
): TriggerWorkflow {
  const groups = new Map<string, SessionTriggerInfo[]>()
  const singles: SessionTriggerInfo[] = []
  for (const trigger of triggers) {
    const join = joinMeta(trigger)
    if (join) {
      const list = groups.get(join.id) ?? []
      list.push(trigger)
      groups.set(join.id, list)
    } else {
      singles.push(trigger)
    }
  }

  const units: TriggerUnit[] = [
    ...[...groups.entries()].map(([id, members]) => ({
      key: `join:${id}`,
      join: { id, expect: joinMeta(members[0])?.expect ?? [] },
      members,
    })),
    ...singles.map((trigger) => ({
      key: `t:${trigger.id}`,
      join: null,
      members: [trigger],
    })),
  ]

  const spawns = (unit: TriggerUnit) =>
    unit.members.map(spawnTarget).filter((s): s is string => s !== null)
  const watches = (unit: TriggerUnit) =>
    unit.members.map(watchedSession).filter((s): s is string => s !== null)

  // parents[k] = units whose spawn target this unit watches.
  const parents = new Map<string, TriggerUnit[]>()
  let hasEdge = false
  for (const child of units) {
    const watched = new Set(watches(child))
    const feeding = units.filter(
      (parent) =>
        parent.key !== child.key &&
        spawns(parent).some((s) => watched.has(s)),
    )
    if (feeding.length > 0) hasEdge = true
    parents.set(child.key, feeding)
  }

  // Longest-path level with a visiting guard (a cycle collapses to level 0).
  const levelByKey = new Map<string, number>()
  const visiting = new Set<string>()
  const levelOf = (unit: TriggerUnit): number => {
    const known = levelByKey.get(unit.key)
    if (known !== undefined) return known
    if (visiting.has(unit.key)) return 0
    visiting.add(unit.key)
    const feeding = parents.get(unit.key) ?? []
    const level =
      feeding.length === 0 ? 0 : 1 + Math.max(...feeding.map(levelOf))
    visiting.delete(unit.key)
    levelByKey.set(unit.key, level)
    return level
  }

  const levels: TriggerUnit[][] = []
  for (const unit of units) {
    const level = levelOf(unit)
    ;(levels[level] ??= []).push(unit)
  }

  return {
    levels: levels.filter((l) => l.length > 0),
    hasStructure: hasEdge || groups.size > 0,
  }
}

/** `console-9a8a0cbc-…` → `console-9a8a0cbc`; short ids pass through. */
function shortSession(sessionId: string): string {
  return sessionId.length > 24 ? `${sessionId.slice(0, 21)}…` : sessionId
}

/**
 * Metadata keys already surfaced as field rows (label/once/subscription) or
 * implied by the listing itself (the owner session IS this conversation).
 * What remains is the interesting part — e.g. a react binding's reaction
 * spec (model, task, join).
 */
const SURFACED_METADATA_KEYS = new Set([
  '__owner_session_id',
  '__subscription_id',
  'subscription_id',
  'session_id',
  'label',
  'once',
])

function remainingMetadata(
  metadata: Record<string, unknown> | undefined,
): Record<string, unknown> | null {
  if (!metadata) return null
  const rest = Object.fromEntries(
    Object.entries(metadata).filter(([k]) => !SURFACED_METADATA_KEYS.has(k)),
  )
  return Object.keys(rest).length > 0 ? rest : null
}

function isEmptyConfig(config: unknown): boolean {
  if (config === null || config === undefined) return true
  if (typeof config === 'object')
    return Object.keys(config as Record<string, unknown>).length === 0
  return false
}

function subscriptionId(trigger: SessionTriggerInfo): string | null {
  const meta = trigger.metadata ?? {}
  const id = meta.__subscription_id ?? meta.subscription_id
  return typeof id === 'string' ? id : null
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
        {copied ? <Check size={11} aria-hidden /> : <Copy size={11} aria-hidden />}
      </button>
    </span>
  )
}

interface TriggerRowProps {
  trigger: SessionTriggerInfo
  busy: boolean
  onOpen: () => void
  onUnregister: () => void
  /** `├` / `└` prefix for join members. */
  connector?: string
  /** The member's join key, shown as the row's name. */
  memberKey?: string
  /** Annotate watched / spawn-target sessions (workflow view). */
  showTargets?: boolean
}

function TriggerRow({
  trigger,
  busy,
  onOpen,
  onUnregister,
  connector,
  memberKey,
  showTargets,
}: TriggerRowProps) {
  const watched = watchedSession(trigger)
  const target = spawnTarget(trigger)
  const name = memberKey ?? trigger.label ?? null
  return (
    <div className="flex items-center gap-2 border-b border-rule-2 px-3 py-1.5 text-[12px] last:border-b-0">
      {connector ? (
        <span className="shrink-0 pl-1 text-ink-ghost" aria-hidden>
          {connector}
        </span>
      ) : (
        <Zap size={12} className="shrink-0 text-ink-ghost" aria-hidden />
      )}
      <button
        type="button"
        onClick={onOpen}
        className="min-w-0 flex-1 truncate text-left hover:text-ink transition-colors"
        title="show trigger detail"
      >
        {name || trigger.triggerType}
        <span className="text-ink-ghost">
          {name ? ` · ${trigger.triggerType}` : ''}
          {connector
            ? '' /* the join header already says what the group does */
            : ` · ${targetLabel(trigger)}`}
          {showTargets || connector ? (
            <>
              {watched ? ` · on ${shortSession(watched)}` : ''}
              {target ? ` → ${shortSession(target)}` : ''}
            </>
          ) : null}
          {trigger.once ? ' · once' : ''}
        </span>
      </button>
      <button
        type="button"
        disabled={busy}
        onClick={onUnregister}
        className="shrink-0 lowercase text-ink-ghost hover:text-ink transition-colors disabled:opacity-50"
        aria-label={`unregister ${name ?? trigger.triggerType}`}
      >
        {busy ? '…' : '✕'}
      </button>
    </div>
  )
}

/**
 * The conversation's registered trigger subscriptions, stacked above the
 * composer next to the queued-messages strip. Joined / chained bindings
 * render as a staged workflow (fan-in groups + ↓ between stages); anything
 * unconnected stays a flat row. Click a row for the full detail dialog;
 * ✕ (or the dialog button) unregisters the engine trigger.
 */
export function SessionTriggers({
  triggers,
  onUnregister,
}: SessionTriggersProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const workflow = useMemo(() => buildTriggerWorkflow(triggers), [triggers])
  const selected = triggers.find((t) => t.id === selectedId) ?? null
  const selectedMetadata = selected
    ? remainingMetadata(selected.metadata)
    : null
  const selectedSubscription = selected ? subscriptionId(selected) : null

  if (triggers.length === 0) return null

  const unregister = async (id: string) => {
    setBusyId(id)
    try {
      await onUnregister(id)
      setSelectedId((current) => (current === id ? null : current))
    } finally {
      setBusyId(null)
    }
  }

  return (
    <>
      <div
        className="mb-1 border border-rule bg-bg"
        aria-label="registered triggers"
      >
        {workflow.hasStructure
          ? workflow.levels.map((units, levelIdx) => (
              <div key={units[0]?.key ?? levelIdx}>
                {levelIdx > 0 ? (
                  <div
                    className="border-b border-rule-2 py-0.5 text-center text-[10px] leading-none text-ink-ghost"
                    aria-label="then"
                  >
                    ↓
                  </div>
                ) : null}
                {units.map((unit) =>
                  unit.join ? (
                    <div key={unit.key}>
                      <div className="flex items-center gap-2 border-b border-rule-2 bg-paper-2 px-3 py-1.5 text-[12px]">
                        <GitMerge
                          size={12}
                          className="shrink-0 text-ink-ghost"
                          aria-hidden
                        />
                        <span className="min-w-0 flex-1 truncate">
                          join {unit.join.id}
                          <span className="text-ink-ghost">
                            {' '}
                            · waits for {unit.join.expect.join(' + ')} · spawns
                            sub-agent
                          </span>
                        </span>
                      </div>
                      {unit.members.map((member, memberIdx) => (
                        <TriggerRow
                          key={member.id}
                          trigger={member}
                          connector={
                            memberIdx === unit.members.length - 1 ? '└' : '├'
                          }
                          memberKey={joinMeta(member)?.key}
                          busy={busyId === member.id}
                          onOpen={() => setSelectedId(member.id)}
                          onUnregister={() => void unregister(member.id)}
                        />
                      ))}
                    </div>
                  ) : (
                    <TriggerRow
                      key={unit.key}
                      trigger={unit.members[0]}
                      showTargets
                      busy={busyId === unit.members[0].id}
                      onOpen={() => setSelectedId(unit.members[0].id)}
                      onUnregister={() => void unregister(unit.members[0].id)}
                    />
                  ),
                )}
              </div>
            ))
          : triggers.map((trigger) => (
              <TriggerRow
                key={trigger.id}
                trigger={trigger}
                busy={busyId === trigger.id}
                onOpen={() => setSelectedId(trigger.id)}
                onUnregister={() => void unregister(trigger.id)}
              />
            ))}
      </div>

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
                <dd className="text-ink">
                  {targetLabel(selected)}
                  <span className="text-ink-faint">
                    {' '}
                    · {selected.functionId}
                  </span>
                </dd>
                <dt className="lowercase text-ink-ghost">lifetime</dt>
                <dd className="text-ink">
                  {selected.once ? 'once — retires after first fire' : 'until unregistered'}
                </dd>
                {selectedSubscription ? (
                  <>
                    <dt className="lowercase text-ink-ghost">subscription</dt>
                    <dd className="text-ink-faint">
                      <CopyableId value={selectedSubscription} />
                    </dd>
                  </>
                ) : null}
                <dt className="lowercase text-ink-ghost">trigger id</dt>
                <dd className="text-ink-faint">
                  <CopyableId value={selected.id} />
                </dd>
              </dl>
              {isEmptyConfig(selected.config) ? null : (
                <JsonSection label="config" value={selected.config} />
              )}
              {selectedMetadata ? (
                <JsonSection
                  label={
                    selected.functionId === 'harness::react'
                      ? 'reaction spec'
                      : 'metadata'
                  }
                  value={selectedMetadata}
                />
              ) : null}
              <div className="flex justify-end">
                <Button
                  type="button"
                  variant="pill"
                  size="sm"
                  disabled={busyId === selected.id}
                  onClick={() => void unregister(selected.id)}
                >
                  {busyId === selected.id ? 'unregistering…' : 'unregister'}
                </Button>
              </div>
            </div>
          ) : null}
        </DialogContent>
      </Dialog>
    </>
  )
}
