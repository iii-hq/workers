/**
 * The `context` session chip — a live view of the session's context window
 * matching the console's ContextUsage aesthetic (`ctx` label, bar, `12.3k/200k`
 * counts). Hydrates from the stored snapshot (`state::get` on
 * `harness_context/<session id>`) on mount and per session change; stays live
 * over the state worker's own `state` trigger for that key (Message-path
 * binding, GC'd with the tab), which fires on every generate step. Click opens
 * the breakdown — a `Dialog` on desktop, a `BottomSheet` on phones — with a
 * stacked segment bar, legend, and last-turn actuals.
 *
 * The chip answers to the PANE, not the viewport: under a 30rem-wide chat
 * pane (a phone, or a narrow split) it drops to its counts — the bar and the
 * `ctx` eyebrow hide (styles.css, `@container`), `12.3k/200k` and the caret
 * stay. `data-has-counts` marks the states that have counts to fall back to;
 * the waiting state (`ctx —`) keeps its eyebrow.
 */

import {
  Badge,
  BottomSheet,
  BottomSheetContent,
  BottomSheetTrigger,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
  type Host,
  uiClasses,
} from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { ChevronDown } from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useState } from 'react'
import { formatCost, formatTokens } from '../lib/format'
import {
  type ContextSnapshot,
  isSnapshot,
  type SnapshotUsage,
} from '../lib/metrics'
import { TONE_COLOR, toneFor } from '../lib/tone'

/** Per-tab handler id (host.iii.on namespaces it `::<browserId>`). */
const STATE_FN = 'iii::harness-ui::ctx-state'
/** Phone chrome only (sheet vs dialog), the console's one viewport breakpoint. */
const PHONE_QUERY = '(max-width: 639px)'

export interface SessionChipProps {
  sessionId: string
  modelId?: string
  contextWindow?: number
}

/** The message a `state` function trigger delivers per write (the state
    worker's own streaming trigger type — no polling anywhere). */
interface StateEvent {
  type?: string
  event_type?: string
  scope?: string
  key?: string
  new_value?: unknown
}

function usePhone(): boolean {
  const [phone, setPhone] = useState(
    () => typeof window !== 'undefined' && window.matchMedia?.(PHONE_QUERY).matches === true,
  )
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return
    const media = window.matchMedia(PHONE_QUERY)
    const update = () => setPhone(media.matches)
    update()
    media.addEventListener('change', update)
    return () => media.removeEventListener('change', update)
  }, [])
  return phone
}

const ink = (opacity: number) =>
  `color-mix(in srgb, var(--color-ink) ${opacity}%, transparent)`
const accent = (opacity: number) =>
  `color-mix(in srgb, var(--color-accent) ${opacity}%, transparent)`

const COLOR_FREE = 'var(--color-rule-2)'

type CategoryKey =
  | 'system'
  | 'skills'
  | 'tools'
  | 'user'
  | 'assistant'
  | 'results'
  | 'hooks'
  | 'overhead'

interface Category {
  key: CategoryKey
  label: string
  color: string
  tokens: number
}

/** The three categories the legend shows as one "Conversation" row. */
const CONVERSATION_KEYS: CategoryKey[] = ['user', 'assistant', 'results']

/**
 * Every category of the assembled window, in bar order. The single source
 * for both the stacked bar (which draws each entry) and the legend (which
 * merges the conversation entries into one row).
 */
function categories(snapshot: ContextSnapshot): Category[] {
  const cats = snapshot.categories
  const messages = cats.messages
  return [
    {
      key: 'system',
      label: 'System prompt',
      color: ink(80),
      tokens: cats.system_prompt,
    },
    {
      key: 'skills',
      label: 'Skills',
      color: ink(65),
      tokens: cats.skills ?? 0,
    },
    {
      key: 'tools',
      label: 'Function schemas',
      color: ink(55),
      tokens: cats.tools,
    },
    { key: 'user', label: 'User', color: accent(95), tokens: messages.user },
    {
      key: 'assistant',
      label: 'Assistant',
      color: accent(70),
      tokens: messages.assistant,
    },
    {
      key: 'results',
      label: 'Function results',
      color: accent(45),
      tokens: messages.function_result + messages.custom,
    },
    {
      key: 'hooks',
      label: 'Hook guidance',
      color: ink(35),
      // Optional on the wire (serde default): absent in snapshots written
      // before the category existed.
      tokens: cats.hook_guidance ?? 0,
    },
    {
      key: 'overhead',
      label: 'Overhead',
      color: ink(20),
      tokens: cats.overhead,
    },
  ]
}

/**
 * The prompt cache view of the last generation. Providers bill a cache read
 * at a fraction of fresh input and a cache write at a premium, so on a long
 * session the hit rate drives cost more than the window size does. `null`
 * when the provider reported no cache activity at all.
 */
function cacheSummary(usage: SnapshotUsage | undefined) {
  const read = usage?.cache_read ?? 0
  const write = usage?.cache_write ?? 0
  if (read === 0 && write === 0) return null
  const prompt = (usage?.input ?? 0) + read + write
  const hitPct = prompt > 0 ? Math.round((read / prompt) * 100) : 0
  return {
    read,
    write,
    hitPct,
    // A cold or broken prefix means the whole prompt is re-billed at the
    // write premium every turn, which is worth flagging rather than dimming.
    tone: hitPct >= 70 ? 'ok' : hitPct < 30 ? 'warn' : 'plain',
  }
}

interface LegendEntry {
  key: string
  label: string
  /** `null` renders an invisible swatch — the row is not a bar segment. */
  color: string | null
  tokens: number
  badge?: string
}

/**
 * The legend, derived from the same category array the bar draws: the three
 * conversation entries collapse into one row, empty optional rows are dropped,
 * and the two rows that are not bar segments (the compaction summary, free
 * space) take their place around the overhead row.
 */
function legendRows(
  snapshot: ContextSnapshot,
  cats: Category[],
): LegendEntry[] {
  const conversation = cats.filter((c) => CONVERSATION_KEYS.includes(c.key))
  const rows: LegendEntry[] = []
  for (const category of cats) {
    if (category.key === 'assistant') {
      rows.push({
        ...category,
        label: 'Conversation',
        tokens: conversation.reduce((sum, entry) => sum + entry.tokens, 0),
      })
      continue
    }
    if (CONVERSATION_KEYS.includes(category.key)) continue
    if (
      (category.key === 'hooks' || category.key === 'skills') &&
      category.tokens <= 0
    )
      continue
    if (category.key === 'overhead' && snapshot.compacted) {
      rows.push({
        key: 'summary',
        label: 'Summary',
        color: null,
        tokens: snapshot.summarized_head_tokens ?? 0,
        badge: 'compacted',
      })
    }
    rows.push(category)
  }
  rows.push({
    key: 'free',
    label: 'Free',
    color: COLOR_FREE,
    tokens: snapshot.free,
  })
  return rows
}

function LegendRow({
  color,
  label,
  tokens,
  usable,
  badge,
}: {
  color: string | null
  label: string
  tokens: number
  usable: number
  badge?: string
}) {
  const pct = usable > 0 ? Math.round((tokens / usable) * 100) : 0
  return (
    <div className="harness-ui-legend-row">
      <span
        className="harness-ui-swatch"
        style={color ? { background: color } : { visibility: 'hidden' }}
      />
      <span className="harness-ui-legend-label">{label}</span>
      {badge ? <Badge variant="warn">{badge}</Badge> : null}
      <span className="harness-ui-legend-val">{formatTokens(tokens)}</span>
      <span className="harness-ui-legend-pct">{pct}%</span>
    </div>
  )
}

/** Footer row: the session id (the `state::get` / session-store key),
    truncated to fit the popover, with a copy affordance. */
function SessionIdRow({ sessionId }: { sessionId: string }) {
  const { state, copy } = useCopyFlash(sessionId, 1200)
  return (
    <span className="harness-ui-pop-session">
      <span className="harness-ui-pop-session-id" title={sessionId}>
        session {sessionId}
      </span>
      <Button variant="ghost" size="sm" onClick={copy} aria-label="copy session id">
        {state === 'copied' ? 'copied' : 'copy'}
      </Button>
    </span>
  )
}

/** The dialog's title and one-line descriptor: model, then usage. */
function PopoverHead({ model, usage }: { model: string; usage: string }) {
  return (
    <div className="harness-ui-pop-head">
      <DialogTitle>Context breakdown</DialogTitle>
      <DialogDescription className="harness-ui-pop-usage">
        {model} · {usage}
      </DialogDescription>
    </div>
  )
}

function ContextPopover({
  snapshot,
  modelId,
  sessionId,
}: {
  snapshot: ContextSnapshot
  modelId?: string
  sessionId: string
}) {
  const usable = snapshot.usable
  const pct =
    usable > 0 ? Math.round(Math.min(1, snapshot.total / usable) * 100) : 0
  const cats = categories(snapshot)
  const usage = snapshot.usage
  const hasActuals =
    usage != null && (usage.input != null || usage.cache_read != null)
  const cache = cacheSummary(usage)
  return (
    <div className="harness-ui-pop">
      <PopoverHead
        model={snapshot.model || modelId || 'model'}
        usage={`${pct}% of ${formatTokens(usable)}`}
      />
      <div className="harness-ui-stack">
        {cats
          .filter((segment) => segment.tokens > 0)
          .map((segment) => (
            <span
              key={segment.key}
              className="harness-ui-seg"
              style={{
                width: `${usable > 0 ? (segment.tokens / usable) * 100 : 0}%`,
                background: segment.color,
              }}
            />
          ))}
      </div>
      <div className="harness-ui-legend">
        {legendRows(snapshot, cats).map((row) => (
          <LegendRow
            key={row.key}
            color={row.color}
            label={row.label}
            tokens={row.tokens}
            usable={usable}
            badge={row.badge}
          />
        ))}
      </div>
      <div className="harness-ui-pop-foot">
        <span>
          {!snapshot.estimator || snapshot.estimator === 'heuristic'
            ? `est. ${snapshot.estimator ?? 'unknown'}`
            : `exact · ${
                snapshot.estimator === 'provider'
                  ? 'provider tokenizer'
                  : snapshot.estimator
              }`}
        </span>
        {hasActuals ? (
          <span>
            last step {formatTokens(usage?.input ?? 0)} in · output{' '}
            {formatTokens(usage?.output ?? 0)}
            {usage?.cost_usd != null ? (
              <> · {formatCost(usage.cost_usd)}</>
            ) : null}
          </span>
        ) : null}
        {cache ? (
          <span
            title={
              'cache read is billed at a fraction of fresh input; cache write ' +
              'carries a premium. Hit rate is the cached share of the prompt.'
            }
          >
            cache {formatTokens(cache.read)} read · {formatTokens(cache.write)}{' '}
            write ·{' '}
            <span className="harness-ui-cache-hit" data-tone={cache.tone}>
              {cache.hitPct}% hit
            </span>
          </span>
        ) : null}
        {snapshot.session_cost_usd != null ? (
          <span
            title={
              'every generation step of this session summed. The per-step ' +
              'cost on the line above swings with cache hits; this one only grows.'
            }
          >
            session total {formatCost(snapshot.session_cost_usd)}
          </span>
        ) : null}
        <SessionIdRow sessionId={sessionId} />
      </div>
    </div>
  )
}

function EmptyContextPopover({
  modelId,
  contextWindow,
  sessionId,
}: {
  modelId?: string
  contextWindow?: number
  sessionId: string
}) {
  const hasWindow = contextWindow !== undefined && contextWindow > 0
  return (
    <div className="harness-ui-pop">
      <PopoverHead model={modelId || 'model'} usage="waiting for usage" />
      {hasWindow ? (
        <>
          <div className="harness-ui-stack" />
          <div className="harness-ui-legend">
            <LegendRow
              color={COLOR_FREE}
              label="Free"
              tokens={contextWindow}
              usable={contextWindow}
            />
          </div>
        </>
      ) : null}
      <div className="harness-ui-pop-foot">
        <span>
          {hasWindow
            ? 'the breakdown appears after the first generation step'
            : 'the selected model did not report a context-window limit'}
        </span>
        <SessionIdRow sessionId={sessionId} />
      </div>
    </div>
  )
}

interface ContextChipSurfaceProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  triggerLabel: string
  trigger: ReactNode
  popover: ReactNode
  /** The trigger carries a `used/usable` count — what a narrow pane keeps
      when it hides the bar and the eyebrow (see styles.css). */
  hasCounts?: boolean
}

/**
 * One surface for every context-chip state: the chip is the trigger, the
 * breakdown opens as a `Dialog` on desktop and a `BottomSheet` on phones.
 * Focus, Escape, outside click and scroll locking are the dialog's.
 */
function ContextChipSurface({
  open,
  onOpenChange,
  triggerLabel,
  trigger,
  popover,
  hasCounts = false,
}: ContextChipSurfaceProps) {
  const phone = usePhone()
  const Root = phone ? BottomSheet : Dialog
  const Trigger = phone ? BottomSheetTrigger : DialogTrigger
  return (
    <div className="harness-ui-chip" data-has-counts={hasCounts ? '' : undefined}>
      <Root open={open} onOpenChange={onOpenChange}>
        <Trigger asChild>
          <button type="button" className="harness-ui-chip-btn" aria-label={triggerLabel}>
            {trigger}
          </button>
        </Trigger>
        {phone ? (
          <BottomSheetContent className="harness-ui-sheet">{popover}</BottomSheetContent>
        ) : (
          <DialogContent className="harness-ui-dialog">{popover}</DialogContent>
        )}
      </Root>
    </div>
  )
}

export function createContextChip(host: Host) {
  return function ContextChip({
    sessionId,
    modelId,
    contextWindow,
  }: SessionChipProps) {
    const [snapshot, setSnapshot] = useState<ContextSnapshot | null>(null)
    const [open, setOpen] = useState(false)

    // Both the hydration read and the streamed trigger write this state;
    // keep whichever snapshot is newest so a slow state::get can never
    // overwrite a fresher streamed step.
    const acceptNewer = (value: ContextSnapshot) =>
      setSnapshot((current) =>
        current && current.timestamp >= value.timestamp ? current : value,
      )

    useEffect(() => {
      let cancelled = false
      setSnapshot(null)
      setOpen(false)
      host.iii
        .trigger('state::get', { scope: 'harness_context', key: sessionId })
        .then((value) => {
          if (cancelled) return
          if (isSnapshot(value) && value.session_id === sessionId)
            acceptNewer(value)
        })
        .catch(() => {})
      return () => {
        cancelled = true
      }
    }, [host, sessionId])

    // Snapshots are written after every generate step; the state worker's
    // `state` trigger streams each write (engine-side scope/key filter), so
    // long multi-step turns tick live without any polling.
    useEffect(() => {
      // The id carries the session, because the engine-side filter is keyed to
      // one: two chips mounted at once (two sessions visible, or an unmount
      // racing the next mount) would otherwise register conflicting filters
      // under one id, and either teardown would take the other's stream with
      // it. `on()` appends `::<browserId>` itself, so the trigger repeats it
      // to address the same handler.
      const eventFn = `${STATE_FN}::${sessionId}`
      const offHandler = host.iii.on<StateEvent>(eventFn, (event) => {
        if (!event || event.key !== sessionId) return
        if (event.event_type === 'state:deleted') return
        if (
          isSnapshot(event.new_value) &&
          event.new_value.session_id === sessionId
        )
          acceptNewer(event.new_value)
      })
      const offTrigger = host.iii.registerTrigger({
        type: 'state',
        function_id: `${eventFn}::${host.iii.browserId}`,
        config: { scope: 'harness_context', key: sessionId },
      })
      return () => {
        offTrigger()
        offHandler()
      }
    }, [host, sessionId])

    const caret = <ChevronDown className="harness-ui-chip-caret" size={16} aria-hidden />
    const label = (
      <span className={`${uiClasses.eyebrow} harness-ui-chip-label`}>ctx</span>
    )

    if (!snapshot || snapshot.usable <= 0) {
      if (contextWindow && contextWindow > 0) {
        return (
          <ContextChipSurface
            open={open}
            onOpenChange={setOpen}
            triggerLabel={`context: waiting for usage, ${contextWindow.toLocaleString()} token window — click for the breakdown`}
            hasCounts
            trigger={
              <>
                {label}
                <span
                  className="harness-ui-chip-bar"
                  role="progressbar"
                  aria-label="context window usage"
                  aria-valuenow={0}
                  aria-valuemin={0}
                  aria-valuemax={100}
                >
                  <span className="harness-ui-chip-fill" style={{ width: 0 }} />
                </span>
                <span className="harness-ui-chip-counts">
                  0/{formatTokens(contextWindow)}
                </span>
                {caret}
              </>
            }
            popover={
              <EmptyContextPopover
                modelId={modelId}
                contextWindow={contextWindow}
                sessionId={sessionId}
              />
            }
          />
        )
      }
      return (
        <ContextChipSurface
          open={open}
          onOpenChange={setOpen}
          triggerLabel="context: waiting for usage — click for the breakdown"
          trigger={
            <>
              {label}
              <span className="harness-ui-chip-empty">—</span>
              {caret}
            </>
          }
          popover={<EmptyContextPopover modelId={modelId} sessionId={sessionId} />}
        />
      )
    }

    const ratio = Math.min(1, snapshot.total / snapshot.usable)
    const pct = Math.round(ratio * 100)
    const tone = toneFor(ratio)

    return (
      <ContextChipSurface
        open={open}
        onOpenChange={setOpen}
        triggerLabel={`context: ${snapshot.total.toLocaleString()} of ${snapshot.usable.toLocaleString()} tokens (${pct}%) — click for the breakdown`}
        hasCounts
        trigger={
          <>
            {label}
            <span
              className="harness-ui-chip-bar"
              role="progressbar"
              aria-label="context window usage"
              aria-valuenow={pct}
              aria-valuemin={0}
              aria-valuemax={100}
            >
              <span
                className="harness-ui-chip-fill"
                style={{ width: `${pct}%`, background: TONE_COLOR[tone] }}
              />
            </span>
            {/* The bar, the percentage and the counts were three encodings of
                one quantity. The bar carries proportion; the counts carry the
                number you actually act on. The percentage keeps its job in the
                tooltip and the popover, and hands its tone to the counts. */}
            <span
              className="harness-ui-chip-counts"
              style={tone === 'ok' ? undefined : { color: TONE_COLOR[tone] }}
            >
              {formatTokens(snapshot.total)}/{formatTokens(snapshot.usable)}
            </span>
            {caret}
          </>
        }
        popover={
          <ContextPopover
            snapshot={snapshot}
            modelId={modelId}
            sessionId={sessionId}
          />
        }
      />
    )
  }
}
