const parameters = new URLSearchParams(location.search)
const names = [
  'ade',
  'agent',
  'approval-gate',
  'cron',
  'database',
  'llm-router',
  'provider-openai',
  'session-manager',
  'shell',
  'state',
  'storage',
  'terminal',
]
if (parameters.has('specialWorkers'))
  names.push('constructor', 'toString', '__proto__')
const file = '/home/developer/iii/worker-compose.yaml'
const packageWorker = parameters.get('package')
const configurationSchemas = [
  {
    id: 'actual-ade-config',
    name: 'Registered ADE configuration',
    description: 'The schema advertised by this worker.',
    schema: {
      type: 'object',
      properties: {
        port: {
          type: 'integer',
          default: 3113,
          description: 'Port declared by the registered worker schema.',
        },
        api_token: {
          type: 'string',
          description: 'Authentication credential; no default is declared.',
        },
      },
    },
  },
  {
    id: 'actual-agent-config',
    name: 'Registered agent configuration',
    description: 'The agent schema advertised by this worker.',
    schema: {
      type: 'object',
      properties: {
        poll_interval: {
          type: 'string',
          default: '30s',
          description:
            'Polling interval declared by the registered worker schema.',
        },
      },
    },
  },
]
if (parameters.get('configurationReference') === 'constraints') {
  // Representative registered schemas; these remain memory-only fixtures.
  configurationSchemas[0].schema = {
    type: 'object',
    properties: {
      http_port: {
        type: 'integer',
        minimum: 0,
        maximum: 65535,
        default: 3113,
        description: 'TCP port for the ADE UI, injected assets, and /ws proxy.',
      },
      http_host: {
        type: 'string',
        minLength: 1,
        default: '0.0.0.0',
        description: 'Interface the ADE listens on.',
      },
      data_dir: {
        type: 'string',
        minLength: 1,
        description: 'Directory for ephemeral per-instance state.',
      },
      api_token: {
        type: 'string',
        description: 'Authentication credential; no default is declared.',
      },
    },
  }
  configurationSchemas.push({
    id: 'actual-harness-config',
    name: 'Registered harness configuration',
    description: 'The call-reconciliation schema registered by the harness.',
    schema: {
      type: 'object',
      properties: {
        call_reconciliation: {
          allOf: [{ $ref: '#/definitions/CallReconciliation' }],
          default: 'judge',
          description:
            'Repair malformed function-call arguments before dispatch.',
        },
      },
      definitions: {
        CallReconciliation: {
          type: 'string',
          enum: ['off', 'coerce', 'judge'],
        },
      },
    },
  })
}
const entries = Object.fromEntries(
  names.map((name) => [
    name,
    {
      name,
      worker: `path:///home/developer/iii/${name}/iii.toml`,
      version: null,
      start_after: name === 'ade' ? ['state', 'storage'] : [],
      env_file: ['.env'],
      environment: [
        { key: 'III_PORT', value: '3113', secret: false },
        { key: 'LOG_LEVEL', value: 'info', secret: false },
        { key: 'API_TOKEN', value: null, secret: true },
      ],
      run: `./target/debug/${name}`,
      config_override: 'ui:\n  theme: system\n  compact: false',
    },
  ]),
)
if (parameters.has('cycleWorkers')) {
  entries.ade.start_after = []
  entries.state.start_after = ['ade']
  entries.storage.start_after = ['state']
}

export const harness = {
  calls: [],
  dirty: false,
  saveMode: 'success',
  saveDelay: 0,
  loadFailures: Number(parameters.get('loadFailures') ?? 0),
  snapshotMode: 'success',
  restartMode: 'success',
  readDelay: 0,
  savedCount: 0,
  readsCompleted: 0,
  configurationReferenceMode:
    parameters.get('configurationReference') ?? 'none',
  configurationReferenceFailures:
    parameters.get('configurationReference') === 'error' ? 1 : 0,
  externalEdit(name, patch) {
    Object.assign(entries[name], structuredClone(patch))
  },
}

window.__WORKERS_HARNESS__ = harness
const operations = new Map()
const containers = names.map((container, index) => ({
  container,
  state:
    parameters.has('stoppedWorkers') && ['state', 'storage'].includes(container)
      ? 'stopped'
      : 'ready',
  owned: true,
  pid:
    parameters.has('stoppedWorkers') && ['state', 'storage'].includes(container)
      ? null
      : 1200 + index,
}))

async function trigger(functionId, payload = {}, options) {
  harness.calls.push({
    functionId,
    payload: structuredClone(payload),
    ...(options?.namespace ? { options: structuredClone(options) } : {}),
  })
  switch (functionId) {
    case 'compose::status':
      if (harness.snapshotMode === 'error')
        throw new Error('Mock runtime state is unavailable')
      return {
        namespace: 'iii-local',
        file,
        state_dir: '/tmp/iii-compose',
        daemon_pid: 1010,
        containers,
      }
    case 'compose::list':
      return { projects: [{ namespace: 'iii-local', file, containers }] }
    case 'console::compose::project':
      return {
        file,
        namespace: 'iii-local',
        engine_url: 'ws://mock.invalid',
        startup_timeout: '30s',
        stop_timeout: '5s',
        containers: names.map((name) => ({
          name,
          source: name === packageWorker ? 'package' : 'path',
          ref:
            name === packageWorker
              ? name
              : `/home/developer/iii/${name}/iii.toml`,
          version: name === packageWorker ? '1.2.0' : null,
          start_after: entries[name].start_after,
          environment: entries[name].environment.map((row) => row.key),
          run: entries[name].run,
        })),
      }
    case 'engine::workers::list':
      if (harness.snapshotMode === 'error')
        throw new Error('Mock runtime state is unavailable')
      return {
        workers:
          harness.configurationReferenceMode === 'none'
            ? []
            : ['ade', 'agent'].map((name) => ({
                id: `fixture-${name}`,
                name,
                namespace: 'workers-test',
                status: 'connected',
                function_count: 1,
                connected_at_ms: 1000,
                active_invocations: 0,
              })),
      }
    case 'engine::workers::info': {
      const name = String(payload.name)
      return {
        worker: {
          id: `fixture-${name}`,
          name,
          namespace: 'workers-test',
          status: 'connected',
          function_count: 1,
          connected_at_ms: 1000,
          active_invocations: 0,
          internal: false,
        },
        functions: ['none', 'catalog'].includes(
          harness.configurationReferenceMode,
        )
          ? []
          : [
              {
                function_id: `${name === 'ade' ? 'console' : name}::configuration-id`,
                worker_name: name,
                namespace: 'workers-test',
              },
            ],
        trigger_types: [],
        registered_triggers: [],
      }
    }
    case 'console::configuration-id':
      return { id: 'actual-ade-config' }
    case 'agent::configuration-id':
      return { id: 'actual-agent-config' }
    case 'configuration::list':
      return { configurations: structuredClone(configurationSchemas) }
    case 'configuration::schema': {
      if (harness.configurationReferenceFailures > 0) {
        harness.configurationReferenceFailures -= 1
        throw new Error('Mock registered schema is temporarily unavailable')
      }
      return structuredClone(
        configurationSchemas.find((entry) => entry.id === payload.id) ?? null,
      )
    }
    case 'engine::functions::list':
      return { functions: [] }
    case 'console::compose::container': {
      if (harness.readDelay)
        await new Promise((resolve) => setTimeout(resolve, harness.readDelay))
      if (harness.loadFailures > 0) {
        harness.loadFailures -= 1
        throw new Error('Mock compose declaration is temporarily unavailable')
      }
      if (harness.saveMode === 'reload-error' && harness.savedCount > 0)
        throw new Error('Mock saved declaration cannot be read')
      const entry = structuredClone(entries[String(payload.container)])
      entry.environment = entry.environment.map((row) =>
        row.secret ? { ...row, value: null } : row,
      )
      harness.readsCompleted += 1
      return entry
    }
    case 'compose::logs':
      return {
        containers: [
          {
            container: String(payload.container),
            entries:
              parameters.get('logCase') === 'structured'
                ? [
                    {
                      stream: 'stdout',
                      message:
                        '2026-10-09T12:34:56.123456Z TRACE ade::rpc: Routine poll began.',
                    },
                    {
                      stream: 'stdout',
                      message:
                        '2026-10-09T12:34:57.234567Z DEBUG ade::rpc: Poll details recorded.',
                    },
                    {
                      stream: 'stdout',
                      message:
                        '2026-10-09T12:34:58.345678Z INFO ade::rpc: REQUEST accepted.',
                    },
                    {
                      stream: 'stderr',
                      message:
                        '2026-10-09T12:34:59.456789Z WARN ade::rpc: REQUEST retry scheduled.',
                    },
                    {
                      stream: 'stderr',
                      message:
                        '2026-10-09T12:35:00.567890Z ERROR ade::rpc: REQUEST failed after retries.',
                    },
                  ]
                : parameters.get('logCase') === 'copy'
                  ? [
                      {
                        stream: 'stdout',
                        message: 'Worker is ready and listening for requests.',
                      },
                      {
                        stream: 'stderr',
                        message: 'A background maintenance cycle completed.',
                      },
                      {
                        stream: 'stdout',
                        message: 'A second request completed successfully.',
                      },
                    ]
                  : [
                      {
                        stream: 'stdout',
                        message: 'Worker is ready and listening for requests.',
                      },
                    ],
            cursor: null,
            truncated: false,
          },
        ],
      }
    case 'console::compose::versions':
      return {
        container: payload.container,
        reference: payload.container,
        declared: null,
        versions: [],
      }
    case 'console::compose::edit': {
      if (harness.saveDelay)
        await new Promise((resolve) => setTimeout(resolve, harness.saveDelay))
      if (harness.saveMode === 'error')
        throw new Error('Mock compose cannot write worker-compose.yaml')
      const name = String(payload.container)
      const entry = entries[name]
      const patch = payload
      if (patch.run !== undefined) entry.run = patch.run
      if (patch.start_after !== undefined) entry.start_after = patch.start_after
      if (patch.config_override !== undefined)
        entry.config_override = patch.config_override
      if (patch.environment) {
        entry.environment = entry.environment.filter(
          (row) => !patch.environment?.unset.includes(row.key),
        )
        for (const [key, value] of Object.entries(patch.environment.set)) {
          const existing = entry.environment.find((row) => row.key === key)
          if (existing) existing.value = value
          else entry.environment.push({ key, value, secret: false })
        }
      }
      harness.savedCount += 1
      const operationId = String(payload.operation_id)
      operations.set(operationId, {
        operation_id: operationId,
        status: harness.saveMode === 'restart-error' ? 'failed' : 'succeeded',
        phase: harness.saveMode === 'restart-error' ? 'restart' : 'done',
        completed: 1,
        total: 1,
        last_event:
          harness.saveMode === 'restart-error'
            ? {
                phase: 'restart',
                detail: 'Mock restart failed after configuration was written',
              }
            : null,
      })
      return { operation_id: operationId, requested: 1, status: 'accepted' }
    }
    case 'compose::operation':
      return operations.get(String(payload.operation_id))
    case 'compose::restart':
      if (harness.restartMode === 'error')
        return {
          status: 'failed',
          error: {
            code: 'RESTART_FAILED',
            message: 'Mock package worker restart failed',
          },
        }
      return { status: 'ok', changed: true }
    case 'compose::up':
    case 'compose::down':
      return { status: 'ok', changed: true }
    default:
      throw new Error(`Unexpected mocked RPC: ${functionId}`)
  }
}

const iii = {
  browserId: 'workers-harness',
  trigger,
  on: () => () => {},
  registerTrigger: () => () => {},
  addConnectionStateListener: () => () => {},
}

export const getIiiClient = async () => iii
