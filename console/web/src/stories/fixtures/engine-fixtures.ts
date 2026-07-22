import type { FunctionCallMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionCallMessage>,
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId,
    input,
    output,
    durationMs: 2,
    createdAt: now,
    ...extra,
  }
}

/* ---------------- engine::functions::list ---------------- */

export const engineFunctionsListDone = base(
  'engine-fn-list',
  'engine::functions::list',
  { prefix: 'todo::' },
  wrapHarness({
    functions: [
      {
        function_id: 'todo::create',
        worker_name: 'todo-app',
        description: 'Create a new todo',
      },
      {
        function_id: 'todo::delete',
        worker_name: 'todo-app',
        description: 'Delete a todo by ID',
      },
      {
        function_id: 'todo::get',
        worker_name: 'todo-app',
        description: 'Get a single todo by ID',
      },
      {
        function_id: 'todo::html',
        worker_name: 'todo-app',
        description: 'Serve the Todo App HTML page',
      },
      {
        function_id: 'todo::list',
        worker_name: 'todo-app',
        description: 'List all todos',
      },
      {
        function_id: 'todo::update',
        worker_name: 'todo-app',
        description: 'Update a todo by ID',
      },
    ],
  }),
)

export const engineFunctionsListEmpty = base(
  'engine-fn-list-empty',
  'engine::functions::list',
  { worker: 'iii-directory', search: 'nonexistent' },
  { functions: [] },
)

export const engineFunctionsListRaw = base(
  'engine-fn-list-raw',
  'engine::functions::list',
  {},
  {
    functions: [
      {
        function_id: 'directory::skills::download',
        worker_name: 'iii-directory',
        description: 'Download skill bundle from the registry',
      },
      {
        function_id: 'directory::skills::list',
        worker_name: 'iii-directory',
        // description intentionally omitted for visual variance
      },
    ],
  },
)

/* ---------------- engine::functions::info ---------------- */

const ANY_VALUE_SCHEMA = {
  $schema: 'http://json-schema.org/draft-07/schema#',
  title: 'AnyValue',
}

/** Mirrors the screenshot: `sandbox::fs::write` with AnyValue placeholders
 * and no registered triggers. The view should collapse the schema slots
 * to "· any" and surface the empty-trigger state cleanly. */
export const engineFunctionInfoAnyValue = base(
  'engine-fn-info-any',
  'engine::functions::info',
  { function_id: 'sandbox::fs::write' },
  wrapHarness({
    description: 'Stream-upload a file into a sandbox',
    function_id: 'sandbox::fs::write',
    registered_triggers: [],
    request_schema: ANY_VALUE_SCHEMA,
    response_schema: ANY_VALUE_SCHEMA,
    worker_name: 'iii-sandbox',
  }),
)

/** Rich payload with real JSON Schemas + one registered trigger so the
 * view's expandable schema panes and config block both fire. */
export const engineFunctionInfoRich = base(
  'engine-fn-info-rich',
  'engine::functions::info',
  { function_id: 'todo::create' },
  {
    function_id: 'todo::create',
    worker_name: 'todo-app',
    description: 'Create a new todo and persist it to the store.',
    request_schema: {
      $schema: 'http://json-schema.org/draft-07/schema#',
      title: 'CreateTodoInput',
      type: 'object',
      required: ['title'],
      properties: {
        title: { type: 'string', minLength: 1 },
        done: { type: 'boolean', default: false },
      },
    },
    response_schema: {
      $schema: 'http://json-schema.org/draft-07/schema#',
      title: 'Todo',
      type: 'object',
      properties: {
        id: { type: 'string', format: 'uuid' },
        title: { type: 'string' },
        done: { type: 'boolean' },
      },
    },
    registered_triggers: [
      {
        id: '8c7e9d12-2a4f-4b5c-9d3e-1f2a3b4c5d6e',
        trigger_type: 'http',
        config: { api_path: '/todos', http_method: 'POST' },
      },
    ],
  },
)

export const engineFunctionInfoNotFound = base(
  'engine-fn-info-notfound',
  'engine::functions::info',
  { function_id: 'nope::missing' },
  {
    error: {
      kind: 'function_error',
      message: "Function 'nope::missing' is not registered.",
      details: {
        code: 'NOT_FOUND',
        message: "Function 'nope::missing' is not registered.",
      },
      content: [
        {
          type: 'text',
          text: "Function 'nope::missing' is not registered.",
        },
      ],
    },
  },
)

/* ---------------- engine::triggers::list ---------------- */

export const engineTriggersListDone = base(
  'engine-tr-list',
  'engine::triggers::list',
  {},
  wrapHarness({
    triggers: [
      {
        id: 'database::row-change',
        worker_name: 'Andersons-MBP.localdomain:28943',
        description:
          'Postgres logical replication. Stubbed in v1.0 pending tokio-postgres replication slot support.',
      },
      {
        id: 'directory::prompts::on-change',
        worker_name: 'iii-directory',
        description:
          'Fires after every successful directory::skills::download that wrote at least one prompt.',
      },
      {
        id: 'directory::skills::on-change',
        worker_name: 'iii-directory',
        description:
          'Fires after every successful directory::skills::download that wrote at least one skill.',
      },
      { id: 'http', worker_name: 'http', description: 'HTTP API trigger' },
      { id: 'log', worker_name: 'log', description: 'Log event trigger' },
      { id: 'state', worker_name: 'state', description: 'State trigger' },
      { id: 'stream', worker_name: 'stream', description: 'Stream trigger' },
    ],
  }),
)

export const engineTriggersListFiltered = base(
  'engine-tr-list-filter',
  'engine::triggers::list',
  { search: 'http', include_internal: true },
  {
    triggers: [
      { id: 'http', worker_name: 'http', description: 'HTTP API trigger' },
    ],
  },
)

/* ---------------- engine::registered-triggers::list ---------------- */

export const engineRegisteredTriggersListDone = base(
  'engine-reg-tr',
  'engine::registered-triggers::list',
  { function_id: 'todo::html' },
  wrapHarness({
    registered_triggers: [
      {
        id: '6ebe7d64-3717-4acc-a02d-3f050fb86df2',
        trigger_type: 'http',
        function_id: 'todo::html',
        worker_name: 'todo-app',
        config_summary: '{"api_path":"/todos/html","http_method":"GET"}',
      },
      {
        id: 'f3c6b688-6840-48e8-8e7f-fec48d8cffbf',
        trigger_type: 'http',
        function_id: 'todo::html',
        worker_name: 'todo-app',
        config_summary: '{"api_path":"/todos/html","http_method":"GET"}',
      },
    ],
  }),
)

export const engineRegisteredTriggersListPlainSummary = base(
  'engine-reg-tr-plain',
  'engine::registered-triggers::list',
  { worker: 'iii-directory' },
  {
    registered_triggers: [
      {
        id: 'a1b2c3d4-5e6f-7a8b-9c0d-1e2f3a4b5c6d',
        trigger_type: 'log',
        function_id: 'directory::log::watch',
        worker_name: 'iii-directory',
        config_summary: 'level=info, source=stdout',
      },
    ],
  },
)

/* ---------------- error fixture ---------------- */

export const engineFunctionsListGateError = base(
  'engine-fn-list-gate',
  'engine::functions::list',
  { prefix: 'todo::' },
  {
    error: {
      kind: 'function_error',
      message:
        'trigger_failed: IIIInvocationError: invocation_failed: handler error',
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'gate_unavailable',
        function_id: 'engine::functions::list',
        reason: 'trigger_failed: directory engine unreachable',
      },
      content: [
        {
          type: 'text',
          text: 'trigger_failed: directory engine unreachable',
        },
      ],
    },
  },
)

export const engineRunning = base(
  'engine-running',
  'engine::triggers::list',
  {},
  undefined,
  { running: true },
)

/* ---------------- engine::workers::list ---------------- */

const tenMinutesAgo = now - 10 * 60 * 1000

export const engineWorkersListDone = base(
  'engine-wrk-list',
  'engine::workers::list',
  {},
  wrapHarness({
    workers: [
      {
        id: '7fa8e8a4-1c3d-44b2-9a5f-1234567890ab',
        name: 'todo-app',
        description: null,
        version: '0.4.7',
        runtime: 'node',
        os: 'darwin',
        status: 'connected',
        function_count: 6,
        connected_at_ms: tenMinutesAgo,
        active_invocations: 0,
        isolation: 'process',
        ip_address: '127.0.0.1',
      },
      {
        id: '00000000-0000-0000-0000-000000000aaa',
        name: 'iii-directory',
        description: null,
        version: '0.1.5',
        runtime: 'rust',
        os: 'darwin',
        status: 'connected',
        function_count: 12,
        connected_at_ms: now - 2 * 3600 * 1000,
        active_invocations: 1,
        isolation: 'embedded',
        ip_address: null,
      },
      {
        id: '11111111-2222-3333-4444-555555555555',
        name: 'iii-sandbox',
        description: null,
        version: '0.4.7',
        runtime: 'rust',
        os: 'linux',
        status: 'disconnected',
        function_count: 15,
        connected_at_ms: now - 24 * 3600 * 1000,
        active_invocations: 0,
        isolation: 'libkrun',
        ip_address: null,
      },
    ],
  }),
)

export const engineWorkersListFiltered = base(
  'engine-wrk-list-filter',
  'engine::workers::list',
  { runtime: 'rust', status: 'connected' },
  {
    workers: [
      {
        id: '00000000-0000-0000-0000-000000000aaa',
        name: 'iii-directory',
        description: null,
        version: '0.1.5',
        runtime: 'rust',
        os: 'darwin',
        status: 'connected',
        function_count: 12,
        connected_at_ms: now - 2 * 3600 * 1000,
        active_invocations: 0,
        isolation: 'embedded',
        ip_address: null,
      },
    ],
  },
)

export const engineWorkersListEmpty = base(
  'engine-wrk-list-empty',
  'engine::workers::list',
  { search: 'no-such-worker' },
  { workers: [] },
)

/* ---------------- engine::workers::info ---------------- */

export const engineWorkerInfoDone = base(
  'engine-wrk-info',
  'engine::workers::info',
  { name: 'todo-app' },
  wrapHarness({
    worker: {
      id: '7fa8e8a4-1c3d-44b2-9a5f-1234567890ab',
      name: 'todo-app',
      description: 'Demo todo app worker.',
      version: '0.4.7',
      runtime: 'node',
      os: 'darwin',
      status: 'connected',
      function_count: 6,
      connected_at_ms: tenMinutesAgo,
      active_invocations: 0,
      isolation: 'process',
      ip_address: '127.0.0.1',
      pid: 28943,
      internal: false,
      latest_metrics: {
        memory_heap_used: 28 * 1024 * 1024,
        memory_heap_total: 64 * 1024 * 1024,
        memory_rss: 102 * 1024 * 1024,
        cpu_percent: 1.2,
        event_loop_lag_ms: 0.8,
        uptime_seconds: 612,
        timestamp_ms: now,
        runtime: 'node',
      },
    },
    functions: [
      { function_id: 'todo::create', worker_name: 'todo-app' },
      { function_id: 'todo::delete', worker_name: 'todo-app' },
      { function_id: 'todo::get', worker_name: 'todo-app' },
      { function_id: 'todo::html', worker_name: 'todo-app' },
      { function_id: 'todo::list', worker_name: 'todo-app' },
      { function_id: 'todo::update', worker_name: 'todo-app' },
    ],
    trigger_types: [],
    registered_triggers: [
      {
        id: '6ebe7d64-3717-4acc-a02d-3f050fb86df2',
        trigger_type: 'http',
        function_id: 'todo::html',
        worker_name: 'todo-app',
        config_summary: '{"api_path":"/todos/html","http_method":"GET"}',
      },
      {
        id: 'aa11bb22-cc33-dd44-ee55-ff6677889900',
        trigger_type: 'http',
        function_id: 'todo::list',
        worker_name: 'todo-app',
        config_summary: '{"api_path":"/todos","http_method":"GET"}',
      },
    ],
  }),
)

export const engineWorkerInfoInternal = base(
  'engine-wrk-info-internal',
  'engine::workers::info',
  { name: 'iii-engine-functions' },
  {
    worker: {
      id: '00000000-0000-0000-0000-deadbeefcafe',
      name: 'iii-engine-functions',
      description: null,
      version: null,
      runtime: 'rust',
      os: 'darwin',
      status: 'connected',
      function_count: 7,
      connected_at_ms: now - 5 * 60 * 1000,
      active_invocations: 0,
      isolation: 'embedded',
      ip_address: null,
      internal: true,
    },
    functions: [
      {
        function_id: 'engine::functions::list',
        worker_name: 'iii-engine-functions',
      },
      {
        function_id: 'engine::functions::info',
        worker_name: 'iii-engine-functions',
      },
      {
        function_id: 'engine::triggers::list',
        worker_name: 'iii-engine-functions',
      },
      {
        function_id: 'engine::registered-triggers::list',
        worker_name: 'iii-engine-functions',
      },
      {
        function_id: 'engine::workers::list',
        worker_name: 'iii-engine-functions',
      },
      {
        function_id: 'engine::workers::info',
        worker_name: 'iii-engine-functions',
      },
      {
        function_id: 'engine::workers::register',
        worker_name: 'iii-engine-functions',
      },
    ],
    trigger_types: [],
    registered_triggers: [],
  },
)

export const engineWorkerInfoNotFound = base(
  'engine-wrk-info-notfound',
  'engine::workers::info',
  { name: 'no-such-worker' },
  {
    error: {
      kind: 'function_error',
      message: "Worker 'no-such-worker' is not connected.",
      details: {
        code: 'NOT_FOUND',
        message: "Worker 'no-such-worker' is not connected.",
      },
      content: [
        {
          type: 'text',
          text: "Worker 'no-such-worker' is not connected.",
        },
      ],
    },
  },
)

/* ---------------- engine::workers::register ---------------- */

export const engineWorkerRegisterDone = base(
  'engine-wrk-register',
  'engine::workers::register',
  {
    _caller_worker_id: '7fa8e8a4-1c3d-44b2-9a5f-1234567890ab',
    name: 'todo-app',
    runtime: 'node',
    version: '0.4.7',
    os: 'darwin',
    telemetry: {
      device_id: 'dev-9a8b7c6d5e4f3a2b1c0d',
      install_kind: 'npm',
    },
  },
  wrapHarness({ success: true }),
)

export const engineWorkerRegisterRunning = base(
  'engine-wrk-register-running',
  'engine::workers::register',
  {
    _caller_worker_id: '00000000-0000-0000-0000-000000000000',
    name: 'new-worker',
    runtime: 'python',
    version: '0.1.0',
    os: 'linux',
  },
  undefined,
  { running: true },
)

/* ---------------- engine::register_trigger ---------------- */

export const engineRegisterTriggerReact = base(
  'engine-register-trigger-react',
  'engine::register_trigger',
  {
    trigger_type: 'state',
    function_id: 'harness::react',
    config: { key: 'build', scope: 'ops' },
    metadata: {
      model: 'claude-sonnet-5',
      session_id: 'console-f0aac029-62a3-43f8-953b-45d0d903a866',
      task: 'You are the GATE REVIEWER for a deploy pipeline. Both state records ops/build and ops/tests must be green before you approve.',
      options: { functions: { allow: ['state::get'] } },
      join: {
        id: 'gate-decision-join',
        key: 'build',
        expect: ['build', 'tests'],
        rearm: true,
      },
    },
  },
  wrapHarness({ id: 'trg-abc123def456' }),
)

export const engineRegisterTriggerCron = base(
  'engine-register-trigger-cron',
  'engine::register_trigger',
  {
    trigger_type: 'cron',
    function_id: 'ops::nightly-sweep',
    config: { expression: '0 0 3 * * *' },
  },
  wrapHarness({ id: 'trg-cron-01' }),
)

/** Call-mode reaction: the event dispatches an fp::pipe instead of spawning
 * a sub-agent — the THEN pane renders the pipeline's step route. Mirrors a
 * live turn-completed → count-check → state::set binding. */
export const engineRegisterTriggerCallPipe = base(
  'engine-register-trigger-call-pipe',
  'engine::register_trigger',
  {
    trigger_type: 'harness::turn-completed',
    function_id: 'harness::react',
    config: {
      parent_session_id: 'console-b048edc6-3a09-41f7-884e-fec9ba1b17e2',
    },
    metadata: {
      call: {
        function_id: 'fp::pipe',
        payload: {
          through: [
            {
              function: 'database::query',
              payload: {
                db: 'sqlite_db',
                sql: "SELECT COUNT(*) AS n FROM scan_progress WHERE scan_run = 'sec-m8k4' AND status IN ('done','failed')",
              },
            },
            { function: 'fp::get', payload: { path: '/rows/0/n' } },
            { function: 'fp::when', payload: { op: '>=', to: 12 } },
            {
              function: 'state::set',
              into: '/value/batches_done',
              payload: {
                key: 'turn_complete',
                scope: 'run-sec-m8k4',
                value: { done: true, run_id: 'sec-m8k4' },
              },
            },
          ],
        },
      },
    },
  },
  wrapHarness({ id: 'sub_2a77e6f39bad' }),
)

export const engineRegisterTriggerSubscribe = base(
  'engine-register-trigger-subscribe',
  'engine::register_trigger',
  {
    trigger_type: 'state',
    config: { key: 'progress', scope: 'research' },
    label: 'research-progress-watch',
    once: false,
  },
  wrapHarness({
    once: false,
    subscription_id: 'sub_6c9c9f043f5f449dab6569d2f27a8c05',
  }),
)

export const engineRegisterTriggerRunning = base(
  'engine-register-trigger-running',
  'engine::register_trigger',
  {
    trigger_type: 'state',
    function_id: 'harness::react',
    config: { scope: 'ops' },
    metadata: { model: 'claude-sonnet-5', task: 'watch for changes' },
  },
  undefined,
  { running: true },
)

export const engineFixtures = [
  engineFunctionsListDone,
  engineFunctionsListRaw,
  engineFunctionsListEmpty,
  engineFunctionInfoAnyValue,
  engineFunctionInfoRich,
  engineFunctionInfoNotFound,
  engineTriggersListDone,
  engineTriggersListFiltered,
  engineRegisteredTriggersListDone,
  engineRegisteredTriggersListPlainSummary,
  engineWorkersListDone,
  engineWorkersListFiltered,
  engineWorkersListEmpty,
  engineWorkerInfoDone,
  engineWorkerInfoInternal,
  engineWorkerInfoNotFound,
  engineWorkerRegisterDone,
  engineWorkerRegisterRunning,
  engineRegisterTriggerReact,
  engineRegisterTriggerCron,
  engineRegisterTriggerCallPipe,
  engineRegisterTriggerSubscribe,
  engineRegisterTriggerRunning,
  engineRunning,
  engineFunctionsListGateError,
] as const
