import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { SystemMessage, UserMessage } from '@/types/chat'
import { registrationFromCall } from './model'
import { TriggerActivityCard } from './TriggerActivityCard'

function record(
  id: string,
  trigger: NonNullable<SystemMessage['trigger']>,
): SystemMessage {
  return {
    id,
    role: 'system',
    kind: 'trigger-fired',
    content: 'trigger activity',
    trigger,
    createdAt: trigger.fired_at,
  }
}

function notification(id: string, payload = '{"build":42}'): UserMessage {
  return {
    id,
    role: 'user',
    content: `[notification] deploy-ready: ${payload}`,
    notification: true,
    triggerBindingId: 'sub_1',
    createdAt: 1,
  }
}

describe('TriggerActivityCard', () => {
  it('renders a delivered once binding as one consumed fire', () => {
    const html = renderToStaticMarkup(
      createElement(TriggerActivityCard, {
        record: record('e_trigfired_sub_1_1', {
          subscription_id: 'sub_1',
          trigger_type: 'state',
          config: { scope: 'deploys', key: 'ready' },
          label: 'deploy-ready',
          target: 'harness::send',
          once: true,
          fires: 1,
          retired: true,
          fired_at: 1,
          outcome: 'delivered',
          retirement_reason: 'once_consumed',
        }),
        notification: notification('e_fire_sub_1_1'),
        defaultOpen: true,
      }),
    )
    expect(html).toContain('Trigger fired')
    expect(html).toContain('deploy-ready')
    expect(html).toContain('this chat')
    expect(html).toContain('ONCE · consumed')
    expect(html).toContain('automatically unbound')
    expect(html).toContain('&quot;build&quot;')
    expect(html).toContain('>42</span>')
    expect(html).toContain('iii-ui-card')
    expect(html).toContain('iii-ui-collapsible-card')
    expect(html).toContain('iii-ui-collapsible-card__trigger')
    expect(html).toContain('trigger-activity-collapsible__trigger')
    expect(html).toContain('iii-ui-collapsible-card__content')
    expect(html).toContain('aria-expanded="true"')
    expect(html).toContain('aria-hidden="false"')
    expect(html).not.toContain('<details')
    expect(html).not.toContain('Open details')
    expect(html).not.toContain('data-trigger-activity-status')
    expect(html).toContain('Terminal')
    expect(html).toContain('Raw JSON')
    expect(html).toContain('When')
    expect(html).toContain('Then')
    expect(html).toContain('@container')
    expect(html).toContain('@xl:grid-cols-')
    expect(html).toContain('data-trigger-flow-card="when"')
    expect(html).toContain('data-trigger-flow-card="then"')
    expect(html).toContain('data-trigger-execution-trace="true"')
    expect(html).toContain('iii-ui-card-highlight')
    const flowCardClasses = [
      ...html.matchAll(/class="([^"]+)" data-trigger-flow-card/g),
    ].map(([, className]) => className)
    expect(flowCardClasses).toHaveLength(2)
    expect(new Set(flowCardClasses).size).toBe(1)
    expect(flowCardClasses[0]).not.toContain('border')
    expect(flowCardClasses[0]).not.toContain('rounded')
    expect(html).not.toContain('text-accent')
    expect(html).not.toContain('function-trigger-surface')
    expect(html).not.toContain('Notification triggered')
  })

  it('keeps manual removal distinct from automatic consumption', () => {
    const html = renderToStaticMarkup(
      createElement(TriggerActivityCard, {
        record: record('e_trigexpired_sub_1', {
          subscription_id: 'sub_1',
          trigger_type: 'cron',
          config: { expression: '0 0 9 * * *' },
          target: 'harness::send',
          once: true,
          retired: true,
          fired_at: 1,
          outcome: 'unregistered',
          retirement_reason: 'unregistered',
        }),
        defaultOpen: true,
      }),
    )
    expect(html).toContain('Binding manually removed')
    expect(html).not.toContain('Open details')
    expect(html).not.toContain('automatically unbound')
    expect(html).not.toContain('ONCE · consumed')
  })

  it('uses neutral labels and delivery copy when a fired call fails', () => {
    const html = renderToStaticMarkup(
      createElement(TriggerActivityCard, {
        record: record('e_trigfired_sub_1_1', {
          subscription_id: 'sub_1',
          trigger_type: 'state',
          config: { scope: 'deploys', key: 'ready' },
          label: 'deploy-ready',
          target: 'deploy::release',
          once: false,
          fires: 1,
          retired: false,
          fired_at: 1,
          outcome: 'delivery_failed',
          payload: { event_type: 'state.created', build: 42 },
        }),
        defaultOpen: true,
      }),
    )

    expect(html).not.toContain('data-badge-variant="alert"')
    expect(html).toContain('data-badge-variant="default"')
    expect(html).toContain('This trigger fired, but its delivery failed.')
    expect(html).toContain('Attempted call payload')
    expect(html).toContain('Delivery failed')
    expect(html).toContain('class="activity-status-icon" data-status="error"')
    expect(html).toContain('state.created')
    expect(html).toContain('deploy::release')
  })

  it('shows a generic source for any worker-defined trigger type', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      subscriptionId: 'sub_1',
      input: {
        trigger_type: 'database::row-changed',
        config: { database: 'primary', table: 'orders' },
        function_id: 'orders::reindex',
      },
    })
    const html = renderToStaticMarkup(
      createElement(TriggerActivityCard, {
        record: record('e_trigfired_sub_1_1', {
          subscription_id: 'sub_1',
          target: 'orders::reindex',
          once: false,
          fires: 1,
          retired: false,
          fired_at: 1,
        }),
        registration,
        defaultOpen: true,
      }),
    )
    expect(html).toContain('database::row-changed')
    expect(html).toContain('database')
    expect(html).toContain('primary')
    expect(html).toContain('orders::reindex')
    expect(html).toContain('Binding remains active')
  })

  it('uses the same Trigger fired card for an unpaired notification', () => {
    const html = renderToStaticMarkup(
      createElement(TriggerActivityCard, {
        notification: notification('e_fire_sub_1_0'),
        defaultOpen: true,
      }),
    )
    expect(html).toContain('Trigger fired')
    expect(html).toContain('deploy-ready')
    expect(html).not.toContain('Notification triggered')
  })

  it('optimistically consumes an unpaired delivered once wake', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      subscriptionId: 'sub_1',
      effectiveOnce: true,
      input: { trigger_type: 'state', config: { key: 'ready' } },
    })
    const html = renderToStaticMarkup(
      createElement(TriggerActivityCard, {
        notification: notification('e_fire_sub_1_0'),
        registration,
        defaultOpen: true,
      }),
    )
    expect(html).toContain('ONCE · consumed')
    expect(html).toContain('automatically unbound')
    expect(html).toContain('fires')
    expect(html).not.toContain('Binding remains active')
  })

  it('keeps a compact fire header while the full card stays mounted and collapsed', () => {
    const html = renderToStaticMarkup(
      createElement(TriggerActivityCard, {
        record: record('e_trigfired_sub_1_1', {
          subscription_id: 'sub_1',
          trigger_type: 'on-message',
          config: { scope: 'explorer' },
          label: 'explorer-messages',
          action: 'new Explorer message received',
          target: 'harness::send',
          once: false,
          fires: 1,
          retired: false,
          fired_at: 1,
          outcome: 'delivered',
        }),
      }),
    )
    expect(html).toContain('new Explorer message received')
    expect(html).toContain('lucide-check')
    expect(html).toContain('data-timeline-activity-kind="trigger"')
    expect(html).toContain('data-icon="trigger"')
    expect(html).toContain('lucide-chevron-right')
    expect(html).not.toContain('left-full')
    expect(html).toContain('class="activity-status-icon" data-status="done"')
    expect(html).toContain('data-activity-status-layer="error"')
    expect(html).toContain('data-activity-status-layer="done"')
    expect(html.indexOf('lucide-check')).toBeLessThan(
      html.indexOf('data-timeline-activity-kind="trigger"'),
    )
    expect(html).toContain('data-expanded="false"')
    expect(html).toContain('iii-ui-collapsible-card')
    expect(html).toContain('iii-ui-collapsible-card__content')
    expect(html).toContain('aria-hidden="true"')
    expect(html).toContain('Trigger fired')
    expect(html).toContain('explorer-messages</span>')
  })

  it('preserves actionable non-fire notification prose', () => {
    const message: UserMessage = {
      id: 'e_condfail_sub_1',
      role: 'user',
      content:
        '[notification] binding sub_1 fired but was NOT delivered: condition failed; unregister and re-register it.',
      notification: true,
      triggerBindingId: 'sub_1',
      createdAt: 1,
    }
    const html = renderToStaticMarkup(
      createElement(TriggerActivityCard, { notification: message }),
    )
    expect(html).toContain('fired but was NOT delivered')
    expect(html).toContain('unregister and re-register')
    expect(html).not.toContain('Trigger fired</span>')
  })
})
