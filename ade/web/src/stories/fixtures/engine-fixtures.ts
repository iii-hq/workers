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
 * and no observers. The view should collapse the schema slots to "· any". */
export const engineFunctionInfoAnyValue = base(
  'engine-fn-info-any',
  'engine::functions::info',
  { function_id: 'sandbox::fs::write' },
  wrapHarness({
    description: 'Stream-upload a file into a sandbox',
    function_id: 'sandbox::fs::write',
    observer_count: 0,
    request_schema: ANY_VALUE_SCHEMA,
    response_schema: ANY_VALUE_SCHEMA,
    worker_name: 'iii-sandbox',
  }),
)

/** Rich payload with real JSON Schemas + observer facts so the view's
 * expandable schema panes and the observer chips both fire. */
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
    observer_count: 1,
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

/* ---------------- engine::observers::list ---------------- */

const httpObserver = {
  id: '6ebe7d64-3717-4acc-a02d-3f050fb86df2',
  function_id: 'http::on-request',
  namespace: 'default',
  args: { method: 'GET', path: '/todos/html' },
  filters: {},
  target: { function_id: 'todo::html', namespace: 'default' },
  capture: ['input', 'output', 'caller'],
  on: 'success',
  deliver: 'output',
  worker: { worker_id: 'w-todo', worker_name: 'todo-app' },
  state: 'active',
  fires: 12,
  dropped: {},
  activation_failures: {},
  max_in_flight: 64,
  created_at_ms: now - 10 * 60 * 1000,
}

export const engineObserversListDone = base(
  'engine-obs-list',
  'engine::observers::list',
  { function_id: 'http::on-request' },
  wrapHarness({
    observers: [
      httpObserver,
      {
        ...httpObserver,
        id: 'f3c6b688-6840-48e8-8e7f-fec48d8cffbf',
        args: { method: 'GET', path: '/todos' },
        target: { function_id: 'todo::list', namespace: 'default' },
        fires: 0,
      },
    ],
  }),
)

export const engineObserversListFiltered = base(
  'engine-obs-list-state',
  'engine::observers::list',
  { worker: 'iii-directory' },
  {
    observers: [
      {
        ...httpObserver,
        id: 'a1b2c3d4-5e6f-7a8b-9c0d-1e2f3a4b5c6d',
        function_id: 'state::on-changed',
        args: {},
        filters: { 'input.scope': { $eq: 'directory' } },
        target: { function_id: 'directory::reindex', namespace: 'default' },
        worker: { worker_id: 'w-dir', worker_name: 'iii-directory' },
        state: 'pending',
        reason: {
          code: 'FUNCTION_NOT_FOUND',
          message: 'state::on-changed is not registered yet',
        },
      },
    ],
  },
)

export const engineObserverInfoDone = base(
  'engine-obs-get',
  'engine::observers::get',
  { id: httpObserver.id },
  wrapHarness(httpObserver),
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
        'call_failed: IIIInvocationError: invocation_failed: handler error',
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'gate_unavailable',
        function_id: 'engine::functions::list',
        reason: 'call_failed: directory engine unreachable',
      },
      content: [
        {
          type: 'text',
          text: 'call_failed: directory engine unreachable',
        },
      ],
    },
  },
)

export const engineRunning = base(
  'engine-running',
  'engine::observers::list',
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
    observers: [
      '6ebe7d64-3717-4acc-a02d-3f050fb86df2',
      'aa11bb22-cc33-dd44-ee55-ff6677889900',
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
        function_id: 'engine::observers::list',
        worker_name: 'iii-engine-functions',
      },
      {
        function_id: 'engine::observers::get',
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
    observers: [],
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

/* ---------------- harness::observe ---------------- */

export const harnessObserveCron = base(
  'harness-observe-cron',
  'harness::observe',
  {
    function_id: 'cron::on-tick',
    args: { expression: '0 0 3 * * * *' },
    target: { function_id: 'ops::nightly-sweep' },
  },
  wrapHarness({ observer_id: 'obs-cron-01', once: false }),
)

export const harnessObserveState = base(
  'harness-observe-state',
  'harness::observe',
  {
    function_id: 'state::on-changed',
    filters: {
      'input.scope': { $eq: 'research' },
      'input.key': { $eq: 'progress' },
    },
    label: 'research-progress-watch',
    action: 'research progress changed',
    once: false,
  },
  wrapHarness({
    once: false,
    observer_id: 'obs_6c9c9f043f5f449dab6569d2f27a8c05',
  }),
)

export const harnessObserveRunning = base(
  'harness-observe-running',
  'harness::observe',
  {
    function_id: 'state::on-changed',
    filters: { 'input.scope': { $eq: 'ops' } },
    label: 'ops watch',
  },
  undefined,
  { running: true },
)

/** A guarded call observer: long scope id, gating condition, call target with
 * payload, and a bounded lifecycle — exercises every block of the details view. */
export const harnessObserveCallGuarded = base(
  'harness-observe-call-guarded',
  'harness::observe',
  {
    function_id: 'state::on-changed',
    filters: {
      'input.scope': {
        $eq: 'code-review-console-7a7a991e-2394-4636-ac45-59964ebc57bc-k4m8',
      },
      'input.key': { $eq: 'ready/agent1' },
    },
    label: 'Watching for Agent 1 readiness',
    action: 'Agent 1 reported ready',
    target: {
      function_id: 'harness::send',
      event_into: 'event',
      payload: { session_id: 'sess_01', text: 'Agent 1 is ready' },
    },
    conditions: [
      {
        function_id: 'state::equals',
        config: { path: 'status', value: 'ready', strict: true },
      },
    ],
    lifecycle: { once: true, max_fires: 1, expires_in_ms: 7_200_000 },
  },
  wrapHarness({
    once: true,
    observer_id: 'obs_c8d6d6d6d77b429d85a01dd633106138',
  }),
)

/** Settled without an id: the harness answered but no observer was created. */
export const harnessObserveFailed = base(
  'harness-observe-failed',
  'harness::observe',
  {
    function_id: 'state::on-changed',
    filters: { 'input.key': { $eq: 'progress' } },
    label: 'research-progress-watch',
  },
  {
    error: {
      code: 'NOT_OBSERVABLE',
      message: 'state::on-changed is not observable in this scope',
    },
  },
)

export const engineFixtures = [
  engineFunctionsListDone,
  engineFunctionsListRaw,
  engineFunctionsListEmpty,
  engineFunctionInfoAnyValue,
  engineFunctionInfoRich,
  engineFunctionInfoNotFound,
  engineObserversListDone,
  engineObserversListFiltered,
  engineObserverInfoDone,
  engineWorkersListDone,
  engineWorkersListFiltered,
  engineWorkersListEmpty,
  engineWorkerInfoDone,
  engineWorkerInfoInternal,
  engineWorkerInfoNotFound,
  engineWorkerRegisterDone,
  engineWorkerRegisterRunning,
  harnessObserveCron,
  harnessObserveState,
  harnessObserveRunning,
  harnessObserveCallGuarded,
  harnessObserveFailed,
  engineRunning,
  engineFunctionsListGateError,
] as const
