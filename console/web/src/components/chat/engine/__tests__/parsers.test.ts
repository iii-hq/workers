import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import {
  coerceJsonObject,
  configFilters,
  describeCron,
  ENGINE_FUNCTION_IDS,
  functionDetailSchema,
  functionInfoRequestSchema,
  functionsListRequestSchema,
  functionsListResponseSchema,
  isEngineListFunction,
  parseFunctionInfoResponse,
  parseTriggerInfoResponse,
  registeredTriggersListRequestSchema,
  registeredTriggersListResponseSchema,
  registerTriggerRequestSchema,
  registerTriggerResponseSchema,
  safeParseRequest,
  safeParseResponse,
  triggerInfoRequestSchema,
  triggersListRequestSchema,
  triggersListResponseSchema,
  unwrapEnvelope,
  workerInfoRequestSchema,
  workerInfoResponseSchema,
  workersListRequestSchema,
  workersListResponseSchema,
  workersRegisterRequestSchema,
  workersRegisterResponseSchema,
} from '../parsers'
import { RegisterTriggerView } from '../RegisterTriggerView'

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
    expect(isEngineListFunction('engine::triggers::create')).toBe(false)
  })
})

describe('coerceJsonObject', () => {
  it('recovers a double-encoded (stringified) payload', () => {
    const payload = { trigger_type: 'state', config: { scope: 'wiki' } }
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
      registered_triggers: [],
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
      registered_triggers: [],
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
    expect(parsed?.registered_triggers).toEqual([])
  })

  it('parses a rich payload with registered triggers + schemas', () => {
    const detail = {
      function_id: 'todo::create',
      worker_name: 'todo-app',
      description: 'Create a new todo',
      request_schema: { type: 'object' },
      response_schema: { type: 'object' },
      registered_triggers: [
        {
          id: '8c7e9d12-2a4f-4b5c-9d3e-1f2a3b4c5d6e',
          trigger_type: 'http',
          config: { api_path: '/todos', http_method: 'POST' },
        },
      ],
    }
    const parsed = safeParseResponse(functionDetailSchema, detail)
    expect(parsed?.registered_triggers).toHaveLength(1)
    expect(parsed?.registered_triggers[0].trigger_type).toBe('http')
  })

  it('rejects a payload missing registered_triggers', () => {
    const parsed = safeParseResponse(functionDetailSchema, {
      function_id: 'x',
      worker_name: 'y',
    })
    expect(parsed).toBeNull()
  })
})

describe('engine::triggers::list', () => {
  it('parses a wrapped success payload', () => {
    const payload = {
      triggers: [
        { id: 'http', worker_name: 'http', description: 'HTTP API trigger' },
        {
          id: 'log',
          worker_name: 'log',
          description: 'Log event trigger',
        },
      ],
    }
    const resp = safeParseResponse(triggersListResponseSchema, wrap(payload))
    expect(resp?.triggers).toHaveLength(2)
  })

  it('parses a search-applied request', () => {
    expect(
      safeParseRequest(triggersListRequestSchema, { search: 'http' }),
    ).toEqual({ search: 'http' })
  })

  it('rejects payloads missing the `triggers` array', () => {
    const resp = safeParseResponse(triggersListResponseSchema, { foo: [] })
    expect(resp).toBeNull()
  })
})

describe('engine::triggers::info', () => {
  it('parses the request payload', () => {
    expect(safeParseRequest(triggerInfoRequestSchema, { id: 'state' })).toEqual(
      { id: 'state' },
    )
    expect(safeParseRequest(triggerInfoRequestSchema, {})).toBeNull()
  })

  it('parses a wrapped trigger-type detail (mirrors the live shape)', () => {
    const detail = {
      id: 'state',
      worker_name: 'iii-state',
      description: 'State trigger',
      instance_count: 3,
      configuration_schema: { type: 'object', title: 'StateTriggerConfig' },
      request_schema: { type: 'object' },
    }
    const parsed = parseTriggerInfoResponse(wrap(detail))
    expect(parsed?.id).toBe('state')
    expect(parsed?.instance_count).toBe(3)
  })

  it('returns null for unknown shapes (card falls back to panes)', () => {
    expect(parseTriggerInfoResponse({ nonsense: true })).toBeNull()
    expect(parseTriggerInfoResponse(undefined)).toBeNull()
  })
})

describe('engine::registered-triggers::list', () => {
  it('parses a wrapped registered-triggers payload', () => {
    const payload = {
      registered_triggers: [
        {
          id: '6ebe7d64-3717-4acc-a02d-3f050fb86df2',
          trigger_type: 'http',
          function_id: 'todo::html',
          worker_name: 'todo-app',
          config_summary: '{"api_path":"/todos/html","http_method":"GET"}',
        },
      ],
    }
    const resp = safeParseResponse(
      registeredTriggersListResponseSchema,
      wrap(payload),
    )
    expect(resp?.registered_triggers).toHaveLength(1)
  })

  it('parses a function_id filter request', () => {
    expect(
      safeParseRequest(registeredTriggersListRequestSchema, {
        function_id: 'todo::html',
      }),
    ).toEqual({ function_id: 'todo::html' })
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
      functions: [{ function_id: 'todo::list', worker_name: 'todo-app' }],
      trigger_types: [],
      registered_triggers: [],
    }
    const parsed = safeParseResponse(workerInfoResponseSchema, wrap(payload))
    expect(parsed?.worker.internal).toBe(false)
    expect(parsed?.functions).toHaveLength(1)
    expect(parsed?.worker.latest_metrics?.cpu_percent).toBe(1.2)
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
      trigger_types: [],
      registered_triggers: [],
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

describe('engine::register_trigger', () => {
  it('is included in the engine function id set', () => {
    expect(ENGINE_FUNCTION_IDS).toContain('engine::register_trigger')
    expect(isEngineListFunction('engine::register_trigger')).toBe(true)
  })

  it('parses a state-trigger → call-binding registration', () => {
    const req = safeParseRequest(registerTriggerRequestSchema, {
      trigger_type: 'state',
      function_id: 'database::execute',
      config: { key: 'build', scope: 'ops' },
      metadata: {
        payload: { db: 'primary', sql: 'INSERT INTO builds DEFAULT VALUES' },
        event_into: '/event',
      },
    })
    expect(req?.trigger_type).toBe('state')
    expect(req?.function_id).toBe('database::execute')
    expect(req?.metadata).toMatchObject({ event_into: '/event' })
  })

  it('renders an explicit long-form target', () => {
    const input = {
      trigger_type: 'state',
      config: { key: 'build', scope: 'ops' },
      target: {
        function_id: 'database::execute',
        payload: { deployment: 'deploy-42' },
        event_into: '/args/event',
      },
    }
    const req = safeParseRequest(registerTriggerRequestSchema, input)
    expect(req?.target).toEqual(input.target)

    const html = renderToStaticMarkup(
      createElement(RegisterTriggerView, {
        input,
        output: { subscription_id: 'sub-1', once: false },
      }),
    )
    expect(html).toContain('database::execute')
    expect(html).toContain('deploy-42')
    expect(html).toContain('/args/event')
    expect(html).not.toContain('this session')
  })

  it('parses the harness subscribe variant (no function_id, has label/once)', () => {
    const req = safeParseRequest(registerTriggerRequestSchema, {
      trigger_type: 'state',
      config: { key: 'progress', scope: 'research' },
      label: 'research-progress-watch',
      once: false,
    })
    expect(req?.trigger_type).toBe('state')
    expect(req?.function_id).toBeUndefined()
    expect(req?.label).toBe('research-progress-watch')
    expect(req?.once).toBe(false)
  })

  it('rejects a request missing the required trigger_type', () => {
    expect(
      safeParseRequest(registerTriggerRequestSchema, { config: {} }),
    ).toBeNull()
  })

  it('parses the engine response { id }', () => {
    expect(
      safeParseResponse(registerTriggerResponseSchema, wrap({ id: 'trg-1' })),
    ).toEqual({ id: 'trg-1' })
  })

  it('parses the harness-intercepted response { subscription_id, once }', () => {
    expect(
      safeParseResponse(registerTriggerResponseSchema, {
        subscription_id: 'sub-1',
        once: false,
      }),
    ).toEqual({ subscription_id: 'sub-1', once: false })
  })

  it('recovers a double-encoded request through the full render chain', () => {
    // The whole payload arrives as one JSON string; index.tsx does
    // coerceJsonObject(unwrapEnvelope(input)).
    const stringified = JSON.stringify({
      trigger_type: 'state',
      function_id: 'database::execute',
      config: { session_id: 'analyst-2-deep' },
      metadata: { payload: { db: 'primary' }, event_into: '/event' },
    })
    const input = coerceJsonObject(unwrapEnvelope(stringified))
    const req = safeParseRequest(registerTriggerRequestSchema, input)
    expect(req?.trigger_type).toBe('state')
    expect(req?.function_id).toBe('database::execute')
    expect(configFilters(req?.config)).toEqual([
      { label: 'session', value: 'analyst-2-deep' },
    ])
  })

  it('extracts filter chips across state and turn-event configs', () => {
    // state config
    expect(
      configFilters({
        scope: 'ops',
        key: 'build',
        condition_function_id: 'gate::ok',
      }),
    ).toEqual([
      { label: 'scope', value: 'ops' },
      { label: 'key', value: 'build' },
      { label: 'if', value: 'gate::ok' },
    ])
    // turn-event config (previously fell through to raw JSON)
    expect(configFilters({ session_id: 'reviewer-cr7k2' })).toEqual([
      { label: 'session', value: 'reviewer-cr7k2' },
    ])
    expect(configFilters({ parent_session_id: 'root-1' })).toEqual([
      { label: 'parent', value: 'root-1' },
    ])
    // no known filters → null (caller shows "no filter" / raw JSON)
    expect(configFilters({})).toBeNull()
    expect(configFilters({ unknown_field: 'x' })).toBeNull()
    expect(configFilters(undefined)).toBeNull()
  })
})

describe('unwrapEnvelope re-export', () => {
  it('peels the harness envelope', () => {
    const inner = { functions: [] }
    expect(unwrapEnvelope(wrap(inner))).toEqual(inner)
  })
})

describe('describeCron', () => {
  it('humanizes the common shapes', () => {
    // the live screenshot shape: seconds-first, one-shot date+time
    expect(describeCron('0 0 17 21 7 *')).toBe('at 17:00 on Jul 21')
    expect(describeCron('0 30 9 * * *')).toBe('every day at 09:30')
    expect(describeCron('30 9 * * *')).toBe('every day at 09:30') // 5-field classic
    expect(describeCron('0 */5 * * * *')).toBe('every 5 min')
    expect(describeCron('0 * * * * *')).toBe('every minute')
    expect(describeCron('* * * * * *')).toBe('every second')
    expect(describeCron('0 15 * * * *')).toBe('at :15 every hour')
    expect(describeCron('0 0 */6 * * *')).toBe('every 6h at :00')
    // seconds-first form = Rust cron crate dialect: weekdays 1=Sun..7=Sat
    expect(describeCron('0 0 9 * * 1')).toBe('every Sun at 09:00')
    expect(describeCron('0 0 9 * * 2,6')).toBe('every Mon, Fri at 09:00')
    // classic five-field cron: weekdays 0=Sun..6=Sat, 7 also Sunday
    expect(describeCron('0 9 * * 1')).toBe('every Mon at 09:00')
    expect(describeCron('0 9 * * 0')).toBe('every Sun at 09:00')
    expect(describeCron('0 9 * * 7')).toBe('every Sun at 09:00')
    expect(describeCron('0 0 0 1 * *')).toBe('at 00:00 on day 1 of every month')
    expect(describeCron('0 0 12 * 7 *')).toBe('at 12:00 in Jul')
  })

  it('falls back to null on shapes a translation would get wrong', () => {
    expect(describeCron('0 0 17 21 7 1')).toBeNull() // dom+dow = OR semantics
    expect(describeCron('0 0-30 9 * * *')).toBeNull() // ranges
    expect(describeCron('0 0 9 L * *')).toBeNull() // L extension
    expect(describeCron('0 0 25 * * *')).toBeNull() // hour out of range
    expect(describeCron('0 0 17 21 7 * 2027')).toBeNull() // pinned year
    // wild seconds fire 60×/min — "at 17:00" would hide that
    expect(describeCron('* 0 17 * * *')).toBeNull()
    expect(describeCron('30 0 17 * * *')).toBeNull() // nonzero seconds pin
    expect(describeCron('0 0 9 * * 0')).toBeNull() // 0 invalid in Rust dialect
    expect(describeCron('nonsense')).toBeNull()
    expect(describeCron('')).toBeNull()
  })
})
