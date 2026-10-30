import {
  Activity,
  Bell,
  CircleAlert,
  CircleOff,
  FunctionSquare,
  RadioTower,
  ShieldCheck,
} from 'lucide-react'
import { registrationFromCall } from '@/components/observer-activity/model'
import {
  ObserverEyebrow,
  ObserverGlyph,
  ObserverJsonPane,
  ObserverStats,
  ObserverTrace,
  ObserverTraceNode,
} from '@/components/observer-activity/ObserverDetails'
import { ObserverSource } from '@/components/observer-activity/ObserverSource'
import {
  firstRenderedObserverActivitySlot,
  useObserverActivityRenderers,
} from '@/components/observer-activity/renderer-registry'
import { ActivityMetadata } from '@/components/ui/ActivityMetadata'
import { ActivityStatus } from '@/components/ui/ActivityStatus'
import { OpenDetailsAffordance } from '@/components/ui/OpenDetailsAffordance'
import { CardHighlight } from '@/components/ui/Surface'
import { useRelativeClock } from '@/hooks/use-relative-clock'
import { formatElapsed } from '@/lib/relative-time'
import { cn } from '@/lib/utils'
import { useRegisteredObserverActive } from '../RegisteredObserverStatus'
import {
  type ObserveRequest,
  type ObserveResponse,
  observeRequestSchema,
  observeResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'
import { FilterChip } from './shared'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

interface ObserveViewProps {
  messageId: string
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * An agent's `harness::observe` is a cause→effect rule: WHEN the observed
 * function is called (its args and filters say which calls) THEN deliver
 * (notify the session, or call a plain function — an observer never starts
 * an agent). The view reads that way — a labeled `when` block (the function
 * + its args and filter chips) above a `then` block — so an observer's
 * meaning is legible at a glance. Raw payloads stay one tab away in RAW
 * JSON; this view is the readable one.
 */
/** "45s" / "12m" / "3h" / "2d" — coarse duration for the relative deadline
 * stat; mirrors formatElapsed's buckets without the timestamp semantics. */
function formatDurationMs(ms: number): string {
  const seconds = Math.max(1, Math.round(ms / 1000))
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes}m`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours}h`
  return `${Math.round(hours / 24)}d`
}

export function ObserveView({
  messageId,
  input,
  output,
  running,
}: ObserveViewProps) {
  const observerRenderers = useObserverActivityRenderers()
  const req = safeParseRequest<ObserveRequest>(observeRequestSchema, input)
  // Never render blank: an unrecognized payload falls back to raw JSON rather
  // than an empty terminal pane (the switch always mounts this component).
  if (!req) return <LabeledJson label="request" value={input} />

  const resp = running
    ? null
    : safeParseResponse<ObserveResponse>(observeResponseSchema, output)
  const observerId = resp?.observer_id
  const registered = Boolean(observerId)
  const once = resp?.once ?? req.once ?? req.lifecycle?.once
  const target = req.target
  const registration = registrationFromCall({
    id: messageId,
    input: req,
    ...(observerId ? { observerId } : {}),
    ...(typeof once === 'boolean' ? { effectiveOnce: once } : {}),
    ...(resp?.note ? { note: resp.note } : {}),
  })
  const workerDetails = registered
    ? firstRenderedObserverActivitySlot(
        observerRenderers,
        registration.activity,
        (renderer) =>
          renderer.tryRenderDetails?.(registration.activity) ?? null,
      )
    : null

  if (workerDetails) return workerDetails.node

  const eventInto = target?.event_into
  const callPayload = target?.payload
  const stats = [
    {
      label: 'Mode',
      value: once === true ? 'Once' : 'Recurring',
    },
    ...(req.lifecycle?.max_fires !== undefined
      ? [{ label: 'Fire limit', value: String(req.lifecycle.max_fires) }]
      : []),
    ...(req.lifecycle?.expires_in_ms !== undefined
      ? [
          {
            label: 'Expires',
            value: `in ${formatDurationMs(req.lifecycle.expires_in_ms)}`,
          },
        ]
      : []),
    ...(observerId ? [{ label: 'ID', value: observerId, mono: true }] : []),
  ]

  // The card's Terminal tab hands this view the full pane, so the view owns
  // its padding and its container query scope (the WHEN/THEN row switches to
  // two columns by the card's width, not the chat column's).
  return (
    <div
      className="@container flex min-w-0 flex-col gap-4 p-4 sm:p-3"
      data-observer-registration-details=""
    >
      {running || !registered ? (
        <div className="flex min-w-0 items-start gap-3">
          <ObserverGlyph tone={running ? 'accent' : 'warn'}>
            {running ? (
              <RadioTower
                aria-hidden
                className="animate-pulse motion-reduce:animate-none"
              />
            ) : (
              <CircleAlert aria-hidden />
            )}
          </ObserverGlyph>
          <div className="min-w-0 flex-1 font-sans">
            <div className="text-base font-medium text-ink sm:text-sm">
              {running ? 'Adding observer…' : 'Observer not added'}
            </div>
            <p className="text-pretty text-base text-ink-faint sm:text-sm">
              {running
                ? 'Creating this observer and waiting for it to become active.'
                : 'The observer was not created. Check the raw response for details.'}
            </p>
          </div>
        </div>
      ) : null}

      <ObserverTrace
        when={
          <ObserverTraceNode
            kind="when"
            icon={<RadioTower aria-hidden />}
            label="When"
            title={registration.activity.functionId}
            mono
          >
            <ObserverSource
              activity={registration.activity}
              presentation="compact"
            />
          </ObserverTraceNode>
        }
        then={
          <ObserverTraceNode
            kind="then"
            icon={
              target ? <FunctionSquare aria-hidden /> : <Bell aria-hidden />
            }
            label="Then"
            title={target ? 'Call' : 'Notify'}
          >
            <div className="flex min-w-0 flex-col gap-2">
              {target ? (
                <div className="min-w-0 font-mono text-[13px] break-all text-ink">
                  {target.function_id}
                </div>
              ) : (
                <div className="min-w-0 font-sans text-base text-ink-faint sm:text-sm">
                  this session
                </div>
              )}
              {eventInto !== undefined ? (
                <FilterChip label="event into" value={eventInto || '(root)'} />
              ) : null}
            </div>
          </ObserverTraceNode>
        }
      />

      <ObserverStats items={stats} />

      {req.conditions?.length ? (
        <CardHighlight>
          <div className="flex min-w-0 items-start gap-3 p-3 @xl:p-4">
            <ObserverGlyph>
              <ShieldCheck aria-hidden />
            </ObserverGlyph>
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <ObserverEyebrow>Only if</ObserverEyebrow>
              <div className="flex min-w-0 flex-col divide-y divide-edge">
                {req.conditions.map((condition, index) => (
                  <ConditionRow
                    // biome-ignore lint/suspicious/noArrayIndexKey: conditions have no id; their declared order is their identity and execution order.
                    key={`${condition.function_id ?? 'condition'}-${index}`}
                    condition={condition}
                  />
                ))}
              </div>
            </div>
          </div>
        </CardHighlight>
      ) : null}

      {callPayload !== undefined ? (
        <ObserverJsonPane
          label="Call payload"
          value={callPayload}
          variant="secondary"
        />
      ) : null}
    </div>
  )
}

interface ObserverAddedDisplayProps {
  messageId?: string
  input: unknown
  output: unknown
  createdAt?: number
  now?: number
}

/** Compact observe receipt. The full WHEN/IF/THEN model remains in the
 * expanded renderer; this surface makes the new active observer unmistakable. */
export function ObserverAddedDisplay({
  messageId,
  input,
  output,
  createdAt,
  now,
}: ObserverAddedDisplayProps) {
  const observerRenderers = useObserverActivityRenderers()
  const req = safeParseRequest<ObserveRequest>(observeRequestSchema, input)
  const resp = safeParseResponse<ObserveResponse>(observeResponseSchema, output)
  const observerId = resp?.observer_id
  const active = useRegisteredObserverActive({
    observerId,
    registered: Boolean(observerId),
  })
  const clock = useRelativeClock(createdAt)
  const currentTime = now ?? clock

  if (!req || !observerId) return null
  const registration = registrationFromCall({
    id: messageId ?? `observer-registration:${observerId}`,
    input: req,
    observerId,
    ...(typeof (resp?.once ?? req.once ?? req.lifecycle?.once) === 'boolean'
      ? { effectiveOnce: resp?.once ?? req.once ?? req.lifecycle?.once }
      : {}),
    ...(resp?.note ? { note: resp.note } : {}),
  })
  const workerDisplay = firstRenderedObserverActivitySlot(
    observerRenderers,
    registration.activity,
    (renderer) => renderer.tryRenderDisplay?.(registration.activity) ?? null,
  )
  if (workerDisplay) return workerDisplay.node
  const label = req.label?.trim() || 'Unlabeled observer'
  const once = resp?.once ?? req.once ?? req.lifecycle?.once
  const createdAge = formatElapsed(createdAt, currentTime)

  return (
    <div
      className="grid min-w-0 gap-4 @xl:grid-cols-[minmax(0,1fr)_auto] @xl:items-center"
      data-observer-registration-state={active ? 'active' : 'inactive'}
      data-observer-registration-id={observerId}
    >
      <div className="flex min-w-0 items-start gap-3">
        <div
          className={cn(
            'flex size-12 shrink-0 items-center justify-center rounded-md sm:size-10',
            active ? 'bg-ok-muted text-ok' : 'bg-surface text-ink-ghost',
          )}
        >
          <RadioTower
            aria-hidden
            strokeWidth={2.25}
            className={cn(
              'size-6 h-lh shrink-0 sm:size-5',
              active
                ? 'animate-pulse stroke-ok motion-reduce:animate-none'
                : 'stroke-ink-ghost',
            )}
          />
        </div>
        <div className="min-w-0 flex-1">
          <div className="font-sans text-base font-semibold text-ink sm:text-sm">
            Observer added
          </div>
          <div
            className="truncate font-sans text-base text-ink-faint sm:text-sm"
            data-observer-registration-label=""
          >
            {label}
          </div>
          <div className="truncate font-mono text-base text-ink-ghost sm:text-[0.6875rem]">
            {req.function_id}
            {once === true ? ' · one-shot' : ' · persistent'}
          </div>
          <ActivityMetadata
            className="mt-3"
            createdAt={createdAt}
            identifier={observerId}
            now={currentTime}
          />
        </div>
      </div>

      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 border-t border-rule-2 pt-3 @xl:flex @xl:flex-col @xl:items-stretch @xl:border-t-0 @xl:pt-0">
        <ActivityStatus
          label={active ? 'Active' : 'Inactive'}
          detail={
            active
              ? createdAge === 'just now'
                ? 'Active now'
                : createdAge
                  ? `Active for ${createdAge}`
                  : 'Listening for events'
              : 'No longer listening'
          }
          icon={active ? Activity : CircleOff}
          tone={active ? 'positive' : 'neutral'}
        />
        <OpenDetailsAffordance />
      </div>
    </div>
  )
}

const isScalar = (v: unknown) =>
  v === null ||
  typeof v === 'string' ||
  typeof v === 'number' ||
  typeof v === 'boolean'

/**
 * One gating predicate: the condition function (accent, like the THEN call)
 * with its config as chips — scalars and primitive arrays inline, anything
 * nested in a compact JSON block so no field is silently dropped.
 */
function ConditionRow({
  condition,
}: {
  condition: { function_id?: string; config?: unknown }
}) {
  const config =
    condition.config &&
    typeof condition.config === 'object' &&
    !Array.isArray(condition.config)
      ? (condition.config as Record<string, unknown>)
      : null
  const entries = config ? Object.entries(config) : []
  const chippable = entries.filter(
    ([, v]) => isScalar(v) || (Array.isArray(v) && v.every(isScalar)),
  )
  const rest = entries.filter(
    ([, v]) => !(isScalar(v) || (Array.isArray(v) && v.every(isScalar))),
  )
  return (
    <div className="flex min-w-0 flex-col gap-2 py-3 first:pt-0 last:pb-0">
      <div
        className={cn(
          'min-w-0 break-all text-ink',
          condition.function_id
            ? 'font-mono text-[13px]'
            : 'font-sans text-base font-medium sm:text-sm',
        )}
      >
        {condition.function_id ?? 'Condition'}
      </div>
      {chippable.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          {chippable.map(([key, value]) => (
            <FilterChip
              key={key}
              label={key}
              value={
                Array.isArray(value)
                  ? value.map(String).join(', ')
                  : String(value)
              }
            />
          ))}
        </div>
      ) : null}
      {rest.length > 0 ? (
        <ObserverJsonPane
          label="Condition config"
          value={Object.fromEntries(rest)}
          variant="secondary"
        />
      ) : config === null && condition.config !== undefined ? (
        <ObserverJsonPane
          label="Condition config"
          value={condition.config}
          variant="secondary"
        />
      ) : null}
    </div>
  )
}

function LabeledJson({ label, value }: { label: string; value: unknown }) {
  return (
    <ObserverJsonPane
      label={label.charAt(0).toUpperCase() + label.slice(1)}
      value={value}
    />
  )
}
