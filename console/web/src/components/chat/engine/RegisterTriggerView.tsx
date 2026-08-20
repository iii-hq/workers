import {
  Activity,
  Bell,
  CircleAlert,
  CircleOff,
  FunctionSquare,
  Info,
  RadioTower,
  ShieldCheck,
} from 'lucide-react'
import { useEffect, useState } from 'react'
import { registrationFromCall } from '@/components/trigger-activity/model'
import {
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
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { formatElapsed, timestampMilliseconds } from '@/lib/relative-time'
import { cn } from '@/lib/utils'
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
export function RegisterTriggerView({
  messageId,
  input,
  output,
  running,
}: RegisterTriggerViewProps) {
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

  const metadata = objectOf(req.metadata)
  const eventInto =
    target?.event_into ??
    (typeof metadata?.event_into === 'string' ? metadata.event_into : undefined)
  const callPayload =
    target?.payload !== undefined ? target.payload : metadata?.payload
  const stats = [
    {
      label: 'Mode',
      value: once === true ? 'Once' : 'Recurring',
    },
    ...(req.lifecycle?.max_fires !== undefined
      ? [{ label: 'Fire limit', value: String(req.lifecycle.max_fires) }]
      : []),
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
    ...(regId ? [{ label: 'ID', value: regId }] : []),
  ]

  return (
    <div
      className="flex min-w-0 flex-col gap-4"
      data-trigger-registration-details=""
    >
      {running || !registered ? (
        <div className="flex min-w-0 items-start gap-3">
          <div
            className={cn(
              'flex size-9 shrink-0 items-center justify-center rounded-full',
              running ? 'bg-accent-muted' : 'bg-warn-muted',
            )}
          >
            {running ? (
              <RadioTower
                aria-hidden
                className="size-5 animate-pulse stroke-accent motion-reduce:animate-none"
              />
            ) : (
              <CircleAlert aria-hidden className="size-5 stroke-warn" />
            )}
          </div>
          <div className="min-w-0 flex-1 font-sans">
            <div className="text-base font-medium text-ink sm:text-sm">
              {running ? 'registering trigger…' : 'trigger registration failed'}
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
              <div
                className={cn(
                  'min-w-0 font-sans text-base break-all sm:text-sm',
                  target ? 'text-ink' : 'text-ink-faint italic',
                )}
              >
                {target?.function_id ?? 'this session'}
              </div>
              {eventInto !== undefined ? (
                <FilterChip label="event into" value={eventInto || '(root)'} />
              ) : null}
            </div>
          </TriggerTraceNode>
        }
      />

      <TriggerStats items={stats} />

      {req.conditions?.length ? (
        <CardHighlight className="p-3 @xl:p-4">
          <div className="flex min-w-0 items-start gap-3">
            <div className="flex size-10 shrink-0 items-center justify-center rounded-full bg-accent-muted">
              <ShieldCheck
                aria-hidden
                className="size-5 shrink-0 stroke-accent"
              />
            </div>
            <div className="min-w-0 flex-1">
              <div className="font-mono text-base tracking-wide text-ink-ghost uppercase sm:text-xs">
                Only if
              </div>
              <div className="mt-2 flex min-w-0 flex-col divide-y divide-edge">
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
      {req.metadata !== undefined && !isEmpty(req.metadata) ? (
        <TriggerJsonPane
          label="Registration metadata"
          value={req.metadata}
          variant="secondary"
        />
      ) : null}

      {resp?.note ? (
        <div className="flex min-w-0 items-start gap-3 border-t border-edge pt-4">
          <div className="flex size-9 shrink-0 items-center justify-center rounded-full bg-accent-muted">
            <Info aria-hidden className="size-5 shrink-0 stroke-accent" />
          </div>
          <div className="min-w-0 flex-1 font-sans">
            <div className="text-base font-medium text-ink sm:text-sm">
              Registration note
            </div>
            <p className="text-pretty text-base wrap-break-word text-ink-faint sm:text-sm">
              {resp.note}
            </p>
          </div>
        </div>
      ) : null}
    </div>
  )
}

interface TriggerRegisteredDisplayProps {
  input: unknown
  output: unknown
  sessionId?: string
  createdAt?: number
  now?: number
}

/** Compact registration receipt. The full WHEN/IF/THEN model remains in the
 * expanded renderer; this surface makes the new active binding unmistakable. */
export function TriggerRegisteredDisplay({
  input,
  output,
  sessionId,
  createdAt,
  now,
}: TriggerRegisteredDisplayProps) {
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
    sessionId,
    subscriptionId: resp?.subscription_id,
    registered: Boolean(registrationId),
  })
  const clock = useRelativeClock(createdAt)
  const currentTime = now ?? clock

  if (!req || !registrationId) return null
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
          <div className="truncate font-sans text-base text-ink-faint sm:text-sm">
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

function useRegisteredTriggerActive({
  sessionId,
  subscriptionId,
  registered,
}: {
  sessionId?: string
  subscriptionId?: string
  registered: boolean
}): boolean {
  const ctx = useConversationsCtxOptional()
  const [active, setActive] = useState(registered)

  useEffect(() => {
    setActive(registered)
    const listTriggers = ctx?.backend.listTriggers
    if (!registered || !sessionId || !subscriptionId || !listTriggers) {
      return
    }
    let cancelled = false
    const refresh = () => {
      void listTriggers(sessionId)
        .then((rows) => {
          if (cancelled) return
          setActive(
            rows.some((row) => row.id === subscriptionId && row.fired !== true),
          )
        })
        .catch(() => {})
    }
    refresh()
    const off = ctx.backend.onTriggersChanged?.(sessionId, refresh)
    return () => {
      cancelled = true
      off?.()
    }
  }, [ctx?.backend, registered, sessionId, subscriptionId])

  return active
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
      <div className="font-sans text-base font-medium break-all text-ink sm:text-sm">
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
