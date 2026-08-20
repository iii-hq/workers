import {
  Bell,
  CircleAlert,
  FunctionSquare,
  Info,
  RadioTower,
  Zap,
} from 'lucide-react'
import { useState } from 'react'
import { FilterChip } from '@/components/chat/engine/shared'
import { Badge } from '@/components/ui/Badge'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from '@/components/ui/CollapsibleCard'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { timestampMilliseconds } from '@/lib/relative-time'
import { cn } from '@/lib/utils'
import type { SystemMessage, UserMessage } from '@/types/chat'
import type { TriggerActivityMessage } from '@/types/injectable-ui'
import {
  activityFromTriggerRecord,
  parseNotification,
  type TriggerRegistration,
} from './model'
import {
  TriggerJsonPane,
  TriggerStats,
  TriggerTrace,
  TriggerTraceNode,
} from './TriggerDetails'
import { TriggerSource } from './TriggerSource'

interface TriggerActivityCardProps {
  record?: SystemMessage
  notification?: UserMessage
  registration?: TriggerRegistration
}

/** One host-owned card for registration delivery, fire, and retirement state. */
export function TriggerActivityCard({
  record,
  notification,
  registration,
}: TriggerActivityCardProps) {
  const parsed = notification ? parseNotification(notification.content) : null
  const fromRecord = record
    ? activityFromTriggerRecord(record, registration)
    : null
  const base =
    fromRecord ?? activityFromNotification(notification, registration)
  const activity =
    base && base.payload === undefined && parsed
      ? { ...base, payload: parsed.payload }
      : base
  const [tab, setTab] = useState<'terminal' | 'json'>('terminal')

  if (!activity) {
    return (
      <article
        className="flex items-start gap-3 rounded-md border border-edge bg-panel-raised p-4 shadow-raised sm:p-3"
        data-message-role="trigger-activity"
      >
        <div className="flex size-10 shrink-0 items-center justify-center rounded-md bg-warn-muted sm:size-9">
          <Zap
            aria-hidden
            className="size-5 shrink-0 stroke-warn sm:size-4"
            strokeWidth={2.25}
          />
        </div>
        <div className="min-w-0 flex-1 font-sans text-base wrap-break-word text-ink sm:text-sm">
          {notification?.content ?? record?.content ?? 'Trigger activity'}
        </div>
      </article>
    )
  }

  const title = activityTitle(activity)
  const source = activity.label ?? activity.triggerType
  const target =
    activity.delivery.kind === 'call'
      ? activity.delivery.functionId
      : 'this chat'

  return (
    <article
      className="w-full"
      data-message-role="trigger-activity"
      data-trigger-activity-kind={activity.kind}
    >
      <CollapsibleCard className="@container">
        <CollapsibleCardTrigger className="p-4 select-none sm:p-3">
          <div className="flex min-w-0 items-start gap-3">
            <div
              className={cn(
                'flex size-12 shrink-0 items-center justify-center rounded-md sm:size-10',
                activity.kind === 'retirement' ? 'bg-surface' : 'bg-warn-muted',
              )}
            >
              <Zap
                aria-hidden
                className={cn(
                  'size-6 shrink-0 sm:size-5',
                  activity.kind === 'retirement'
                    ? 'stroke-ink-ghost'
                    : 'stroke-warn',
                )}
                strokeWidth={2.25}
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
                {activityDescription(activity)}
              </p>
            </div>
          </div>
        </CollapsibleCardTrigger>

        <CollapsibleCardContent>
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
              <TriggerActivityTerminal activity={activity} />
            </TabsContent>
            <TabsContent value="json" className="p-4 sm:p-3">
              <div className="flex min-w-0 flex-col gap-3">
                {registration ? (
                  <TriggerJsonPane
                    label="Registration"
                    value={registration.raw}
                  />
                ) : null}
                {notification ? (
                  <TriggerJsonPane
                    label="Notification"
                    value={parsed?.payload ?? notification.content}
                  />
                ) : null}
                {record?.trigger ? (
                  <TriggerJsonPane label="Fire" value={record.trigger} />
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
  registration?: TriggerRegistration,
): TriggerActivityMessage | null {
  if (!notification) return null
  const parsed = parseNotification(notification.content)
  // Harness also uses the trusted notification origin for actionable delivery
  // failures. They are not successful fires and their prose must remain fully
  // visible instead of being forced into trigger-fired chrome.
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
    triggerType: inherited?.triggerType ?? 'trigger',
    ...(inherited?.config !== undefined ? { config: inherited.config } : {}),
    ...((parsed?.name ?? inherited?.label)
      ? { label: parsed?.name ?? inherited?.label }
      : {}),
    ...(inherited?.conditions ? { conditions: inherited.conditions } : {}),
    delivery: { kind: 'notify' },
    lifecycle: {
      ...inheritedLifecycle,
      state: inheritedLifecycle.once ? 'retired' : inheritedLifecycle.state,
      fires: Math.max(inheritedLifecycle.fires, 1),
    },
    ...(notification.triggerBindingId
      ? { subscriptionId: notification.triggerBindingId }
      : {}),
    ...(parsed ? { payload: parsed.payload } : {}),
    firedAt: notification.createdAt,
    outcome: 'delivered',
  }
}

function TriggerActivityTerminal({
  activity,
}: {
  activity: TriggerActivityMessage
}) {
  const lifecycle = lifecycleLabel(activity)
  const eventType = eventTypeOf(activity.payload)
  const showDeliveryBadge =
    activity.outcome === 'delivery_failed' || activity.outcome === 'skipped'
  return (
    <div className="flex min-w-0 flex-col gap-4">
      <TriggerTrace
        when={
          <TriggerTraceNode
            kind="when"
            icon={<RadioTower aria-hidden />}
            label="When"
            title={activity.triggerType}
          >
            <TriggerSource activity={activity} presentation="compact" />
          </TriggerTraceNode>
        }
        then={<TriggerTarget activity={activity} eventType={eventType} />}
      />

      <ActivityStats activity={activity} />

      {activity.payload !== undefined ? (
        <TriggerJsonPane
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
        <div
          className={cn(
            'flex size-9 shrink-0 items-center justify-center rounded-full',
            showDeliveryBadge ? 'bg-warn-muted' : 'bg-accent-muted',
          )}
        >
          {showDeliveryBadge ? (
            <CircleAlert aria-hidden className="size-5 shrink-0 stroke-warn" />
          ) : (
            <Info aria-hidden className="size-5 shrink-0 stroke-accent" />
          )}
        </div>
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

function ActivityStats({ activity }: { activity: TriggerActivityMessage }) {
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
      <TriggerStats
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
          ) : activity.outcome === 'skipped' ? (
            <Badge>Delivery skipped</Badge>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function TriggerTarget({
  activity,
  eventType,
}: {
  activity: TriggerActivityMessage
  eventType: string | null
}) {
  const target =
    activity.delivery.kind === 'call'
      ? activity.delivery.functionId
      : 'this session'
  const call = activity.delivery.kind === 'call'
  return (
    <TriggerTraceNode
      kind="then"
      icon={call ? <FunctionSquare aria-hidden /> : <Bell aria-hidden />}
      label="Then"
      title={call ? 'Call' : 'Notify'}
    >
      <div className="flex min-w-0 flex-col gap-2">
        <div
          className={cn(
            'min-w-0 font-sans text-base break-all sm:text-sm',
            call ? 'text-ink' : 'text-ink-faint italic',
          )}
        >
          {target}
        </div>
        {eventType ? <FilterChip label="event" value={eventType} /> : null}
      </div>
    </TriggerTraceNode>
  )
}

function activityTitle(activity: TriggerActivityMessage): string {
  if (activity.kind !== 'retirement') return 'Trigger fired'
  switch (activity.retirementReason ?? activity.outcome) {
    case 'expired':
      return 'Binding expired'
    case 'unregistered':
      return 'Binding manually removed'
    case 'invalidated':
      return 'Binding invalidated'
    case 'exhausted':
      return 'Binding exhausted'
    default:
      return 'Binding retired'
  }
}

function activityDescription(activity: TriggerActivityMessage): string {
  if (activity.kind === 'retirement') {
    switch (activity.retirementReason ?? activity.outcome) {
      case 'expired':
        return 'This binding reached its expiration time and is no longer listening.'
      case 'unregistered':
        return 'This binding was manually removed and is no longer listening.'
      case 'invalidated':
        return 'This binding became invalid and was removed.'
      case 'exhausted':
        return 'This binding exhausted its delivery attempts and was removed.'
      default:
        return 'This binding is no longer active.'
    }
  }
  if (activity.outcome === 'delivery_failed') {
    return 'This trigger fired, but its delivery failed.'
  }
  if (activity.outcome === 'skipped') {
    return 'This trigger fired, but delivery was skipped.'
  }
  return 'This trigger fired and delivered its flow.'
}

function lifecycleLabel(activity: TriggerActivityMessage): {
  title: string
  detail?: string
} {
  switch (activity.retirementReason) {
    case 'once_consumed':
      return {
        title: 'ONCE · consumed',
        detail: 'This was a once trigger and was automatically unbound.',
      }
    case 'max_fires':
      return { title: 'Binding consumed', detail: 'Delivery limit reached.' }
    case 'expired':
      return { title: 'Binding expired' }
    case 'unregistered':
      return { title: 'Binding manually removed' }
    case 'invalidated':
      return { title: 'Binding invalidated' }
    case 'exhausted':
      return { title: 'Binding exhausted' }
  }
  if (
    activity.kind === 'fired' &&
    activity.lifecycle.once &&
    activity.lifecycle.state === 'retired' &&
    (!activity.outcome || activity.outcome === 'delivered')
  ) {
    return {
      title: 'ONCE · consumed',
      detail: 'This was a once trigger and was automatically unbound.',
    }
  }
  if (activity.lifecycle.state === 'active') {
    return { title: 'Binding remains active' }
  }
  return { title: 'Binding retired' }
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
