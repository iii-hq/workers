import { describe, expect, it } from 'vitest'
import {
  functionOfStoredType,
  observerFiredOf,
  sourceOfStored,
  takesArgs,
} from './stored-records'

describe('sourceOfStored', () => {
  it('reads the console asset types as console::ui::asset with kind and path', () => {
    for (const kind of ['script', 'style', 'module']) {
      expect(
        sourceOfStored(`console:${kind}`, { path: `state/page.${kind}` }),
      ).toEqual({
        function_id: 'console::ui::asset',
        args: { kind, path: `state/page.${kind}` },
      })
    }
    expect(sourceOfStored('console:assets', {})).toEqual({
      function_id: 'console::ui::on-assets-changed',
    })
  })

  it('reads the call hooks as on-pre-call and on-post-call', () => {
    expect(functionOfStoredType('harness::hook::pre-trigger')).toBe(
      'harness::hook::on-pre-call',
    )
    expect(functionOfStoredType('harness::hook::post-trigger')).toBe(
      'harness::hook::on-post-call',
    )
    expect(
      sourceOfStored('harness::hook::pre-trigger', {
        functions: ['shell::exec'],
        timeout_ms: 2000,
        on_error: 'allow',
      }),
    ).toEqual({
      function_id: 'harness::hook::on-pre-call',
      args: { functions: ['shell::exec'], timeout_ms: 2000, on_error: 'allow' },
    })
  })

  it('keeps a hook config as the observer args', () => {
    expect(
      sourceOfStored('harness::hook::post-turn', {
        sessions: ['session-1'],
        priority: 10,
        timeout_ms: 5000,
        metadata: { action: 'check the answer' },
      }),
    ).toEqual({
      function_id: 'harness::hook::on-post-turn',
      args: { sessions: ['session-1'], priority: 10, timeout_ms: 5000 },
    })
  })

  it('maps a configuration watch to input.id', () => {
    expect(
      sourceOfStored('configuration', {
        configuration_id: 'llm-router',
        event_types: ['configuration:updated'],
      }),
    ).toEqual({
      function_id: 'configuration::on-updated',
      filters: { 'input.id': { $eq: 'llm-router' } },
    })
  })

  it('maps an error log watch to severity_number 17 and up', () => {
    expect(sourceOfStored('log', { level: 'error' })).toEqual({
      function_id: 'observability::on-log',
      filters: { 'input.severity_number': { $gte: 17 } },
    })
    expect(sourceOfStored('log', { level: 'info' })).toEqual({
      function_id: 'observability::on-log',
    })
  })

  it('maps the engine and trace types without filters', () => {
    expect(sourceOfStored('trace', { status: 'error' })).toEqual({
      function_id: 'observability::on-span',
    })
    expect(sourceOfStored('engine::functions-available', {})).toEqual({
      function_id: 'engine::on-functions-changed',
    })
    expect(sourceOfStored('engine::workers-available', {})).toEqual({
      function_id: 'engine::on-workers-changed',
    })
  })

  it('maps a compose operation watch to its operation and terminal flag', () => {
    expect(
      sourceOfStored('compose-operation', {
        operation_id: 'op-1',
        terminal_only: true,
      }),
    ).toEqual({
      function_id: 'compose::on-operation-progress',
      filters: {
        'input.operation_id': { $eq: 'op-1' },
        'input.terminal': { $eq: true },
      },
    })
  })

  it('keeps only the fields the cron, timer and stream sources read', () => {
    expect(
      sourceOfStored('cron', { expression: '0 * * * * * *', metadata: {} }),
    ).toEqual({
      function_id: 'cron::on-tick',
      args: { expression: '0 * * * * * *' },
    })
    expect(sourceOfStored('timer', { at: 1700000000000 })).toEqual({
      function_id: 'harness::on-timer',
      args: { at: 1700000000000 },
    })
    expect(
      sourceOfStored('stream', {
        stream_name: 'chat',
        group_id: 'room-1',
        id: 'x',
      }),
    ).toEqual({
      function_id: 'stream::on-changed',
      filters: {
        'input.stream_name': { $eq: 'chat' },
        'input.group_id': { $eq: 'room-1' },
      },
    })
  })

  it('reads other worker types by the naming rule, with args or filters', () => {
    expect(sourceOfStored('claude::agent-event', { session_id: 's' })).toEqual({
      function_id: 'claude::on-agent-event',
      args: { session_id: 's' },
    })
    expect(sourceOfStored('kanban:change', { board: 'b' })).toEqual({
      function_id: 'kanban::on-change',
      args: { board: 'b' },
    })
    expect(
      sourceOfStored('browser::navigated', { tab_id: 't', nested: { a: 1 } }),
    ).toEqual({
      function_id: 'browser::on-navigated',
      filters: { 'input.tab_id': { $eq: 't' } },
    })
  })
})

describe('takesArgs', () => {
  it('lists every function of the contract that takes args', () => {
    const withArgs = [
      'cron::on-tick',
      'http::on-request',
      'queue::on-message',
      'database::on-row-changed',
      'harness::hook::on-pre-turn',
      'harness::hook::on-pre-generate',
      'harness::hook::on-post-generate',
      'harness::hook::on-pre-call',
      'harness::hook::on-post-call',
      'harness::hook::on-post-turn',
      'harness::on-timer',
      'console::ui::asset',
      'shell::on-changed',
      'shell::on-git-changed',
      'shell::on-job-finished',
      'github::pr::on-event',
      'storage::on-object-created',
      'storage::on-object-deleted',
      'kanban::on-change',
      'kanban::on-comment',
      'stories::on-changed',
      'computer::on-frame-changed',
      'browser::on-frame-event',
      'browser::on-crawl-item',
      'ios-simulator::on-frame-event',
      'demo::on-counter-changed',
      ...[
        'claude',
        'codex',
        'cursor',
        'devin',
        'grok',
        'hermes',
        'opencode',
        'pi',
      ].flatMap((agent) => [
        `${agent}::on-agent-event`,
        `${agent}::on-raw-event`,
      ]),
    ]
    for (const functionId of withArgs) expect(takesArgs(functionId)).toBe(true)
    for (const functionId of [
      'state::on-changed',
      'configuration::on-updated',
      'harness::on-turn-completed',
      'session::on-created',
    ]) {
      expect(takesArgs(functionId)).toBe(false)
    }
  })
})

describe('observerFiredOf', () => {
  it('reads a stored post-turn validator record with its args', () => {
    expect(
      observerFiredOf({
        subscription_id: 'sub-1',
        trigger_id: 'trig-1',
        trigger_type: 'harness::hook::post-turn',
        config: { sessions: ['session-1'], priority: 10 },
        target: 'fp::pipe',
        once: false,
        retired: false,
        fired_at: 3,
      }),
    ).toEqual({
      observer_id: 'sub-1',
      function_id: 'harness::hook::on-post-turn',
      args: { sessions: ['session-1'], priority: 10 },
      target: 'fp::pipe',
      once: false,
      retired: false,
      fired_at: 3,
    })
  })

  it('reads a stored console asset record as console::ui::asset', () => {
    expect(
      observerFiredOf({
        subscription_id: 'sub-2',
        trigger_type: 'console:module',
        config: { path: 'ide/xterm.js' },
        target: 'ide::ui::content',
        once: false,
        retired: true,
        fired_at: 4,
      }),
    ).toMatchObject({
      observer_id: 'sub-2',
      function_id: 'console::ui::asset',
      args: { kind: 'module', path: 'ide/xterm.js' },
    })
  })
})
