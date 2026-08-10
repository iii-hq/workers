import type { WorkerRow } from '../types'

export const WORKERS_FIXTURE_ROWS: WorkerRow[] = [
  {
    id: '7fa8e8a4-1c3d-44b2-9a5f-1234567890ab',
    name: 'harness',
    runtime: 'rust',
    ipAddress: '127.0.0.1',
    version: '0.4.7',
    pid: 48291,
    tag: 'agent',
    managementKind: 'config',
    status: 'connected',
    configurationId: 'harness',
    stopEnabled: false,
    stopDisabledReason:
      'workers declared in config.yaml are managed by the engine',
  },
  {
    id: '00000000-0000-0000-0000-000000000aaa',
    name: 'iii-directory',
    runtime: 'rust',
    ipAddress: '127.0.0.1',
    version: '0.1.5',
    pid: 48302,
    tag: 'platform',
    managementKind: 'supervisor',
    status: 'connected',
    configurationId: null,
    stopEnabled: true,
    stopDisabledReason: null,
  },
  {
    id: '11111111-2222-3333-4444-555555555555',
    name: 'todo-app',
    runtime: 'node',
    ipAddress: '192.168.1.42',
    version: '0.4.7',
    pid: 51003,
    tag: 'dev',
    managementKind: 'standalone',
    status: 'connected',
    configurationId: null,
    stopEnabled: false,
    stopDisabledReason:
      'standalone workers must be stopped from the process that started them',
  },
  {
    id: '22222222-3333-4444-5555-666666666666',
    name: 'iii-engine-functions',
    runtime: 'rust',
    ipAddress: null,
    version: '0.19.4',
    pid: 1,
    tag: null,
    managementKind: 'internal',
    status: 'connected',
    configurationId: null,
    stopEnabled: false,
    stopDisabledReason:
      'internal engine workers cannot be stopped from the console',
  },
  {
    id: '33333333-4444-5555-6666-777777777777',
    name: 'pdfkit',
    runtime: 'rust',
    ipAddress: null,
    version: '0.2.0',
    pid: null,
    tag: 'platform',
    managementKind: 'supervisor',
    status: 'stopped',
    configurationId: null,
    stopEnabled: false,
    stopDisabledReason: 'worker is not running',
  },
]

export const WORKERS_FIXTURE_EMPTY: WorkerRow[] = []

export const WORKERS_FIXTURE_LOADING: null = null

/**
 * One worker's registered surface, as `engine::workers::info` answers it.
 * Backs the expanded-row story so the drill-down renders without an engine.
 */
export const WORKER_SURFACE_FIXTURE = {
  worker: {
    id: '7fa8e8a4-1c3d-44b2-9a5f-1234567890ab',
    name: 'harness',
    status: 'connected',
    function_count: 3,
    connected_at_ms: 1_785_930_000_000,
    active_invocations: 1,
    internal: false,
  },
  functions: [
    {
      function_id: 'harness::spawn',
      worker_name: 'harness',
      description: 'Spawn a sub-agent for a task and return its session id.',
    },
    {
      function_id: 'harness::triggers::list',
      worker_name: 'harness',
      description: 'List the triggers this session has registered.',
    },
    {
      function_id: 'harness::sweep-pending',
      worker_name: 'harness',
      description: null,
    },
  ],
  trigger_types: [
    {
      id: 'harness::turn-completed',
      worker_name: 'harness',
      description: 'A turn finished, successfully or not.',
    },
    {
      id: 'harness::hook::pre-generate',
      worker_name: 'harness',
      description: 'Runs before every model generation.',
    },
  ],
  registered_triggers: [
    {
      id: 'f8aa4183-1549-4dd2-a3e9-83719f5ee2cc',
      trigger_type: 'cron',
      function_id: 'harness::sweep-pending',
      worker_name: 'harness',
      config_summary: '{"expression":"0 0 0 * * *"}',
    },
    {
      id: '05238e9a-1a9f-4966-9f8b-3949fcf5dcc6',
      trigger_type: 'harness::hook::pre-generate',
      function_id: 'fp::inject-guidance',
      worker_name: 'fp',
      config_summary: '{"on_error":"fail_open"}',
    },
  ],
}
