import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import {
  ObserverAddedDisplay,
  ObserveView,
} from '@/components/chat/engine/ObserveView'
import { SpawnActivityCard } from '@/components/chat/harness/SpawnView'
import { RegisteredObserverStatusProvider } from '@/components/chat/RegisteredObserverStatus'
import {
  engineFunctionsListEmpty,
  harnessObserveState,
} from '@/stories/fixtures/engine-fixtures'
import { FunctionCallCard } from './FunctionCallCard'
import { FIRST_PARTY_RENDERERS } from './renderer-registry'

describe('featured first-party renderers', () => {
  it('marks only the spawn card as prominent', () => {
    const spawn = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/harness-spawn',
    )
    const observe = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/observe',
    )
    const harnessFamily = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/harness',
    )
    const engineFamily = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/engine',
    )

    expect(spawn?.metadata).toEqual({ display: true })
    expect(spawn?.tryRenderDisplay).toBeTypeOf('function')
    expect(spawn?.isMatch('harness::spawn')).toBe(true)
    expect(spawn?.isMatch('submit_result')).toBe(false)
    expect(observe?.metadata?.display).not.toBe(true)
    expect(observe?.tryRenderDisplay).toBeTypeOf('function')
    expect(harnessFamily?.metadata?.display).not.toBe(true)
    expect(engineFamily?.metadata?.display).not.toBe(true)
  })

  it('does not let the observe receipt promote other engine calls', () => {
    const observe = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/observe',
    )
    const engineFamily = FIRST_PARTY_RENDERERS.find(
      (renderer) => renderer.id === 'first-party/engine',
    )

    expect(observe?.tryRender(engineFunctionsListEmpty)).toBeNull()
    expect(engineFamily?.tryRender(engineFunctionsListEmpty)).not.toBeNull()

    const collapsed = renderToStaticMarkup(
      <FunctionCallCard message={engineFunctionsListEmpty} />,
    )
    const expanded = renderToStaticMarkup(
      <FunctionCallCard message={engineFunctionsListEmpty} defaultOpen />,
    )
    expect(collapsed).not.toContain('no functions returned')
    expect(expanded).toContain('no functions returned')
  })
})

describe('spawn activity display', () => {
  const now = Date.UTC(2026, 7, 19, 12, 2)

  it.each([
    ['queued', 'Queued'],
    ['waiting', 'Waiting'],
    ['working', 'Working'],
    ['thinking', 'Thinking'],
    ['messaging', 'Sending a message'],
    ['error', 'Needs attention'],
  ] as const)('renders the %s live state', (status, label) => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Review the release"
        task="Audit the release candidate and report blockers."
        status={status}
        activityAt={now - 120_000}
        now={now}
      />,
    )

    expect(html).toContain(`data-subagent-status="${status}"`)
    expect(html).toContain(label)
    expect(html).toContain('Review the release')
    expect(html).toContain('Sub-agent')
    expect(html).toContain(`${label} for 2m`)
  })

  it.each([
    ['ended', 'Ended'],
    ['stopped', 'Stopped'],
  ] as const)('renders the %s terminal state', (status, label) => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Reviewer"
        task="Audit the release candidate."
        status={status}
        activityAt={now - 120_000}
        now={now}
      />,
    )

    expect(html).toContain(label)
    expect(html).toContain(`${label} 2m ago`)
  })

  it('never claims the task completed: an ended turn is neutral, not a success', () => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Reviewer"
        task="Audit the release candidate."
        status="ended"
        activityAt={now - 5_000}
        now={now}
      />,
    )

    expect(html).toContain('Ended just now')
    expect(html).toContain('data-activity-status-tone="neutral"')
    expect(html).not.toContain('Completed')
    expect(html).not.toContain('Done')
    expect(html).not.toContain('lucide-circle-check')
  })

  it('shows one status flag: the badge, with no dot beside the detail line', () => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Reviewer"
        task="Audit the release candidate."
        status="working"
        activityAt={now - 120_000}
        now={now}
      />,
    )

    expect(html.match(/data-badge-variant=/g)).toHaveLength(1)
    expect(html).not.toContain('size-2 shrink-0 rounded-full')
  })

  it('does not present stale activity age as disconnect duration', () => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Reviewer"
        task="Audit the release candidate."
        status="disconnected"
        activityAt={now - 10_800_000}
        now={now}
      />,
    )

    expect(html).toContain('Disconnected')
    expect(html).not.toContain('Disconnected for')
  })

  it('renders the requested metadata and the entire widget as the chat destination', () => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Review the release"
        task="Audit the release candidate."
        status="working"
        sessionId="child-1"
        icon="review"
        color="purple"
        createdAt={now - 120_000}
        activityAt={now - 120_000}
        now={now}
        onOpen={vi.fn()}
      />,
    )

    expect(html.match(/<button/g)).toHaveLength(1)
    expect(html).toContain(
      'aria-label="Open Review the release sub-agent in a new panel"',
    )
    expect(html).toContain('data-color="purple"')
    expect(html).toContain('lucide-clipboard-check')
    expect(html).toContain('Open chat')
    expect(html).not.toContain('Open details')
    expect(html).toContain('2m ago')
    expect(html).toContain('ID: child-1')
    expect(html).not.toContain('Created by you')
    expect(html).not.toContain('aria-label="More')
  })

  it('shows the assigned task once when it is also the session title', () => {
    const task = 'Audit the release candidate.'
    const html = renderToStaticMarkup(
      <SpawnActivityCard title={task} task={task} status="working" />,
    )

    expect(html.match(/Audit the release candidate\./g)).toHaveLength(1)
  })

  it('keeps its mobile controls on one overflow-safe row below the content', () => {
    const html = renderToStaticMarkup(
      <SpawnActivityCard
        title="Review the release"
        task="Audit the release candidate."
        status="working"
        sessionId="child-1"
        onOpen={vi.fn()}
      />,
    )

    expect(html).toContain('@xl:grid-cols-[minmax(0,1fr)_auto]')
    expect(html).toContain('grid-cols-[minmax(0,1fr)_auto]')
    expect(html).toContain('min-w-0')
  })
})

describe('observe receipt display', () => {
  const now = Date.UTC(2026, 7, 20, 12, 5)

  it('makes the label, active state, creation time, and observer id explicit', () => {
    const html = renderToStaticMarkup(
      <ObserverAddedDisplay
        input={{
          function_id: 'state::on-changed',
          label: 'research-progress-watch',
          lifecycle: { once: false },
        }}
        output={{ observer_id: 'obs-1', once: false }}
        createdAt={now - 300_000}
        now={now}
      />,
    )

    expect(html).toContain('Observer added')
    expect(html).toContain('research-progress-watch')
    expect(html).toContain('state::on-changed · persistent')
    expect(html).toContain('data-observer-registration-state="active"')
    expect(html).toContain('data-activity-status-tone="positive"')
    expect(html).toContain('Active for 5m')
    expect(html).toContain('5m ago')
    expect(html).toContain('ID: obs-1')
    expect(html).toContain('Open details')
    expect(html).toContain('size-6')
    expect(html).toContain('bg-ok-muted')
    expect(html).toContain('stroke-ok')
    expect(html).toContain('animate-pulse')
  })

  it('derives every receipt state from one shared observer snapshot', () => {
    const html = renderToStaticMarkup(
      <RegisteredObserverStatusProvider
        loaded
        observersById={
          new Map([
            [
              'obs-active',
              {
                id: 'obs-active',
                functionId: 'state::on-changed',
                delivery: { kind: 'notify' as const },
                fired: false,
              },
            ],
            [
              'obs-fired',
              {
                id: 'obs-fired',
                functionId: 'state::on-changed',
                delivery: { kind: 'notify' as const },
                fired: true,
              },
            ],
          ])
        }
      >
        <ObserverAddedDisplay
          input={{ function_id: 'state::on-changed', label: 'active observer' }}
          output={{ observer_id: 'obs-active' }}
        />
        <ObserverAddedDisplay
          input={{ function_id: 'state::on-changed', label: 'fired observer' }}
          output={{ observer_id: 'obs-fired' }}
        />
        <ObserverAddedDisplay
          input={{
            function_id: 'state::on-changed',
            label: 'removed observer',
          }}
          output={{ observer_id: 'obs-removed' }}
        />
      </RegisteredObserverStatusProvider>,
    )

    expect(
      html.match(/data-observer-registration-state="active"/g),
    ).toHaveLength(1)
    expect(
      html.match(/data-observer-registration-state="inactive"/g),
    ).toHaveLength(2)
  })

  it('keeps a receipt active until the shared snapshot is loaded', () => {
    const html = renderToStaticMarkup(
      <RegisteredObserverStatusProvider
        loaded={false}
        observersById={new Map()}
      >
        <ObserverAddedDisplay
          input={{
            function_id: 'state::on-changed',
            label: 'loading observer',
          }}
          output={{ observer_id: 'obs-loading' }}
        />
      </RegisteredObserverStatusProvider>,
    )

    expect(html).toContain('data-observer-registration-state="active"')
  })

  it('keeps action hidden until the observer fires', () => {
    const html = renderToStaticMarkup(
      <ObserverAddedDisplay
        input={{
          function_id: 'session::on-message-added',
          label: 'explorer-messages',
          action: 'new Explorer message received',
        }}
        output={{ observer_id: 'obs-1', once: false }}
      />,
    )

    expect(html).toContain('data-observer-registration-label=""')
    expect(html).toContain('explorer-messages')
    expect(html).not.toContain('new Explorer message received')
  })

  it('omits action from readable observe details while showing filters', () => {
    const html = renderToStaticMarkup(
      <ObserveView
        messageId="observe-1"
        input={{
          function_id: 'session::on-message-added',
          label: 'explorer-messages',
          action: 'new Explorer message received',
          filters: { 'input.session_id': { $eq: 'explorer' } },
        }}
        output={{ observer_id: 'obs-1', once: false }}
      />,
    )

    expect(html).not.toContain('new Explorer message received')
    expect(html).toContain('input.session_id')
    expect(html).toContain('explorer')
  })

  it('places status above open details only at the desktop container breakpoint', () => {
    const html = renderToStaticMarkup(
      <ObserverAddedDisplay
        input={{ function_id: 'cron::on-tick', label: 'nightly cleanup' }}
        output={{ observer_id: 'obs-1' }}
      />,
    )

    expect(html).toContain('grid-cols-[minmax(0,1fr)_auto]')
    expect(html).toContain('@xl:flex')
    expect(html).toContain('@xl:flex-col')
    expect(html.indexOf('role="status"')).toBeLessThan(
      html.indexOf('Open details'),
    )
  })

  it('does not claim an unsuccessful observe', () => {
    expect(
      renderToStaticMarkup(
        <ObserverAddedDisplay
          input={{ function_id: 'state::on-changed', label: 'watch' }}
          output={{}}
        />,
      ),
    ).toBe('')
  })

  it('keeps observe details behind the normal function-call disclosure', () => {
    const collapsed = renderToStaticMarkup(
      <FunctionCallCard message={harnessObserveState} />,
    )
    const expanded = renderToStaticMarkup(
      <FunctionCallCard message={harnessObserveState} defaultOpen />,
    )

    expect(collapsed).not.toContain('Observer added')
    expect(collapsed).not.toContain('data-observer-registration-details')
    expect(collapsed).not.toContain('data-function-display-slot=""')
    expect(collapsed).toContain(
      'data-timeline-activity-kind="observer-registration"',
    )
    expect(collapsed).toContain('aria-expanded="false"')

    expect(expanded).not.toContain('Observer added')
    expect(expanded).toContain('data-observer-registration-details')
    expect(expanded).toContain('data-observer-execution-trace')
    expect(expanded).toContain('data-observer-flow-card="when"')
    expect(expanded).toContain('data-observer-flow-card="then"')
    expect(expanded).toContain('Show raw')
    expect(expanded).toContain('chat-tool-row__body')
    expect(expanded).toContain('data-timeline-activity-kind="function"')
    expect(expanded).not.toContain(
      'data-timeline-activity-kind="observer-registration"',
    )
    expect(expanded).toContain('aria-expanded="true"')
  })
})
