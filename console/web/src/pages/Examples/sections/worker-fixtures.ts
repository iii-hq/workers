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
