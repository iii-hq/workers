// biome-ignore-all lint/suspicious/noTemplateCurlyInString: fixtures intentionally carry literal `${VAR}` env templates — backticks would change the stored value.
import type {
  ConfigurationSchemaView,
  JsonSchema,
  JsonValue,
} from '@/pages/Configuration/tabs/WorkersTab/api'
import type { FunctionTriggerMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionTriggerMessage>,
): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId,
    input,
    output,
    durationMs: 769,
    createdAt: now,
    ...extra,
  }
}

/* ---------------- worker::list ---------------- */

/** Mirrors the user's screenshot: 7+ workers, mix of running engine builtins
 * (null pid, null version) and a managed iii-directory with a real pid. */
export const workerListDone = base(
  'worker-list',
  'worker::list',
  {},
  wrapHarness({
    workers: [
      { name: 'iii-worker-manager', pid: null, running: true },
      { name: 'iii-pubsub', pid: null, running: true },
      { name: 'iii-observability', pid: null, running: true },
      { name: 'iii-directory', pid: 19052, running: true, version: '0.5.2' },
      { name: 'iii-queue', pid: null, running: true, version: '0.11.6' },
      { name: 'iii-state', pid: null, running: true, version: '0.11.6' },
      { name: 'iii-stream', pid: null, running: true, version: '0.11.6' },
      { name: 'iii-http', pid: null, running: false, version: '0.11.6' },
    ],
  }),
)

export const workerListRunningOnly = base(
  'worker-list-running',
  'worker::list',
  { running_only: true },
  {
    workers: [
      { name: 'iii-directory', pid: 19052, running: true, version: '0.5.2' },
      { name: 'iii-queue', pid: null, running: true, version: '0.11.6' },
    ],
  },
)

export const workerListEmpty = base(
  'worker-list-empty',
  'worker::list',
  { running_only: true },
  { workers: [] },
)

export const workerListRunning = base(
  'worker-list-loading',
  'worker::list',
  {},
  undefined,
  { running: true },
)

/* ---------------- worker::status ---------------- */

/** Healthy managed OCI worker: running, real pid, version, both tails populated. */
export const workerStatusRunning = base(
  'worker-status-running',
  'worker::status',
  { name: 'pdfkit' },
  wrapHarness({
    name: 'pdfkit',
    installed: true,
    worker_type: 'oci',
    running: true,
    pid: 28943,
    version: '1.0.0',
    logs_dir: '/Users/anderson/.iii/logs/pdfkit',
    stderr_tail: [],
    stdout_tail: [
      '[pdfkit] booting render pool',
      '[pdfkit] listening on :4101',
    ],
    hint: 'worker is healthy; trigger it with `pdfkit::render`.',
  }),
)

/** Crashed worker: installed, not running, failure captured in stderr_tail. */
export const workerStatusStopped = base(
  'worker-status-stopped',
  'worker::status',
  { name: 'pdfkit' },
  {
    name: 'pdfkit',
    installed: true,
    worker_type: 'local',
    running: false,
    pid: null,
    version: '1.0.0',
    logs_dir: '/Users/anderson/.iii/logs/pdfkit',
    stderr_tail: [
      'npm ERR! code ELIFECYCLE',
      'npm ERR! pdfkit@1.0.0 start: `node server.js`',
      'npm ERR! Exit status 1',
    ],
    stdout_tail: [],
    hint: 'worker crashed on boot; inspect stderr then `worker::start pdfkit`.',
  },
)

/** Mid-boot worker: installed, not running, both tails empty (still provisioning). */
export const workerStatusProvisioning = base(
  'worker-status-provisioning',
  'worker::status',
  { name: 'pdfkit' },
  {
    name: 'pdfkit',
    installed: true,
    worker_type: 'oci',
    running: false,
    pid: null,
    version: null,
    logs_dir: null,
    stderr_tail: [],
    stdout_tail: [],
    hint: 'worker is provisioning; re-check `worker::status pdfkit` shortly.',
  },
)

/** Unknown worker: not declared in config.yaml. */
export const workerStatusNotInstalled = base(
  'worker-status-not-installed',
  'worker::status',
  { name: 'ghost' },
  {
    name: 'ghost',
    installed: false,
    worker_type: 'not-installed',
    running: false,
    pid: null,
    version: null,
    logs_dir: null,
    stderr_tail: [],
    stdout_tail: [],
    hint: 'not declared in config.yaml; run `worker::add` first.',
  },
)

export const workerStatusRunningLoading = base(
  'worker-status-loading',
  'worker::status',
  { name: 'pdfkit' },
  undefined,
  { running: true },
)

/* ---------------- worker::start ---------------- */

export const workerStartDone = base(
  'worker-start',
  'worker::start',
  { name: 'pdfkit', wait: true },
  wrapHarness({ name: 'pdfkit', pid: 28943, port: 4101 }),
)

export const workerStartNoPid = base(
  'worker-start-no-pid',
  'worker::start',
  { name: 'iii-stream' },
  { name: 'iii-stream', pid: null, port: null },
)

/* ---------------- worker::stop ---------------- */

export const workerStopDone = base(
  'worker-stop',
  'worker::stop',
  { name: 'pdfkit', yes: true },
  wrapHarness({ name: 'pdfkit', stopped: true }),
)

export const workerStopFailed = base(
  'worker-stop-failed',
  'worker::stop',
  { name: 'pdfkit', yes: true },
  { name: 'pdfkit', stopped: false },
)

/* ---------------- worker::add ---------------- */

export const workerAddDone = base(
  'worker-add',
  'worker::add',
  {
    source: { kind: 'registry', name: 'pdfkit', version: '1.0.0' },
    force: false,
    reset_config: false,
    wait: true,
  },
  wrapHarness({
    name: 'pdfkit',
    version: '1.0.0',
    status: 'installed',
    awaited_ready: true,
    config_path: '/Users/anderson/code/demo/iii.config.yaml',
  }),
)

export const workerAddOci = base(
  'worker-add-oci',
  'worker::add',
  {
    source: { kind: 'oci', reference: 'ghcr.io/iii-hq/node:latest' },
    force: true,
    wait: true,
  },
  {
    name: 'node',
    version: null,
    status: 'replaced',
    awaited_ready: true,
    config_path: '/Users/anderson/code/demo/iii.config.yaml',
  },
)

export const workerAddAlreadyCurrent = base(
  'worker-add-current',
  'worker::add',
  {
    source: { kind: 'registry', name: 'pdfkit' },
    wait: true,
  },
  {
    name: 'pdfkit',
    version: '1.0.0',
    status: 'already_current',
    awaited_ready: true,
    config_path: '/Users/anderson/code/demo/iii.config.yaml',
  },
)

/* ---------------- worker::remove ---------------- */

export const workerRemoveDone = base(
  'worker-remove',
  'worker::remove',
  { names: ['pdfkit', 'old-worker'], yes: true },
  wrapHarness({ removed: ['pdfkit', 'old-worker'] }),
)

export const workerRemoveAll = base(
  'worker-remove-all',
  'worker::remove',
  { all: true, yes: true },
  { removed: ['pdfkit', 'iii-stream', 'todo-app'] },
)

export const workerRemoveNothing = base(
  'worker-remove-empty',
  'worker::remove',
  { names: ['gone'], yes: true },
  { removed: [] },
)

/* ---------------- worker::update ---------------- */

export const workerUpdateDone = base(
  'worker-update',
  'worker::update',
  { names: ['pdfkit', 'iii-stream'] },
  wrapHarness({
    updated: [
      { name: 'pdfkit', from_version: '1.0.0', to_version: '1.1.0' },
      { name: 'iii-stream', from_version: '0.11.6', to_version: '0.12.0' },
    ],
  }),
)

export const workerUpdateAlreadyCurrent = base(
  'worker-update-current',
  'worker::update',
  {},
  { updated: [] },
)

/* ---------------- worker::clear ---------------- */

export const workerClearDone = base(
  'worker-clear',
  'worker::clear',
  { all: true, yes: true },
  wrapHarness({ cleared: ['pdfkit', 'iii-stream'] }),
)

/* ---------------- error fixture ---------------- */

export const workerStopGateError = base(
  'worker-stop-gate',
  'worker::stop',
  { name: 'pdfkit' },
  {
    error: {
      kind: 'function_error',
      message: 'trigger_failed: W104: ConsentRequired',
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'permissions',
        function_id: 'worker::stop',
        reason: 'destructive op requires yes=true',
      },
      content: [
        {
          type: 'text',
          text: 'trigger_failed: destructive op requires yes=true',
        },
      ],
    },
  },
)

export const workerFixtures = [
  workerListDone,
  workerListRunningOnly,
  workerListEmpty,
  workerListRunning,
  workerStatusRunning,
  workerStatusStopped,
  workerStatusProvisioning,
  workerStatusNotInstalled,
  workerStatusRunningLoading,
  workerStartDone,
  workerStartNoPid,
  workerStopDone,
  workerStopFailed,
  workerAddDone,
  workerAddOci,
  workerAddAlreadyCurrent,
  workerRemoveDone,
  workerRemoveAll,
  workerRemoveNothing,
  workerUpdateDone,
  workerUpdateAlreadyCurrent,
  workerClearDone,
  workerStopGateError,
] as const

/* ------------------------------------------------------------------ */
/*  Worker-configuration spec-sheet fixtures                           */
/* ------------------------------------------------------------------ */

/**
 * Mock data for the worker-configuration stories. Lives under `src/stories`
 * so it's only pulled into the Storybook build, never the production app
 * bundle. Reuses the real `JsonSchema` / `JsonValue` / `ConfigurationSchemaView`
 * types so the fixtures stay structurally identical to what the
 * `configuration` worker actually returns.
 *
 * Two consumers:
 *   - `SchemaForm.stories.tsx` maps over `SCHEMA_EXAMPLES` to render one live
 *     `SchemaForm` per field-type variation.
 *   - `WorkersTab.stories.tsx` drives a master-detail harness from
 *     `WORKER_CONFIG_FIXTURES` + `mockValidate`.
 */

function isObject(value: JsonValue): value is { [key: string]: JsonValue } {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/* ------------------------------------------------------------------ */
/*  Schema-form field variations                                       */
/* ------------------------------------------------------------------ */

export interface SchemaExample {
  id: string
  label: string
  schema: JsonSchema
  /** Seed value for the live form. `null` shows the unset/placeholder state. */
  initial: JsonValue
  /** Span two columns in the gallery grid (for wide controls). */
  wide?: boolean
  /** Pre-seeded server error map (JSON Pointer -> message) for the error demo. */
  errors?: ReadonlyMap<string, string>
}

export const SCHEMA_EXAMPLES: SchemaExample[] = [
  {
    id: 'string',
    label: 'string',
    schema: {
      type: 'string',
      title: 'connection name',
      description: 'free-form text. the default shows as placeholder.',
      default: 'primary',
    },
    initial: 'primary',
  },
  {
    id: 'string-env',
    label: 'string + env template',
    wide: true,
    schema: {
      type: 'string',
      title: 'database url',
      description: 'supports ${VAR} and ${VAR:default} pills inline.',
    },
    initial: 'postgres://${PGUSER:app}:${PGPASSWORD}@db.internal:5432/app',
  },
  {
    id: 'string-password',
    label: 'string · password',
    schema: { type: 'string', format: 'password', title: 'api secret' },
    initial: 's3cr3t-value',
  },
  {
    id: 'string-date',
    label: 'string · date',
    schema: { type: 'string', format: 'date', title: 'expires on' },
    initial: '2026-01-01',
  },
  {
    id: 'integer',
    label: 'integer · min/max',
    schema: {
      type: 'integer',
      title: 'max connections',
      minimum: 1,
      maximum: 100,
      default: 10,
    },
    initial: 10,
  },
  {
    id: 'number',
    label: 'number',
    schema: {
      type: 'number',
      title: 'sample rate',
      minimum: 0,
      maximum: 1,
      default: 0.25,
      description: 'fractional values; step is "any".',
    },
    initial: 0.25,
  },
  {
    id: 'boolean',
    label: 'boolean',
    schema: {
      type: 'boolean',
      title: 'tls enabled',
      description: 'segmented true/false toggle.',
    },
    initial: true,
  },
  {
    id: 'enum',
    label: 'enum · enumNames',
    schema: {
      type: 'string',
      enum: ['debug', 'info', 'warn', 'error'],
      enumNames: ['Debug', 'Info', 'Warning', 'Error'],
      title: 'log level',
      default: 'info',
    },
    initial: 'info',
  },
  {
    id: 'enum-optional',
    label: 'enum · optional (unset)',
    schema: {
      type: 'string',
      enum: ['json', 'text', 'pretty'],
      title: 'output format',
      description: 'optional enum — pick "unset" to clear it.',
    },
    // null renders the placeholder; the dropdown offers an "unset" option.
    initial: null,
  },
  {
    id: 'enum-numeric',
    label: 'enum · numeric',
    schema: {
      enum: [1, 2, 3],
      enumNames: ['one', 'two', 'three'],
      title: 'replicas',
    },
    initial: 2,
  },
  {
    id: 'oneof-collapsed',
    label: 'oneOf · collapsed string-enum',
    schema: {
      title: 'load-balancing strategy',
      oneOf: [
        { enum: ['round_robin'], title: 'round robin' },
        { enum: ['least_conn'], title: 'least connections' },
        { enum: ['ip_hash'], title: 'ip hash' },
      ],
    },
    initial: 'round_robin',
  },
  {
    id: 'oneof-hetero',
    label: 'oneOf · heterogeneous',
    wide: true,
    schema: {
      title: 'request timeout',
      description: 'pick a shape, then edit the value.',
      oneOf: [
        { type: 'integer', title: 'seconds' },
        { type: 'string', title: 'duration string' },
        {
          type: 'object',
          title: 'detailed',
          properties: {
            value: { type: 'integer', title: 'value' },
            unit: {
              type: 'string',
              enum: ['s', 'ms'],
              title: 'unit',
              default: 's',
            },
          },
          required: ['value'],
        },
      ],
    },
    initial: 30,
  },
  {
    id: 'nullable',
    label: 'nullable · set/unset',
    schema: {
      type: ['string', 'null'],
      title: 'fallback host',
      description: 'flip to "unset" to force null.',
    },
    initial: 'cache.internal',
  },
  {
    id: 'array-primitives',
    label: 'array · primitives',
    schema: {
      type: 'array',
      title: 'allowed origins',
      items: { type: 'string' },
      description: 'numbered rows; add / delete inline.',
    },
    initial: ['https://app.example.com', 'https://admin.example.com'],
  },
  {
    id: 'array-objects',
    label: 'array · objects · minItems',
    wide: true,
    schema: {
      type: 'array',
      title: 'routes',
      minItems: 1,
      items: {
        type: 'object',
        title: 'route',
        properties: {
          path: { type: 'string', title: 'path' },
          upstream: { type: 'string', title: 'upstream' },
          weight: { type: 'integer', title: 'weight', default: 1 },
        },
        required: ['path', 'upstream'],
      },
    },
    initial: [{ path: '/api', upstream: 'http://api:8080', weight: 1 }],
  },
  {
    id: 'object',
    label: 'object · nested + required',
    wide: true,
    schema: {
      type: 'object',
      title: 'connection pool',
      description: 'object sections nest with a left rule + header.',
      properties: {
        min: { type: 'integer', title: 'min', default: 0, minimum: 0 },
        max: { type: 'integer', title: 'max', default: 10, minimum: 1 },
        idle: {
          type: 'object',
          title: 'idle',
          properties: {
            timeout_ms: { type: 'integer', title: 'timeout (ms)' },
            enabled: { type: 'boolean', title: 'enabled' },
          },
        },
      },
      required: ['max'],
    },
    initial: { min: 0, max: 20, idle: { timeout_ms: 30000, enabled: true } },
  },
  {
    id: 'dictionary',
    label: 'dictionary · additionalProperties',
    wide: true,
    schema: {
      type: 'object',
      title: 'environment',
      description: 'env-style key/value rows (drag the handle to reorder).',
      additionalProperties: { type: 'string' },
    },
    initial: { LOG_FORMAT: 'json', REGION: 'us-east-1' },
  },
  {
    id: 'ref',
    label: '$ref · allOf wrapper',
    wide: true,
    schema: {
      title: 'tls config',
      type: 'object',
      definitions: {
        Mode: {
          type: 'string',
          enum: ['off', 'optional', 'required'],
          enumNames: ['off', 'optional', 'required'],
        },
      },
      properties: {
        // allOf wrapper carries sibling default/description next to a $ref —
        // exercises ref-resolver pattern 2.
        mode: {
          allOf: [{ $ref: '#/definitions/Mode' }],
          default: 'optional',
          description: 'resolved through #/definitions/Mode.',
          title: 'mode',
        },
        cert_path: { type: 'string', title: 'cert path' },
      },
    },
    initial: { mode: 'optional', cert_path: '/etc/tls/cert.pem' },
  },
  {
    id: 'unsupported',
    label: 'unsupported · read-only',
    schema: {
      title: 'opaque blob',
      type: 'object',
    },
    initial: { raw: { nested: [1, 2, 3], note: 'no properties → read-only' } },
  },
  {
    id: 'errors',
    label: 'validation errors',
    wide: true,
    schema: {
      type: 'object',
      title: 'pool (with server error)',
      description: 'a JSON-Pointer error map surfaces inline on the leaf.',
      properties: {
        min: { type: 'integer', title: 'min', default: 0 },
        max: { type: 'integer', title: 'max', default: 10 },
      },
      required: ['max'],
    },
    initial: { min: 10, max: 5 },
    errors: new Map([['/max', 'max must be greater than or equal to min']]),
  },
  {
    // Mirrors session-manager's registered schema: an adjacently tagged
    // `StorageAdapter` enum (`name` + `config`) that schemars emits as a
    // `oneOf`. The console renders a variant picker plus only the selected
    // adapter's `config` fields; switching resets `config` to that adapter's
    // defaults.
    id: 'adapter-oneof',
    label: 'oneOf · adapter (name + config)',
    wide: true,
    schema: {
      type: 'object',
      title: 'session-manager',
      description:
        'adjacently tagged adapter (name + config). switch fs ↔ bridge to see only its fields.',
      definitions: {
        FsBackendConfig: {
          type: 'object',
          additionalProperties: false,
          description: 'settings for the fs adapter.',
          properties: {
            data_dir: {
              type: 'string',
              default: '~/.iii/data/session-manager',
              title: 'data dir',
              description: 'one <session_id>.jsonl per session.',
            },
          },
        },
        BridgeBackendConfig: {
          type: 'object',
          additionalProperties: false,
          description: 'settings for the bridge adapter.',
          properties: {
            url: {
              type: 'string',
              title: 'url',
              description: 'websocket url of the main instance.',
            },
            timeout_ms: {
              type: 'integer',
              default: 5000,
              title: 'timeout ms',
              description: 'per store/publish call timeout (ms).',
            },
          },
          required: ['url'],
        },
        StorageAdapter: {
          description: 'storage adapter selection.',
          oneOf: [
            {
              type: 'object',
              description: 'one append-only JSONL file per session.',
              properties: {
                name: { type: 'string', enum: ['fs'] },
                config: { $ref: '#/definitions/FsBackendConfig' },
              },
              required: ['config', 'name'],
            },
            {
              type: 'object',
              description: 'defer storage to a main session-manager.',
              properties: {
                name: { type: 'string', enum: ['bridge'] },
                config: { $ref: '#/definitions/BridgeBackendConfig' },
              },
              required: ['config', 'name'],
            },
          ],
        },
      },
      properties: {
        adapter: {
          allOf: [{ $ref: '#/definitions/StorageAdapter' }],
          default: {
            name: 'fs',
            config: { data_dir: '~/.iii/data/session-manager' },
          },
          title: 'adapter',
          description: 'storage adapter (fs | bridge) plus its settings.',
        },
        default_list_limit: {
          type: 'integer',
          default: 50,
          title: 'default list limit',
        },
        max_list_limit: {
          type: 'integer',
          default: 500,
          title: 'max list limit',
        },
      },
    },
    initial: {
      adapter: {
        name: 'fs',
        config: { data_dir: '~/.iii/data/session-manager' },
      },
      default_list_limit: 50,
      max_list_limit: 500,
    },
  },
  {
    // Mirrors the shell worker's registered schema (a subset), with the
    // real doc-comment descriptions from `shell/src/config.rs` and
    // `shell/src/code/config.rs`. This is the stress case for description
    // rendering: multi-line technical prose between nearly every control.
    id: 'long-descriptions',
    label: 'long descriptions · shell worker',
    wide: true,
    schema: {
      type: 'object',
      title: 'shell',
      properties: {
        allowed_env: {
          type: 'array',
          items: { type: 'string' },
          default: ['PATH', 'HOME', 'LANG', 'LC_ALL', 'TERM'],
        },
        allowlist: {
          type: 'array',
          items: { type: 'string' },
          default: [],
        },
        denylist_patterns: {
          type: 'array',
          items: { type: 'string' },
          default: [],
          description:
            'ADVISORY ONLY. Regular expressions matched against the whole command line (`argv.join(" ")`). A match rejects the exec, but this is a best-effort guardrail, NOT the security boundary — the sandbox backend is. Do not rely on it to contain untrusted input: regexes over a joined argv are trivially evadable (quoting, env indirection, alternate paths).',
        },
        fs: {
          type: 'object',
          properties: {
            host_roots: {
              type: 'array',
              items: { type: 'string' },
              default: [],
              description:
                'Allowed jail roots. The FIRST entry is the PRIMARY root: relative wire paths and a relative per-call `cwd`/`base_dir` resolve against it. Absolute paths are accepted when they canonicalize inside ANY listed root. Empty (and `host_root` unset) means unjailed — refused at boot unless `allow_unjailed` is true.',
            },
            allow_unjailed: {
              type: 'boolean',
              default: false,
              description:
                'Operator opt-in for running with `host_root: null`. When false (the default) the worker refuses to start unjailed — the entire host filesystem is reachable through `shell::fs::*` aside from the small denylist, which is rarely what the operator actually wants. Setting this to true is equivalent to acknowledging that fact (test harnesses, sandbox-only deployments).',
            },
          },
        },
        code: {
          type: 'object',
          description:
            "The folded `code` surface (`coder::*`) config: glob protection (`non_accessible_globs`), noise excludes (`default_exclude_globs`), and per-file/response budgets. The code resolver's ROOTS are NOT taken from here — it uses `fs.host_roots` so there is a single jail config; any `base_path`/`base_paths` set under `code` is ignored.",
          properties: {
            base_path: {
              type: ['string', 'null'],
              default: null,
              description:
                'Legacy single-root form. Honored as a one-entry `base_paths` list. Setting BOTH `base_path` and `base_paths` is a startup error (checked at `PathResolver` construction).',
            },
            base_paths: {
              type: 'array',
              items: { type: 'string' },
              default: [],
              description:
                'Root directories the worker operates inside. The FIRST entry is the primary root: relative wire paths resolve against it. Absolute wire paths are accepted when they canonicalize inside ANY listed root. When neither this nor `base_path` is set, the effective default is `["./", "/tmp"]` (resolved at `PathResolver` construction).',
            },
            non_accessible_globs: {
              type: 'array',
              items: { type: 'string' },
              default: [],
              description:
                'Glob patterns matched against the path *relative to its containing root*. Matching files can be listed but not read/written/deleted/created.',
            },
            default_exclude_globs: {
              type: 'array',
              items: { type: 'string' },
              default: [],
              description:
                'Noise-exclusion globs (matched against the path relative to its containing root, same convention as `non_accessible_globs`). `coder::tree` and `coder::search` suppress descent into matching directories and omit matching files; callers opt out per call with `use_default_excludes: false`. Unlike `non_accessible_globs` this only HIDES results — it grants no access protection.',
            },
            max_output_bytes: {
              type: 'integer',
              default: 131072,
              description:
                "Context budget for single-path FULL reads in `coder::read-file` (no `line_from`/`line_to`, `stat: false`), measured in BYTES OF RETURNED CONTENT after UTF-8 sanitization (numbered prefixes included) — the same accounting unit as `batch_read_budget_bytes`. A full read whose converted content would exceed this budget fails with a C213 that reports the file's size and line count and names the recovery paths (window, stat probe, or per-call `max_output_bytes` raise, clamped to `max_read_bytes`). Windowed reads and batch mode are NOT governed by this key.",
            },
          },
        },
      },
    },
    initial: {
      allowed_env: ['PATH', 'HOME', 'LANG', 'LC_ALL', 'TERM'],
      allowlist: [],
      denylist_patterns: ['rm\\s+-rf\\s+/', 'mkfs', 'shutdown'],
      fs: { host_roots: ['/tmp'], allow_unjailed: false },
      code: {
        base_path: null,
        base_paths: [],
        non_accessible_globs: ['**/.env', '**/*.pem', '**/secrets/**'],
        default_exclude_globs: ['**/.git/**', '**/node_modules/**'],
        max_output_bytes: 131072,
      },
    },
  },
]

/* ------------------------------------------------------------------ */
/*  Worker-config master-detail fixtures                               */
/* ------------------------------------------------------------------ */

export interface WorkerConfigFixture {
  view: ConfigurationSchemaView
  /** Raw value (env templates preserved), as `configuration::get` returns. */
  value: JsonValue
}

export const WORKER_CONFIG_FIXTURES: WorkerConfigFixture[] = [
  {
    view: {
      id: 'database',
      name: 'database',
      description: 'postgresql / mysql / sqlite connection pool.',
      schema: {
        type: 'object',
        properties: {
          url: {
            type: 'string',
            title: 'connection url',
            description: 'supports ${VAR:default} templates.',
          },
          pool: {
            type: 'object',
            title: 'pool',
            properties: {
              min: { type: 'integer', title: 'min', default: 0, minimum: 0 },
              max: { type: 'integer', title: 'max', default: 10, minimum: 1 },
            },
            required: ['max'],
          },
          tls: {
            type: 'string',
            enum: ['off', 'optional', 'required'],
            enumNames: ['off', 'optional', 'required'],
            default: 'optional',
            title: 'tls mode',
          },
          read_replicas: {
            type: 'array',
            title: 'read replicas',
            items: { type: 'string' },
          },
        },
        required: ['url'],
      },
    },
    value: {
      url: 'postgres://${PGUSER:app}:${PGPASSWORD}@db.internal:5432/app',
      pool: { min: 2, max: 20 },
      tls: 'required',
      read_replicas: ['replica-1.internal', 'replica-2.internal'],
    },
  },
  {
    view: {
      id: 'http-gateway',
      name: 'http gateway',
      description: 'reverse-proxy routes, cors, and request timeout.',
      schema: {
        type: 'object',
        properties: {
          listen_addr: {
            type: 'string',
            title: 'listen address',
            default: '0.0.0.0:8080',
          },
          routes: {
            type: 'array',
            title: 'routes',
            minItems: 1,
            items: {
              type: 'object',
              title: 'route',
              properties: {
                path: { type: 'string', title: 'path' },
                upstream: { type: 'string', title: 'upstream' },
                strip_prefix: {
                  type: 'boolean',
                  title: 'strip prefix',
                  default: false,
                },
              },
              required: ['path', 'upstream'],
            },
          },
          cors: {
            type: 'object',
            title: 'cors',
            properties: {
              enabled: { type: 'boolean', title: 'enabled', default: false },
              origins: {
                type: 'array',
                title: 'origins',
                items: { type: 'string' },
              },
            },
          },
          timeout: {
            title: 'request timeout',
            oneOf: [
              { type: 'integer', title: 'seconds' },
              { type: 'string', title: 'duration string' },
              { type: 'boolean', title: 'disabled' },
            ],
          },
        },
        required: ['listen_addr'],
      },
    },
    value: {
      listen_addr: '0.0.0.0:8080',
      routes: [
        { path: '/api', upstream: 'http://api:9000', strip_prefix: true },
        { path: '/auth', upstream: 'http://auth:9100', strip_prefix: false },
      ],
      cors: { enabled: true, origins: ['https://app.example.com'] },
      timeout: 30,
    },
  },
  {
    view: {
      id: 'feature-flags',
      name: 'feature flags',
      description: 'per-flag rollout config (additionalProperties).',
      schema: {
        type: 'object',
        title: 'feature flags',
        additionalProperties: {
          type: 'object',
          title: 'flag',
          properties: {
            enabled: { type: 'boolean', title: 'enabled', default: false },
            rollout: {
              type: 'integer',
              title: 'rollout %',
              default: 0,
              minimum: 0,
              maximum: 100,
            },
          },
          required: ['enabled'],
        },
      },
    },
    value: {
      new_dashboard: { enabled: true, rollout: 100 },
      beta_search: { enabled: false, rollout: 25 },
    },
  },
  {
    view: {
      id: 'telemetry',
      name: 'telemetry',
      description: 'log level, sampling, exporter, and otlp headers.',
      schema: {
        type: 'object',
        properties: {
          level: {
            type: 'string',
            enum: ['debug', 'info', 'warn', 'error'],
            enumNames: ['debug', 'info', 'warn', 'error'],
            default: 'info',
            title: 'log level',
          },
          sample_rate: {
            type: 'number',
            title: 'sample rate',
            minimum: 0,
            maximum: 1,
            default: 0.1,
            description: 'must be between 0 and 1 — try saving > 1.',
          },
          exporter: {
            title: 'exporter',
            oneOf: [
              { enum: ['otlp'], title: 'otlp' },
              { enum: ['jaeger'], title: 'jaeger' },
              { enum: ['none'], title: 'none' },
            ],
          },
          endpoint: {
            type: ['string', 'null'],
            title: 'endpoint',
            description: 'unset to disable export.',
          },
          headers: {
            type: 'object',
            title: 'headers',
            additionalProperties: { type: 'string' },
          },
        },
      },
    },
    value: {
      level: 'info',
      sample_rate: 0.1,
      exporter: 'otlp',
      endpoint: 'https://otel.internal:4317',
      headers: { 'x-api-key': '${OTEL_KEY}' },
    },
  },
]

/**
 * Stand-in for the server's `configuration::set` validation. Returns a
 * `{ pointer, message }` to reject (mapped onto the leaf field), or `null`
 * to accept. Deliberately small: just enough to demo the inline-error path
 * in the harness without a real backend.
 */
export function mockValidate(
  id: string,
  value: JsonValue,
): { pointer: string; message: string } | null {
  if (!isObject(value)) return null

  if (id === 'database') {
    const url = value.url
    if (url === undefined || url === null || url === '') {
      return { pointer: '/url', message: 'url is required' }
    }
  }

  if (id === 'telemetry') {
    const rate = value.sample_rate
    if (typeof rate === 'number' && (rate < 0 || rate > 1)) {
      return {
        pointer: '/sample_rate',
        message: 'sample_rate must be between 0 and 1',
      }
    }
  }

  return null
}
