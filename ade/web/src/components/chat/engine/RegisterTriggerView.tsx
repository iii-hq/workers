import {
  Activity,
  Bell,
  CircleAlert,
  CircleOff,
  FunctionSquare,
  RadioTower,
  ShieldCheck,
} from 'lucide-react'
import { registrationFromCall } from '@/components/trigger-activity/model'
import {
  firstRenderedTriggerActivitySlot,
  useTriggerActivityRenderers,
} from '@/components/trigger-activity/renderer-registry'
import {
  TriggerEyebrow,
  TriggerGlyph,
  TriggerJsonPane,
  TriggerStats,
  TriggerTrace,
  TriggerTraceNode,
} from '@/components/trigger-activity/TriggerDetails'
import { TriggerSource } from '@/components/trigger-activity/TriggerSource'
import { ActivityMetadata } from '@/components/ui/ActivityMetadata'
import { ActivityStatus } from '@/components/ui/ActivityStatus'
import { OpenDetailsAffordance } from '@/components/ui/OpenDetailsAffordance'
import { CardHighlight } from '@/components/ui/Surface'
import { useRelativeClock } from '@/hooks/use-relative-clock'
import { formatElapsed, timestampMilliseconds } from '@/lib/relative-time'
import { cn } from '@/lib/utils'
import { useRegisteredTriggerActive } from '../RegisteredTriggerStatus'
import {
  type RegisterTriggerRequest,
  type RegisterTriggerResponse,
  registerTriggerRequestSchema,
  registerTriggerResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'
import { FilterChip } from './shared'

interface RegisterTriggerViewProps {
  messageId: string
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * A trigger registration is a cause→effect rule: WHEN an event fires (filtered
 * by `config`) THEN deliver (notify the session, or call a plain function —
 * a binding never starts an agent). The view reads that way — a labeled
 * `when` block (the event + its filter chips) above a `then` block — so a
 * binding's meaning is legible at a glance. Raw payloads stay one tab away in
 * RAW JSON; this view is the readable one.
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

export function RegisterTriggerView({
  messageId,
  input,
  output,
  running,
}: RegisterTriggerViewProps) {
  const triggerRenderers = useTriggerActivityRenderers()
  const req = safeParseRequest<RegisterTriggerRequest>(
    registerTriggerRequestSchema,
    input,
  )
  // Never render blank: an unrecognized payload falls back to raw JSON rather
  // than an empty terminal pane (the switch always mounts this component).
  if (!req) return <LabeledJson label="request" value={input} />

  const resp = running
    ? null
    : safeParseResponse<RegisterTriggerResponse>(
        registerTriggerResponseSchema,
        output,
      )
  const regId = resp?.id ?? resp?.subscription_id
  const registered = Boolean(regId)
  const once = resp?.once ?? req.once ?? req.lifecycle?.once
  const target =
    req.target ??
    (req.function_id ? { function_id: req.function_id } : undefined)
  const registration = registrationFromCall({
    id: messageId,
    input: req,
    ...(regId ? { subscriptionId: regId } : {}),
    ...(typeof once === 'boolean' ? { effectiveOnce: once } : {}),
    ...(resp?.note ? { note: resp.note } : {}),
  })
  const workerDetails = registered
    ? firstRenderedTriggerActivitySlot(
        triggerRenderers,
        registration.activity,
        (renderer) =>
          renderer.tryRenderDetails?.(registration.activity) ?? null,
      )
    : null

  if (workerDetails) return workerDetails.node

  const metadata = objectOf(req.metadata)
  const eventInto =
    target?.event_into ??
    (typeof metadata?.event_into === 'string' ? metadata.event_into : undefined)
  const callPayload =
    target?.payload !== undefined ? target.payload : metadata?.payload
  const registrationMetadata = metadata
    ? Object.fromEntries(
        Object.entries(metadata).filter(([key]) => key !== 'action'),
      )
    : req.metadata
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
    // Legacy cards: requests recorded before `expires_at` was retired.
    ...(req.lifecycle?.expires_at !== undefined
      ? [
          {
            label: 'Expires',
            value: new Date(
              timestampMilliseconds(req.lifecycle.expires_at),
            ).toLocaleString(),
          },
        ]
      : []),
    ...(regId ? [{ label: 'ID', value: regId, mono: true }] : []),
  ]

  // The card's Terminal tab hands this view the full pane, so the view owns
  // its padding and its container query scope (the WHEN/THEN row switches to
  // two columns by the card's width, not the chat column's).
  return (
    <div
      className="@container flex min-w-0 flex-col gap-4 p-4 sm:p-3"
      data-trigger-registration-details=""
    >
      {running || !registered ? (
        <div className="flex min-w-0 items-start gap-3">
          <TriggerGlyph tone={running ? 'accent' : 'warn'}>
            {running ? (
              <RadioTower
                aria-hidden
                className="animate-pulse motion-reduce:animate-none"
              />
            ) : (
              <CircleAlert aria-hidden />
            )}
          </TriggerGlyph>
          <div className="min-w-0 flex-1 font-sans">
            <div className="text-base font-medium text-ink sm:text-sm">
              {running ? 'Registering trigger…' : 'Trigger registration failed'}
            </div>
            <p className="text-pretty text-base text-ink-faint sm:text-sm">
              {running
                ? 'Creating this binding and preparing it to listen for events.'
                : 'The binding was not created. Check the raw response for details.'}
            </p>
          </div>
        </div>
      ) : null}

      <TriggerTrace
        when={
          <TriggerTraceNode
            kind="when"
            icon={<RadioTower aria-hidden />}
            label="When"
            title={registration.activity.triggerType}
            mono
          >
            <TriggerSource
              activity={registration.activity}
              presentation="compact"
            />
          </TriggerTraceNode>
        }
        then={
          <TriggerTraceNode
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
          </TriggerTraceNode>
        }
      />

      <TriggerStats items={stats} />

      {req.conditions?.length ? (
        <CardHighlight>
          <div className="flex min-w-0 items-start gap-3 p-3 @xl:p-4">
            <TriggerGlyph>
              <ShieldCheck aria-hidden />
            </TriggerGlyph>
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <TriggerEyebrow>Only if</TriggerEyebrow>
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
        <TriggerJsonPane
          label="Call payload"
          value={callPayload}
          variant="secondary"
        />
      ) : null}
      {registrationMetadata !== undefined && !isEmpty(registrationMetadata) ? (
        <TriggerJsonPane
          label="Registration metadata"
          value={registrationMetadata}
          variant="secondary"
        />
      ) : null}
    </div>
  )
}

interface TriggerRegisteredDisplayProps {
  messageId?: string
  input: unknown
  output: unknown
  createdAt?: number
  now?: number
}

/** Compact registration receipt. The full WHEN/IF/THEN model remains in the
 * expanded renderer; this surface makes the new active binding unmistakable. */
export function TriggerRegisteredDisplay({
  messageId,
  input,
  output,
  createdAt,
  now,
}: TriggerRegisteredDisplayProps) {
  const triggerRenderers = useTriggerActivityRenderers()
  const req = safeParseRequest<RegisterTriggerRequest>(
    registerTriggerRequestSchema,
    input,
  )
  const resp = safeParseResponse<RegisterTriggerResponse>(
    registerTriggerResponseSchema,
    output,
  )
  const registrationId = resp?.subscription_id ?? resp?.id
  const active = useRegisteredTriggerActive({
    subscriptionId: registrationId,
    registered: Boolean(registrationId),
  })
  const clock = useRelativeClock(createdAt)
  const currentTime = now ?? clock

  if (!req || !registrationId) return null
  const registration = registrationFromCall({
    id: messageId ?? `trigger-registration:${registrationId}`,
    input: req,
    subscriptionId: registrationId,
    ...(typeof (resp?.once ?? req.once ?? req.lifecycle?.once) === 'boolean'
      ? { effectiveOnce: resp?.once ?? req.once ?? req.lifecycle?.once }
      : {}),
    ...(resp?.note ? { note: resp.note } : {}),
  })
  const workerDisplay = firstRenderedTriggerActivitySlot(
    triggerRenderers,
    registration.activity,
    (renderer) => renderer.tryRenderDisplay?.(registration.activity) ?? null,
  )
  if (workerDisplay) return workerDisplay.node
  const label = req.label?.trim() || 'Unlabeled trigger'
  const once = resp?.once ?? req.once ?? req.lifecycle?.once
  const createdAge = formatElapsed(createdAt, currentTime)

  return (
    <div
      className="grid min-w-0 gap-4 @xl:grid-cols-[minmax(0,1fr)_auto] @xl:items-center"
      data-trigger-registration-state={active ? 'active' : 'inactive'}
      data-trigger-registration-id={registrationId}
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
            Trigger registered
          </div>
          <div
            className="truncate font-sans text-base text-ink-faint sm:text-sm"
            data-trigger-registration-label=""
          >
            {label}
          </div>
          <div className="truncate font-mono text-base text-ink-ghost sm:text-[0.6875rem]">
            {req.trigger_type}
            {once === true ? ' · one-shot' : ' · persistent'}
          </div>
          <ActivityMetadata
            className="mt-3"
            createdAt={createdAt}
            identifier={registrationId}
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
        <TriggerJsonPane
          label="Condition config"
          value={Object.fromEntries(rest)}
          variant="secondary"
        />
      ) : config === null && condition.config !== undefined ? (
        <TriggerJsonPane
          label="Condition config"
          value={condition.config}
          variant="secondary"
        />
      ) : null}
    </div>
  )
}

function isEmpty(v: unknown): boolean {
  if (v === null || v === undefined) return true
  if (typeof v === 'object') {
    return Object.keys(v as Record<string, unknown>).length === 0
  }
  return false
}

function LabeledJson({ label, value }: { label: string; value: unknown }) {
  return (
    <TriggerJsonPane
      label={label.charAt(0).toUpperCase() + label.slice(1)}
      value={value}
    />
  )
}

const objectOf = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
