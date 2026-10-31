import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ObserveView } from '../ObserveView'
import {
  coerceJsonObject,
  disposeObserverResponseSchema,
  ENGINE_FUNCTION_IDS,
  functionDetailSchema,
  functionInfoRequestSchema,
  functionsListRequestSchema,
  functionsListResponseSchema,
  isEngineListFunction,
  observeRequestSchema,
  observeResponseSchema,
  observerIdRequestSchema,
  observersListRequestSchema,
  observersListResponseSchema,
  parseFunctionInfoResponse,
  parseObserverInfoResponse,
  safeParseRequest,
  safeParseResponse,
  unobserveRequestSchema,
  unobserveResponseSchema,
  unwrapEnvelope,
  workerInfoRequestSchema,
  workerInfoResponseSchema,
  workersListRequestSchema,
  workersListResponseSchema,
  workersRegisterRequestSchema,
  workersRegisterResponseSchema,
} from '../parsers'
import { UnobserveView } from '../UnobserveView'

function wrap<T>(details: T) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: false,
  }
}

describe('isEngineListFunction', () => {
  it('matches every id in the explicit allowlist', () => {
    for (const id of ENGINE_FUNCTION_IDS) {
      expect(isEngineListFunction(id)).toBe(true)
    }
  })

  it('rejects unrelated ids', () => {
    expect(isEngineListFunction('engine::functions')).toBe(false)
    expect(isEngineListFunction('sandbox::exec')).toBe(false)
    expect(isEngineListFunction('engine::observers::create')).toBe(false)
  })
})

describe('coerceJsonObject', () => {
  it('recovers a double-encoded (stringified) payload', () => {
    const payload = {
      function_id: 'state::on-changed',
      filters: { 'input.scope': { $eq: 'wiki' } },
    }
    expect(coerceJsonObject(JSON.stringify(payload))).toEqual(payload)
    expect(coerceJsonObject('  [1, 2]  ')).toEqual([1, 2])
  })

  it('passes non-JSON-object values through untouched', () => {
    const obj = { already: 'parsed' }
    expect(coerceJsonObject(obj)).toBe(obj)
    expect(coerceJsonObject('a plain string request')).toBe(
      'a plain string request',
    )
    expect(coerceJsonObject('{ broken json')).toBe('{ broken json')
    expect(coerceJsonObject(undefined)).toBeUndefined()
  })
})

describe('engine::functions::list', () => {
  it('parses an empty request payload', () => {
    const req = safeParseRequest(functionsListRequestSchema, {})
    expect(req).toEqual({})
  })

  it('parses a filter-applied request', () => {
    const req = safeParseRequest(functionsListRequestSchema, {
      prefix: 'todo::',
      include_internal: true,
    })
    expect(req).toEqual({ prefix: 'todo::', include_internal: true })
  })

  it('parses a raw response payload', () => {
    const resp = safeParseResponse(functionsListResponseSchema, {
      functions: [
        {
          function_id: 'todo::list',
          worker_name: 'todo-app',
          description: 'List all todos',
        },
      ],
    })
    expect(resp?.functions).toHaveLength(1)
    expect(resp?.functions[0].function_id).toBe('todo::list')
  })

  it('parses a harness-wrapped response payload', () => {
    const payload = {
      functions: [
        { function_id: 'todo::create', worker_name: 'todo-app' },
        {
          function_id: 'todo::delete',
          worker_name: 'todo-app',
          description: null,
        },
      ],
    }
    const resp = safeParseResponse(functionsListResponseSchema, wrap(payload))
    expect(resp?.functions).toHaveLength(2)
  })

  it('parses an empty list', () => {
    expect(
      safeParseResponse(functionsListResponseSchema, wrap({ functions: [] })),
    ).toEqual({ functions: [] })
  })
})

describe('engine::functions::info', () => {
  it('parses the request payload', () => {
    expect(
      safeParseRequest(functionInfoRequestSchema, {
        function_id: 'sandbox::fs::write',
      }),
    ).toEqual({ function_id: 'sandbox::fs::write' })
  })

  it('parses a batch request (function_ids)', () => {
    expect(
      safeParseRequest(functionInfoRequestSchema, {
        function_ids: ['state::get', 'state::set'],
      }),
    ).toEqual({ function_ids: ['state::get', 'state::set'] })
  })

  it('normalizes single and batch responses to a detail list', () => {
    const detail = {
      function_id: 'state::get',
      worker_name: 'iii-state',
      observer_count: 0,
    }
    expect(
      parseFunctionInfoResponse(detail)?.map((d) => d.function_id),
    ).toEqual(['state::get'])
    expect(
      parseFunctionInfoResponse(
        wrap({ functions: [detail, { ...detail, function_id: 'state::set' }] }),
      )?.map((d) => d.function_id),
    ).toEqual(['state::get', 'state::set'])
    // Neither shape → null, so the card falls back to the generic panes.
    expect(parseFunctionInfoResponse({ nonsense: true })).toBeNull()
  })

  it('parses a wrapped AnyValue-schema detail (mirrors the screenshot)', () => {
    const detail = {
      description: 'Stream-upload a file into a sandbox',
      function_id: 'sandbox::fs::write',
      observer_count: 0,
      request_schema: {
        $schema: 'http://json-schema.org/draft-07/schema#',
        title: 'AnyValue',
      },
      response_schema: {
        $schema: 'http://json-schema.org/draft-07/schema#',
        title: 'AnyValue',
      },
      worker_name: 'iii-sandbox',
    }
    const parsed = safeParseResponse(functionDetailSchema, wrap(detail))
    expect(parsed?.function_id).toBe('sandbox::fs::write')
    expect(parsed?.observer_count).toBe(0)
  })

  it('parses an observable source with args_format + schemas', () => {
    const detail = {
      function_id: 'cron::on-tick',
      worker_name: 'cron',
      description: 'Called on each tick of an observed expression',
      request_schema: { type: 'object' },
      response_schema: { type: 'object' },
      args_format: {
        type: 'object',
        properties: { expression: { type: 'string' } },
      },
      private: true,
      observer_count: 2,
    }
    const parsed = safeParseResponse(functionDetailSchema, detail)
    expect(parsed?.private).toBe(true)
    expect(parsed?.observer_count).toBe(2)
    expect(parsed?.args_format).toMatchObject({ type: 'object' })
  })

  it('parses a function with only its identity', () => {
    const parsed = safeParseResponse(functionDetailSchema, {
      function_id: 'x',
      worker_name: 'y',
    })
    expect(parsed?.function_id).toBe('x')
    expect(parsed?.observer_count).toBeUndefined()
  })
})

const observer = {
  id: 'obs-1',
  function_id: 'http::on-request',
  namespace: 'default',
  args: { method: 'POST', path: '/todos' },
  filters: {},
  target: { function_id: 'todo::create', namespace: 'default' },
  capture: ['input', 'output', 'caller'],
  on: 'success',
  deliver: 'output',
  worker: { worker_id: 'w-1', worker_name: 'todo-app' },
  state: 'active',
  fires: 3,
  dropped: {},
  activation_failures: {},
  max_in_flight: 64,
  created_at_ms: 1_700_000_000_000,
}

describe('engine::observers::list', () => {
  it('parses a wrapped success payload', () => {
    const resp = safeParseResponse(
      observersListResponseSchema,
      wrap({ observers: [observer] }),
    )
    expect(resp?.observers).toHaveLength(1)
    expect(resp?.observers[0].target.function_id).toBe('todo::create')
  })

  it('parses a function_id filter request', () => {
    expect(
      safeParseRequest(observersListRequestSchema, {
        function_id: 'http::on-request',
      }),
    ).toEqual({ function_id: 'http::on-request' })
  })

  it('rejects payloads missing the `observers` array', () => {
    expect(safeParseResponse(observersListResponseSchema, {})).toBeNull()
  })
})

describe('engine::observers::get', () => {
  it('parses the request payload', () => {
    expect(safeParseRequest(observerIdRequestSchema, { id: 'obs-1' })).toEqual({
      id: 'obs-1',
    })
  })

  it('reads the observer bare or under `observer`', () => {
    expect(parseObserverInfoResponse(wrap(observer))?.id).toBe('obs-1')
    expect(parseObserverInfoResponse({ observer })?.state).toBe('active')
  })

  it('returns null for unknown shapes (card falls back to panes)', () => {
    expect(parseObserverInfoResponse({ nonsense: true })).toBeNull()
  })
})

describe('engine::observers::dispose', () => {
  it('parses the response', () => {
    expect(
      safeParseResponse(disposeObserverResponseSchema, { disposed: true }),
    ).toEqual({ disposed: true })
  })
})

describe('engine::workers::list', () => {
  it('parses an empty request payload', () => {
    expect(safeParseRequest(workersListRequestSchema, {})).toEqual({})
  })

  it('parses a runtime+status filter', () => {
    expect(
      safeParseRequest(workersListRequestSchema, {
        runtime: 'rust',
        status: 'connected',
      }),
    ).toEqual({ runtime: 'rust', status: 'connected' })
  })

  it('parses an optional tag filter', () => {
    expect(
      safeParseRequest(workersListRequestSchema, { tag: 'platform' }),
    ).toEqual({ tag: 'platform' })
  })

  it('parses a wrapped success payload', () => {
    const payload = {
      workers: [
        {
          id: 'w-1',
          name: 'todo-app',
          description: null,
          namespace: 'project-a',
          version: '0.4.7',
          runtime: 'node',
          os: 'darwin',
          status: 'connected',
          function_count: 6,
          connected_at_ms: 1_700_000_000_000,
          active_invocations: 0,
          isolation: 'process',
          ip_address: '127.0.0.1',
          tag: 'dev',
        },
      ],
    }
    const parsed = safeParseResponse(workersListResponseSchema, wrap(payload))
    expect(parsed?.workers).toHaveLength(1)
    expect(parsed?.workers[0].runtime).toBe('node')
    expect(parsed?.workers[0].tag).toBe('dev')
    expect(parsed?.workers[0]).toHaveProperty('namespace', 'project-a')
  })

  it('parses an empty workers list', () => {
    expect(
      safeParseResponse(workersListResponseSchema, { workers: [] }),
    ).toEqual({ workers: [] })
  })
})

describe('engine::workers::info', () => {
  it('parses the request payload', () => {
    expect(
      safeParseRequest(workerInfoRequestSchema, { name: 'todo-app' }),
    ).toEqual({ name: 'todo-app' })
  })

  it('rejects a request missing name', () => {
    expect(safeParseRequest(workerInfoRequestSchema, {})).toBeNull()
  })

  it('parses a wrapped full detail payload', () => {
    const payload = {
      worker: {
        id: 'w-1',
        name: 'todo-app',
        description: 'demo',
        namespace: 'project-a',
        version: '0.4.7',
        runtime: 'node',
        os: 'darwin',
        status: 'connected',
        function_count: 1,
        connected_at_ms: 1_700_000_000_000,
        active_invocations: 0,
        isolation: 'process',
        ip_address: null,
        internal: false,
        latest_metrics: {
          memory_heap_used: 1024,
          cpu_percent: 1.2,
          timestamp_ms: 1_700_000_001_000,
          runtime: 'node',
        },
      },
      functions: [
        {
          function_id: 'todo::list',
          worker_name: 'todo-app',
          namespace: 'project-a',
        },
      ],
      observers: ['obs-1'],
    }
    const parsed = safeParseResponse(workerInfoResponseSchema, wrap(payload))
    expect(parsed?.worker.internal).toBe(false)
    expect(parsed?.functions).toHaveLength(1)
    expect(parsed?.worker.latest_metrics?.cpu_percent).toBe(1.2)
    expect(parsed?.worker).toHaveProperty('namespace', 'project-a')
    expect(parsed?.functions[0]).toHaveProperty('namespace', 'project-a')
    expect(parsed?.observers).toEqual(['obs-1'])
  })

  it('parses an internal worker without optional fields', () => {
    const parsed = safeParseResponse(workerInfoResponseSchema, {
      worker: {
        id: 'engine-fns',
        name: 'iii-engine-functions',
        description: null,
        version: null,
        runtime: 'rust',
        os: 'darwin',
        status: 'connected',
        function_count: 7,
        connected_at_ms: 0,
        active_invocations: 0,
        isolation: 'embedded',
        ip_address: null,
        internal: true,
      },
      functions: [],
    })
    expect(parsed?.worker.internal).toBe(true)
  })
})

describe('engine::workers::register', () => {
  it('parses a full request with telemetry', () => {
    const parsed = safeParseRequest(workersRegisterRequestSchema, {
      _caller_worker_id: 'w-1',
      name: 'todo-app',
      runtime: 'node',
      version: '0.4.7',
      os: 'darwin',
      telemetry: { device_id: 'dev-1', install_kind: 'npm' },
    })
    expect(parsed?._caller_worker_id).toBe('w-1')
    expect(parsed?.telemetry?.install_kind).toBe('npm')
  })

  it('rejects a request missing the _caller_worker_id', () => {
    expect(
      safeParseRequest(workersRegisterRequestSchema, { name: 'no-id' }),
    ).toBeNull()
  })

  it('parses the success envelope', () => {
    expect(
      safeParseResponse(workersRegisterResponseSchema, wrap({ success: true })),
    ).toEqual({ success: true })
  })
})

describe('harness::observe', () => {
  it('is included in the engine function id set', () => {
    expect(ENGINE_FUNCTION_IDS).toContain('harness::observe')
    expect(isEngineListFunction('harness::observe')).toBe(true)
  })

  it('parses a state wake (no target, has label/once)', () => {
    const req = safeParseRequest(observeRequestSchema, {
      function_id: 'state::on-changed',
      filters: {
        'input.scope': { $eq: 'research' },
        'input.key': { $eq: 'progress' },
      },
      label: 'research-progress-watch',
      once: false,
    })
    expect(req?.function_id).toBe('state::on-changed')
    expect(req?.target).toBeUndefined()
    expect(req?.label).toBe('research-progress-watch')
    expect(req?.once).toBe(false)
  })

  it('renders a call target with its payload and event pointer', () => {
    const input = {
      function_id: 'state::on-changed',
      filters: { 'input.key': { $eq: 'build' } },
      target: {
        function_id: 'database::execute',
        payload: { deployment: 'deploy-42' },
        event_into: '/args/event',
      },
    }
    const req = safeParseRequest(observeRequestSchema, input)
    expect(req?.target).toEqual(input.target)

    const html = renderToStaticMarkup(
      createElement(ObserveView, {
        messageId: 'observe-1',
        input,
        output: { observer_id: 'obs-1', once: false },
      }),
    )
    expect(html).toContain('database::execute')
    expect(html).toContain('deploy-42')
    expect(html).toContain('/args/event')
    expect(html).toContain('input.key')
    expect(html).not.toContain('this session')
  })

  it('parses and renders gating conditions', () => {
    const input = {
      function_id: 'state::on-changed',
      filters: { 'input.scope': { $eq: 'rx_run' } },
      label: 'wake when all couriers done',
      once: true,
      conditions: [
        {
          function_id: 'state::barrier',
          config: {
            expect: ['acme', 'globex', 'initech'],
            id: 'all_couriers_done',
            key_from: '/key',
          },
        },
      ],
    }
    const req = safeParseRequest(observeRequestSchema, input)
    expect(req?.conditions).toEqual(input.conditions)

    const html = renderToStaticMarkup(
      createElement(ObserveView, {
        messageId: 'observe-2',
        input,
        output: undefined,
      }),
    )
    expect(html).toContain('Only if')
    expect(html).toContain('state::barrier')
    expect(html).toContain('acme, globex, initech')
    expect(html).toContain('all_couriers_done')
  })

  it('renders args and lifecycle limits for the generic renderer', () => {
    const input = {
      function_id: 'cron::on-tick',
      args: { expression: '0 0 9 * * * *' },
      lifecycle: { once: false, max_fires: 3, expires_in_ms: 3_600_000 },
    }
    const req = safeParseRequest(observeRequestSchema, input)
    expect(req?.lifecycle).toEqual({
      once: false,
      max_fires: 3,
      expires_in_ms: 3_600_000,
    })
    const html = renderToStaticMarkup(
      createElement(ObserveView, {
        messageId: 'observe-3',
        input,
        output: undefined,
      }),
    )
    expect(html).toContain('0 0 9 * * * *')
    expect(html).toContain('Recurring')
    expect(html).toContain('Fire limit')
    expect(html).toContain('in 1h')
  })

  it('rejects a request missing the observed function', () => {
    expect(safeParseRequest(observeRequestSchema, { args: {} })).toBeNull()
  })

  it('parses the response { observer_id, once, note }', () => {
    expect(
      safeParseResponse(
        observeResponseSchema,
        wrap({
          observer_id: 'obs-1',
          once: true,
          note: 'observer created; verify the watched key',
        }),
      ),
    ).toEqual({
      observer_id: 'obs-1',
      once: true,
      note: 'observer created; verify the watched key',
    })
  })

  it('does not claim the observer was added for an invalid result', () => {
    const html = renderToStaticMarkup(
      createElement(ObserveView, {
        messageId: 'observe-failed',
        input: { function_id: 'state::on-changed' },
        output: { error: { message: 'forbidden' } },
      }),
    )
    expect(html).toContain('Observer not added')
    expect(html).not.toContain('Observer added</div>')
  })

  it('recovers a double-encoded request through the full render chain', () => {
    // The whole payload arrives as one JSON string; index.tsx does
    // coerceJsonObject(unwrapEnvelope(input)).
    const stringified = JSON.stringify({
      function_id: 'state::on-changed',
      filters: { 'input.session_id': { $eq: 'analyst-2-deep' } },
      target: { function_id: 'database::execute', event_into: '/event' },
    })
    const input = coerceJsonObject(unwrapEnvelope(stringified))
    const req = safeParseRequest(observeRequestSchema, input)
    expect(req?.function_id).toBe('state::on-changed')
    expect(req?.target?.function_id).toBe('database::execute')
    expect(req?.filters).toEqual({
      'input.session_id': { $eq: 'analyst-2-deep' },
    })
  })
})

describe('harness::unobserve', () => {
  it('parses the request and idempotent response', () => {
    expect(safeParseRequest(unobserveRequestSchema, { id: 'obs-1' })).toEqual({
      id: 'obs-1',
    })
    expect(
      safeParseResponse(unobserveResponseSchema, wrap({ removed: true })),
    ).toEqual({ removed: true })
  })

  it('renders manual removal distinctly from an idempotent miss', () => {
    const removed = renderToStaticMarkup(
      createElement(UnobserveView, {
        input: { id: 'obs-1' },
        output: { removed: true },
      }),
    )
    expect(removed).toContain('observer manually removed')
    expect(removed).toContain('explicitly removed')

    const missing = renderToStaticMarkup(
      createElement(UnobserveView, {
        input: { id: 'obs-404' },
        output: { removed: false },
      }),
    )
    expect(missing).toContain('observer not found')
    expect(missing).not.toContain('explicitly removed')

    const failed = renderToStaticMarkup(
      createElement(UnobserveView, {
        input: { id: 'obs-1' },
        output: { error: { message: 'store unavailable' } },
      }),
    )
    expect(failed).toContain('observer removal failed')
    expect(failed).toContain('valid removal result')
  })

  it('reads the engine dispose response too', () => {
    const html = renderToStaticMarkup(
      createElement(UnobserveView, {
        input: { id: 'obs-1' },
        output: { disposed: true },
      }),
    )
    expect(html).toContain('observer manually removed')
  })
})

describe('unwrapEnvelope re-export', () => {
  it('peels the harness envelope', () => {
    const inner = { functions: [] }
    expect(unwrapEnvelope(wrap(inner))).toEqual(inner)
  })
})
