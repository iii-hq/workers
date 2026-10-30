import {
  Bell,
  Check,
  CircleAlert,
  CircleDashed,
  FunctionSquare,
  Info,
  RadioTower,
  X,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { FilterChip } from '@/components/chat/engine/shared'
import {
  TimelineActivityDisclosure,
  TimelineActivityTrail,
} from '@/components/chat/TimelineActivityTrail'
import { Badge } from '@/components/ui/Badge'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from '@/components/ui/CollapsibleCard'
import { ObserverIcon } from '@/components/ui/ObserverIcon'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { timestampMilliseconds } from '@/lib/relative-time'
import { cn } from '@/lib/utils'
import type { SystemMessage, UserMessage } from '@/types/chat'
import type { ObserverActivityMessage } from '@/types/injectable-ui'
import {
  activityFromObserverRecord,
  isConditionWait,
  type ObserverRegistration,
  parseNotification,
} from './model'
import {
  ObserverGlyph,
  ObserverJsonPane,
  ObserverStats,
  ObserverTrace,
  ObserverTraceNode,
} from './ObserverDetails'
import { ObserverSource } from './ObserverSource'
import {
  firstRenderedObserverActivitySlot,
  observerActivityRawRedactor,
  useObserverActivityRenderers,
} from './renderer-registry'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

interface ObserverActivityCardProps {
  record?: SystemMessage
  notification?: UserMessage
  registration?: ObserverRegistration
  /** Open the full activity card on first render (showcase/detail surfaces). */
  defaultOpen?: boolean
}

function ObserverActivityOutcomeIcon({
  status,
}: {
  status: 'done' | 'error' | 'pending'
}) {
  return (
    <span
      aria-hidden="true"
      className="activity-status-icon"
      data-status={status}
    >
      <span data-activity-status-layer="pending">
        <CircleDashed
          strokeWidth={2}
          className="size-4 stroke-muted-foreground"
        />
      </span>
      <span data-activity-status-layer="error">
        <X strokeWidth={2.5} className="size-4 stroke-alert" />
      </span>
      <span data-activity-status-layer="done">
        <Check strokeWidth={2.5} className="size-4 stroke-muted-foreground" />
      </span>
    </span>
  )
}

/** One host-owned card for registration delivery, fire, and retirement state. */
export function ObserverActivityCard({
  record,
  notification,
  registration,
  defaultOpen,
}: ObserverActivityCardProps) {
  const parsed = notification ? parseNotification(notification.content) : null
  const fromRecord = record
    ? activityFromObserverRecord(record, registration)
    : null
  const base =
    fromRecord ?? activityFromNotification(notification, registration)
  const activity =
    base && base.payload === undefined && parsed
      ? { ...base, payload: parsed.payload }
      : base
  const renderers = useObserverActivityRenderers()
  const redactor = useMemo(
    () =>
      activity
        ? observerActivityRawRedactor(renderers, activity.functionId)
        : undefined,
    [activity, renderers],
  )
  const raw = useMemo(
    () => ({
      registration: redactor ? redactor(registration?.raw) : registration?.raw,
      notification: redactor
        ? redactor(parsed?.payload ?? notification?.content)
        : (parsed?.payload ?? notification?.content),
      fire: redactor ? redactor(record?.observer) : record?.observer,
    }),
    [
      notification?.content,
      parsed?.payload,
      record?.observer,
      redactor,
      registration?.raw,
    ],
  )
  const [tab, setTab] = useState<'terminal' | 'json'>('terminal')
  const [open, setOpen] = useState(!!defaultOpen)

  if (!activity) {
    return (
      <article
        className="flex items-start gap-3 rounded-md border border-edge bg-panel-raised p-4 shadow-raised sm:p-3"
        data-message-role="observer-activity"
      >
        <div className="flex size-10 shrink-0 items-center justify-center rounded-md bg-warn-muted sm:size-9">
          <ObserverIcon
            aria-hidden
            className="size-5 shrink-0 fill-warn sm:size-4"
          />
        </div>
        <div className="min-w-0 flex-1 font-sans text-base wrap-break-word text-ink sm:text-sm">
          {notification?.content ?? record?.content ?? 'Observer activity'}
        </div>
      </article>
    )
  }

  const title = activityTitle(activity)
  const source = activity.label ?? activity.functionId
  const target =
    activity.delivery.kind === 'call'
      ? activity.delivery.functionId
      : 'this chat'
  const display = firstRenderedObserverActivitySlot(
    renderers,
    activity,
    (renderer) => renderer.tryRenderDisplay?.(activity) ?? null,
  )
  const details = firstRenderedObserverActivitySlot(
    renderers,
    activity,
    (renderer) => renderer.tryRenderDetails?.(activity) ?? null,
  )
  const eventText = activityEventText(activity)
  // A condition that answered "not yet" is the observer waiting, not a failure.
  const fireStatus = isConditionWait(activity)
    ? 'pending'
    : activity.outcome === 'skipped' || activity.outcome === 'delivery_failed'
      ? 'error'
      : 'done'

  const expandedSummary = (
    <div className="flex min-w-0 items-start gap-3">
      <div
        className={cn(
          'flex size-12 shrink-0 items-center justify-center rounded-md sm:size-10',
          activity.kind === 'retirement' ? 'bg-surface' : 'bg-warn-muted',
        )}
      >
        <ObserverIcon
          aria-hidden
          className={cn(
            'size-6 shrink-0 sm:size-5',
            activity.kind === 'retirement' ? 'fill-ink-ghost' : 'fill-warn',
          )}
        />
      </div>

      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1 font-sans text-base sm:text-sm">
          <div className="font-semibold text-ink">{title}</div>
          <span aria-hidden className="text-ink-ghost">
            ·
          </span>
          <div className="flex min-w-0 items-baseline gap-2 text-ink-faint">
            <span className="min-w-0 truncate" title={source}>
              {source}
            </span>
            <span aria-hidden className="shrink-0 text-ink-ghost">
              →
            </span>
            <span className="min-w-0 truncate" title={target}>
              {target}
            </span>
          </div>
        </div>
        <p className="text-pretty font-sans text-base text-ink-faint sm:text-sm">
          {activity.kind === 'fired'
            ? (activity.action ?? activityDescription(activity))
            : activityDescription(activity)}
        </p>
      </div>
    </div>
  )

  return (
    <article
      className="w-full"
      data-message-id={activity.id}
      data-message-role="observer-activity"
      data-observer-activity-kind={activity.kind}
      data-expanded={activity.kind === 'fired' ? open : undefined}
    >
      <CollapsibleCard
        open={activity.kind === 'fired' ? open : undefined}
        onOpenChange={activity.kind === 'fired' ? setOpen : undefined}
        className={cn(
          '@container',
          activity.kind === 'fired' && 'observer-activity-collapsible',
          activity.kind === 'fired' &&
            !open &&
            'observer-activity-collapsible--compact',
        )}
      >
        <CollapsibleCardTrigger
          aria-label={
            activity.kind === 'fired'
              ? `${open ? 'Hide' : 'Show'} observer details for ${eventText}`
              : undefined
          }
          className={
            activity.kind === 'fired'
              ? 'group observer-activity-collapsible__trigger select-none'
              : 'p-4 select-none sm:p-3'
          }
        >
          {activity.kind === 'fired' ? (
            <div className="flex w-full min-w-0 items-center gap-2">
              <ObserverActivityOutcomeIcon status={fireStatus} />
              <TimelineActivityTrail kind="observer-fired" />
              <div className="min-w-0 flex-1 truncate font-sans text-sm text-muted-foreground sm:text-[0.8125rem]">
                {display?.node ?? eventText}
              </div>
              <TimelineActivityDisclosure />
            </div>
          ) : (
            (display?.node ?? expandedSummary)
          )}
        </CollapsibleCardTrigger>

        <CollapsibleCardContent>
          {activity.kind === 'fired' ? (
            <div className="border-t border-edge p-4 sm:p-3">
              {expandedSummary}
            </div>
          ) : null}
          <Tabs
            value={tab}
            onValueChange={(value) => setTab(value as 'terminal' | 'json')}
            className="border-t border-edge"
          >
            <div className="overflow-x-auto px-4 sm:px-3">
              <TabsList>
                <TabsTrigger value="terminal">Terminal</TabsTrigger>
                <TabsTrigger value="json">Raw JSON</TabsTrigger>
              </TabsList>
            </div>
            <TabsContent value="terminal" className="p-4 sm:p-3">
              {details?.node ?? (
                <ObserverActivityTerminal activity={activity} />
              )}
            </TabsContent>
            <TabsContent value="json" className="p-4 sm:p-3">
              <div className="flex min-w-0 flex-col gap-3">
                {registration ? (
                  <ObserverJsonPane
                    label="Registration"
                    value={raw.registration}
                  />
                ) : null}
                {notification ? (
                  <ObserverJsonPane
                    label="Notification"
                    value={raw.notification}
                  />
                ) : null}
                {record?.observer ? (
                  <ObserverJsonPane label="Fire" value={raw.fire} />
                ) : null}
              </div>
            </TabsContent>
          </Tabs>
        </CollapsibleCardContent>
      </CollapsibleCard>
    </article>
  )
}

function activityFromNotification(
  notification?: UserMessage,
  registration?: ObserverRegistration,
): ObserverActivityMessage | null {
  if (!notification) return null
  const parsed = parseNotification(notification.content)
  // Harness also uses the trusted notification origin for actionable delivery
  // failures. They are not successful fires and their prose must remain fully
  // visible instead of being forced into observer-fired chrome.
  const isFireEntry = /^(?:e_fire_|e_notify_)/.test(notification.id)
  if (!parsed && !isFireEntry) return null
  const inherited = registration?.activity
  const inheritedLifecycle = inherited?.lifecycle ?? {
    state: 'active' as const,
    once: false,
    fires: 0,
  }
  return {
    id: notification.id,
    kind: 'fired',
    functionId: inherited?.functionId ?? 'observer',
    ...(inherited?.args !== undefined ? { args: inherited.args } : {}),
    ...(inherited?.filters !== undefined ? { filters: inherited.filters } : {}),
    ...((parsed?.name ?? inherited?.label)
      ? { label: parsed?.name ?? inherited?.label }
      : {}),
    ...(inherited?.action ? { action: inherited.action } : {}),
    ...(inherited?.conditions ? { conditions: inherited.conditions } : {}),
    delivery: { kind: 'notify' },
    lifecycle: {
      ...inheritedLifecycle,
      state: inheritedLifecycle.once ? 'retired' : inheritedLifecycle.state,
      fires: Math.max(inheritedLifecycle.fires, 1),
    },
    ...(notification.observerId ? { observerId: notification.observerId } : {}),
    ...(parsed ? { payload: parsed.payload } : {}),
    firedAt: notification.createdAt,
    outcome: 'delivered',
  }
}

function ObserverActivityTerminal({
  activity,
}: {
  activity: ObserverActivityMessage
}) {
  const lifecycle = lifecycleLabel(activity)
  const eventType = eventTypeOf(activity.payload)
  const showDeliveryBadge =
    activity.outcome === 'delivery_failed' ||
    (activity.outcome === 'skipped' && !isConditionWait(activity))
  return (
    <div className="flex min-w-0 flex-col gap-4">
      <ObserverTrace
        when={
          <ObserverTraceNode
            kind="when"
            icon={<RadioTower aria-hidden />}
            label="When"
            title={activity.functionId}
            mono
          >
            <ObserverSource activity={activity} presentation="compact" />
          </ObserverTraceNode>
        }
        then={<ObserverTarget activity={activity} eventType={eventType} />}
      />

      <ActivityStats activity={activity} />

      {activity.payload !== undefined ? (
        <ObserverJsonPane
          label={
            activity.delivery.kind === 'call'
              ? activity.outcome === 'delivery_failed'
                ? 'Attempted call payload'
                : 'Call payload'
              : 'Event data'
          }
          value={activity.payload}
          variant="secondary"
        />
      ) : null}

      <div className="flex min-w-0 flex-col gap-3 border-t border-edge pt-4 @md:flex-row @md:items-start">
        <ObserverGlyph tone={showDeliveryBadge ? 'warn' : 'neutral'}>
          {showDeliveryBadge ? (
            <CircleAlert aria-hidden />
          ) : (
            <Info aria-hidden />
          )}
        </ObserverGlyph>
        <div className="min-w-0 flex-1 font-sans">
          <div className="text-base font-medium text-ink sm:text-sm">
            {lifecycle.title}
          </div>
          {lifecycle.detail ? (
            <p className="text-pretty text-base text-ink-faint sm:text-sm">
              {lifecycle.detail}
            </p>
          ) : null}
          {activity.note ? (
            <p className="text-pretty text-base wrap-break-word text-ink-ghost sm:text-sm">
              {activity.note}
            </p>
          ) : null}
        </div>
        {showDeliveryBadge ? (
          <Badge className="shrink-0 self-start @md:self-center">
            {activity.outcome === 'delivery_failed'
              ? 'Delivery failed'
              : 'Delivery skipped'}
          </Badge>
        ) : null}
      </div>
    </div>
  )
}

function ActivityStats({ activity }: { activity: ObserverActivityMessage }) {
  const fires =
    activity.lifecycle.maxFires !== undefined
      ? `${activity.lifecycle.fires} / ${activity.lifecycle.maxFires}`
      : String(activity.lifecycle.fires)
  const firedAt =
    activity.firedAt === undefined
      ? '—'
      : new Date(timestampMilliseconds(activity.firedAt)).toLocaleString()
  const hasConditions = Boolean(activity.conditions?.length)
  const conditionsMet =
    hasConditions &&
    (activity.outcome === 'delivered' ||
      activity.outcome === 'delivery_failed' ||
      (activity.kind === 'fired' && activity.outcome === undefined))

  return (
    <div className="flex min-w-0 flex-col gap-2 @md:flex-row @md:items-center @md:justify-between">
      <ObserverStats
        items={[
          {
            label: 'Mode',
            value: activity.lifecycle.once ? 'Once' : 'Recurring',
          },
          { label: 'Fires', value: fires },
          { label: 'At', value: firedAt },
        ]}
      />
      {hasConditions ||
      activity.outcome === 'delivery_failed' ||
      activity.outcome === 'skipped' ? (
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {conditionsMet ? <Badge>Conditions met</Badge> : null}
          {activity.outcome === 'delivery_failed' ? (
            <Badge>Delivery failed</Badge>
          ) : isConditionWait(activity) ? (
            <Badge>Waiting</Badge>
          ) : activity.outcome === 'skipped' ? (
            <Badge>Delivery skipped</Badge>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function ObserverTarget({
  activity,
  eventType,
}: {
  activity: ObserverActivityMessage
  eventType: string | null
}) {
  const target =
    activity.delivery.kind === 'call'
      ? activity.delivery.functionId
      : 'this session'
  const call = activity.delivery.kind === 'call'
  return (
    <ObserverTraceNode
      kind="then"
      icon={call ? <FunctionSquare aria-hidden /> : <Bell aria-hidden />}
      label="Then"
      title={call ? 'Call' : 'Notify'}
    >
      <div className="flex min-w-0 flex-col gap-2">
        <div
          className={cn(
            'min-w-0',
            call
              ? 'font-mono text-[13px] break-all text-ink'
              : 'font-sans text-base text-ink-faint sm:text-sm',
          )}
        >
          {target}
        </div>
        {eventType ? <FilterChip label="event" value={eventType} /> : null}
      </div>
    </ObserverTraceNode>
  )
}

/** Human-readable compact event copy. `action` describes what happened;
 * `label` remains the fallback that names the observer. */
export function activityEventText(activity: ObserverActivityMessage): string {
  const action = activity.action?.trim()
  if (action) return action
  const label = activity.label?.trim()
  if (label) return label
  if (activity.functionId === 'state::on-changed') {
    const key = eqFilter(activity.filters, 'input.key')
    if (key) {
      const scope = eqFilter(activity.filters, 'input.scope')
      return scope ? `${scope}/${key}` : key
    }
  }
  return activity.functionId
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

function activityTitle(activity: ObserverActivityMessage): string {
  if (activity.kind !== 'retirement') {
    return isConditionWait(activity) ? 'Waiting on condition' : 'Observer fired'
  }
  switch (activity.retirementReason ?? activity.outcome) {
    case 'expired':
      return 'Observer expired'
    case 'unregistered':
      return 'Observer manually removed'
    case 'invalidated':
      return 'Observer invalidated'
    case 'exhausted':
      return 'Observer exhausted'
    default:
      return 'Observer retired'
  }
}

function activityDescription(activity: ObserverActivityMessage): string {
  if (activity.kind === 'retirement') {
    switch (activity.retirementReason ?? activity.outcome) {
      case 'expired':
        return 'This observer reached its expiration time and stopped listening.'
      case 'unregistered':
        return 'This observer was manually removed and stopped listening.'
      case 'invalidated':
        return 'This observer became invalid and was removed.'
      case 'exhausted':
        return 'This observer exhausted its delivery attempts and was removed.'
      default:
        return 'This observer is inactive.'
    }
  }
  if (activity.outcome === 'delivery_failed') {
    return 'This observer fired, but its delivery failed.'
  }
  if (isConditionWait(activity)) {
    return 'An event arrived, but a condition is still holding the delivery. The observer keeps listening.'
  }
  if (activity.outcome === 'skipped') {
    return 'This observer fired, but delivery was skipped.'
  }
  return 'This observer fired and delivered its flow.'
}

function lifecycleLabel(activity: ObserverActivityMessage): {
  title: string
  detail?: string
} {
  switch (activity.retirementReason) {
    case 'once_consumed':
      return {
        title: 'ONCE · consumed',
        detail: 'This was a once observer and was removed automatically.',
      }
    case 'max_fires':
      return { title: 'Observer consumed', detail: 'Delivery limit reached.' }
    case 'expired':
      return { title: 'Observer expired' }
    case 'unregistered':
      return { title: 'Observer manually removed' }
    case 'invalidated':
      return { title: 'Observer invalidated' }
    case 'exhausted':
      return { title: 'Observer exhausted' }
  }
  if (
    activity.kind === 'fired' &&
    activity.lifecycle.once &&
    activity.lifecycle.state === 'retired' &&
    (!activity.outcome || activity.outcome === 'delivered')
  ) {
    return {
      title: 'ONCE · consumed',
      detail: 'This was a once observer and was removed automatically.',
    }
  }
  if (activity.lifecycle.state === 'active') {
    return { title: 'Observer remains active' }
  }
  return { title: 'Observer retired' }
}

function eventTypeOf(payload: unknown): string | null {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) {
    return null
  }
  const eventType = (payload as Record<string, unknown>).event_type
  return typeof eventType === 'string' && eventType.length > 0
    ? eventType
    : null
}
