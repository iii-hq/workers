import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  GitMerge,
  Trash2,
  Workflow,
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
import { TriggerDag } from './TriggerDag'
import {
  buildTriggerWorkflow,
  joinMeta,
  levelWatches,
  reactModel,
  reactTask,
  shortSession,
  spawnTarget,
  stateWatch,
  watchedSession,
} from './trigger-graph'

// Re-exported for SessionTriggers.test.ts, which predates the trigger-graph
// extraction; the logic now lives in ./trigger-graph.
export { buildTriggerWorkflow, levelWatches, stateWatch }

interface SessionTriggersProps {
  triggers: SessionTriggerInfo[]
  onUnregister: (triggerId: string) => Promise<void> | void
  /** Unregister every binding at once (see ChatView). */
  onClearAll?: () => Promise<void> | void
  /** Backend probe: does this state key currently exist? (`null` = unknown) */
  checkStateKey?: (
    scope: string | undefined,
    key: string,
  ) => Promise<boolean | null>
}

function targetLabel(trigger: SessionTriggerInfo): string {
  return trigger.functionId === 'harness::react'
    ? 'spawns sub-agent'
    : 'notifies this chat'
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
  '__once',
])

/** React-spec keys surfaced as dedicated dialog rows / the task section. */
const SURFACED_REACT_KEYS = new Set(['model', 'task', 'join', 'provider'])

function remainingMetadata(
  metadata: Record<string, unknown> | undefined,
  isReact: boolean,
): Record<string, unknown> | null {
  if (!metadata) return null
  const rest = Object.fromEntries(
    Object.entries(metadata).filter(
      ([k]) =>
        !SURFACED_METADATA_KEYS.has(k) &&
        !(isReact && SURFACED_REACT_KEYS.has(k)),
    ),
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
  /** `├` / `└` prefix for join members. */
  connector?: string
  /** The member's join key, shown as the row's name. */
  memberKey?: string
  /** Annotate watched / spawn-target sessions (workflow view). */
  showTargets?: boolean
  /** Watched state key + whether it exists yet ("on scope/key — not written yet"). */
  stateNote?: string | null
}

function TriggerRow({
  trigger,
  busy,
  onOpen,
  onUnregister,
  connector,
  memberKey,
  showTargets,
  stateNote,
}: TriggerRowProps) {
  const watched = watchedSession(trigger)
  const target = spawnTarget(trigger)
  const model = reactModel(trigger)
  const task = reactTask(trigger)
  const name = memberKey ?? trigger.label ?? null
  return (
    <div
      className={`flex items-center gap-2 border-b border-rule-2 px-3 py-1.5 text-[12px] last:border-b-0${trigger.fired ? ' opacity-55' : ''}`}
    >
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
        title={task ?? 'show trigger detail'}
      >
        {name || trigger.triggerType}
        <span className="text-ink-ghost">
          {name ? ` · ${trigger.triggerType}` : ''}
          {connector
            ? '' /* the join header already says what the group does */
            : ` · ${targetLabel(trigger)}`}
          {!connector && model ? ` · ${model}` : ''}
          {showTargets || connector ? (
            <>
              {watched ? ` · on ${shortSession(watched)}` : ''}
              {target ? ` → ${shortSession(target)}` : ''}
            </>
          ) : null}
          {stateNote ? ` · ${stateNote}` : ''}
          {trigger.fired
            ? ' · fired · unregistered'
            : trigger.once
              ? ' · once'
              : ''}
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
 * count header; expanding shows the rows. Joined / chained bindings render
 * as a staged workflow (fan-in groups + an "after <session> completes"
 * divider between stages); anything unconnected stays a flat row. Click a
 * row for the full detail dialog; ✕ (or the dialog button) unregisters the
 * engine trigger.
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
  const [flowOpen, setFlowOpen] = useState(false)
  const [clearArming, setClearArming] = useState(false)
  const [clearing, setClearing] = useState(false)
  // Fired ghost rows the user dismissed — local per-tab view state; they
  // resurrect from the transcript on reload, so no persistence needed.
  const [dismissed, setDismissed] = useState<Set<string>>(() => new Set())

  const visibleTriggers = useMemo(
    () => triggers.filter((t) => !dismissed.has(t.id)),
    [triggers, dismissed],
  )
  // Still-registered bindings only — the flow DAG and the counts must not
  // include fired ghosts, which are history, not pipeline structure.
  const liveTriggers = useMemo(
    () => visibleTriggers.filter((t) => !t.fired),
    [visibleTriggers],
  )
  const registeredCount = liveTriggers.length
  const firedCount = visibleTriggers.length - registeredCount

  // The DAG probes presence for every state binding, not just the visible
  // rows, so the flow view can color unwritten roots even while collapsed.
  const probeKeys = expanded || flowOpen

  // Whether each state binding's watched key exists yet — the row-level
  // diagnosis for a reaction armed on a key nothing ever writes.
  // ponytail: refetches on each trigger-poll tick while open; cache if it matters.
  const [keyPresence, setKeyPresence] = useState<Record<string, boolean>>({})
  useEffect(() => {
    if (!probeKeys || !checkStateKey) return
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
  }, [probeKeys, checkStateKey, visibleTriggers])

  const stateNote = (trigger: SessionTriggerInfo): string | null => {
    const watch = stateWatch(trigger)
    if (!watch) return null
    const label = watch.scope ? `${watch.scope}/${watch.key}` : watch.key
    const present = keyPresence[trigger.id]
    if (present === undefined) return `on ${label}`
    return present ? `on ${label} — written` : `on ${label} — not written yet`
  }
  const workflow = useMemo(
    () => buildTriggerWorkflow(visibleTriggers),
    [visibleTriggers],
  )
  const selected = visibleTriggers.find((t) => t.id === selectedId) ?? null
  const selectedIsReact = selected?.functionId === 'harness::react'
  const selectedMetadata = selected
    ? remainingMetadata(selected.metadata, selectedIsReact)
    : null
  const selectedSubscription = selected ? subscriptionId(selected) : null
  const selectedModel = selected ? reactModel(selected) : null
  const selectedTask = selected ? reactTask(selected) : null
  const selectedTarget = selected ? spawnTarget(selected) : null
  const selectedJoin = selected ? joinMeta(selected) : null
  const selectedProvider =
    selectedIsReact && typeof selected?.metadata?.provider === 'string'
      ? selected.metadata.provider
      : null

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

  // A fired ghost row has no engine handle — its ✕ dismisses locally; a live
  // row's ✕ unregisters the engine trigger.
  const rowAction = (t: SessionTriggerInfo) =>
    t.fired ? dismiss(t.id) : void unregister(t.id)

  const clearAll = async () => {
    setClearing(true)
    try {
      await onClearAll?.()
      // Live bindings are unregistered by onClearAll; fired ghosts have no
      // engine handle, so sweep them from view here too.
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
                this tears down the pipeline.
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
                  {workflow.hasStructure && workflow.levels.length > 1
                    ? ` · ${workflow.levels.length} stages`
                    : ''}
                  {firedCount > 0 ? ` · ${firedCount} fired` : ''}
                </span>
              </span>
            </button>
            {onClearAll || workflow.hasStructure ? (
              <div className="flex shrink-0 items-center gap-1 pr-1 font-mono text-[11px] uppercase tracking-[0.06em]">
                {workflow.hasStructure ? (
                  <button
                    type="button"
                    onClick={() => setFlowOpen(true)}
                    className="flex items-center gap-1 px-2 py-1.5 text-ink-faint hover:text-ink transition-colors"
                    title="view the pipeline flow"
                  >
                    <Workflow size={12} aria-hidden />
                    flow
                  </button>
                ) : null}
                {onClearAll ? (
                  <button
                    type="button"
                    onClick={() => setClearArming(true)}
                    className="flex items-center gap-1 px-2 py-1.5 text-ink-faint hover:text-alert transition-colors"
                    title="unregister every trigger"
                  >
                    <Trash2 size={12} aria-hidden />
                    clear all
                  </button>
                ) : null}
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
            {workflow.hasStructure
              ? workflow.levels.map((units, levelIdx) => {
                  const upstream = levelWatches(units)
                  const upstreamLabel = upstream.map(shortSession).join(', ')
                  return (
                    <div key={units[0]?.key ?? levelIdx}>
                      {levelIdx > 0 ? (
                        <div className="truncate border-b border-rule-2 px-3 py-0.5 text-center text-[10px] leading-none text-ink-ghost">
                          {upstreamLabel ? (
                            `↓ after ${upstreamLabel} ${upstream.length === 1 ? 'completes' : 'complete'}`
                          ) : (
                            <span aria-hidden>↓</span>
                          )}
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
                                  · waits for {unit.join.expect.join(' + ')} ·
                                  spawns sub-agent
                                  {reactModel(unit.members[0])
                                    ? ` · ${reactModel(unit.members[0])}`
                                    : ''}
                                </span>
                              </span>
                            </div>
                            {unit.members.map((member, memberIdx) => (
                              <TriggerRow
                                key={member.id}
                                trigger={member}
                                stateNote={stateNote(member)}
                                connector={
                                  memberIdx === unit.members.length - 1
                                    ? '└'
                                    : '├'
                                }
                                memberKey={joinMeta(member)?.key}
                                busy={busyId === member.id}
                                onOpen={() => setSelectedId(member.id)}
                                onUnregister={() => rowAction(member)}
                              />
                            ))}
                          </div>
                        ) : (
                          <TriggerRow
                            key={unit.key}
                            trigger={unit.members[0]}
                            showTargets
                            stateNote={stateNote(unit.members[0])}
                            busy={busyId === unit.members[0].id}
                            onOpen={() => setSelectedId(unit.members[0].id)}
                            onUnregister={() => rowAction(unit.members[0])}
                          />
                        ),
                      )}
                    </div>
                  )
                })
              : visibleTriggers.map((trigger) => (
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
                <dd className="text-ink">
                  {targetLabel(selected)}
                  <span className="text-ink-faint">
                    {' '}
                    · {selected.functionId}
                  </span>
                </dd>
                {selectedModel ? (
                  <>
                    <dt className="lowercase text-ink-ghost">model</dt>
                    <dd className="text-ink">
                      {selectedModel}
                      {selectedProvider ? (
                        <span className="text-ink-faint">
                          {' '}
                          · {selectedProvider}
                        </span>
                      ) : null}
                    </dd>
                  </>
                ) : null}
                {selectedIsReact ? (
                  <>
                    <dt className="lowercase text-ink-ghost">spawns into</dt>
                    <dd className="text-ink-faint">
                      {selectedTarget ? (
                        <CopyableId value={selectedTarget} />
                      ) : (
                        'this chat (owner session)'
                      )}
                    </dd>
                  </>
                ) : null}
                {selectedJoin ? (
                  <>
                    <dt className="lowercase text-ink-ghost">join</dt>
                    <dd className="text-ink">
                      {selectedJoin.id}
                      <span className="text-ink-faint">
                        {' '}
                        · waits for {selectedJoin.expect.join(' + ')}
                        {selectedJoin.key
                          ? ` · fires as ${selectedJoin.key}`
                          : ''}
                        {selectedJoin.rearm ? ' · re-arms after firing' : ''}
                      </span>
                    </dd>
                  </>
                ) : null}
                <dt className="lowercase text-ink-ghost">lifetime</dt>
                <dd className="text-ink">
                  {selected.fired
                    ? 'fired — already unregistered'
                    : selected.once
                      ? 'once — retires after first fire'
                      : 'until unregistered'}
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
              {selectedTask ? (
                <div className="border border-rule-2">
                  <div className="border-b border-rule-2 bg-paper-2 px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
                    task
                  </div>
                  <div className="max-h-48 overflow-y-auto whitespace-pre-wrap break-words px-3 py-2 text-[12px] text-ink">
                    {selectedTask}
                  </div>
                </div>
              ) : null}
              {isEmptyConfig(selected.config) ? null : (
                <JsonSection label="config" value={selected.config} />
              )}
              {selectedMetadata ? (
                <JsonSection
                  label={selectedIsReact ? 'spawn options' : 'metadata'}
                  value={selectedMetadata}
                />
              ) : null}
              <div className="flex justify-end">
                {/* A fired row has no engine binding left to unregister —
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

      <Dialog open={flowOpen} onOpenChange={setFlowOpen}>
        <DialogContent className="max-w-[min(92vw,64rem)]">
          <DialogTitle className="text-[14px] lowercase">
            <span
              className="mr-2 inline-flex align-baseline text-ink-ghost"
              aria-hidden
            >
              <Workflow size={13} />
            </span>
            pipeline flow
          </DialogTitle>
          <DialogDescription className="mt-1">
            the reactive graph these {visibleTriggers.length} bindings form —
            state writes and completions on the left, the sub-agents they spawn
            flowing right. fired bindings stay in the graph as pipeline history.
          </DialogDescription>
          <div className="mt-4">
            <TriggerDag triggers={visibleTriggers} keyPresence={keyPresence} />
          </div>
          <DagLegend />
        </DialogContent>
      </Dialog>
    </>
  )
}

/** Reads the DAG's node/edge vocabulary in one compact row. */
function DagLegend() {
  return (
    <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1.5 font-mono text-[10px] lowercase text-ink-ghost">
      <span className="flex items-center gap-1.5">
        <span className="inline-block size-2.5 border border-rule bg-bg" />
        agent
      </span>
      <span className="flex items-center gap-1.5">
        <span className="uppercase tracking-[0.06em] text-ink-faint">
          state
        </span>
        state key
      </span>
      <span className="flex items-center gap-1.5">
        <GitMerge size={11} className="text-ink-faint" aria-hidden />
        join gate
      </span>
      <span className="flex items-center gap-1.5">
        <span className="inline-block size-2.5 border border-accent bg-bg" />
        this chat
      </span>
      <span className="flex items-center gap-1.5">
        <span className="inline-block h-px w-4 bg-rule" />
        spawns / watches
      </span>
      <span className="flex items-center gap-1.5">
        <span
          className="inline-block h-px w-4"
          style={{
            backgroundImage:
              'repeating-linear-gradient(to right, var(--color-ink-ghost) 0 3px, transparent 3px 5px)',
          }}
        />
        into a join
      </span>
    </div>
  )
}
