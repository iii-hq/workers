import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { SystemMessage, UserMessage } from '@/types/chat'
import { registrationFromCall } from './model'
import { ObserverActivityCard } from './ObserverActivityCard'

function record(
  id: string,
  observer: NonNullable<SystemMessage['observer']>,
): SystemMessage {
  return {
    id,
    role: 'system',
    kind: 'observer-fired',
    content: 'observer activity',
    observer,
    createdAt: observer.fired_at,
  }
}

function notification(id: string, payload = '{"build":42}'): UserMessage {
  return {
    id,
    role: 'user',
    content: `[notification] deploy-ready: ${payload}`,
    notification: true,
    observerId: 'obs_1',
    createdAt: 1,
  }
}

describe('ObserverActivityCard', () => {
  it('renders a delivered once observer as one consumed fire', () => {
    const html = renderToStaticMarkup(
      createElement(ObserverActivityCard, {
        record: record('e_obsfired_obs_1_1', {
          observer_id: 'obs_1',
          function_id: 'state::on-changed',
          filters: {
            'input.scope': { $eq: 'deploys' },
            'input.key': { $eq: 'ready' },
          },
          label: 'deploy-ready',
          target: 'harness::send',
          once: true,
          fires: 1,
          retired: true,
          fired_at: 1,
          outcome: 'delivered',
          retirement_reason: 'once_consumed',
        }),
        notification: notification('e_fire_obs_1_1'),
        defaultOpen: true,
      }),
    )
    expect(html).toContain('Observer fired')
    expect(html).toContain('deploy-ready')
    expect(html).toContain('this chat')
    expect(html).toContain('ONCE · consumed')
    expect(html).toContain('removed automatically')
    expect(html).toContain('&quot;build&quot;')
    expect(html).toContain('>42</span>')
    expect(html).toContain('iii-ui-card')
    expect(html).toContain('iii-ui-collapsible-card')
    expect(html).toContain('iii-ui-collapsible-card__trigger')
    expect(html).toContain('observer-activity-collapsible__trigger')
    expect(html).toContain('iii-ui-collapsible-card__content')
    expect(html).toContain('aria-expanded="true"')
    expect(html).toContain('aria-hidden="false"')
    expect(html).not.toContain('<details')
    expect(html).not.toContain('Open details')
    expect(html).not.toContain('data-observer-activity-status')
    expect(html).toContain('Terminal')
    expect(html).toContain('Raw JSON')
    expect(html).toContain('When')
    expect(html).toContain('Then')
    expect(html).toContain('@container')
    expect(html).toContain('@xl:grid-cols-')
    expect(html).toContain('data-observer-flow-card="when"')
    expect(html).toContain('data-observer-flow-card="then"')
    expect(html).toContain('data-observer-execution-trace="true"')
    expect(html).toContain('iii-ui-card-highlight')
    const flowCardClasses = [
      ...html.matchAll(/class="([^"]+)" data-observer-flow-card/g),
    ].map(([, className]) => className)
    expect(flowCardClasses).toHaveLength(2)
    expect(new Set(flowCardClasses).size).toBe(1)
    expect(flowCardClasses[0]).not.toContain('border')
    expect(flowCardClasses[0]).not.toContain('rounded')
    expect(html).not.toContain('text-accent')
    expect(html).not.toContain('function-call-surface')
    expect(html).not.toContain('Notification called')
  })

  it('keeps manual removal distinct from automatic consumption', () => {
    const html = renderToStaticMarkup(
      createElement(ObserverActivityCard, {
        record: record('e_obsexpired_obs_1', {
          observer_id: 'obs_1',
          function_id: 'cron::on-tick',
          args: { expression: '0 0 9 * * * *' },
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
    expect(html).toContain('Observer manually removed')
    expect(html).not.toContain('Open details')
    expect(html).not.toContain('removed automatically')
    expect(html).not.toContain('ONCE · consumed')
  })

  it('uses neutral labels and delivery copy when a fired call fails', () => {
    const html = renderToStaticMarkup(
      createElement(ObserverActivityCard, {
        record: record('e_obsfired_obs_1_1', {
          observer_id: 'obs_1',
          function_id: 'state::on-changed',
          filters: {
            'input.scope': { $eq: 'deploys' },
            'input.key': { $eq: 'ready' },
          },
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
    expect(html).toContain('This observer fired, but its delivery failed.')
    expect(html).toContain('Attempted call payload')
    expect(html).toContain('Delivery failed')
    expect(html).toContain('class="activity-status-icon" data-status="error"')
    expect(html).toContain('state.created')
    expect(html).toContain('deploy::release')
  })

  it('shows a condition that answered "not yet" as waiting, not as a failure', () => {
    const skip = (gate: string, note: string) =>
      renderToStaticMarkup(
        createElement(ObserverActivityCard, {
          record: record(
            `e_obsskip_obs_1_${gate}_0123456789abcdef0123456789abcdef`,
            {
              observer_id: 'obs_1',
              function_id: 'state::on-changed',
              filters: { 'input.scope': { $eq: 'run-r1-w1' } },
              label: 'run r1 w1',
              target: 'harness::send',
              once: true,
              fires: 0,
              retired: false,
              fired_at: 1,
              outcome: 'skipped',
              note,
            },
          ),
          defaultOpen: true,
        }),
      )

    const waiting = skip(
      'condition',
      '`state::barrier` skipped this fire: 1/3 arrived',
    )
    expect(waiting).toContain(
      'class="activity-status-icon" data-status="pending"',
    )
    expect(waiting).toContain('Waiting on condition')
    expect(waiting).toContain('Waiting')
    expect(waiting).toContain('1/3 arrived')
    expect(waiting).not.toContain('Delivery skipped')
    expect(waiting).not.toContain('delivery was skipped')

    const failed = skip('condition-error', '`state::barrier`: call failed')
    expect(failed).toContain('class="activity-status-icon" data-status="error"')
    expect(failed).toContain('Delivery skipped')
    expect(failed).not.toContain('Waiting on condition')
  })

  it('shows a generic source for any worker-defined function', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      observerId: 'obs_1',
      input: {
        function_id: 'database::on-row-changed',
        args: { db: 'primary', table: 'orders' },
        target: { function_id: 'orders::reindex' },
      },
    })
    const html = renderToStaticMarkup(
      createElement(ObserverActivityCard, {
        record: record('e_obsfired_obs_1_1', {
          observer_id: 'obs_1',
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
    expect(html).toContain('database::on-row-changed')
    expect(html).toContain('db')
    expect(html).toContain('primary')
    expect(html).toContain('orders::reindex')
    expect(html).toContain('Observer remains active')
  })

  it('uses the same Observer fired card for an unpaired notification', () => {
    const html = renderToStaticMarkup(
      createElement(ObserverActivityCard, {
        notification: notification('e_fire_obs_1_0'),
        defaultOpen: true,
      }),
    )
    expect(html).toContain('Observer fired')
    expect(html).toContain('deploy-ready')
    expect(html).not.toContain('Notification called')
  })

  it('optimistically consumes an unpaired delivered once wake', () => {
    const registration = registrationFromCall({
      id: 'call-1',
      observerId: 'obs_1',
      effectiveOnce: true,
      input: {
        function_id: 'state::on-changed',
        filters: { 'input.key': { $eq: 'ready' } },
      },
    })
    const html = renderToStaticMarkup(
      createElement(ObserverActivityCard, {
        notification: notification('e_fire_obs_1_0'),
        registration,
        defaultOpen: true,
      }),
    )
    expect(html).toContain('ONCE · consumed')
    expect(html).toContain('removed automatically')
    expect(html).toContain('fires')
    expect(html).not.toContain('Observer remains active')
  })

  it('keeps a compact fire header while the full card stays mounted and collapsed', () => {
    const html = renderToStaticMarkup(
      createElement(ObserverActivityCard, {
        record: record('e_obsfired_obs_1_1', {
          observer_id: 'obs_1',
          function_id: 'session::on-message-added',
          filters: { 'input.session_id': { $eq: 'explorer' } },
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
    expect(html).toContain('data-timeline-activity-kind="observer-fired"')
    expect(html).toContain('data-icon="observer"')
    expect(html).toContain('lucide-chevron-down')
    expect(html).not.toContain('left-full')
    expect(html).toContain('class="activity-status-icon" data-status="done"')
    expect(html).toContain('data-activity-status-layer="error"')
    expect(html).toContain('data-activity-status-layer="done"')
    expect(html.indexOf('lucide-check')).toBeLessThan(
      html.indexOf('data-timeline-activity-kind="observer-fired"'),
    )
    expect(html).toContain('data-expanded="false"')
    expect(html).toContain('iii-ui-collapsible-card')
    expect(html).toContain('iii-ui-collapsible-card__content')
    expect(html).toContain('aria-hidden="true"')
    expect(html).toContain('Observer fired')
    expect(html).toContain('explorer-messages</span>')
  })

  it('preserves actionable non-fire notification prose', () => {
    const message: UserMessage = {
      id: 'e_condfail_obs_1',
      role: 'user',
      content:
        '[notification] observer obs_1 fired but was NOT delivered: condition failed; remove it and observe again.',
      notification: true,
      observerId: 'obs_1',
      createdAt: 1,
    }
    const html = renderToStaticMarkup(
      createElement(ObserverActivityCard, { notification: message }),
    )
    expect(html).toContain('fired but was NOT delivered')
    expect(html).toContain('remove it and observe again')
    expect(html).not.toContain('Observer fired</span>')
  })
})
